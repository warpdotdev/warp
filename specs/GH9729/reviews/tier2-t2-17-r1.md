---
item: tier2-t2-17
commit: dff6822
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-17 is a diagnostic-iteration follow-up on t2-16. It (1) promotes the three
zoom-button `on_click` diagnostic logs from `log::debug!` to `log::warn!` so they
clear the default `LevelFilter::Info` threshold and actually appear in `warp.log`,
and (2) tightens two layout constants — introduces `ZOOM_ICON_GAP = 6.` (now fed to
`Flex::row::with_spacing`) and drops `ZOOM_RESET_GAP_FROM_ICONS` from `16.` to `8.`
— in response to explicit user feedback. No behavior outside the zoom toolbar,
no test edits.

# Findings

- [pass] Both spec deliverables land exactly as written. Diff at `crates/ui_components/src/lightbox.rs:539, 558, 603` shows all three `log::debug!` sites converted to `log::warn!` with refreshed message strings (`"GH9729 t2-17 DIAG: zoom_out (−) on_click fired"`, etc.). Diff at `crates/ui_components/src/lightbox.rs:27–40` introduces `ZOOM_ICON_GAP = 6.` and reduces `ZOOM_RESET_GAP_FROM_ICONS` from `16.` to `8.`. `Flex::row().with_spacing(ZOOM_ICON_GAP)` is wired at the icon-cluster builder.
- [pass] The level bump is the right knob. `crates/warp_logging/src/native.rs:382` sets `base_logger.filter_level(LevelFilter::Info)` as the global default. `log::debug!` records were structurally filtered out — the t2-16 commit could never have produced visible output without `RUST_LOG=ui_components=debug` set, which the commit body did not instruct the user to do. Bumping to `warn!` makes the diagnostic *actually* observable under the default configuration. This is a correct, minimum-mechanism change.
- [pass] No regression for the error-scrim path. The modified region is inside the `if let Some(on_zoom) = params.options.on_zoom` toolbar branch only; the `LightboxImageSource::Error` path earlier in `Component::render` and its independent positioned-children nav buttons are untouched. `ZOOM_RESET_GAP_FROM_ICONS` is still referenced by the reset-button offset arithmetic (lines ~605–610), so dropping its value from 16 to 8 is the *only* visible effect of the constant change there — no dead-code risk.
- [pass] No new lint or import surface. `log` is already declared in `crates/ui_components/Cargo.toml` (added in t2-16), `Flex::row::with_spacing` is already used, and `with_spacing(ZOOM_ICON_GAP)` is a single-token swap.
- [minor] The diagnostic harness does not match the user's stated capture mechanism. The commit body says the user runs `log stream --predicate "process == warp-oss"` (macOS Unified Logging / `os_log`), but this repo's logger is `env_logger` writing to either stderr or `warp.log` (`crates/warp_logging/src/native.rs:421–438`) — it does **not** publish to `os_log`. So even at `warn!` level the records will land in `~/Library/Logs/warp/warp.log` (or stderr) and *will not appear* in `log stream`. If the user is genuinely using `log stream`, the level bump is necessary but not sufficient. R1's recommendation: the iteration body should ask the user to `tail -f` the file under `simple_logger::manager::resolve_log_path` (or whatever the GUI variant evaluates to) instead, OR add an `os_log` sink. Not blocking — the logs land *somewhere* visible — but the commit theory is one cache-miss removed from the user's reported workflow.
- [minor] `log::warn!` is the wrong level *semantically* for this content. Warn is reserved for operator-actionable conditions (the lightbox itself follows this convention at `crates/ui_components/src/lightbox.rs:390`, which directs OS error strings to `log::warn!` for the operator). A click-arrived informational tracer is `log::info!` or `log::trace!`, not `warn!`. Using `warn!` for this works as a diagnostic dial but conflates "system in a degraded state" with "happy-path click fired". Since the commit body itself flags these as "remove once the + bug is closed", non-blocking. But: a `// TODO(GH9729 t2-17): downgrade or delete before merge to master` marker is still missing on the call sites themselves (called out as a nit on t2-16-r1; the level promotion makes the omission louder, not quieter, because warn-level noise survives the default filter).
- [minor] Edge case: warn-level diagnostics in release builds. `log` is configured at `Cargo.toml:180` *without* the `release_max_level_*` feature, and `simple_logger`/`warp_logging` does not gate by debug/release. A release Warp build with this commit would emit three `warn`-level records per zoom click into `warp.log`, plus into any `sentry_log` filter chain (`crates/warp_logging/src/native.rs:438–441`). The Sentry filter (`sentry_log_filter`) needs to be checked: if it forwards `warn` records to Sentry, opening the lightbox and clicking zoom would generate Sentry breadcrumbs for every user click on every dogfood build. Not blocking for a `spec/GH9729-image-preview` branch that never ships, but it is a real reason these MUST come out before any merge to master.
- [nit] The new log strings drop the `t2-16` prefix in favor of `t2-17 DIAG`. Fine, but it means if the user has both builds in their history they need to know to grep for the newer prefix — only matters in practice if there is rollback churn.
- [nit] Gap constant `ZOOM_ICON_GAP = 6.` is hard-coded rather than referencing a spacing token in `warpui::tokens` (or similar). The rest of the file uses raw float literals for spacing too, so this is consistent with local convention, not worth fighting on a diagnostic-iteration commit.

