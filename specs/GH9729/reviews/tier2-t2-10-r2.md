---
item: tier2-t2-10
commit: af7d5f5
reviewer: R2-quality
spec_ref: tech.md §182 + §695 (supplemental)
verdict: pass-with-nits
---

# Spec

The fix supplements `specs/GH9729/tech.md` in two places. §182 is part of
the change-3 refactor brief in `lightbox_view.rs`; §695 is the
follow-up bullet.

§182 (verbatim, from the change-3 brief, lines 194-205):

> In `app/src/workspace/lightbox_view.rs`, refactor the asset-load
> callback shape so it can rewrite the entry on failure or on
> `Unrecognized`. This is a small refactor, not a one-liner:
>
> - Today, `start_asset_load` is an associated function
>   (`Self::start_asset_load(asset_source, ctx)`) called from inside
>   the `images.iter()` loop in `start_asset_loads` (lines 108-114).
>   The future returned by `handle.when_loaded(asset_cache)` resolves
>   to `()`; the spawn callback is `|_me, (), ctx| { ctx.notify(); }`
>   and has no entry index in scope.
> - Refactor `start_asset_load` to a method on `&mut self` that takes
>   the entry index alongside the `AssetSource`. Capture
>   `(asset_source, index)` into the spawn callback so the callback
>   can re-query the asset cache for the post-load state and mutate
>   `self.params.images[index]`.
> - Inside the callback, after `ctx.notify()`, look up the asset
>   state via
>   `AssetCache::as_ref(ctx).load_asset::<ImageType>(asset_source)`.
>   The state transitions are observable as:
>   - `AssetState::Loaded { data: ImageType::Unrecognized }` →
>     rewrite to `LightboxImageSource::Error { message:
>     "could not detect image format" }`. …
>   - `AssetState::FailedToLoad(err)` → rewrite to
>     `LightboxImageSource::Error { message: sanitize(err) }` …
>   - All other states leave the entry unchanged.

§695 (verbatim, line 695, the follow-ups list):

> - **Convert `ImageType::Unrecognized` to `Err` globally** with an
>   audit of every `try_from_bytes` caller. Closes the
>   mislabeled-file rough edge for surfaces other than the Lightbox
>   file-tree path (which v1 already closes locally).

Together these pin the contract this commit fixes: the rewrite path
exists, but §182 phrased it only as "inside the callback", and t2-4
implemented the §695 follow-up by switching the mislabeled path from
`Loaded { Unrecognized }` to a synchronous `Err`. Once the cache can
return `FailedToLoad` *inline*, the "inside the callback" wording
under-specifies the contract — the callback never installs.

# Findings

## 1. Helper shape — `&mut [LightboxImage]` + index vs `&mut LightboxImage` (nit)

