//! Dylint lint: `MODEL_HANDLE_IN_SUBSCRIPTION`
//!
//! Detects `ModelHandle` or `ViewHandle` values captured by a closure passed
//! to `subscribe_to_model` or `subscribe_to_view`.
//!
//! # Setup
//!
//! ```
//! cargo install cargo-dylint dylint-link
//! cargo dylint --manifest-path lints/Cargo.toml -- --workspace
//! ```
//!
//! See `lints/rust-toolchain.toml` for the required nightly pin.

#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;

use rustc_errors::Diag;
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty;

dylint_linting::declare_late_lint! {
    /// **What it does:**
    /// Flags closures passed to `subscribe_to_model` or `subscribe_to_view`
    /// that capture a strong `ModelHandle<_>` or `ViewHandle<_>` value.
    ///
    /// **Why is this bad?**
    /// The WarpUI subscription machinery keeps the callback closure alive for
    /// the entire lifetime of the emitting entity.  If that closure holds a
    /// strong `ModelHandle`, a reference cycle can form that prevents entities
    /// from being freed when they should be.
    ///
    /// **Same-entity captures are always flagged.**  The subscribed entity's handle
    /// is already provided as a callback parameter in all three context types:
    ///
    /// - `ViewContext` / `ModelContext` (4-param): handle is the **second** param.
    /// - `AppContext` (3-param): handle is the **first** param.
    ///
    /// Capturing a clone of it is redundant.  Use the provided parameter instead.
    ///
    /// Cross-entity captures are intentionally skipped: those are typically
    /// deliberate lifetime associations (e.g. a manager being updated in response
    /// to events from a different model).
    ///
    /// **Suppressing:** if you genuinely need to capture the handle (e.g. to pass
    /// it to a function that requires ownership), suppress with:
    /// ```rust
    /// #[allow(unknown_lints, model_handle_in_subscription)]
    /// ctx.subscribe_to_model(&handle, move |me, _, event, ctx| { ... });
    /// ```
    /// `unknown_lints` is required so that regular `cargo clippy` (which does not
    /// know this custom lint name) does not fail with `-D unknown-lints`.
    ///
    /// **Limitation:** same-entity detection compares handle types, not identities.
    /// Two distinct `ModelHandle<T>` instances that happen to share the same `T`
    /// will both be flagged even if only one is the subscribed handle.  Suppress
    /// with `#[allow(model_handle_in_subscription)]` when this is intentional.
    ///
    /// **Example (bad):**
    /// ```rust
    /// let fetcher_clone = fetcher.clone();
    /// ctx.subscribe_to_model(&fetcher, move |this, _model, event, ctx| {
    ///     fetcher_clone.read(ctx, |f, _| f.data());  // cycle!
    /// });
    /// ```
    ///
    /// **Example (good):**
    /// ```rust
    /// ctx.subscribe_to_model(&fetcher, move |this, model, event, ctx| {
    ///     model.read(ctx, |f, _| f.data());  // use the provided parameter
    /// });
    /// ```
    pub MODEL_HANDLE_IN_SUBSCRIPTION,
    Deny,
    "captured `ModelHandle` or `ViewHandle` in subscription closure"
}

impl<'tcx> LateLintPass<'tcx> for ModelHandleInSubscription {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        // ── 1. Match subscribe_to_model / subscribe_to_view calls ────────────
        let ExprKind::MethodCall(method, _receiver, args, _call_span) = &expr.kind else {
            return;
        };

        if !matches!(
            method.ident.name.as_str(),
            "subscribe_to_model" | "subscribe_to_view"
        ) {
            return;
        }

        // args = [handle, callback]
        if args.len() < 2 {
            return;
        }
        let handle_arg = &args[0];
        let callback_expr = peel_blocks(&args[args.len() - 1]);

        let ExprKind::Closure(closure) = &callback_expr.kind else {
            return;
        };

