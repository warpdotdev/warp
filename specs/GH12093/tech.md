# Tech Spec: Render image previews in the code review panel

**Issue:** [warpdotdev/warp#12093](https://github.com/warpdotdev/warp/issues/12093)
**Product spec:** [`product.md`](./product.md)
**Researched at commit:** `118c6a4ef9d58110f11b75217d416ed269e9e8be`

## Context

The code review panel renders each changed file through `CodeReviewView::render_file_content`. Binary files short-circuit to a static "Binary file - no diff available" text node — this is the exact branch the feature replaces:

- [`code_review_view.rs (5214-5295) @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/code_review_view.rs#L5214-L5295) — `render_file_content`; the `if file.file_diff.is_binary` branch at L5231 is where image previews slot in (before the existing text placeholder).
- [`code_review_view.rs (2587-2720) @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/code_review_view.rs#L2587-L2720) — `build_view_state_for_file_diffs` builds each `FileState` from a `FileDiffAndContent`. Per-file view state (incl. the "Open file" button at L2646-L2660) is constructed here.
- [`code_review_view.rs:356 @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/code_review_view.rs#L356) — `FileState` (the per-file view struct).

Diff data model and how binary content is (not) loaded:

- [`diff_state/mod.rs (167-224) @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/diff_state/mod.rs#L167-L224) — `FileDiff` (`is_binary` at L176) and `FileDiffAndContent` (`content_at_head: Option<String>`, explicitly `None` for binary files, L210-L224).
- [`diff_state/local.rs:2342 @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/diff_state/local.rs#L2342) — `get_file_content_at_head`, which loads base content via `git show HEAD:<path>`. Binary files never reach it.
- [`warp_util/src/git.rs (8-58) @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/crates/warp_util/src/git.rs#L8-L58) — `run_git_command` decodes stdout with `String::from_utf8_lossy` (L45), which **corrupts binary bytes**. Reading image blobs needs a bytes-preserving variant.
- [`diff_state/remote.rs @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/diff_state/remote.rs) — remote/PR diffs arrive over the `GetDiffState` protobuf RPC carrying only text content. Out of scope (see `product.md` Non-goals).

Existing infrastructure to reuse (no new image stack needed):

- [`app/src/util/image.rs @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/util/image.rs) — `infer_mime_type` (magic-byte sniffing via `infer`, extension fallback via `mime_guess`) and `SUPPORTED_IMAGE_MIME_TYPES`.
- [`block.rs (6802-6816) @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/ai/blocklist/block.rs#L6802-L6816) — the canonical raw-bytes → image pattern: `AssetCache::insert_raw_asset_bytes::<ImageType>(id, &bytes, ctx)` then reference via `AssetSource::Raw { id }`. The WarpUI `Image` element renders it and supports `.first_frame_preview()`.
- Feature-flag pattern in this view: `FeatureFlag::GitOperationsInCodeReview.is_enabled()` ([`code_review_view.rs:1151 @ 118c6a4`](https://github.com/warpdotdev/warp/blob/118c6a4ef9d58110f11b75217d416ed269e9e8be/app/src/code_review/code_review_view.rs#L1151)).

## Proposed changes

Gate the entire feature behind a new `FeatureFlag::ImagePreviewInCodeReview` (add via the `add-feature-flag` skill). When the flag is off, behavior is byte-for-byte unchanged.

**1. Binary-safe git read — `crates/warp_util/src/git.rs`.**
Add `run_git_command_bytes(repo_path, args) -> Result<Vec<u8>>` that mirrors `run_git_command` but returns `output.stdout` without UTF-8 decoding (factor the shared `Command` setup so the existing string variant calls it and decodes). Keep the same exit-code handling.

**2. Image classification + byte loading — `app/src/code_review/diff_state/local.rs`.**
For a `FileDiff` with `is_binary == true` (flag on, local diffs only), attempt to load preview bytes:
- **Base side** (statuses other than new/untracked): `git show <rev>:<base_path>` via `run_git_command_bytes`, where `<rev>` mirrors the revision `get_file_content_at_head` already uses (HEAD or merge-base for branch mode). For renamed/copied files `<base_path>` is the file's **old path** (the pre-rename path from the diff), not the current `<path>` — otherwise `git show` misses the base-side blob; fall back to `<path>` when the file was not renamed/copied.
- **Working side** (statuses other than deleted): read `repo_path/<path>` from disk.
- Enforce the **≈10 MB cap per side** before decoding — use `git cat-file -s <rev>:<path>` for the base and `fs::metadata` for the working file; skip the side if over cap, marking it `TooLarge`.
- Classify each loaded buffer by its **contents only**, accepting only `image/png|jpeg|gif|webp` (raster only — SVG excluded). Do **not** reuse `infer_mime_type`'s `mime_guess` extension fallback: a text `.png` or a Git LFS pointer would resolve to `image/png` by extension despite not being an image. Acceptance requires magic-byte sniffing via the `infer` crate **and** a successful `image`-crate decode; any side that fails either check is rejected → no preview for that side (satisfying the misclassification guard). The successful decode also yields the dimensions for the metadata line.

Introduce `ImagePreviewData { old: Option<ImageSide>, new: Option<ImageSide> }` where `ImageSide` is one of:
- `Image { bytes: Arc<Vec<u8>>, mime, width, height, byte_len }` — a successfully classified image;
- `TooLarge { byte_len }` — the side exceeded the ≈10 MB cap;
- `Unavailable { reason: UnavailableReason }` — the side is expected for this status but could not be previewed, where `reason` distinguishes `ReadFailed` (blob/file couldn't be read) from `Rejected` (bytes failed the image-content check).

`Option<ImageSide>` is reserved for **side not applicable to the file's status** (`None` = no such side, e.g. no base side for an added file / no working side for a deleted file). This lets the view satisfy product spec §7: a modified image whose base blob is unreadable renders the new side and shows an explicit "other side unavailable" note, rather than silently collapsing "missing", "read failed", and "rejected" into one absent state.

Add `image_preview: Option<ImagePreviewData>` to `FileDiffAndContent` (it already carries the "expensive, do-not-clone" payload, so this is the natural home; `Arc` the bytes so the view can register them without a deep copy). `image_preview` is `Some` whenever at least one applicable side is `Image`, `TooLarge`, or `Unavailable`.

**3. View state — `code_review_view.rs`.**
In `build_view_state_for_file_diffs`, when `file.image_preview.is_some()`, register each `ImageSide::Image`'s bytes once via `AssetCache::insert_raw_asset_bytes::<ImageType>` with a stable id (`code-review-img-{old|new}-{path}@{rev}`), and store an `image_preview_state: Option<ImagePreviewState>` (the `AssetSource::Raw` ids + metadata + status) on `FileState`. No `editor_state` is built for image files.

In `render_file_content`, add a branch **before** the `is_binary` text placeholder (L5231): if `image_preview_state` is present, render the preview — per-status layout from `product.md` §2 (single image for added/deleted/renamed-unchanged; old + new for modified). Each side is rendered by its `ImageSide` kind, side-specifically:
- `Image` → an `Image::new(AssetSource::Raw { id }, CacheOption::Original).first_frame_preview()` plus a theme-styled metadata line (dimensions + size) and a status badge, wrapped in the existing `styled_file_content_container`.
- `TooLarge` → a **required**, side-specific "too large to preview" note (with the byte size) in place of that side's image. This is not optional and does not fall back to the generic binary placeholder: for a modified image with one oversized side, the other side still renders its image and the oversized side shows its note.
- `Unavailable` → a **required**, side-specific note ("other side unavailable"), satisfying product spec §7.

The generic `is_binary` text placeholder is reached only when the file has **no applicable previewable side at all** (i.e. `image_preview` is `None`) — e.g. a binary file that isn't a supported raster image. This keeps the graceful-fallback invariants without new error UI while guaranteeing the too-large/unavailable states are always surfaced per side.

**Tradeoff (byte loading location).** Bytes are loaded eagerly during diff computation (alongside `content_at_head`) rather than lazily in the view. This matches the existing content-loading pattern and keeps the view synchronous; the ≈10 MB cap and image-only restriction bound memory. A lazy `AssetSource::Async` fetch is possible but adds a loading-state path the product spec deliberately doesn't require.

## Testing and validation

Unit (`cargo nextest run`):
- `run_command_bytes` round-trips arbitrary binary (PNG fixture bytes in == bytes out), proving §1 doesn't corrupt blobs — foundational to `product.md` §1.
- Classification helper: PNG/JPEG/GIF/WebP magic bytes accepted; SVG, a text file with `.png` extension, and an LFS pointer rejected → `product.md` §8 (misclassification guard).
- Cap logic: a >10 MB side yields `TooLarge`; the other side still classifies → `product.md` §5.

Integration (`crates/integration`, Builder/TestStep — see `warp-integration-test` skill), with PNG fixtures committed to a scratch repo:
- Added / deleted / modified PNG render previews with the correct per-status content → `product.md` §1, §2; metadata line present, no "0/0" stat → §3.
- Corrupt/truncated image bytes fall back to the placeholder and the rest of the panel still renders → §6.
- Modified image with an unreadable base blob renders the new side and notes the old is unavailable → §7.
- Flag off → identical-to-today binary placeholder (regression guard).

## Risks and mitigations

- **Binary corruption via the string git path** — the dedicated `run_git_command_bytes` is the mitigation; never route image blobs through `run_git_command`.
- **Memory from buffering image bytes in the diff model** — bounded by the per-side ≈10 MB cap and the image-only restriction; `Arc` avoids copies into the view; the existing `TextureCache` evicts GPU textures for off-screen images.
- **"Open file" zoom assumption** — `product.md` defers full-size zoom on the premise that the existing "Open file" button (L2646) renders the image. This is **unverified**: the button emits `OpenInNewTab`, and it's not confirmed that opening an image file renders it (vs. raw/text). Verify during implementation; if it doesn't render, reopen the zoom decision as a follow-up rather than silently shipping a dead deferral.

## Parallelization

Not beneficial. The change is small and tightly coupled across four files in one crate (`git.rs` → `local.rs` → `mod.rs` → `code_review_view.rs`) with a strict data dependency (bytes accessor → loader/model → view). Parallel agents would collide on `code_review_view.rs` and serialize on the model change anyway. Implement sequentially on branch `MattSkala/code-review-image-preview` as a single PR.