# What I checked

- `git show dff6822` — full diff is exactly the three `debug!→warn!` conversions, one new const, one const value change, and one `with_spacing(0.0) → with_spacing(ZOOM_ICON_GAP)`. Nothing else moves.
- `crates/warp_logging/src/native.rs:380–402` — confirmed `base_logger.filter_level(LevelFilter::Info)` is the unconditional default, so `debug!` is filtered, `warn!` is not. No `release_max_level_*` feature on `log` (root `Cargo.toml:180`).
- `crates/warp_logging/src/native.rs:421–445` — log sink is `env_logger::Target::Pipe(...)` to a rotated `warp.log`, or stderr when `stdout_is_a_tty`. Not `os_log`. The `log stream --predicate "process == warp-oss"` workflow in the commit body would not pick these up regardless of level.
- `crates/ui_components/src/lightbox.rs:390` — existing in-file convention: `log::warn!` is documented as "for the operator" attached to error-path information. The t2-17 usage diverges from this convention by using `warn!` for happy-path tracers.
- `specs/GH9729/tech.md` §698 — supplemental "Zoom and pan controls" entry; non-normative for layout constants and logging, so this iteration is in scope for spec.
- t2-16 R1 review (`specs/GH9729/reviews/tier2-t2-16-r1.md`) — confirms the `// TODO: remove before merge` markers nit was already raised at t2-16 and remains unresolved; t2-17 inherits the same caveat.
- Hit-test surface unchanged: `Flex::row` still iterates all children in `dispatch_event`, so the layout-tightening change does not alter which closure receives a `+` click. This is consistent with the t2-16 R1 finding that the Flex partitioning is a *layout* property, not a *dispatch* property.
- Hypothesis test: at level `warn!`, all three log strings will surface in `warp.log`. Outcomes (a)/(b)/(c) in the commit body are mutually exclusive and each distinguishable. The diagnostic is well-designed in the abstract. The remaining gap is whether the user's capture tool sees the file (see [minor] #1).

# Suggestions

1. (Important for merge-to-master) Add an explicit removal marker to each of the three `log::warn!` lines, e.g. `// TODO(GH9729 t2-17): delete before promoting branch — DO NOT ship`. Both t2-16-r1 and now t2-17-r1 have flagged this; landing a third diagnostic iteration without it makes the commitment to remove progressively harder to honor.
2. Either redirect the user to `tail -f ~/Library/Logs/warp/warp.log` (or `~/Library/Logs/dev.warp.WarpOss/warp.log` for the OSS build path, per `simple_logger::manager::resolve_log_path`) for capture, OR add a one-line `os_log` re-emit behind the diagnostic block for the duration of the saga. As written, `warn!` is necessary but the user's `log stream` recipe still produces no output, which would falsely look like outcome (c) ("click never reaches the Hoverable") and send the next iteration down a wrong branch.
3. Once the saga closes (whichever of (a)/(b)/(c) lands), prefer demoting the surviving call site to `log::trace!` behind an `#[cfg(debug_assertions)]` gate rather than deleting it outright — it's a useful regression marker for any future zoom-toolbar work, but it must not survive into release builds as `warn!`.
4. Consider whether `sentry_log_filter` in `crates/warp_logging/src/native.rs` would forward these as Sentry breadcrumbs in dogfood/release. If yes, that strengthens suggestion #1 from "polite" to "required".