The chosen `apply_rewrite_to_slot(images, index, state)` shape is
testable without a `ViewContext` and shared by both call sites — both
real wins. The downside is that the helper owns the bounds check,
which is a concern not all callers care about: the synchronous call
site has just constructed/mutated the slice and *knows* the index is
in range (it's the same index it just iterated to). Pushing the
bounds tolerance into the helper is right for the asynchronous
callback (stale-index race against `update_params`) but mildly
overshoots for the synchronous path. The cost of this overshoot is
one `get_mut` per inline call — negligible. The current shape is
fine; the alternative (`apply_rewrite_to_slot(slot: &mut
LightboxImage, state)` + caller-side bounds check on the async path)
would split the contract across two sites and lose one test
(`tolerates_out_of_bounds_index`) without a real gain. **Keep as-is.**

## 2. `AssetState::Evicted` as a stand-in for `Loading` (nit)

The "leaves Loading alone" test uses `Evicted` because constructing
a real `Loading { handle: AssetHandle }` requires a `LoadHandle` from
the asset cache. `rewrite_image_for_load_state` matches `FailedToLoad`
and the wildcard `_`, so `Evicted` exercises the exact same arm as
`Loading` and `Loaded { data }`. The test pins the contract "any
non-failure state is a no-op" rather than the narrower contract "the
`Loading` arm specifically is a no-op" — and the inline comment
explicitly calls this out. A tighter test would require either:

- exposing a test constructor on `AssetHandle` (invasive — public-API
  surface change in `warpui_core` for a private test), or
- the view-test harness approach (see finding 9): drive a real asset
  cache with a never-resolving fetcher so `load_asset` returns
  `Loading { handle: <real> }` and assert the slot stays `Resolved`.

The latter is a real test but lives at the integration-test layer,
not at this unit-test layer. **Acceptable as written**, given the
comment makes the substitution explicit; consider naming the test
`apply_rewrite_to_slot_leaves_non_failure_states_alone` to match the
actual contract (see finding 8).

## 3. Doc comment rewrite (pass)

The new two-bullet structure is clearer than the prior single-paragraph
version. The old comment buried "synchronous fallthrough" inside a
parenthetical about §695; the new comment makes the two paths
co-equal and names the bug class for each. Two minor accuracy notes:

- The synchronous bullet correctly identifies the *trigger* (tiny
  mislabeled file) but not the *cache mechanism*. The asset cache
  short-circuits to `FailedToLoad` for *any* size of file once
  `try_from_bytes` returns `Err` synchronously on the foreground
  executor; the "tiny" framing implies a size threshold that doesn't
  exist. Suggest "a mislabeled file whose bytes the asset cache can
  deliver synchronously" or "a mislabeled file that fails decode
  before the future yields". Not blocking — the example is correct
  for the surfaced repro.
- The asynchronous bullet says "the load is still pending" — accurate
  for `Loading`, but the helper is also called from the spawn body
  *after* the future resolves. The "asynchronous path" framing is
  about the call-site dispatch, not about the runtime state at helper
  entry. Reader-friendly as written.

## 4. Comment density inside the helper (nit)

The four-line `// GH9729 §695 / t2-10:` comment at the synchronous
call site (lines 207-212) is partially redundant with the doc comment
above it and partially redundant with the helper's own doc comment.
The information that's *not* in either of those (the no-op-for-common-
path framing) is genuinely useful — that's the regression-guard
insight for future readers. Trim the first sentence
("apply the rewrite inline for any state…") and keep the
"returns `None` for `Loading` and (post-§695) `Loaded`, so this is a
no-op for the common path" framing. Not blocking.

## 5. `AssetSource::Bundled` in tests (pass)

`AssetSource::Bundled { path: &'static str }` is the only variant that
takes a `&'static str` rather than a `String`, has no `Arc`/factory,
and has no construction-time validation. `LocalFile { path: String }`
needs a heap allocation; `Async` needs a fetcher; `Raw { id: String }`
also needs a heap allocation. **Bundled is the lightest construction
and is the right choice.** The `"fake/bundled/path.png"` literal is
never resolved by the helper (which only reads `slot.source` after
mutation), so the bogus path is fine.

## 6. `let-else` shape (pass)

Two consecutive `let Some(...) else { return false }` lines are
idiomatic for this codebase — 91 occurrences of `let Some(...) else
{` in `app/src/workspace/` alone (e.g.
`app/src/workspace/cross_window_tab_drag.rs:540, 625, 754, 880, 921,
1096, 1367, 1496`). The chained `if let Some && let Some` alternative
exists in Rust 1.65+ but is rare in this repo. **The chosen shape
matches the local convention; no change.**

## 7. Function placement (nit)

The helper sits at lines 235-249, after the close of `impl
LightboxView` (line 232) and before the `ZoomDirection` enum (line
252). The peer function `rewrite_image_for_load_state` sits at line
306, with `sanitize_load_error` (line 278) and `step_zoom` (line 263)
between them. The natural grouping is:

```text
impl LightboxView { … }
fn apply_rewrite_to_slot(…)      // image rewrite helpers
fn rewrite_image_for_load_state(…)
fn sanitize_load_error(…)
enum ZoomDirection { … }          // zoom helpers
fn step_zoom(…)
```

Currently the order interleaves zoom and image-rewrite helpers. Moving
`apply_rewrite_to_slot` next to `rewrite_image_for_load_state` (i.e.
to ~line 304, just above it) would group the two rewrite helpers
together and let a reader follow the call chain top-to-bottom without
scanning across the zoom enum. Not blocking — the file is small
enough that this is mostly aesthetic.

## 8. Test naming (nit)

- `apply_rewrite_to_slot_rewrites_synchronous_failed_to_load` —
  clear. **Keep.**
- `apply_rewrite_to_slot_leaves_loading_state_alone` — the test
  exercises `Evicted`, not `Loading`. The name implies a contract the
  test does not pin. Suggest
  `apply_rewrite_to_slot_leaves_non_failure_states_alone` (matches
  what the helper actually guarantees and what the inline comment
  states) or
  `apply_rewrite_to_slot_no_op_for_evicted` (matches what the test
  literally constructs). The first is the better contract anchor.
- `apply_rewrite_to_slot_tolerates_out_of_bounds_index` — clear.
  **Keep.**