        // ── 2. Distinguish ViewContext/ModelContext (4-param) vs AppContext (3-param) ──
        //
        // ViewContext callbacks:  (&mut T, ModelHandle<E>, &Event, &mut ViewContext<T>)  — 4 params
        // ModelContext callbacks: (&mut T, ModelHandle<S>, &Event, &mut ModelContext<T>) — 4 params
        // AppContext callbacks:   (ModelHandle<S>, &Event, &mut AppContext)              — 3 params
        //
        // The entity-context callbacks always have 4 params and the handle is the 2nd;
        // AppContext callbacks have 3 params and the handle is the 1st.
        let is_entity_ctx = closure.fn_decl.inputs.len() == 4;

        // ── 3. Extract the subscribed entity's inner type E from &ModelHandle<E> ──
        //
        // Used to distinguish "same-entity" captures (cycle) from "cross-entity"
        // captures (intentional association vs unnecessary retention).
        let subscribed_inner = first_type_arg(cx.typeck_results().expr_ty(handle_arg).peel_refs());

        let closure_def_id = closure.def_id;

        // ── 4. Use closure_min_captures to catch field-projection captures ───
        //
        // `upvars_mentioned` only gives the root variable, missing patterns like
        // `self.some_handle.clone()` where the root variable type is not ModelHandle.
        // `closure_min_captures` records the actually-captured place, so it catches
        // field projections too.
        let min_caps = cx
            .typeck_results()
            .closure_min_captures
            .get(&closure_def_id);

        // Fall back to an empty iterator if there are no captures at all.
        let captures: Vec<_> = min_caps
            .into_iter()
            .flat_map(|root_map| root_map.values())
            .flat_map(|captures| captures.iter())
            .collect();

        for captured_place in captures {
            let captured_ty = captured_place.place.ty();

            if !is_strong_handle(cx, captured_ty) {
                continue;
            }

            let captured_inner = first_type_arg(captured_ty);

            let same_entity = match (subscribed_inner, captured_inner) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

            // Skip cross-entity captures: those are typically intentional
            // lifetime associations (e.g. a manager updated by another model's
            // events) and produce mostly noise.
            if !same_entity {
                continue;
            }

            let param_position = if is_entity_ctx { "second" } else { "first" };
            let msg = format!(
                "closure captures `{captured_ty}` — \
                 the subscribed entity's handle is already the {param_position} callback parameter; \
                 use that instead of capturing an owned handle"
            );

            let span = captured_place.get_capture_kind_span(cx.tcx);
            cx.span_lint(
                MODEL_HANDLE_IN_SUBSCRIPTION,
                span,
                |diag: &mut Diag<'_, ()>| {
                    diag.primary_message(msg.clone());
                },
            );
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Returns `true` if `ty` is `ModelHandle<_>` or `ViewHandle<_>`.
fn is_strong_handle<'tcx>(cx: &LateContext<'tcx>, ty: ty::Ty<'tcx>) -> bool {
    let ty::Adt(adt_def, _) = ty.kind() else {
        return false;
    };
    let name = cx.tcx.item_name(adt_def.did());
    matches!(name.as_str(), "ModelHandle" | "ViewHandle")
}

/// Returns the first generic type argument of `ty`, e.g. `T` in `ModelHandle<T>`.
fn first_type_arg<'tcx>(ty: ty::Ty<'tcx>) -> Option<ty::Ty<'tcx>> {
    let ty::Adt(_, args) = ty.kind() else {
        return None;
    };
    args.types().next()
}

/// Strip outermost expression-block wrappers (`{ expr }` with no statements).
fn peel_blocks<'tcx>(mut expr: &'tcx Expr<'tcx>) -> &'tcx Expr<'tcx> {
    loop {
        match &expr.kind {
            ExprKind::Block(block, _) if block.stmts.is_empty() => match block.expr {
                Some(inner) => expr = inner,
                None => return expr,
            },
            _ => return expr,
        }
    }
}
