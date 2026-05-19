---
item: tier2-t2-17
commit: dff6822
reviewer: R2-quality
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Findings

- [pass] `log::warn!` macro usage is idiomatic — three plain-string call sites with no format-argument trailing comma, matching the style used elsewhere in `crates/warpui_core` (e.g. `clipboard_utils.rs:315`, `core/app.rs:1453`). No unnecessary `{:?}` interpolation or trailing newlines.
- [pass] Constant placement is clean: `ZOOM_ICON_GAP` and the revised `ZOOM_RESET_GAP_FROM_ICONS` sit alongside the other `ZOOM_*` layout constants at the top of `lightbox.rs` (lines 21–44), each carrying a doc-comment with the `GH9729 §698 / t2-XX` provenance prefix that the rest of the file already uses.
- [pass] Constant names are clear and self-documenting: `ZOOM_ICON_GAP` reads exactly as the gap inside the `[−][+]` cluster, and `ZOOM_RESET_GAP_FROM_ICONS` keeps its prior, already-good name (only the value moved, 16 → 8). No naming churn forced on downstream readers.
- [pass] Doc comments on the constants are unusually good for a diagnostic-iteration commit: each one names the prior tier (`t2-15`, `t2-16`), explains why the value is the way it is ("user feedback that the gap was too wide"), and leaves a breadcrumb for the next reviewer. This matches the comment quality standard the surrounding file already sets.
- [nit] The three `log::warn!` messages all begin with the prefix `GH9729 t2-17 DIAG:`. The `t2-17` tier label inside the runtime log string is fine for the next ~2 commits, but it ages badly — anyone reading the log in t2-18+ has to cross-reference TIER2_TODO.md to know what `t2-17` was. Already partially addressed: the inline comment above `zoom_out` says "Diagnostic only; remove once the + bug is closed." Worth keeping that comment (or an equivalent) above all three call sites, not just the first one, since the cleanup has to land at the same time.
- [nit] The diagnostic comment is duplicated as a TODO marker only on the first call site (`zoom_out`). The other two (`zoom_in`, `zoom_reset`) have no `// remove once...` reminder. A future cleanup pass that greps for `DIAG` will find all three, so this is non-blocking, but a symmetric comment block above each closure would make the cleanup obvious from `git grep "Diagnostic only"`.
- [pass] No stale `t2-16` comments left over inside the icon-cluster block: the in-place comment that previously said "zero spacing" has been rewritten to describe the new positive `ZOOM_ICON_GAP` and explicitly names the t2-16 feedback it supersedes (line 842 in the current file). Good housekeeping for a diagnostic-tier commit.
- [pass] Test rigor: the spec acknowledges (correctly) that pure-diagnostic + layout-constant changes have nothing meaningful to test. Both `log::warn!` substitution and a literal gap-value change are non-functional from the test surface's perspective — UI tests check semantic structure, not pixel-level spacing. Skipping tests here is correct; flagging this as a finding only because it would be the wrong call to skip tests on a future tier-2 commit that *does* touch behaviour.
- [pass] Commit message is exceptionally well-structured for a diagnostic iteration — it enumerates the three branches `(a)/(b)/(c)` of evidence the warn-level logs are designed to distinguish. This is exactly the kind of "what the next commit looks like depending on what we see" reasoning that should be in the commit, not lost in chat.

# What I checked

- `git show dff6822` — full diff inspection, all three `log::debug! → log::warn!` substitutions, the two constant edits, the `Flex::row().with_spacing(0.0) → with_spacing(ZOOM_ICON_GAP)` change, and the surrounding comment block at the icon cluster.
- `tech.md` §698 — confirmed §698 is not a hard-line section number in the current spec body (the spec ends at the "Follow-ups" list well before line 698); the `§698` tag is a supplemental marker used by the t2-* series to brand work against the zoom-toolbar surface. No spec misalignment.
- `rg "log::warn!"` across `crates/ui_components/` and `crates/warpui_core/` — confirmed the call-shape used here matches the dominant pattern (plain string, no trailing comma, no structured fields). One existing comment in `lightbox.rs:390` already documents the project convention of preferring `log::warn!` for operator-facing diagnostics, so this commit is consistent with established style.
- `grep "t2-16"` and `grep "GH9729 t2"` across the lightbox file — confirmed no stale `t2-16: zoom_out_button clicked` strings or zero-spacing comments survive the rewrite, and the remaining `t2-16` reference at line 788 is a legitimate provenance pointer in a doc comment, not dead diagnostic plumbing.
- Naming-convention check on the new `ZOOM_ICON_GAP` constant against its neighbours (`ZOOM_ICON_BUTTON_SLOT`, `ZOOM_RESET_GAP_FROM_ICONS`, `SCRIM_BUTTON_INSET`, `SCRIM_PADDING`) — all-caps `ZOOM_*` prefix is consistent, semantic suffix `_GAP` matches the existing `_GAP_FROM_ICONS` precedent.

# Suggestions

Deferred R2 follow-up (when t2-18 or later closes the + bug):

- Remove the three `log::warn!("GH9729 t2-17 DIAG: ...")` call sites in one cleanup commit; do not silently downgrade them back to `log::debug!` — they have served their purpose and a stale `DIAG` warn-level message in production log streams is worse than no message.
- Either delete `ZOOM_ICON_GAP` if a future layout iteration folds the gap back into the slot width, or keep it and drop the "user explicit feedback after t2-16's zero-spacing layout was too tight" parenthetical from its doc comment — once the bug is closed, the history of how the gap got to 6 is less interesting than the fact that 6 is the chosen value.
- Add a symmetric `// Diagnostic only; remove once the + bug is closed.` comment above the `zoom_in` and `zoom_reset` closures if any further tier touches this file before cleanup, so a future grep for the cleanup marker finds all three.
