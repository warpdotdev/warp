---
item: tier2-t2-14
commit: 46f0a2e
reviewer: R2-quality
spec_ref: tech.md §697-699 (supplemental)
verdict: pass-with-nits

# Findings

- [nit] The new alpha value `250` is left as an inline magic number in
  `scrim_color()` (`crates/ui_components/src/lightbox.rs:372`). The same
  file already extracts a sibling alpha as a named constant — see
  `METADATA_TEXT_ALPHA: u8 = 178; // 255 * 0.7` at line 327, used by
  `metadata_text_color()` on line 334. The repo-wide convention agrees:
  `MOBILE_OVERLAY_SCRIM_ALPHA: u8 = 128` (`app/src/workspace/view.rs:646`),
  `OVERLAY_ALPHA` / `INLINE_OVERLAY_ALPHA`
  (`app/src/code/editor/diff.rs:28-29`), `COMMAND_ALPHA`
  (`app/src/terminal/block_list_element.rs:103`). Extracting
  `const SCRIM_ALPHA: u8 = 250; // 255 * ~0.98` would make this
  consistent with the immediate neighbour and let the alpha be tweaked
  without re-reading the function body.
- [nit] `scrim_color()` is a zero-arg fn returning a const value; once
  the alpha is named, the helper itself becomes a constant expression
  and could just be `const SCRIM_COLOR: ColorU = ColorU::new(0, 0, 0,
  SCRIM_ALPHA);` (mirroring `switch.rs`'s `static TRACK_COLOR` pattern
  but as a `const` since `ColorU::new` is presumably `const`). Not a
  blocker — the fn form is fine.
- [pass] Doc comment is high-quality: explains the v1→v2 motivation,
  cites the specific symptom (text bleed-through at 427% zoom), and
  records the deliberate non-decision (250 vs. 255) for the next reader.
  This is exactly the WHY context the commit message had and the v1
  code did not. The "Set to 255 if a fully opaque modal is preferred"
  closer makes the future tweak path explicit.
- [pass] Module placement is correct: `scrim_color()` already lived in
  `crates/ui_components/src/lightbox.rs` alongside `SCRIM_PADDING` /
  `SCRIM_BUTTON_INSET` / `metadata_text_color()`. The change keeps the
  scrim's visual constants co-located. No reason to move to a shared
  colour module — `MOBILE_OVERLAY_SCRIM_ALPHA` lives in `app/` and is
  cfg-gated to wasm, so there's no shared abstraction worth aligning
  to.
- [pass] No leftover comments, no unused imports, no dead branches
  introduced; the diff is a one-byte value flip plus a doc-comment
  rewrite.
- [pass] No test churn called out, and that's the right call: this is
  a visual-only tweak with no existing pixel/snapshot harness for the
  scrim that the change would invalidate or that would meaningfully
  guard the value. The commit message explicitly names manual
  re-verification as the regression check, which is honest.
- [pass] Tracker bookkeeping in the same commit (`TIER2_TODO.md` row
  + matrix entry) follows the established Tier-2 pattern for this
  feature; the `_pending_` placeholder for the commit hash will be
  filled in on the impl-row tick as usual.
- [minor] The bullet body in `TIER2_TODO.md` notes "(c)" (zoom toolbar
  blends with no background container) and "(d)" t2-7-r1 gotcha as
  out-of-scope for this row. (b) — toolbar prominence — is delivered
  here purely as a side-effect of the scrim getting darker
  ("buttons … should become more visible as a side-effect" in the
  commit message). That's fine on its merits, but the row's title
  promises "toolbar prominence" as a deliverable. If manual
  verification shows the buttons still read as low-contrast on the
  darker scrim, a real toolbar background (rounded chip behind
  `[−] [+] [100%]`) is the structural fix and deserves its own row
  rather than being implicit in t2-14.

# What I checked

- Read the full `git show 46f0a2e` diff. Confirmed scope: one alpha
  literal in `crates/ui_components/src/lightbox.rs` plus the
  `TIER2_TODO.md` row + matrix entry. No other code or spec files
  touched.
- Confirmed `tech.md §697-699` context (those entries describe
  follow-up axes — animated GIF, zoom/pan controls, status footer —
  not the scrim alpha specifically). t2-14 is correctly labelled
  "supplemental" in the tracker. Not a spec-fidelity concern.
- Searched for existing scrim/alpha constants the new value could
  align to:
  - `crates/ui_components/src/lightbox.rs:327`
    `METADATA_TEXT_ALPHA: u8 = 178; // 255 * 0.7` — same file,
    same `_ALPHA: u8` convention, used by `metadata_text_color()`
    immediately below.
  - `app/src/workspace/view.rs:646`
    `const MOBILE_OVERLAY_SCRIM_ALPHA: u8 = 128;` — same
    `_ALPHA: u8` naming, scrim-specific.
  - `app/src/code/editor/diff.rs:28-29`
    `OVERLAY_ALPHA`, `INLINE_OVERLAY_ALPHA`.
  - `app/src/terminal/block_list_element.rs:103`
    `COMMAND_ALPHA`.
  These all support the nit above: the repo's convention is to name
  alpha values as `*_ALPHA: u8` constants.
- Searched for similar `fn *_color() -> ColorU` helpers in
  `ui_components/`. Found `metadata_text_color()` in the same file
  and `TRACK_COLOR` in `switch.rs` (the latter is a `LazyLock<ColorU>`,
  not a fn — different pattern). No shared "scrim colour" abstraction
  exists across crates that this should plug into.
- Searched for visual / snapshot tests covering the lightbox. The
  lightbox file has no `#[cfg(test)]` block beyond comments mentioning
  "test surfaces", and there's no scrim-pixel harness to plug into.
  Manual verification is the documented regression path.
- Checked that no other reference to the old `230` alpha lingers in
  the lightbox file — `rg "230"` in `lightbox.rs` only matched the
  new doc comment. Clean.

# Suggestions

These are deferred R2 follow-ups (small, non-blocking) and could be
folded into a future polish row:

1. Extract `const SCRIM_ALPHA: u8 = 250; // 255 * ~0.98` next to
   `METADATA_TEXT_ALPHA` and have `scrim_color()` consume it. Aligns
   with the same-file neighbour and the repo-wide convention. If
   `ColorU::new` is `const`, collapse the helper to
   `const SCRIM_COLOR: ColorU = ColorU::new(0, 0, 0, SCRIM_ALPHA);`
   and replace the call at line 711 directly — this matches how
   `SCRIM_PADDING` / `SCRIM_BUTTON_INSET` are already used in the
   same `paint` path.
2. If manual re-verification shows the zoom toolbar still reads as
   low-contrast on the 250-alpha scrim, open a separate row for a
   real toolbar chip (rounded background container behind
   `[−] [+] [100%]`) rather than relying on the scrim darkness
   alone. The current row title promises "toolbar prominence" but
   delivers it only as a side-effect.
3. (Speculative, only if the team converges on a fully-opaque modal)
   Move `SCRIM_ALPHA` / `SCRIM_COLOR` into a shared overlay-styling
   module so the lightbox and `MOBILE_OVERLAY_SCRIM_ALPHA` stop
   diverging. Not worth doing for the current asymmetry (one is
   wasm-only and at half opacity by design).
