---
item: tier2-t2-18
commit: 45ccfe2
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-18 reverts the three `log::warn!` diagnostics added in t2-16/t2-17
because the user's run produced 11 successive `ZoomIn` dispatches that
confirmed the `+` button closure fires, the action dispatches, and the
footer percentage updates. The commit message correctly re-attributes
the "nothing happens" perception to the t2-7-r1 `ConstrainedBox::layout`
gotcha (parent `min`-clamping makes zoom-in on already-viewport-bound
images a visual no-op) and defers the real fix to t2-7-pan, which the
t2-19 chain implements.

# Findings

- [pass] All three `log::warn!` DIAG sites are removed cleanly. `grep
  -n "log::warn\|DIAG"` against `crates/ui_components/src/lightbox.rs`
  returns only legitimate doc-comment references (the §182 SSRF-style
  guidance at line 390 mentions `log::warn!` in prose, and lines 33/38
  attribute *spacing constants* to t2-17 — these are unrelated to the
  DIAG call sites and should remain).
- [pass] Hypothesis confirmation is accurate. The commit message's
  recap of `ConstrainedBox::layout`'s `constraint.max =
  constraint.max.min(self.constraint.max)` semantics matches
  `tier2-t2-7-r1.md` lines 92-110 verbatim: the cap is bounded by the
  parent's available size, so for a 4K image (or even a 1024×1024
  image in a ~1404×800 scrim'd viewport) the rendered size is identical
  at `zoom = 1.0` and `zoom = 1.5`. Zoom-in is only visually meaningful
  for images smaller than the viewport (e.g. `05-icon.svg` at 200×200).
- [pass] Honest scope: t2-18 explicitly says "this commit does NOT fix
  the bug — t2-7-pan does." That's correct; the diff is a pure revert
  of three closure-body lines plus a 3-line comment immediately above
  the zoom-out closure. No layout, action, or zoom-factor code is
  touched.
- [minor] `crates/ui_components/Cargo.toml:10` `log.workspace = true`
  is now orphaned. The dep was added by `f960720` (t2-16) specifically
  for these diagnostics; after this revert there is no functional
  `log::` call site anywhere in the crate (`grep -rn "log::warn\|
  log::info\|log::error\|log::debug" crates/ui_components/` returns
  only the doc-comment at line 390). Either drop the dep here, or
  leave a TODO comment noting it's retained for imminent re-use. The
  doc-comment reference at line 390 does not require the crate-level
  dep — it's prose, not a macro invocation — so this would otherwise
  surface as an unused-dependency warning under `cargo udeps` or
  similar tooling.
- [nit] Stale comment block at `lightbox.rs:805-807` still reads:
  *"Diagnostic logging is included; if `+` STILL fails after this
  commit, the log will show whether the click is arriving at the
  closure at all (and which closure)."* This was the rationale for the
  now-deleted `log::warn!` lines and is misleading post-revert. The
  rest of the t2-16 comment block (Flex::row partitioning rationale)
  remains useful and should stay; only the last paragraph is stale.

# What I checked

- `git show 45ccfe2` — confirmed the diff is exactly three deleted
  `log::warn!` lines plus a 3-line `// GH9729 t2-17:` comment above the
  zoom-out site. No other code paths touched.
- `tech.md` §698 supplemental section — confirmed zoom/pan is listed
  as a single deferred bullet ("**Zoom and pan controls**: extend
  `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings
  (`+`, `-`, `0`, drag-to-pan)"), which corroborates the commit's
  framing that the visual zoom-in problem isn't separable from pan.
- `crates/ui_components/Cargo.toml` — `log.workspace = true` present at
  line 10; introduced by t2-16; no other call sites consume it after
  this revert.
- `specs/GH9729/reviews/tier2-t2-7-r1.md` — verified the gotcha cited
  in the commit message matches that review's `constraint.max.min(...)`
  analysis word-for-word, including the "visual no-op for
  window-already-sized images" conclusion and the suggestion to make
  the deferral row in `TIER2_TODO.md` explicit about pan being the real
  prerequisite (which `t2-7-pan` is).
- `grep -rn "log::"` across `crates/ui_components/src/` — only
  doc-comment hits remain; no functional invocations.
- `git log --oneline -- crates/ui_components/Cargo.toml` — confirms
  the `log` dep entered with t2-16 (`f960720`), exclusively for the
  DIAG plumbing.

# Suggestions

1. Either drop `log.workspace = true` from
   `crates/ui_components/Cargo.toml:10` in this same commit (it's a
   one-line followup to a revert that introduced the dep) or add a
   short TODO comment if the next item in the chain will re-use it.
   Otherwise a presubmit pass with `cargo udeps`/`cargo machete` or a
   future tidy commit will flag this and force an extra round-trip.
2. Trim the now-stale "Diagnostic logging is included; …" tail
   paragraph from the t2-16 comment block at `lightbox.rs:805-807`.
   The Flex::row partitioning rationale (lines 788-804) is still
   load-bearing context and should remain.
3. Optional: cross-link this commit (or the t2-19 chain) from
   `TIER2_TODO.md::t2-7-pan` so future readers can trace the bug-vs-fix
   split. Not strictly required by R1 since the tracker is read-only
   for this review, but worth noting for whoever closes t2-7-pan.