## 9. Regression-guard rigor (concern, not blocking)

The bug class is: "a future contributor adjusts `start_asset_load`
and re-introduces the synchronous-fallthrough gap." The three new
tests pin the helper's contract but do not pin the call-site's
*use* of the helper. A regression where someone removes the inline
`apply_rewrite_to_slot(...)` call before the `if let
AssetState::Loading` guard would compile, would pass every existing
test, and would re-introduce the bug exactly.

A view-test harness exists — see
`app/src/code/editor/comment_editor_tests.rs:91` for the
`app.add_window(WindowStyle::NotStealFocus, |ctx| { … })` +
`ctx.add_view(|_| …)` pattern, and `view_test.rs:1801`. An
integration-style regression guard would:

1. Set up a `TestAppContext` with a real `AssetCache`.
2. Register a `LocalFile` source pointing at a temp file whose
   contents fail `try_from_bytes` (e.g., 34 bytes of "not an image").
3. Add a `LightboxView` window pointing at that file.
4. Drive the executor one turn.
5. Assert `view.params.images[0].source` is `Error { … }`.

This requires non-trivial harness wiring (asset-cache fixtures,
foreground-executor pumping) and is plausibly a Tier-3 follow-up
rather than a t2-10 blocker. The commit message and the inline
comment together carry enough institutional knowledge that a future
reader has a fighting chance — but the missing end-to-end guard is
worth noting. **Suggest filing a t3 follow-up: "view-harness
regression test for synchronous-FailedToLoad lightbox rewrite".**

## 10. Commit-message-vs-tracker mismatch (hygiene)

`TIER2_TODO.md` records the commit as `(see commit msg)`; the commit
message itself records `5331016` (the pre-amend SHA, now stale —
the real SHA is `af7d5f5`). Not a code finding; flagged for the loop
because the next reviewer searching for `5331016` will come up empty
and the tracker entry doesn't help.

# What I checked

- `git show af7d5f5` — full diff against current HEAD.
- `specs/GH9729/tech.md` §182 / change-3 brief (lines 194-205) and
  §695 (line 695) for the contract that this commit supplements.
- `AssetSource` enum shape at
  `crates/warpui_core/src/assets/asset_cache.rs:66-87` for the
  lightest-construction question.
- `AssetState<T>` variants at the same file (lines 89-94) to confirm
  `Evicted` is constructible without an `AssetHandle` and `Loading`
  is not.
- `rewrite_image_for_load_state` at
  `app/src/workspace/lightbox_view.rs:306-318` for the helper's
  upstream contract (`FailedToLoad → Some; _ → None`).
- `let-else` precedents in `app/src/workspace/` (91 occurrences,
  e.g. `cross_window_tab_drag.rs:540`) for the idiom-fit question.
- View-test harness existence at
  `app/src/code/editor/comment_editor_tests.rs:91` (the
  `app.add_window` pattern) for the integration-test feasibility
  question.
- Test names and inline test comments in the new tests block
  (`lightbox_view.rs:539-619`).
- File-layout of `lightbox_view.rs` (helpers, enums, impl blocks) for
  the placement question.

# Suggestions

In priority order, all non-blocking:

1. (Finding 8) Rename
   `apply_rewrite_to_slot_leaves_loading_state_alone` →
   `apply_rewrite_to_slot_leaves_non_failure_states_alone` so the
   test name matches the contract the test actually pins.
2. (Finding 9) File a t3 follow-up for a view-harness regression
   test that drives `start_asset_load` end-to-end with a real
   asset cache and a synchronously-failing source. Acceptable to
   ship t2-10 without it given the helper-level coverage.
3. (Finding 7) Move `apply_rewrite_to_slot` from line 235 to ~line
   305 so it sits adjacent to `rewrite_image_for_load_state`.
   Pure aesthetic.
4. (Finding 4) Trim the 4-line `// GH9729 §695 / t2-10:` comment to
   keep only the "no-op for the common path" sentence; the rest is
   covered by the doc comments above and on the helper.
5. (Finding 3) Soften "a tiny mislabeled file" in the synchronous
   bullet of the doc comment — the size threshold is not the
   mechanism.
6. (Finding 10) On the next loop turn, backfill the real SHA
   `af7d5f5` into either the commit-msg (via amend, if the loop
   permits) or the `TIER2_TODO.md` table entry.

**Verdict: pass-with-nits.** The fix is correct, well-tested at the
helper layer, and the rewritten doc comment makes the two-path
contract legible. All findings above are improvements, not blockers.
