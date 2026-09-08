# Spec: Fix misleading "These files do not exist" error for oversized files in read_files

Linear: [APP-4882](https://linear.app/warpdotdev/issue/APP-4882/fix-misleading-these-files-do-not-exist-error-for-oversized-files-in)
Originating thread: https://warp-public.slack.com/archives/C0BDQDW8V5E/p1784468227182559
Estimate: M (3)

## Reconciliation with master (rebased)
This spec was originally written against `warpdotdev/warp@69ce372`, where the
shared result type still carried `missing_files: Vec<String>`. `master` has since
landed a partial fix: `ReadFileContextResult` now carries
`failed_files: Vec<ReadFilesFailedFile>` (where `ReadFilesFailedFile { path,
message }` lives in `crates/ai`), `read_files.rs` already returns partial success
(`ReadFilesResult::Success { files, failed_files }`) when only some files fail,
and `server_model.rs`'s `file_context_result_to_proto` already forwards each
`ReadFilesFailedFile.message` into the proto `FailedFileRead.error.message`.
What master did **not** fix is the root cause: `BinaryFileReadResult::Missing`
still conflates every failure into one generic "File not found or could not be
read" message, so oversized/unprocessable existing files are still reported as if
they do not exist. This change closes that gap while building on master's flat
`ReadFilesFailedFile { path, message }` type (the branch's original structured
`FileReadFailure`/`FileReadFailureReason` enum is dropped in favor of master's
shape).

## PRODUCT
**Summary:** When the agent's `read_files`/`get_files`/`search_codebase` tools
read a local file that *exists* but exceeds the per-file size cap (1 MB), the
client reports `These files do not exist: <path>`. That message is wrong — the
file exists and is readable, it is just too large. A real user hit this on a
3,476,751-byte JPEG that failed repeatedly while an 851 KB downscaled copy of the
same image succeeded. The root cause is that a single failure variant
(`BinaryFileReadResult::Missing`) conflates five distinct failure reasons into
one message. This change makes each file-read failure carry a reason-specific
message so every consumer of the shared code path reports an accurate, actionable
result.

**Key design choices:**
1. **Split the catch-all failure variant.** Replace `BinaryFileReadResult::Missing`
   with `NotFound`, `TooLarge { size_bytes, limit_bytes }`, and
   `ProcessingFailed { detail }`, matched exhaustively (no `_` arm) so a future
   variant forces every call site to be revisited.
2. **Produce a reason-specific message per file.** `read_local_file_context`
   maps each failure to a `ReadFilesFailedFile { path, message }` (master's flat
   type) where `message` is a concise, path-free reason. The `path` is carried in
   its own field, so every renderer that shows `"{path}: {message}"` stays clean
   (no duplicated path).
3. **One shared consumer helper.** All three agent-tool consumers (`read_files`,
   `get_files`, `search_codebase`) build their combined error via the shared
   `describe_failed_files(&[ReadFilesFailedFile])` helper (one `path: reason`
   entry per file), so they all surface the same accurate, per-file reason instead
   of a flat "do not exist" list.
4. **Keep `ProcessImageResult` API-compatible.** The image over-limit and image
   decode/resize failures both map to `ProcessingFailed`; `ProcessImageResult`
   itself is left unchanged (unit `TooLarge` variant) so its other callers (the
   TUI attachment path) are untouched.

**Behavior** (numbered, testable invariants from the agent/consumer's view):
1. **Oversized existing file → accurate "too large" message (default repro).**
   Reading an existing, readable binary file whose size exceeds the per-file
   limit reports it in `failed_files` with a message that states the file is too
   large and names both the file size and the limit — e.g.
   `File is too large to read (3.5 MB > 1.0 MB limit). Downscale/compress it or read a smaller copy.`
   It does **not** say the file does not exist.
2. **Genuinely missing file → "File does not exist".** A path that does not exist
   yields a `ReadFilesFailedFile` whose message is `File does not exist`.
3. **Image that fails processing → distinct "could not be processed" message.**
   When an image cannot be processed (decode/resize error, or still over the
   image send limit after resizing) the message is
   `File could not be processed as an image: <detail>`, distinct from both "too
   large" and "does not exist".
4. **Mixed batch reports every reason.** A single call including a missing file,
   an oversized file, and a processing-failure file surfaces all three via
   `describe_failed_files`, each naming its path with the correct reason — no
   reason dropped or mislabeled.
5. **Same accuracy across all agent-tool consumers.** `read_files`, `get_files`,
   and `search_codebase` all build their failure message from
   `describe_failed_files(&result.failed_files)`; none retains the misleading
   `These files do not exist:` prefix.
6. **Remote read path carries the reason too.** `file_context_result_to_proto`
   already forwards `ReadFilesFailedFile.message` into the proto
   `FailedFileRead.error.message` (unchanged on master), so once the messages are
   accurate the remote consumer surfaces the same accurate text.
7. **All existing successful reads are unaffected.** Text files, in-limit binary
   files, and images that process successfully are read and returned exactly as
   before; byte/batch budgeting and truncation are unchanged.

**Non-goals:**
- Changing master's partial-success behavior in `read_files` (some-failed still
  returns `Success { files, failed_files }`; all-failed returns `Error`).
  `get_files`/`search_codebase` keep their existing all-or-nothing behavior.
- Changing the 1 MB per-file cap (`MAX_FILE_READ_BYTES`) or the image size/pixel
  limits.
- Reworking `ProcessImageResult` into a struct variant, or any change to how
  successful files are read, truncated, or budgeted.

## TECH
**Files changed:**
1. **`app/src/ai/blocklist/action_model/execute.rs`**
   - Split `BinaryFileReadResult::Missing` into `NotFound`,
     `TooLarge { size_bytes, limit_bytes }`, `ProcessingFailed { detail }`.
   - `read_binary_file_context` returns the specific variant at each site:
     `file_size > max_bytes` and processed-content-still-over-limit →
     `TooLarge`; `FileLoadError::DoesNotExist` → `NotFound`;
     `ProcessImageResult::TooLarge` and `ProcessImageResult::Error` →
     `ProcessingFailed`.
   - `read_local_file_context` maps the metadata-`NotFound` site and each
     `BinaryFileReadResult` failure variant (exhaustively) to a
     `ReadFilesFailedFile { path, message }` with the reason-specific, path-free
     message. Adds a `format_mb` helper for the MB-with-one-decimal rendering.
   - Adds the shared, ungated `describe_failed_files(&[ReadFilesFailedFile]) -> String`
     helper used by the three consumers.
2. **`read_files.rs`, `get_files.rs`, `search_codebase.rs`** — build the combined
   failure message via `describe_failed_files(&result.failed_files)` (replacing
   the `These files do not exist: <paths>` blocks). `search_codebase` keeps its
   `SearchCodebaseFailureReason::InvalidFilePaths`.
3. **`passive_suggestions/legacy.rs`** — the failed-files breadcrumb now logs via
   `safe_warn!` (generic count-only message on release channels; full details
   only on dogfood/local) so absolute paths and per-file details are not uploaded
   in crash-reporting breadcrumbs.
4. **`server_model.rs`** — no change needed; `file_context_result_to_proto`
   already forwards `ReadFilesFailedFile.message` into the proto per-file error.

**Message wording / units:** the "too large" message renders both sizes in MB
with one decimal via `format_mb` (bytes / 1_000_000). Messages are path-free
because `ReadFilesFailedFile` carries `path` separately and every renderer
displays `"{path}: {message}"`; embedding the path in the message would duplicate
it.

## Validation & verification criteria (must ALL pass before merge)
1. **Reproduction fixed (regression test).** A unit test creates a temp binary
   file larger than the per-file limit and asserts `read_local_file_context`
   returns it in `failed_files` with a message containing the size, the limit,
   and "too large to read", and **not** "does not exist". This fails against the
   pre-change code (generic "not found or could not be read") and passes after.
   *Test: `oversized_binary_file_reports_too_large_not_missing` in
   `execute_tests.rs`.*
2. **NotFound says "File does not exist".** *Test:
   `missing_file_reports_does_not_exist`.*
3. **Processing failure is distinct.** A `.png`-named file with invalid image
   bytes under the size limit yields a "could not be processed as an image"
   message, distinct from "too large"/"does not exist". *Test:
   `unprocessable_image_reports_processing_failure`.*
4. **Batch helper names each reason per file.** *Test:
   `describe_failed_files_groups_each_reason_per_file`.*
5. **All three tool consumers use the shared helper.** `read_files`, `get_files`,
   and `search_codebase` build their error from `describe_failed_files`; none
   retains the literal `These files do not exist:` prefix. *Checked by code
   review of the three call sites.*
6. **Remote proto mapping carries the reason.** `file_context_result_to_proto`
   forwards `ReadFilesFailedFile.message` (unchanged); accurate producer messages
   flow through automatically.
7. **Successful reads unaffected.** Existing read/binary-detection tests still
   pass unchanged.
8. **No collateral damage / everything compiles.** Exhaustive matches on the new
   `BinaryFileReadResult` variants (no `_` arm); `passive_suggestions/legacy.rs`
   compiles. *Checked by `./script/presubmit`.*
9. **Presubmit passes.** `./script/format`,
   `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`,
   build, and tests are green.

## Notes for implementation
- This is **not** a distinct rendered UI surface: the affected output is the
  `read_files`/`get_files`/`search_codebase` tool-result message text (consumed
  by the agent/model). Its correctness is fully captured by the deterministic
  unit tests above, so no `computer_use`/screenshot proof is required.
- New unit tests live in `execute_tests.rs` (gated `all(test, feature = "local_fs")`),
  following the repo's `${filename}_tests.rs` convention.
