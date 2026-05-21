---
item: tier2-t2-18
commit: 45ccfe2
reviewer: R2-quality
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Findings

- [pass] The revert is clean and idiomatic. Each of the three `log::warn!(...)` lines (and the t2-17 explanatory `//` comment block above the first one) is removed in full, leaving the three on_click closures syntactically tight: `Box::new(move |ctx, app, _| { on_zoom_out(ZoomDirection::Out, ctx, app); })` etc. No empty closure body, no dangling comma, no orphaned `// DIAG` or `// t2-17` marker left behind. The diff is exactly an inverse of the t2-17 diagnostic insertion plus the t2-16 hint comment, which is what a clean revert should look like.
- [pass] No `ZoomDirection`, `step_zoom`, or constant got a t2-17/t2-18 era rename that needs unwinding. Tracker labels in surrounding doc comments (`GH9729 §698 / t2-12`, `t2-19`, `t2-21`) belong to other items and are correctly untouched.
- [pass] Comment quality: the gotcha analysis (ConstrainedBox::layout taking `min(parent_max, self_max)`, viewport-bound at 800x800 hiding the visual effect for 1024x1024) is written up at length, with a numeric example and a small-image counter-case (`05-icon.svg` at 200x200). That is genuinely useful material — but the right home for the diagnosis is the t2-7-pan handoff context, and t2-19's commit message confirms it was reapplied / actioned there, so this commit message is effectively the handoff note. Reasonable choice.
- [minor] The diagnosis lives only in the commit message, not in code. A two-line `// GH9729 t2-7-r1: zoom>1 only grows the visible image once the parent's max also grows; see t2-7-pan` next to either `step_zoom`'s call site or the `ConstrainedBox` wrapper would survive `git blame` better than a commit message that future readers have to dig for. Not blocking — t2-19's PanClippedImage commit landed almost immediately after and supersedes this concern in practice.
- [minor] `log.workspace = true` in `crates/ui_components/Cargo.toml` (added by t2-16, line 10) is now unused by this crate. After t2-18 the only `log::` token left in `ui_components/src/` is a rustdoc reference (`/// log::warn!` in the `Error { message }` doc comment at lightbox.rs:390), which doesn't require the dep. Cargo / clippy won't flag an unused workspace dep automatically, so this will silently linger. Worth a one-line drop, ideally bundled with whatever later commit needs to touch this Cargo.toml again so it doesn't churn for its own sake.
- [pass] No tests deleted; nothing landed in t2-16/t2-17 that needed a test (they were all diagnostic), so there is nothing to test-revert here. The 18/18 lightbox_view tests stay green.
- [pass] Module boundaries otherwise clean — the closures still call into the same `on_zoom_in` / `on_zoom_out` / `on_zoom_reset` helpers, no new exports.

# What I checked

- `git show 45ccfe2` — three localized deletions in `crates/ui_components/src/lightbox.rs` (one comment block + three `log::warn!` lines), nothing else touched.
- `tech.md` §698 supplemental list — zoom-and-pan is the explicit tier-2 deliverable; nothing in §698 demands diagnostic logging, so removal does not regress the spec.
- `crates/ui_components/Cargo.toml` line 10 (`log.workspace = true`) and the `log` workspace declaration at root `Cargo.toml:180`. Grep across `crates/ui_components/src/` confirms zero remaining `log::` callsites after this commit; the only hit is a `log::warn!` mention inside a rustdoc comment.
- `git log -- crates/ui_components/Cargo.toml` to confirm the `log` dep was introduced by t2-16 and nothing else in the crate depends on it.
- Surrounding closures (`on_zoom_out`, `on_zoom_in`, `on_zoom_reset` button blocks, lines ~525-610) for any leftover diagnostic scaffolding — clean.
- The doc comment for `LightboxImageSource::Error` (line 390) that still references `log::warn!` rhetorically — fine as documentation guidance, not actual usage.

# Suggestions

Deferred R2 follow-up (low priority, single-line, no urgency):

- Drop `log.workspace = true` from `crates/ui_components/Cargo.toml` next time that file is opened. The cheapest moment is whenever a future tier-2 item touches that Cargo.toml for an unrelated reason; doing it standalone is fine too but isn't worth its own PR turn.
- If t2-7-pan's eventual landing wants a small in-code anchor for the `ConstrainedBox::layout` gotcha, a 2-3 line comment at the zoom toolbar or PanClippedImage layout site (rather than in the commit message) would make the diagnosis discoverable via `git grep` in 6 months.
