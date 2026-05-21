---
item: 2-tests
commit: 32cee73
reviewer: R1-correctness
spec_ref: tech.md §613
verdict: pass-with-nits
---

# Findings

## Spec fidelity (test names)

The four test names match `tech.md` §613:621-626 character-for-character, in
the prescribed order:

- `image_preview_arm_dispatches_resolved_when_under_size_cap`
- `image_preview_arm_dispatches_error_when_over_size_cap`
- `image_preview_arm_dispatches_error_when_metadata_fails`
- `image_preview_arm_dispatches_error_for_non_regular_file`

All four are present in `app/src/workspace/view_test.rs:3024-3121` and all
four cover the prescribed cases at the right granularity (Resolved with
LocalFile + filename; Error with sanitized "image is too large to preview";
Error with sanitized "could not read image"; Error with "not a regular file"
via a directory path).

## Refactor scope and 1:1 behavior preservation

The helper extraction is justified by the spec note ("a small extracted
helper module to avoid pulling the whole view crate into a test"). I checked
production-behavior preservation against the diff and confirm 1:1:

- The original arm-scope `const MAX_PREVIEW_FILE_BYTES: u64 = 64 * 1024 * 1024`
  is now a module-scope const at `view.rs:23832` with the same value (64 MiB).
- The original arm-scope `const MAX_ERROR_MESSAGE_LEN: usize = 256` is now a
  module-scope const at `view.rs:23837` with the same value.
- The arm at `view.rs:5817-5828` still calls `build_image_preview_entry`
  passing those module-scope constants, so the production cap envelope is
  unchanged.
- The match-arm logic (`metadata` → `is_file()` → `len() > cap` → fallback
  → `log::warn!` of the underlying OS error → sanitized constant) is
  structurally identical inside the helper.
- `path: PathBuf` → `&Path` is a benign coercion via deref; `path.file_name()`,
  `path.to_string_lossy()`, and `std::fs::metadata(path)` all behave
  identically.
- The "metadata follows symlinks; `is_file()` rejects sym-resolved character
  devices, FIFOs, sockets, and directories" comment was preserved in the
  helper docstring at `view.rs:23850-23851`. No semantics dropped.
- Single callsite of `truncate_message` confirmed (only inside
  `build_image_preview_entry`).
- The promoted module-scope consts are not duplicated or shadowed
  anywhere — `grep` across `app/` and `crates/` shows the only matches are
  the single declaration site, the single arm reference, and three
  cross-crate docstring mentions in `assets/asset_cache.rs:500` and
  `image_cache.rs:313`.

## Doc-comment regression introduced by the refactor (NIT, but real)

The new const+fn block was inserted between an existing function and its
following function `tab_bar_rects_for_window`, and split that following
function's pre-existing two-line doc comment in half. At
`view.rs:23828-23832` we now have:

```rust
/// Returns every tab-bar-equivalent rect laid out in `window_id` (horizontal
/// One unified pre-read cap for raster and SVG image previews
/// (specs/GH9729/tech.md §119). The SVG-specific allocation surface is
/// bounded separately by the SVG intrinsic-dimension cap (item 4c).
const MAX_PREVIEW_FILE_BYTES: u64 = 64 * 1024 * 1024;
```

The orphan line `Returns every tab-bar-equivalent rect laid out in window_id (horizontal`
is now glued onto `MAX_PREVIEW_FILE_BYTES` as a leading doc-comment line,
and `tab_bar_rects_for_window` at `view.rs:23906` lost its first doc-comment
sentence — its docstring now starts mid-sentence with
`/// tab bar and/or vertical tabs panel). Both must be considered because a`
which is ungrammatical and obviously truncated. This will compile but it is
an unintentional doc-string hijack and should be cleaned up either in this
commit or a follow-up cargo-fmt pass.

## Test coverage gaps

### `truncate_message` is not exercised on its truncation path

All four tests pass `max_message_len = 256` and exercise messages of
length ≤ 31 characters. The truncation path of `truncate_message` (the
ellipsis branch) and the `max_len <= 1` degenerate branch are NOT
exercised by these four tests. R2's flag is correct.

The current callers in production all pass the categorical short
constants ("could not read image", "image is too large to preview",
"not a regular file") which are never long enough to truncate, and
`MAX_ERROR_MESSAGE_LEN = 256` is well above them — so the truncation
path is dead in production today. The fn docstring already acknowledges
this ("rarely truncates in practice; it exists as a defensive bound").

That said, the spec explicitly carved out the helper so caps could be
overridden in tests (`max_message_len` is a parameter precisely for this).
A direct unit test on `truncate_message` (e.g. a 5-char message with
`max_len = 4` asserting `"abc…"`, plus a `max_len = 1` and `max_len = 0`
case) is one extra test for trivial cost and would close the dead-code
risk. Not in the four prescribed names, so it is properly an add-on.

### Sanitization assertions: only one of three error tests asserts the
underlying string is NOT interpolated

`image_preview_arm_dispatches_error_when_metadata_fails` correctly asserts
`message == "could not read image"`, which proves the
`io::ErrorKind::NotFound` text is not interpolated. The other two error
tests (over-size-cap, non-regular-file) likewise assert the exact constant,
which by exact match also disprove interpolation. So the sanitization
property is covered for all three error categories. Good.

### Description-field assertions: inconsistent across tests

Three of the four tests assert `entry.description.as_deref() == Some("photo.png" | "huge.png" | "does_not_exist.png")` — proving only the
basename leaks, not the absolute path. Good.

The fourth test (`..._for_non_regular_file`) does NOT assert on
`description`. This is a reasonable omission because a directory's
basename comes from the random tempdir suffix and would be brittle to
match exactly, but the test still could assert
`assert!(entry.description.is_some())` and
`assert!(!entry.description.as_deref().unwrap().contains('/'))` to keep
the basename-only invariant under test on this branch too. Minor.

### Spec-divergence in test 2

§613:622 prescribes "a temp file (sparse) of `MAX_PREVIEW_FILE_BYTES + 1`."
The implementation uses 200 bytes against an injected 100-byte cap. This
is functionally equivalent — the helper takes the cap as a parameter
specifically to enable this — and avoids materializing a 64 MiB sparse
file. The commit message explicitly justifies the divergence. Acceptable.

A sparse-file-with-`set_len` variant would be a stricter regression check
(it would catch a future bug where the production const value is changed
without the test moving) but is not necessary for v1.

### Untested edge cases (not prescribed but worth tracking)

- Symlink to a regular file — should resolve and pass (covered transitively
  by `metadata` following symlinks, but not asserted).
- Symlink to a missing target — should hit the `Err(_)` arm.
- Path with non-UTF-8 bytes (Unix only) — `to_string_lossy()` produces
  a `String` with U+FFFD replacements; the asset cache then opens by that
  lossy string and fails to find the file. Today this path silently fails
  later. Not prescribed; should be tracked as a follow-up if non-UTF-8
  paths are in scope.
- TOCTOU between the metadata stat in the arm and the asset-cache open
  — not unit-testable at the arm layer. Already covered at the
  asset-cache layer per §613:633 (`local_file_read_rejects_post_open_non_regular_file`).
  Confirmed in scope elsewhere.

## TOCTOU and cross-platform tempfile semantics

`tempfile::TempDir::new()` creates a uniquely-named directory under
`std::env::temp_dir()`; the `RAII` drop deletes it. No race risk for these
unit tests since each test uses a fresh `TempDir`. Cross-platform: works on
Windows, macOS, Linux. The directory test
(`..._for_non_regular_file`) passes the `TempDir`'s own path, which on
Windows may have non-canonical casing — the assertion is on `message` only,
not on `description`, so this is fine on Windows.

## `#[cfg(feature = "local_fs")]` gating

The production helper `build_image_preview_entry` is NOT itself feature-
gated, and uses `std::fs::metadata` which is always available. So strictly
speaking the test gating is over-cautious. However, the gating matches the
file's convention (other tempfile-using tests in the same file are gated
the same way — see lines 23, 27, 33, 174, 279 etc.), and matches the gating
expected by the `tempfile` import at line 34. Keeping it consistent with
the file's existing pattern is fine. Not blocking.

## Test file location

`app/src/workspace/view_test.rs` is the canonical test module for view-
layer code (the file uses `use super::*` and is conventionally referenced
via `#[cfg(test)] mod view_test;` from `view.rs`). Correct location.

# What I checked

- `git show 32cee73` — full diff of helper extraction, const promotion,
  and the four added tests.
- `specs/GH9729/tech.md` lines 600-650 — confirmed the four prescribed
  test names character-for-character and the spec note that justifies the
  helper extraction.
- `app/src/workspace/view.rs` lines 5810-5830 (the new arm body),
  23825-23910 (the new module-scope consts, helper, and `truncate_message`
  + the disturbed `tab_bar_rects_for_window` doc).
- `grep` across `app/` and `crates/` for `truncate_message`,
  `MAX_PREVIEW_FILE_BYTES`, `MAX_ERROR_MESSAGE_LEN`,
  `build_image_preview_entry` — confirmed single callsite of
  `truncate_message`, single declaration site for each const, no shadowing.
- `app/src/workspace/view_test.rs` lines 1-50 (imports), 3015-3121 (the
  four new tests) — confirmed test conventions and assertion shape.
- The four test cases for spec compliance: case shape, sanitization
  assertions, description-field assertions, and feature gating.
- Production-behavior 1:1 preservation in the refactor (cap values,
  match-arm logic, log line, sanitized constants, symlink semantics).
- Cross-platform tempfile semantics for the four tests.

# Suggestions

1. (NIT, but real and easy) Fix the `tab_bar_rects_for_window` doc-comment
   regression introduced by inserting the new const+fn block mid-doc.
   Either restore the lost first sentence on `tab_bar_rects_for_window`
   and remove the orphan doc line glued to `MAX_PREVIEW_FILE_BYTES`, or
   move the new block to a less disruptive location (right above the arm
   that uses it would be cleanest).
2. (Nice-to-have follow-up) Add direct unit tests on `truncate_message`
   covering the ellipsis branch and the `max_len <= 1` degenerate branch,
   so the dead-in-production code does not silently rot.
3. (Optional) Add `assert!(entry.description.is_some())` to the
   `..._for_non_regular_file` test to keep the basename-only invariant
   under test for that branch as well.
4. (Optional, minor) Consider an additional sparse-file test for the
   over-size-cap branch using `set_len(MAX_PREVIEW_FILE_BYTES + 1)` so
   the production cap value is referenced directly. This would catch a
   future regression where the const value drifts from §119's 64 MiB.
5. (Tracking) Symlink-to-missing-target and non-UTF-8-path cases are not
   prescribed for v1; if they are not already in the GH9729 follow-ups
   list, add them there.

The commit is **pass-with-nits**: the prescribed four tests are correct,
named correctly, gated correctly, and the helper extraction preserves
production behavior 1:1. The only real defect is the
`tab_bar_rects_for_window` doc-comment hijack from the const+fn insertion;
the remaining items are coverage gaps and stylistic improvements that
should be tracked but do not block the commit.
