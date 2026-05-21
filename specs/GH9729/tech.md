# Image preview in the Code Editor file pane: Tech Spec

Product spec: `specs/GH9729/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/9729

## Context

Clicking an image file in the Code Editor file tree currently routes through `is_binary_file` (true for `.png/.jpg/.jpeg/.gif/.bmp/.tif/.tiff/.webp/.ico`) and falls out to `FileTarget::SystemGeneric`, which hands the file to the OS default app via `ctx.open_file_path`. SVG is text-classified and instead lands in `FileTarget::CodeEditor`, opening as raw XML. Neither outcome matches the user expectation of "preview this image inline in Warp."

Maintainer guidance in the issue thread is to dispatch the existing `Lightbox` overlay rather than build a new tab variant or a new image module. The Lightbox already supports a single-image render + Loading-placeholder + Escape/scrim/× dismissal model that v1 needs, and the `OpenLightbox` action handler already wires it to the workspace.

v1 is intentionally scoped to **a single, fully modal, single-image preview**. There is no sibling list, no preload window, no Left/Right navigation, no animated playback, no zoom. Each of those is tracked as a follow-up so v1 can ship without coupling to features that need more design or implementation work.

v1 is therefore five small things:

1. A new `FileTarget::ImagePreview` variant and an early-return branch in the file-target resolver so supported image extensions are captured ahead of both the markdown/code probe (so SVG is captured) and the `is_binary_file → SystemGeneric` fall-through (so raster formats are captured).
2. A new `Workspace::open_file_with_target` arm that performs a synchronous pre-read metadata check (regular-file check + size cap), then invokes a new `Workspace::open_lightbox` helper (extracted from the existing `WorkspaceAction::OpenLightbox` action arm) with a single-element `Vec<LightboxImage>`. If the pre-read check fails, the arm invokes the same helper with the entry's source set to the new `Error` variant so the user sees a per-entry inline error instead of nothing. The helper-call shape is required because this arm runs from a child-view subscription callback where `Workspace` is not in the action dispatcher's responder chain; the existing `WorkspaceAction::OpenLightbox` action remains in place for the artifacts and blocklist call sites that dispatch from focused-view contexts.
3. A new `LightboxImageSource::Error { message }` variant and corresponding render arm so per-entry decode/read failures surface in the Lightbox with the filename instead of spinning forever. Also extended into the asset-load callback so an `AssetState::FailedToLoad`, or a `Loaded { ImageType::Unrecognized }`, rewrites the entry to `Error`.
4. Decoder limits applied to `ImageType::try_from_bytes` via `image::Limits` plus an explicit total-pixel cap (post-decode for static, during-iteration for animated), and an SVG intrinsic-dimension cap applied after `usvg::Tree::from_data` parses. Frame-collection logic for `AnimatedBitmap` is unchanged in shape; the animated arms gain a frame-count cap and a total-pixel cap during iteration so they cannot be tricked into multi-gigabyte allocation.
5. A bounded LocalFile read in the asset cache (Unix `O_NONBLOCK` on open + post-open `is_file()` check on the opened descriptor + a 1 KB content peek that picks the byte cap from content rather than extension + `take(N)` streaming read) so a symlink to a special file (`/dev/zero`, FIFO), a path replaced with a FIFO/special-file/directory between change 2's pre-read stat and the open syscall, a TOCTOU growth between metadata and read, or a non-`.svg`-extension file whose contents are SVG XML, cannot bypass the pre-read cap. The `O_NONBLOCK` flag is required because `open()` of a FIFO with no writer attached blocks indefinitely on Linux/macOS, which would prevent the post-open `is_file()` check from ever running; with `O_NONBLOCK`, `open()` of a FIFO returns immediately and the post-open check rejects it before any read. The content peek replaces the round-6 extension-keyed cap selection: a `.png` containing 50 MB of nested `<g>` SVG would otherwise pass the 64 MB raster cap and reach the parser if `try_from_bytes` ever sniffs SVG by content. Content-keying the byte cap defends against that bypass independently of how `try_from_bytes` routes formats.

Everything else (sibling navigation, zoom, pan, footer, animation control in the Lightbox, EXIF, ICC, thumbnail strip, additional formats, magic-byte sniffing, context menu, drag-out, disk-backed thumbnail cache) is deferred to follow-ups, listed at the end of this spec.

## Relevant code

### Existing Lightbox plumbing (no structural change beyond the new `Error` variant in change 3)

- `crates/ui_components/src/lightbox.rs:28-34`: `pub enum LightboxImageSource { Loading, Resolved { asset_source: AssetSource } }`. v1 adds an `Error { message: String }` variant here.
- `crates/ui_components/src/lightbox.rs:38-43`: `pub struct LightboxImage { source: LightboxImageSource, description: Option<String> }`. v1 does not add fields.
- `crates/ui_components/src/lightbox.rs:55-85`: `pub struct Lightbox` and `pub struct Params<'a>` with `images: &[LightboxImage]`, `current_index`, `on_dismiss`, `current_image_native_size`, `options`. Renders close button, image via `Image::new(asset_source).contain()`, loading indicator, optional description. The chevron block at `lightbox.rs:218-275` is gated on `image_count > 1`, so v1's single-element slice naturally renders no navigation UI; the existing `escape`/`left`/`right` keybindings remain registered but `left`/`right` are inert with one image.
- `crates/ui_components/src/lightbox.rs:152`: the `Image::new(asset_source.clone(), CacheOption::Original).contain()` element. v1 does not call `enable_animation_with_start_time` here, so animated raster files render as a static first frame in the Lightbox (this is the existing behavior, unchanged).
- `app/src/workspace/lightbox_view.rs:30-36`: `pub struct LightboxParams { images: Vec<LightboxImage>, initial_index: usize }`.
- `app/src/workspace/lightbox_view.rs:39-44`: `pub enum LightboxViewEvent { Close, FocusLost }`.
- `app/src/workspace/lightbox_view.rs:15-27`: keybindings `escape → Dismiss`, `left → NavigatePrevious`, `right → NavigateNext` registered in `init`. v1 does not add any new arrow handling; with a single-element `images`, the existing `NavigatePrevious`/`NavigateNext` actions are inert.
- `app/src/workspace/lightbox_view.rs:69-129`: `LightboxView::new`, `update_params`, `update_image_at`, `start_asset_loads`, `start_asset_load`. `start_asset_loads` only kicks off loads for `Resolved` entries; `Loading` and `Error` entries do not start loads. Change 3 below refactors `start_asset_load` to take the entry index so the post-load callback can read state and rewrite to `Error` on failure.
- `app/src/workspace/lightbox_view.rs:124`: `ctx.spawn(future, |_me, (), ctx| { ctx.notify(); })`. The future itself is spawned on the background executor (`ctx.spawn` delegates to `background_executor().spawn_boxed` per `crates/warpui_core/src/core/view/context.rs:602-610`); the bytes-to-RGBA decode step inside the asset-cache pipeline runs on the foreground executor (see Performance posture below).
- `app/src/workspace/action.rs:602-612`: `WorkspaceAction::OpenLightbox { images: Vec<LightboxImage>, initial_index: usize }` and `WorkspaceAction::UpdateLightboxImage { index: usize, image: LightboxImage }`. The action remains in place for the artifacts and blocklist call sites; v1's new file-tree path invokes the extracted `Workspace::open_lightbox` helper directly (the action arm is refactored to forward to the same helper) and does not use `UpdateLightboxImage`.
- `app/src/workspace/view.rs:1028`: `lightbox_view: Option<ViewHandle<LightboxView>>` field on `Workspace`.
- `app/src/workspace/view.rs:21710-21737`: `OpenLightbox` action handler. Reuses an open `LightboxView` via `update_params` if one exists, else creates+subscribes+focuses. `LightboxViewEvent::FocusLost` clears the view; `LightboxViewEvent::Close` clears the view and calls `focus_active_tab`.
- `app/src/workspace/view.rs:22739-22740`: `lightbox_view` rendered as a child of the main render `Stack`. The new file-tree usage gets this for free.

### Modality contract (already enforced; v1 relies on this rather than re-implementing it)

- The Lightbox renders as the topmost child of the workspace's main `Stack`, with a full-window scrim. Pointer input lands on the scrim; file-tree, terminal, and code-editor panes underneath the scrim do not receive pointer events while the Lightbox is open. This is the same modality the existing screenshot/artifact Lightboxes rely on.
- Keyboard input is routed to the focused `LightboxView`; `escape` dispatches `Dismiss`.
- v1 does **not** introduce any "click another file in the File Tree to swap the Lightbox image" path. To open a different file the user must dismiss first (Escape, scrim click, or × button), then click. Because the file tree is behind the scrim while the Lightbox is open, accidental focus-stealing from the tree is not possible. This is the resolution of the modality contradiction flagged in earlier review rounds: v1 picks "fully modal" deterministically.

### Existing image-decode pipeline

- `crates/warpui_core/src/assets/asset_cache.rs:320-328`: `AssetSource::LocalFile { path }` flow. The load future is `async_fs::read(path).await?` and the result `Bytes` is sent to the decode step. **No file-size check before the read** in v1's pre-state; change 2 adds a synchronous pre-read cap on the new file-tree arm before the dispatch even happens, so the asset cache never starts a read on an oversize file.
- `crates/warpui_core/src/assets/asset_cache.rs:425-460`: load pipeline. Read happens on the background executor; the resulting bytes are sent through a channel; **`T::try_from_bytes(&bytes)` is invoked on the foreground executor** (line 460, inside `foreground_executor.spawn_boxed`). v1 inherits this behavior. See Performance posture for the implications.
- `crates/warpui_core/src/image_cache.rs:271-365`: `impl Asset for ImageType` `try_from_bytes`. SVG handled at lines 273-282 via `usvg::Tree::from_data` then `resvg`. Raster formats matched at lines 320-364 via `image::guess_format` + `image::ImageReader::with_format(...).decode().into_rgba8()`. WebP and GIF return `AnimatedBitmap` if `decoder.has_animation()` / has multiple frames. Unknown formats return `ImageType::Unrecognized`. **No `image::Limits` is applied today.** Change 4 adds `image::Limits` to the decode calls; frame collection (`AnimatedBitmap` construction) is unchanged.
- `crates/warpui_core/src/elements/image.rs:128-131`: `enable_animation_with_start_time(Instant)`. Called by `app/src/resource_center/section_views/changelog_section.rs` to drive animated rendering for changelog images. **Not called by the Lightbox.** v1 does not change either of these call sites.
- `app/src/util/image.rs`: agent-mode resize/validation utilities (`MAX_IMAGE_PIXELS = 1.15M`, `MAX_IMAGE_DIMENSION = 2000`, `MAX_IMAGE_SIZE_BYTES = 3.75 MB`). NOT inherited by the asset-cache decode path; cited only as the reference point for v1's caps.

### Existing file-tree → workspace open flow (the integration points)

- `app/src/code/file_tree/view.rs:2174-2215`: `fn open_file()` (under `#[cfg(feature = "local_fs")]`). Calls `resolve_file_target_with_editor_choice`, sends `TelemetryEvent::CodePanelsFileOpened { entrypoint: ProjectExplorer, target }`, emits `FileTreeEvent::OpenFile { path, target, line_col: None }`. v1 needs no change here; the new `FileTarget::ImagePreview` flows through unchanged.
- `app/src/code/file_tree/view.rs:2853-2877`: `enum FileTreeEvent` including `OpenFile { path, target, line_col }`.
- `app/src/server/telemetry/events.rs:483-487`: `pub enum CodePanelsFileOpenEntrypoint { CodeReview, ProjectExplorer, GlobalSearch }`. The entrypoint stays `ProjectExplorer`.
- `app/src/server/telemetry/events.rs:2308-2312`: `CodePanelsFileOpened` event carries `target: FileTarget`. The new variant on `FileTarget` distinguishes the destination via this existing field. No telemetry-enum change is needed.
- `app/src/workspace/view/left_panel.rs:758-768`: re-emits `LeftPanelEvent::OpenFileWithTarget`.
- `app/src/workspace/view.rs:5826-5838`: `LeftPanelEvent::OpenFileWithTarget` handler invokes `self.open_file_with_target(path, target, line_col, CodeSource::FileTree { path }, ctx)`.
- `app/src/workspace/view.rs:5715-5815`: `pub fn open_file_with_target(...)`. The match arms today cover `MarkdownViewer(layout)` (line 5739), `EnvEditor` (5744), `CodeEditor(layout)` (5796), `ExternalEditor(editor)` (5800), `SystemDefault` (5808), `SystemGeneric` (5811). The new arm slots in next to `MarkdownViewer`.
- `app/src/util/openable_file_type.rs:32-39`: `pub enum OpenableFileType { Markdown, Code, Text }`. Not changed.
- `app/src/util/openable_file_type.rs:42-57`: `pub enum FileTarget { MarkdownViewer(EditorLayout), CodeEditor(EditorLayout), ExternalEditor(Editor) [cfg local_fs], EnvEditor, SystemDefault, SystemGeneric }`. v1 adds `ImagePreview` here.
- `app/src/util/openable_file_type.rs:71-82`: `pub fn is_supported_image_file()` already matches `"jpg"|"jpeg"|"png"|"gif"|"webp"|"svg"` (case-insensitive). Reused as-is.
- `app/src/util/openable_file_type.rs:142-156`: `is_file_openable_in_warp()` returns `Option<OpenableFileType>`. Not changed; the resolver's new branch runs ahead of this.
- `app/src/util/openable_file_type.rs:194-233`: `resolve_file_target_with_editor_choice()`. v1 adds a new step 0 ahead of step 1.
- `crates/warp_util/src/file_type.rs:46-124`: `is_binary_file()`. Image extensions (`jpg/jpeg/png/gif/bmp/tiff/tif/webp/ico`) are listed as binary. Not changed; the new resolver branch short-circuits before this matters for the supported set.

## Proposed changes

### 1. Add `FileTarget::ImagePreview`

In `app/src/util/openable_file_type.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTarget {
    MarkdownViewer(EditorLayout),
    CodeEditor(EditorLayout),
    ImagePreview,
    #[cfg(feature = "local_fs")]
    ExternalEditor(Editor),
    EnvEditor,
    SystemDefault,
    SystemGeneric,
}
```

`ImagePreview` is a unit variant; no `EditorLayout` payload is needed because the Lightbox is an overlay, not a pane.

`OpenableFileType` is unchanged. The image probe is resolver-only.

In `resolve_file_target_with_editor_choice` (`app/src/util/openable_file_type.rs:194-233`), add a new step ahead of the markdown / code / binary chain:

```rust
pub fn resolve_file_target_with_editor_choice(
    path: &Path,
    editor_choice: EditorChoice,
    prefer_markdown_viewer: bool,
    default_layout: EditorLayout,
    layout: Option<EditorLayout>,
) -> FileTarget {
    // 0. Image preview takes precedence over text/binary classification so SVG
    //    (currently text-classified) and raster formats (currently binary) both
    //    land in the Lightbox.
    if is_supported_image_file(path) {
        return FileTarget::ImagePreview;
    }

    // ... existing steps 1-5 unchanged ...
}
```

Audit before merge: grep for any other call site that pattern-matches `FileTarget` exhaustively. The compiler will catch missing arms in `match`, but boolean-style `matches!(target, FileTarget::CodeEditor(_) | ...)` needs a manual sweep so `ImagePreview` is treated as "in-Warp, do not hand off to OS." Update the `target: FileTarget` serialization site under `app/src/server/telemetry/events.rs` so the new variant serializes as a stable string (e.g. `"image_preview"`) rather than being dropped or panicking.

### 2. Add the `FileTarget::ImagePreview` arm in `Workspace::open_file_with_target`

In `app/src/workspace/view.rs:5738-5814`, add an arm next to `MarkdownViewer(layout)`. The arm builds the entry via `build_image_preview_entry` and invokes a new `Workspace::open_lightbox(images, initial_index, ctx)` helper directly. The body of the existing `WorkspaceAction::OpenLightbox` action arm at `view.rs:21710-21737` is extracted into the same helper so the artifacts (`mod.rs:311`) and blocklist (`block.rs:6371`) call sites — which dispatch the action from focused-view contexts where `Workspace` is in the responder chain — continue to work unchanged. A direct call rather than `ctx.dispatch_typed_action(&WorkspaceAction::OpenLightbox)` is required here because this arm runs from a child-view subscription callback (file tree → left panel → workspace), so `Workspace` is not in the action dispatcher's responder chain at the moment of dispatch and the action would be silently dropped (`crates/warpui_core/src/core/app.rs:1490-1493` documents that walks stop at the first handler in the chain and explicitly do not propagate to parent views). The direct-call shape also matches the other arms in this match block (`open_code`, `open_file_notebook`, `attach_path_as_context`, `cd_to_directory`).

```rust
const MAX_PREVIEW_FILE_BYTES: u64 = 64 * 1024 * 1024; // 64 MB; one cap for raster and SVG
const MAX_ERROR_MESSAGE_LEN:  usize = 256;

FileTarget::ImagePreview => {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());

    // Synchronous metadata-only check: cheap on a warm filesystem cache,
    // and bounded I/O on a cold cache (one stat call). This runs before
    // any read so an oversize file never enters memory. `metadata`
    // follows symlinks, and `is_file()` rejects symlinks that resolve to
    // character devices (`/dev/zero`), FIFOs, sockets, or directories.
    let size_check: Result<(), &'static str> = match std::fs::metadata(&path) {
        Ok(meta) if !meta.is_file() => Err("not a regular file"),
        Ok(meta) if meta.len() > MAX_PREVIEW_FILE_BYTES => Err("image is too large to preview"),
        Ok(_) => Ok(()),
        // The OS error is logged for the operator (with `log::warn!`) and
        // collapsed to a sanitized constant here so absolute paths and
        // platform-specific error strings never reach the UI panel.
        Err(_) => Err("could not read image"),
    };

    let image = match size_check {
        Ok(()) => LightboxImage {
            source: LightboxImageSource::Resolved {
                asset_source: AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                },
            },
            description: filename,
        },
        Err(message) => LightboxImage {
            source: LightboxImageSource::Error {
                message: truncate_message(message, MAX_ERROR_MESSAGE_LEN),
            },
            description: filename,
        },
    };

    self.open_lightbox(vec![image], 0, ctx);
}
```

`truncate_message` is a small local helper that bounds the message length so a long error string cannot occlude the close button or trigger expensive text shaping.

Notes on this shape:

- **One unified pre-read cap.** Earlier drafts split the cap by file extension (`.svg` got a smaller cap). That is bypassable: `try_from_bytes` classifies SVG by content (first byte `<`, see `image_cache.rs:275`), not by extension, so a `.png` whose contents start with `<` would be routed through `usvg` while passing the looser raster cap. v1 uses one cap for everything pre-read; the SVG-specific allocation surface is bounded by the SVG intrinsic-dimension cap added in change 4 instead.
- **Single-element `Vec`.** `OpenLightbox` accepts `Vec<LightboxImage>`; v1 always passes a one-element vec. The Lightbox's chevron block at `lightbox.rs:218-275` is gated on `image_count > 1`, so no nav UI renders. Escape, scrim click, and × dismiss as today.
- **Why `std::fs::metadata` and not async.** The metadata stat is a single syscall; on warm filesystem cache it returns in microseconds. On cold cache or stalled NFS/sshfs/FUSE it can stall the workspace foreground thread for the FS timeout. v1 accepts this stall as a tradeoff for a small synchronous arm; the existing `MarkdownViewer` and `EnvEditor` arms in `open_file_with_target` do not currently do their own stats, so this arm is a new (small) source of foreground FS work. Moving the metadata check to the background executor and dispatching `OpenLightbox` from a future is enumerated as a follow-up; the synchronous version is shipped first because it keeps the new arm small and within the shape of the existing match block.
- **Why dispatch the Error variant inline rather than abort.** If the user clicked an oversize image, opening nothing is worse UX than opening the Lightbox with an inline "this file is too large to preview" message; the latter is unambiguous feedback that the action was received. The same path is reused for `Err` from `metadata` and, in change 3, for downstream read/decode failures.
- **Sanitized error message.** The Error variant's `message` is one of a small set of constant strings; the underlying OS error is logged via `log::warn!` for the operator but never reaches the UI. This avoids leaking absolute filesystem paths (which often include usernames) and platform-specific error syntax into screenshots or screen-shares.
- **No new fields on `LightboxImage`.** The single-image v1 does not need `path`, `format`, or any other carrier for navigation logic; that was a sibling-navigation requirement and is dropped along with the feature.

### 3. Add `LightboxImageSource::Error` and surface it

In `crates/ui_components/src/lightbox.rs`, extend the enum:

```rust
#[derive(Clone, Debug)]
pub enum LightboxImageSource {
    Loading,
    Resolved { asset_source: AssetSource },
    Error { message: String },
}
```

In `Lightbox::render` (the per-image render branch around lines 152-189), add an `Error` arm that renders a non-blocking error panel showing the entry's `description` (filename) on one line and the `message` on the next. Style consistent with the existing close-button panel; use the existing UI tokens. The panel does not block dismissal; Escape / scrim / × continue to work.

In `app/src/workspace/lightbox_view.rs`, refactor the asset-load callback shape so it can rewrite the entry on failure or on `Unrecognized`. This is a small refactor, not a one-liner:

- Today, `start_asset_load` is an associated function (`Self::start_asset_load(asset_source, ctx)`) called from inside the `images.iter()` loop in `start_asset_loads` (lines 108-114). The future returned by `handle.when_loaded(asset_cache)` resolves to `()`; the spawn callback is `|_me, (), ctx| { ctx.notify(); }` and has no entry index in scope.
- Refactor `start_asset_load` to a method on `&mut self` that takes the entry index alongside the `AssetSource`. Capture `(asset_source, index)` into the spawn callback so the callback can re-query the asset cache for the post-load state and mutate `self.params.images[index]`.
- Inside the callback, after `ctx.notify()`, look up the asset state via `AssetCache::as_ref(ctx).load_asset::<ImageType>(asset_source)`. The state transitions are observable as:
  - `AssetState::Loaded { data: ImageType::Unrecognized }` → rewrite to `LightboxImageSource::Error { message: "could not detect image format" }`. This closes the rough edge where a mislabeled file (e.g. a `.png` containing tarball bytes) currently caches `Unrecognized` and renders a permanent spinner. The rewrite is local to `LightboxView`; the global `try_from_bytes` is unchanged.
  - `AssetState::FailedToLoad(err)` → rewrite to `LightboxImageSource::Error { message: sanitize(err) }` where `sanitize` collapses to a small set of categorical strings (`"could not read image"`, `"could not decode image"`, `"image is too large to preview"`) and never interpolates the raw error or any path. The original error is logged with `log::warn!` for the operator.
  - All other states leave the entry unchanged.
- `update_params` and `LightboxView::new` (lines 69-105) call `start_asset_loads`, which calls `start_asset_load` per entry; both call sites need to pass `&mut self` rather than the current `Self::` associated-function pattern. This is the load-bearing refactor; the rest of the change is straightforward.
- For the `Loading` case before any state transition, the existing render path is unchanged; the loading indicator continues to display while bytes are read and decoded.
- `current_image_native_size` in `render` (lightbox_view.rs:150-165) is unaffected: `Error` entries return `None` for native size, which the existing render logic already tolerates.

This is the only `LightboxImageSource` change. Without it, the per-entry error state cannot render: today's render falls through to the loading element on `AssetState::FailedToLoad` and spins forever. The artifacts call site (`app/src/ai/artifacts/mod.rs:362-365`) silently works around this by stuffing "Failed to load" into the `description` while leaving `source: Loading`; that is a UX bug we inherit if we do not add `Error` here. Adopting `Error` at the artifacts site is listed as a small follow-up.

#### Accessibility (descoped from v1)

The product spec's earlier draft promised that the active image's filename would be exposed as the rendered Image element's accessible label. Verification against the codebase showed the GPUI `Image` element at `crates/warpui_core/src/elements/image.rs` does not currently expose any accessibility-label API (no `accessible_label`, no `with_aria_label`, no `set_accessibility`); the close button at `lightbox.rs:128-143` similarly does not set one. Wiring an accessibility label on the Lightbox image therefore requires first extending the `Image` element with a builder method and connecting its paint pass to register an AX node. That is non-trivial GPUI plumbing outside v1's scope.

v1 ships without the rendered Image's accessible label. The keyboard-only entry path (tree navigation + Enter, Escape to dismiss) remains accessible via existing system keyboard navigation. The filename is rendered as the visible `description` slot in the Lightbox, which is announced when focus enters that text node, but is not announced as the image's own label. Adding the GPUI a11y plumbing and wiring it through the Lightbox is enumerated as the first accessibility follow-up; the product spec is updated to reflect this v1 scope.

### 4. Add decoder limits and total-pixel caps to `ImageType::try_from_bytes`

In `crates/warpui_core/src/image_cache.rs`, the existing decode paths call `image::ImageReader::with_format(...).decode()` without `image::Limits`. A 65535×65535 PNG decompression bomb decodes to ~16 GB RGBA via `into_rgba8()` and OOMs the process. JPEG, static WebP, animated WebP, animated GIF, and SVG each have their own version of this attack.

v1 closes them with three coordinated bounds, sized so that the dimension cap and the allocation cap are **internally consistent** and so that the alloc cap is the binding constraint:

```rust
const MAX_DECODE_DIMENSION:     u32 = 8_192;             // any single dim
const MAX_DECODE_PIXELS:        u64 = 67_108_864;        // 8192 * 8192
const MAX_DECODE_ALLOC:         u64 = 256 * 1024 * 1024; // ~MAX_DECODE_PIXELS * 4 bytes RGBA
const MAX_ANIMATED_FRAMES:      usize = 256;
const MAX_ANIMATED_TOTAL_PIXELS: u64  = 67_108_864;       // 8192 * 8192 across all frames
const MAX_SVG_RENDER_DIMENSION: u32   = 8_192;
```

8192 is well above 4K (3840×2160) and is sufficient for the 99th-percentile project asset, screenshot, and changelog GIF/WebP. RGBA at 8192² = 256 MB, which matches `MAX_DECODE_ALLOC`; this avoids the earlier draft's 16384 dim + 512 MB alloc combination, which was self-inconsistent (16384² × 4 = 1 GB, exceeding the alloc cap so the dimension cap was effectively dead code).

#### Static raster (PNG, JPEG, WebP-static)

```rust
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width  = Some(MAX_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODE_DIMENSION);
    limits.max_alloc        = Some(MAX_DECODE_ALLOC);
    limits
}

fn decode_static_with_limits(data: &[u8], format: image::ImageFormat) -> anyhow::Result<image::RgbaImage> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    reader.limits(decode_limits());
    let img = reader.decode()?;
    let pixels = (img.width() as u64).saturating_mul(img.height() as u64);
    if pixels > MAX_DECODE_PIXELS {
        anyhow::bail!("image is too large to preview");
    }
    Ok(img.into_rgba8())
}
```

`ImageReader::limits` is the documented bound for `decode()` in `image` 0.25.x (`crates/.../image-0.25.9/src/io/image_reader_type.rs:137`). The post-decode `pixels > MAX_DECODE_PIXELS` check is a belt-and-suspenders guard for formats where a decoder might honor `max_image_width/max_image_height` but not refuse a near-cap input that still materializes a near-cap RGBA buffer.

#### Animated WebP and animated GIF

The `image` 0.25.x animated decoders are weaker than the static path. Verified against `image-0.25.9`:

- `GifDecoder` overrides `set_limits` (`codecs/gif.rs:100`) and enforces the dimension cap on `into_frames()`, but does **not** enforce `max_alloc` per frame.
- `WebPDecoder` does not override `set_limits` (`codecs/webp/decoder.rs:21`), so it inherits the trait default that only `check_dimensions`. `max_alloc` is **not** enforced for animated WebP at all.

`image::Limits` alone therefore cannot be relied on to bound animated decode. v1 closes this with an explicit frame budget applied during iteration, before any frame is collected into the output `Vec`:

```rust
fn decode_animated_with_limits(
    data: &[u8],
    format: image::ImageFormat,
) -> anyhow::Result<Vec<image::Frame>> {
    let mut frames = Vec::new();
    let mut total_pixels: u64 = 0;

    // Construct decoder with set_limits applied to bound dimensions and
    // anything the decoder honors. (Animated WebP / GIF may ignore max_alloc.)
    let frame_iter = match format {
        image::ImageFormat::Gif => {
            let mut dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(data))?;
            dec.set_limits(decode_limits())?;
            dec.into_frames()
        }
        image::ImageFormat::WebP => {
            let mut dec = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(data))?;
            dec.set_limits(decode_limits())?;
            dec.into_frames()
        }
        _ => unreachable!(),
    };

    for (i, frame) in frame_iter.enumerate() {
        if i >= MAX_ANIMATED_FRAMES {
            anyhow::bail!("animated image has too many frames");
        }
        let frame = frame?;
        let buf = frame.buffer();
        let pixels = (buf.width() as u64).saturating_mul(buf.height() as u64);
        total_pixels = total_pixels.saturating_add(pixels);
        if total_pixels > MAX_ANIMATED_TOTAL_PIXELS {
            anyhow::bail!("animated image exceeds total pixel budget");
        }
        frames.push(frame);
    }

    if frames.is_empty() {
        anyhow::bail!("animated image has no frames");
    }
    Ok(frames)
}
```

This bounds peak allocation at `MAX_ANIMATED_TOTAL_PIXELS * 4 bytes ≈ 256 MB` regardless of format-decoder honesty. Frame-collection logic still produces an `AnimatedBitmap` with all frames in the legitimate case; only the pathological-input case fails fast.

Survey of current animated consumers, with measured envelopes:

- **Changelog section** (`app/src/resource_center/section_views/changelog_section.rs:147-162`): renders inside a `ConstrainedBox::with_max_height(200.).with_max_width(350.)` and drives animation via `enable_animation_with_start_time(Instant::now())`. Source assets are bundled or fetched at modest dimensions; their per-frame pixel count and total frame count are orders of magnitude below the v1 caps. Verified: the limits do not regress changelog playback.

The animated-image regression flagged in earlier review rounds was about *changing frame-collection behavior* (decoding only frame 0 of an `AnimatedBitmap`). v1 does **not** make that change. `AnimatedBitmap` continues to be constructed for animated WebP / GIF input, and the changelog continues to animate exactly as today. The new failure mode is "pathological animated input that exceeds the cap returns `Err`," which the changelog never triggers.

#### SVG

For SVG, three bounds stack to defend the parser plus the renderer. The 64 MB unified pre-read cap from earlier drafts of change 2 is too loose to feed `usvg::Tree::from_data`, because XML-style parsers exhibit super-linear cost in input size: a 4 MB SVG with thousands of deeply nested `<g>` groups, gradients, filters, or path elements consumes far more parser CPU and intermediate memory than the file size implies, and `usvg` does not expose a parse-budget API today. The intrinsic-dimension cap below also cannot help, because the parse is what allocates the parser's intermediate trees — by the time `usvg::Tree::from_data` returns, the damage is done.

The bounds, applied in order:

1. **SVG-specific pre-parse byte cap (`MAX_SVG_BYTES = 4 * 1024 * 1024`).** Applied in change 5's bounded read on the asset-cache side: bytes whose first 1 KB peek matches `looks_like_svg_xml` are read with a 4 MB ceiling (`MAX_SVG_BYTES + 1`) instead of the 64 MB raster ceiling. The cap is keyed on the **content** of the file, not its extension, so a non-`.svg` file whose contents are SVG XML is also tightened to 4 MB; an over-cap SVG returns `Err` from the asset-cache read step before bytes reach `try_from_bytes`. 4 MB is roomy for any legitimate icon, diagram, or illustration SVG (real-world SVGs are typically a few hundred KB; even maximalist illustration SVGs rarely exceed 1-2 MB) while preventing a 64 MB SVG from being handed to `usvg`.

2. **Content-sanity prefix check on the bytes that reach `try_from_bytes`.** Before `usvg::Tree::from_data` runs, the same `looks_like_svg_xml` helper used by change 5 to pick the byte cap is invoked one more time on the buffer that reached the decoder, and the buffer is rejected if it fails. The check confirms a UTF-8 prefix consistent with XML/SVG: optional UTF-8 BOM, optional whitespace, then one of the supported XML/SVG prelude tokens. This is the second of two uses of the helper: change 5 calls it on the file-side peek to **pick the cap**, and change 4 calls it on the in-memory buffer to **gate the parser**. Reusing the same predicate keeps the cap signal and the parser-gate signal aligned by construction. The check catches the "binary blob renamed to `.svg`" case (which would otherwise either parse-fail expensively or waste cycles in `usvg`'s error paths) and is cheap (a bounded byte scan, no allocation):

   ```rust
   fn looks_like_svg_xml(data: &[u8]) -> bool {
       // Strip optional UTF-8 BOM.
       let bytes = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
       // Skip leading ASCII whitespace, bounded to the first 1 KB to keep
       // the scan O(1) and to refuse pathological "1 GB of whitespace" inputs.
       let scan_end = bytes.len().min(1024);
       let after_ws = bytes[..scan_end]
           .iter()
           .position(|b| !b.is_ascii_whitespace())
           .map(|i| &bytes[i..])
           .unwrap_or(&[]);
       after_ws.starts_with(b"<?xml")
           || after_ws.starts_with(b"<svg")
           || after_ws.starts_with(b"<!--")
           || after_ws.starts_with(b"<!DOCTYPE")
   }
   ```

   **Supported prelude subset.** `looks_like_svg_xml` accepts the following prelude forms (after an optional UTF-8 BOM and bounded ASCII whitespace, capped at the first 1 KB):

   - `<?xml` — XML declaration
   - `<svg` — bare SVG root element
   - `<!--` — XML comment block (e.g. authoring-tool credits such as `<!-- generated by Inkscape -->`)
   - `<!DOCTYPE` — DOCTYPE declaration (e.g. `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" ...>`)

   Anything else (binary blobs, non-XML markup, JSON, etc.) is rejected before reaching `usvg::Tree::from_data`. The predicate is intentionally a coarse content sniff, not a full XML lexer; the actual XML/SVG validation is `usvg`'s job. Files that begin with non-standard preludes (XML processing instructions other than `<?xml`, leading data attributes, lowercase `<!doctype`, etc.) are out of scope and will be reported via the per-entry decode error. XML is case-sensitive and the standard form of both tokens is uppercase (`<!DOCTYPE`) or fixed-case (`<?xml`, `<!--`), so case-insensitive matching is not attempted.

3. **Intrinsic-dimension cap on the parsed tree.** Same as before; closes the "200-byte SVG declares 200000×200000" attack on the renderer:

   ```rust
   if !looks_like_svg_xml(data) {
       anyhow::bail!("svg content does not look like XML");
   }
   let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
   let size = tree.size();
   let w = size.width()  as u32;
   let h = size.height() as u32;
   if w > MAX_SVG_RENDER_DIMENSION || h > MAX_SVG_RENDER_DIMENSION {
       anyhow::bail!("svg dimensions exceed render budget");
   }
   let pixels = (w as u64).saturating_mul(h as u64);
   if pixels > MAX_DECODE_PIXELS {
       anyhow::bail!("svg dimensions exceed render budget");
   }
   // ... existing rasterization ...
   ```

`usvg` 0.47 disables network and external-entity expansion by default; this cap covers the remaining concrete render-time exposure.

This layered approach (file-size cap → content-sanity prefix → intrinsic-dimension cap → renderer) is the correct response while `usvg`/`resvg` do not expose a parse-budget API. If a future `usvg` release exposes such an API (parse-time CPU budget, max element count, max nesting depth), that becomes the right primary defense and the 4 MB byte cap can relax. Tracked as a follow-up.

#### Survey of consumers (full)

Verification that the new bounds do not regress any current surface:

- **Changelog section**: covered above. Behavior unchanged.
- **Artifact / screenshot Lightboxes** (`app/src/ai/artifacts/mod.rs`): screenshots are server-bounded to dimensions far under the cap. Behavior unchanged.
- **Bundled UI assets** (`AssetSource::Bundled`): Warp-shipped assets are far under the cap. Behavior unchanged.
- **Agent attachment / inline preview path** (`app/src/util/image.rs`): already bounded by stricter agent-mode caps (1.15M pixels, 2000 dim, 3.75 MB) before bytes reach `try_from_bytes`. Behavior unchanged.

In all four cases the bounds are invisible because no current input approaches them. The bounds exist to defend a future or out-of-band caller (such as v1's new file-tree path) that pipes user-supplied bytes through the same decoder.

#### `ImageType::Unrecognized` and the file-tree spinner

The catch-all `_` arm of `match image::guess_format(data)` returns `Ok(ImageType::Unrecognized)` for any unknown format. `AssetCache` caches this state by source path, so a future click on the same path returns the cached `Unrecognized` and the Lightbox spins again with no recovery short of a process restart.

v1 does not change global `try_from_bytes` (so other consumers of `Unrecognized` are unaffected). Instead, the post-load callback added in change 3 inspects the loaded state and, when it observes `AssetState::Loaded { ImageType::Unrecognized }` for a Lightbox entry, rewrites the entry to `LightboxImageSource::Error { message: "could not detect image format" }`. This closes the spinner-on-mislabeled-file rough edge for the Lightbox surface; the cache still holds `Unrecognized`, so re-clicking lands in the rewritten Error state immediately rather than spinning. A follow-up (listed below) is to convert `Unrecognized` to `Err` globally with a full audit of every consumer.

### 5. Bound the LocalFile asset-cache read

In `crates/warpui_core/src/assets/asset_cache.rs:320-328`, the `AssetSource::LocalFile { path }` load future is `async_fs::read(path).await`, which has no size cap. Combined with the change-2 pre-read metadata check, this leaves four surfaces:

- A symlink to a special file (`/dev/zero`, FIFO) where `metadata` does not give a meaningful `len()`. Change 2's `is_file()` check rejects most of these, but `async_fs::read` on a FIFO that someone managed to point a regular-file path at can still hang or read until OOM.
- TOCTOU file-type swap: between `metadata()` in the workspace arm and `open()` in the asset cache, the path can be swapped to a FIFO. On Linux/macOS, `open(O_RDONLY)` on a FIFO with no writer attached **blocks indefinitely** waiting for a writer; this means the post-open `is_file()` check (introduced in round 5) never runs, because `open()` itself is the blocking point. The defense therefore needs to make the open call non-blocking on Unix.
- TOCTOU growth: between `metadata()` and `read()`, the file can grow (or be replaced with a larger file) and the unbounded read consumes whatever is on disk at read time.
- **Extension-vs-content mismatch on the SVG cap.** Round 6 keyed the SVG-tighter cap (4 MB) on the `.svg` extension. `try_from_bytes` today routes SVG-vs-raster by content (`image_cache.rs:275` peeks the first byte for `<`), not by extension. A 50 MB file named `evil.png` whose contents start with `<svg ...>` would currently pass the 64 MB raster cap, the bytes would reach `try_from_bytes`, and the SVG branch (or any future content-sniffed SVG router) would receive up to 64 MB. The byte cap therefore needs to be keyed on the same signal `try_from_bytes` uses (content), not on extension.

Replace the unbounded read with a bounded one, apply `O_NONBLOCK` on the open call so a FIFO returns immediately rather than hanging, re-validate the regular-file contract on the **opened handle** (not the path) before any byte is read, and pick the byte cap from a small **content peek** (not from the extension). Doing the `is_file()` check on the opened `File` (via `file.metadata()`, which on POSIX is `fstat` against the open descriptor) closes the TOCTOU window between change 2's pre-read path-based stat and the open syscall: even if the path was swapped to a FIFO, character device, or directory between those two syscalls, the swap cannot affect a check performed against an already-open descriptor. A FIFO that was opened — which would otherwise hang the `open` call until a writer attaches, then hang `read_to_end` until the writer side closes, or stream until OOM — is caught here and rejected before any read is attempted. The `is_file()` rejection on the opened handle ALSO covers the "directory was opened" edge case on platforms where `File::open` succeeds for directories. Content-keying the byte cap closes the bypass where SVG XML hides under a non-`.svg` extension: a 1 KB peek + `looks_like_svg_xml` (the same content-sanity helper used in change 4) selects the 4 MB SVG cap whenever the first bytes look XML-shaped, regardless of file extension. This is independent of any future change to `try_from_bytes`'s routing strategy: even if the decoder is later changed to content-sniff into `usvg`, the parser cannot receive more than 4 MB.

```rust
const MAX_ASSET_LOCAL_FILE_BYTES: u64 = 64 * 1024 * 1024; // matches MAX_PREVIEW_FILE_BYTES (raster cap)
const MAX_SVG_BYTES:              u64 = 4  * 1024 * 1024; // tighter SVG-specific pre-parse cap

async {
    // Apply O_NONBLOCK on Unix so open() of a FIFO returns immediately
    // instead of blocking indefinitely waiting for a writer. Without this,
    // a FIFO swapped in between change 2's pre-read stat and this open
    // syscall would hang the asset-cache load future before the post-open
    // is_file() check can run. O_NONBLOCK has no effect on regular-file
    // reads on POSIX: read(2) on a regular file completes fully regardless
    // of the flag, so no fcntl-clear step is needed before take()/read_to_end.
    // On Windows, OpenOptionsExt::custom_flags exists but the equivalent
    // FIFO/named-pipe failure mode does not apply the same way; the
    // post-open is_file() check is sufficient there. The non-blocking flag
    // is therefore gated on #[cfg(unix)].
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let file = {
        let mut opts = async_fs::OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        opts.custom_flags(libc::O_NONBLOCK);
        opts.open(&path).await?
    };

    // Post-open regular-file check on the opened descriptor (fstat, not
    // path-based stat). This is required in addition to change 2's pre-read
    // metadata check: it closes the TOCTOU window where the path was
    // replaced (e.g. with a FIFO, character device, or directory) between
    // the pre-read stat and this open syscall. The check is on the open
    // file descriptor, so a path swap after open cannot affect it.
    let meta = file.metadata().await?;
    if !meta.file_type().is_file() {
        anyhow::bail!("local asset is not a regular file");
    }

    // Pick the byte cap from CONTENT (not extension). Peek the first 1 KB
    // and run the same `looks_like_svg_xml` helper used in change 4: if the
    // bytes look XML/SVG-shaped, apply the tighter 4 MB cap so a parser
    // (usvg or any future content-sniffed router) cannot receive more than
    // 4 MB of XML, regardless of file extension. Otherwise apply the 64 MB
    // raster cap. The peek itself is bounded; the full read still includes
    // the peeked bytes, so no work is wasted.
    //
    // Content-keying is required because `try_from_bytes` routes SVG-vs-raster
    // by content (image_cache.rs:275) rather than by extension. An
    // extension-keyed cap (round 6) would let a 50 MB file named `evil.png`
    // whose first bytes are `<svg ...>` pass the 64 MB cap and feed the
    // parser. With content-keying, that file caps at 4 MB before any byte
    // reaches `try_from_bytes`.
    use futures_lite::{AsyncReadExt as _, AsyncSeekExt as _};

    let mut peek = [0u8; 1024];
    let n = file.read(&mut peek).await?;
    let cap = if looks_like_svg_xml(&peek[..n]) {
        MAX_SVG_BYTES
    } else {
        MAX_ASSET_LOCAL_FILE_BYTES
    };

    // Buffer the peeked bytes and continue reading without re-seeking.
    // This avoids relying on AsyncSeek, which `async-fs` 2.x supports but
    // which we do not need: we already have the first `n` bytes, and the
    // remaining read is bounded by `cap + 1 - n`. (If a future implementation
    // prefers seeking, `file.seek(SeekFrom::Start(0)).await?` + a single
    // `take(cap + 1)` is an equivalent shape.)
    let mut buf = Vec::with_capacity(n);
    buf.extend_from_slice(&peek[..n]);
    let remaining = (cap + 1).saturating_sub(buf.len() as u64);
    let mut taken = file.take(remaining);
    taken.read_to_end(&mut buf).await?;
    if buf.len() as u64 > cap {
        anyhow::bail!("local asset exceeds size cap");
    }
    Ok(buf.into())
}
```

The post-open `is_file()` check is **in addition to**, not a replacement for, `O_NONBLOCK`, the pre-read size cap from change 2, and the `MAX + 1` bounded read. `O_NONBLOCK` ensures `open()` of a FIFO returns immediately so the post-open check actually runs; the post-open check rejects the FIFO before any read is attempted. Change 2 still rejects oversize regular files synchronously before dispatch (avoiding even an open syscall on the obvious-bad cases), and the bounded read still defends against a regular file that grows past the cap between the pre-read stat and the open. The dual-cap design (64 MB for raster, 4 MB for SVG) bounds the bytes fed to `usvg::Tree::from_data` more tightly because XML parsers have super-linear cost in input size.

`looks_like_svg_xml` is the same helper introduced in change 4 (a bounded scan over up to 1 KB that strips an optional UTF-8 BOM, skips ASCII whitespace, and matches one of `<?xml`, `<svg`, `<!--`, or `<!DOCTYPE`). Reusing it here keeps the SVG-vs-raster cap signal aligned with the SVG parser-routing signal: any byte sequence that the parser would treat as SVG is the same byte sequence the byte cap treats as SVG. Both layers (this change-5 cap-selection peek and the change-4 in-memory parser gate) call into the single helper, so broadening the supported prelude set here automatically widens both layers in lock-step. The helper allocates nothing and is O(1) in the file size.

Reading `MAX + 1` bytes and comparing afterward (rather than reading exactly `MAX`) makes the cap deterministically detect a file whose actual on-disk size exceeded the cap between the metadata stat and the read. That is the TOCTOU close.

#### Other `LocalFile` consumers and rollout impact

This change applies globally to `AssetSource::LocalFile`, so every consumer inherits the 64 MB raster / 4 MB SVG content-keyed cap. The current in-tree consumers and their legitimate-input envelopes:

- **Image preview in the file tree** (this PR's target). Per change 2 the dispatch arm pre-rejects above 64 MB, so this consumer never produces a `LocalFile` source that hits the cap.
- **Markdown image embeds in the editor / notebooks** (`crates/editor/src/content/edit.rs:71-104`, via `resolve_asset_source_relative_to_directory`). Embeds raster or SVG via local path. Typical file size is under a few MB (icons, screenshots, diagrams). 64 MB raster / 4 MB SVG comfortably covers any legitimate embed; a real markdown render that wants to embed a 70 MB raster is overwhelmingly likely a mistake or a misuse, not a legitimate workflow. No regression expected.
- **Custom theme background images** (`app/src/themes/theme.rs:279`, `crates/warp_core/src/ui/theme/mod.rs:80`). Themes ship with a single background image rendered behind the workspace. Theme authors typically use compressed JPEGs in the 0.5-5 MB range; an existing theme with a >64 MB background would already be a memory pressure concern at runtime. No regression expected. The theme deletion preview path (`app/src/themes/theme_deletion_body.rs:85`) re-points at the same `LocalFile` source, so it inherits the same envelope.
- **Settings profile image** (`app/src/settings_view/main_page.rs:417`). Single avatar; small file. Way under cap. No regression expected.
- **Blocklist images** (`app/src/ai/blocklist/block/view_impl/common_tests.rs:279-291` and the production code path it covers). Small icons / inline images. Way under cap. No regression expected.

In every case the legitimate envelope is at least an order of magnitude below the raster cap and well below the SVG cap. Bundled assets, changelog images, and artifact / screenshot Lightboxes do not use `LocalFile` (they use `Bundled`, URL-fetched `Async`, or in-memory `Raw`), so they are unaffected by this change in any case.

Rollout choice: keep a **single global cap** rather than parameterizing `AssetSource::LocalFile` with a per-call `max_bytes`. Parameterizing would expand the public surface of `AssetSource` and require every consumer to make a deliberate choice about caps; the survey shows no consumer needs to do so. If a future consumer genuinely needs larger inputs (e.g. a video-preview surface, a large bundled-export viewer), the right move at that point is to add a `LocalFile { path, max_bytes: Option<u64> }` field with `None` preserving the unbounded legacy behavior and the image-preview path opting in to `Some(MAX_ASSET_LOCAL_FILE_BYTES)`. v1 does not do this because it would introduce an enum-shape change with no current beneficiary.

The follow-up list at the end of this spec adds an item for "parameterize `LocalFile` with optional `max_bytes`" so this option is documented if it is ever needed.

### 6. No change to `crates/warp_util/src/file_type.rs`

Image extensions in `is_binary_file()` (lines 53-61) stay binary. The new resolver branch in change 1 short-circuits before that check is reached for the supported set. SVG remains text-classified; the new branch routes it to `ImagePreview` first regardless. Audit before merging that no other call site assumes "binary ⇒ `SystemGeneric`" without going through `resolve_file_target_with_editor_choice` (the audit is the same one called out in change 1).

### 7. Telemetry

No telemetry-enum change. The new `FileTarget::ImagePreview` variant flows through the existing `TelemetryEvent::CodePanelsFileOpened { entrypoint: ProjectExplorer, target }` event at `app/src/code/file_tree/view.rs:2202-2208`. The dashboards filter on the `target` field, which already serializes the `FileTarget` variant.

Concrete acceptance criterion: after change 1's serialization update, a click on `screenshot.png` produces a `CodePanelsFileOpened` event with `target = "image_preview"` (or whatever string-form the existing serializer uses for `FileTarget` variants; the test in change 1's audit confirms this). Image opens are then distinguishable from `MarkdownViewer`, `CodeEditor`, `SystemGeneric` opens via that single field. No additional event, no additional enum, no aggregation change.

If product later wants to enumerate additional fields on the event (file extension, file size bucket, decode duration), those are additive changes to `TelemetryEvent::CodePanelsFileOpened` and out of scope here.

## Performance posture

Bytes are read on the background executor (the asset-cache load future runs there). The bytes-to-RGBA decode itself runs on the **foreground executor** (`AssetCache::load_asynchronously` invokes `try_from_bytes` at `crates/warpui_core/src/assets/asset_cache.rs:460`). v1 does **not** move decode to the background executor.

Implications and the v1 mitigation:

- A click on a small image (a few MB, normal dimensions) decodes in well under one frame; the user sees the loading indicator briefly or not at all.
- A click on a file at the extreme of the caps (8192 × 8192, 64 MB on disk, 256 MB allocated RGBA) could take a couple hundred milliseconds to decode, during which the foreground executor is occupied. The loading indicator is visible for that duration. Click-to-content latency in this worst case is bounded by the dimension and pixel caps; the user cannot trigger an unbounded-duration decode because anything above the caps is rejected at the pre-read step or at decode time.
- Moving decode to the background executor is a follow-up that affects every Lightbox surface (artifacts, screenshots, file-tree previews) and every asset-cache consumer. It is intentionally not entangled with this v1.

The product spec promises only that "the loading indicator shows while bytes are being read and decoded," which the existing pipeline already provides. v1 makes no claim about async-decode responsiveness.

The synchronous `std::fs::metadata` in change 2 is also on the foreground executor. On a stalled NFS/sshfs/FUSE mount this can freeze the workspace for the FS timeout. v1 accepts this as a tradeoff to keep the new arm small; the metadata-on-background follow-up is enumerated below.

## End-to-end flow

1. User clicks `screenshot.png` in the file tree.
2. `FileTreeView::open_file` (`app/src/code/file_tree/view.rs:2174-2215`) calls `resolve_file_target_with_editor_choice`. Step 0 (change 1) matches `is_supported_image_file` and returns `FileTarget::ImagePreview`.
3. Telemetry is recorded as `CodePanelsFileOpened { entrypoint: ProjectExplorer, target: FileTarget::ImagePreview }`.
4. `FileTreeEvent::OpenFile { path, target: ImagePreview, line_col: None }` is emitted, re-emitted as `LeftPanelEvent::OpenFileWithTarget` by `left_panel.rs:758-768`, and handled by `Workspace::open_file_with_target` at `view.rs:5715-5815`.
5. The new `ImagePreview` arm (change 2) calls `std::fs::metadata` for the pre-read check (regular-file + size cap). On success it builds a single-element `Vec<LightboxImage>` with `source: Resolved { LocalFile { path } }` and invokes `Workspace::open_lightbox(images, 0, ctx)` directly. On metadata error, non-regular-file, or oversize file it builds the same single-element vec with `source: Error { message }` (a sanitized constant) and invokes the same helper.
6. `Workspace::open_lightbox` (the body extracted from the original `OpenLightbox` action arm at `view.rs:21710-21737`) creates `lightbox_view` and focuses it. The scrim covers the workspace; pointer input is intercepted by the scrim; the file tree, terminal panes, code editor panes, and tab bar are all inert. `LightboxView::start_asset_loads` queues `AssetCache::load_asset` for the single `Resolved` entry (no-op for `Error`).
7. The asset cache opens the file with `O_NONBLOCK` on Unix (change 5) so that `open()` of a FIFO swapped in via TOCTOU returns immediately rather than hanging waiting for a writer; the post-open `fstat`-based `is_file()` then rejects any non-regular-file descriptor before any byte is read. For regular files, the load future peeks the first 1 KB of the opened descriptor and runs `looks_like_svg_xml` on the peek to pick the byte cap from **content** rather than extension: `cap` is `MAX_SVG_BYTES` (4 MB) when the peek matches XML/SVG shape (regardless of file extension, so SVG content under `.png` or any other extension still tightens to 4 MB), and `MAX_ASSET_LOCAL_FILE_BYTES` (64 MB) otherwise. The load future then reads up to `cap + 1` bytes via `take()` (the peeked bytes are buffered in, not re-read), and bails if the cap was exceeded. The bytes are sent through the asset-cache channel and `try_from_bytes` runs on the foreground executor with the static-decode limits, the animated frame budget, the SVG content-sanity prefix check, and the SVG intrinsic-dimension cap from change 4. The Lightbox renders as a child of the workspace's main `Stack` (`view.rs:22739-22740`); the image fits via `Image::new(asset_source).contain()`. Until the bytes have been decoded, the loading indicator is shown. Animated GIF and animated WebP files render their first frame statically because the Lightbox does not call `enable_animation_with_start_time` (no v1 change to this).
8. If the asset load fails (`AssetState::FailedToLoad`) or resolves to `Loaded { ImageType::Unrecognized }`, the post-load callback (change 3) rewrites the entry to `LightboxImageSource::Error { message: <sanitized> }`. The Lightbox re-renders into the per-entry error panel showing the filename and the message. No crash; the user can dismiss.
9. Escape, scrim click, or × emits `LightboxViewEvent::Close`. The handler at `view.rs:21722-21726` clears `lightbox_view` and calls `focus_active_tab(ctx)` to restore focus to the previously-active pane.

## Risks and mitigations

### Pre-read size envelope (Critical, addressed in changes 2 and 5)

Without a pre-read cap, a 1 GB `.png` triggers a 1 GB `async_fs::read` allocation on the asset-cache background executor before any decoder limit kicks in (`asset_cache.rs:325`). Three coordinated changes close this:

- Change 2 caps file size via `std::fs::metadata` plus `is_file()` before invoking the Lightbox, so `Workspace::open_lightbox` is never called with a `Resolved` source for an oversize file or for a symlink to a special file (`/dev/zero`, FIFO). On rejection, the `Error` variant is passed inline with a sanitized constant message.
- Change 5 caps the actual read in the asset cache via `take(MAX + 1)`, so a TOCTOU growth between metadata stat and read cannot bypass the cap, and additionally re-validates the regular-file contract on the opened descriptor (`fstat`-based `is_file()` rather than path-based) so a path replacement between change 2's stat and the open syscall — for example a regular file replaced with a FIFO, character device, or directory — is caught before any byte is read.
- Change 5 also applies `O_NONBLOCK` on the open syscall on Unix. Without this, `open(path, O_RDONLY)` on a FIFO with no writer attached blocks indefinitely on Linux/macOS, which would prevent the post-open `is_file()` check from ever running — the open call itself is the blocking point. With `O_NONBLOCK`, `open()` of a FIFO returns immediately, the post-open `fstat`-based `is_file()` rejects the FIFO, and the load future surfaces `Err` to the Lightbox as the per-entry error state without hanging the asset-cache executor. POSIX specifies that `O_NONBLOCK` does not affect `read(2)` on regular files (regular-file reads always complete fully regardless of the flag), so no fcntl-clear is needed before `take()`/`read_to_end`. On Windows, the equivalent FIFO/named-pipe failure mode does not apply the same way, so the flag is gated `#[cfg(unix)]` and the post-open `is_file()` check alone covers Windows.

The dual-cap design (raster vs SVG) sets the unified raster cap at 64 MB, which comfortably covers normal photos and project assets while rejecting pathological inputs. The SVG-specific cap at 4 MB is tighter because XML parsers (`usvg`) have super-linear cost in input size and a 64 MB SVG with deeply nested elements consumes far more parser memory than the file size implies; 4 MB is roomy for any legitimate icon, diagram, or illustration SVG.

The SVG cap is **content-keyed**, not extension-keyed: change 5 peeks the first 1 KB and runs `looks_like_svg_xml` (the same helper change 4 uses ahead of `usvg::Tree::from_data`) to pick the cap. This defends against a non-`.svg` extension hiding SVG content. A 50 MB file named `evil.png` whose first bytes are `<svg ...>` caps at 4 MB at the read step, before a single byte reaches `try_from_bytes`. This is independent of `try_from_bytes`'s routing strategy: even if a future change made `try_from_bytes` content-sniff into `usvg`, the parser cannot receive more than 4 MB. Earlier review rounds proposed extension-keying; that approach was discarded because `try_from_bytes` in `image_cache.rs:275` already routes by content (`<` as the first non-whitespace byte), so extension-keying the byte cap and content-keying the parser route diverge in exactly the case the cap is meant to defend.

### Decoder allocation envelope (Critical, addressed in change 4)

Without `image::Limits`, a 65535×65535 PNG decompression bomb (or its WebP / JPEG / animated equivalent) decodes to many gigabytes of RGBA before the `image` crate returns. Change 4 applies three coordinated bounds:

- Static raster (PNG, JPEG, static WebP) goes through `ImageReader::limits` with 8192 × 8192 dimensions and 256 MB alloc, plus a post-decode `pixels > MAX_DECODE_PIXELS` belt-and-suspenders check. The dimension and alloc bounds are internally consistent (`8192² × 4 bytes = 256 MB`).
- Animated WebP / GIF goes through `set_limits` on the decoder (which only reliably enforces dimensions in `image` 0.25.x), plus an explicit frame-count cap (`MAX_ANIMATED_FRAMES = 256`) and a total-pixel cap accumulated **during iteration** (not after `collect_frames`). This bounds peak allocation for animated decode at ~256 MB regardless of decoder honesty.
- SVG goes through an intrinsic-dimension cap on `usvg::Tree` immediately after parsing, before rasterization. This closes the "200-byte SVG declares 200000×200000" attack that no byte cap can address.

`AnimatedBitmap` continues to be produced for legitimate animated input; the changelog continues to animate. The new failure mode is "pathological input returns `Err`," which the changelog never triggers.

### Foreground-executor decode (acknowledged, accepted for v1)

Decode runs on the foreground executor. Combined with the caps, worst-case decode time for the file-tree path is bounded but can reach a couple hundred milliseconds for an 8192 × 8192 input. The product spec does not promise async-decode responsiveness; the loading indicator is the visible affordance. Moving decode to a background executor is enumerated as a follow-up that affects all Lightbox call sites, not just file-tree.

### Foreground-executor metadata stat (acknowledged, accepted for v1)

The `std::fs::metadata` call in change 2 also runs on the workspace foreground thread. On a stalled NFS/sshfs/FUSE mount this can freeze the workspace for the FS timeout (typically tens of seconds, sometimes longer). Other arms in `open_file_with_target` (`MarkdownViewer`, `EnvEditor`) do not currently do their own stats; this arm is a new (small) source of foreground FS work. v1 accepts the tradeoff to keep the new arm in the existing match-block shape; the metadata-on-background follow-up is enumerated below.

### Modality is fully strict; no "click-to-swap" path

Earlier drafts described a flow where clicking a second file in the File Tree dismissed the active Lightbox via `FocusLost` and reopened cold on the new file. That flow exposed a contradiction with the "underlying panes do not receive input" promise: the file tree both was-and-was-not interactive while the Lightbox was open. v1 resolves this by being unambiguously modal: while the Lightbox is open, all underlying input is intercepted by the scrim, including the file tree. Opening a second image requires Escape (or scrim click or ×) first. This is a small UX cost relative to deferring the contradiction; sibling navigation, when it lands as a follow-up, will design the swap interaction deliberately rather than emerging from a focus-stealing side effect.

### `LightboxImageSource::Error` is a public-API change in `ui_components`

Adding a new variant to a public enum is a breaking change for any external consumer. There are none today. All in-tree consumers are updated as part of change 3: `crates/ui_components/src/lightbox.rs` render, `app/src/workspace/lightbox_view.rs` asset-load callback. Adopting `Error` at the artifacts call site (`app/src/ai/artifacts/mod.rs:362-365`) is a separate small follow-up.

### `SystemGeneric` regression for non-image binary files

The new resolver branch is gated strictly on `is_supported_image_file(path)`. Non-image binary extensions (`.zip`, `.mp3`, `.exe`, `.pdf`, `.bmp`, `.tiff`, `.ico`) skip the new branch and continue to fall through to `SystemGeneric` exactly as before. Covered by unit tests in the validation section.

### Telemetry distinguishability

Verified above (change 7): the existing event already carries `target: FileTarget`. The audit in change 1 confirms the variant is serialized as a stable string so the dashboards can filter on it; the unit test in the validation section asserts the serialized form.

### SVG decode

SVG is rendered via `usvg` 0.47 + `resvg` in `ImageType::Svg` (`image_cache.rs:271-282`). `usvg` 0.47 disables network and external-entity expansion by default. The render-time exposure for a small-but-pathological SVG (e.g. `<svg width="200000" height="200000">`) is closed by the `MAX_SVG_RENDER_DIMENSION` cap applied to `tree.size()` immediately after `usvg::Tree::from_data` parses (change 4). The parser-time exposure for a large-but-pathological SVG (e.g. 4 MB of deeply nested `<g>` groups consuming far more parser memory than file size implies) is closed by the **content-keyed** 4 MB byte cap applied in change 5's bounded read (any file whose first 1 KB looks like SVG XML caps at 4 MB regardless of extension, including a `.png` whose contents are SVG), plus the content-sanity prefix check in change 4 that rejects non-XML inputs before they reach the parser. `usvg`/`resvg` do not currently expose a parse-time CPU/memory budget API; if a future release adds one, it becomes the right primary defense and the byte cap can relax. Smoke-test with one well-formed SVG fixture, one pathological-intrinsic-dimension fixture (e.g. `<svg width="200000" height="200000">...</svg>`), one over-cap (>4 MB) deeply-nested fixture, one binary-blob renamed `.svg` fixture, and one SVG-content-under-`.png`-extension fixture (4.5 MB of SVG XML named `evil.png`) to exercise the content-keyed cap.

### Mislabeled-file rough edge (closed at the file-tree path)

A `.png` file that contains tarball bytes (or any extension/content mismatch) resolves to `AssetState::Loaded { ImageType::Unrecognized }`, cached by path. v1 closes this for the file-tree Lightbox surface by inspecting the post-load state in change 3's callback and rewriting the entry to `Error { "could not detect image format" }` when `Unrecognized` is observed. The cache still holds `Unrecognized`, so a re-click on the same path lands in the rewritten Error state immediately rather than spinning. Other `try_from_bytes` consumers continue to see `Unrecognized` as today; converting it to `Err` globally is enumerated as a follow-up.

### Accessibility plumbing (descoped from v1)

The product spec previously promised a screen-reader label on the Lightbox image element. Verification against `crates/warpui_core/src/elements/image.rs` showed no accessibility-label API exists on the GPUI Image element; the close button at `lightbox.rs:128-143` similarly does not register one. v1 ships without the rendered Image's a11y label; the keyboard-only entry path remains accessible, and the filename is rendered as the visible `description` slot. Adding the GPUI a11y plumbing is the first accessibility follow-up listed below.

## Testing and validation

### Unit tests

`app/src/util/openable_file_type.rs` (new test module section):

- `resolve_file_target_image_preview_for_each_supported_extension`: each of `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg` (lower and upper case) resolves to `FileTarget::ImagePreview`, regardless of `prefer_markdown_viewer` and `editor_choice`.
- `resolve_file_target_image_preview_takes_precedence_over_markdown`: a `.svg` file resolves to `ImagePreview` even when `prefer_markdown_viewer = true`.
- `resolve_file_target_non_image_binary_still_system_generic`: `.zip`, `.mp3`, `.exe`, `.pdf`, `.bmp`, `.tiff`, `.ico` resolve to `SystemGeneric`.

`app/src/workspace/view.rs` (or a small extracted helper module to avoid pulling the whole view crate into a test):

- `image_preview_arm_builds_resolved_when_under_size_cap`: with a temp file of 1 KB, `build_image_preview_entry` returns one entry whose source is `Resolved` (the helper is the unit-testable boundary; the arm forwards its output to `Workspace::open_lightbox`).
- `image_preview_arm_builds_error_when_over_size_cap`: with a temp file (sparse) of `MAX_PREVIEW_FILE_BYTES + 1`, `build_image_preview_entry` returns one entry whose source is `Error` and whose message is the sanitized constant.
- `image_preview_arm_builds_error_when_metadata_fails`: with a path that does not exist, the helper returns `Error`.
- `image_preview_arm_builds_error_for_non_regular_file`: with a path that resolves to a directory or a FIFO (where supported by the test runner), the helper returns `Error`.

`crates/warpui_core/src/assets/asset_cache.rs` (new test module section):

- `local_file_read_caps_at_max_bytes`: with a temp file slightly larger than `MAX_ASSET_LOCAL_FILE_BYTES`, the load future returns `Err` and the read does not allocate beyond the cap.
- `local_file_read_passes_under_cap`: with a temp file of 1 KB, the load future returns the bytes.
- `local_file_read_rejects_post_open_non_regular_file`: open a regular-file path, race-replace it with a FIFO (or character device, or directory) between the pre-read metadata stat and the asset-cache open call, and confirm the post-open `is_file()` check on the opened descriptor returns `Err` before any read is attempted. On platforms without convenient FIFO support in the test runner, simulate by opening a path that already resolves to a non-regular file and asserting the post-open check fires.
- `local_file_read_does_not_block_on_fifo` (`#[cfg(unix)]`): create a FIFO via `nix::unistd::mkfifo` (or `libc::mkfifo`) in a temp directory with NO writer attached, pass that path directly to the asset-cache `LocalFile` load future, and confirm the future returns `Err` (the post-open `is_file()` rejection) within a small bounded duration (e.g. under 100 ms). The test guards specifically against regression of the `O_NONBLOCK` flag: without it, this future would hang indefinitely and the test would time out. Mark the test `#[cfg(unix)]` since `mkfifo` is POSIX.
- `local_file_read_caps_svg_at_smaller_limit`: with a temp file named `huge.svg` containing slightly larger than `MAX_SVG_BYTES` (4 MB) of valid SVG XML, confirm the load future returns `Err` (the SVG cap fires from the content peek). With the same SVG byte payload renamed `huge.bin` (no extension match), confirm the load future still returns `Err` (content-keying, not extension-keying, picks the 4 MB cap).
- `local_file_read_caps_svg_content_under_png_extension`: write 4.5 MB of SVG XML (e.g. `<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg">` followed by `<g/>` repeated until ~4.5 MB) to a temp file named `evil.png`, pass to the asset-cache `LocalFile` load future, and confirm `Err` (the SVG cap fires despite the `.png` extension). Without the round-7 content-keying fix this test fails because the file passes the 64 MB raster cap and the bytes flow to `try_from_bytes`. With the fix, the 1 KB peek returns `<?xml ...><svg ...>`, `looks_like_svg_xml` returns true, and the cap selection picks 4 MB before any further read happens.
- `local_file_read_uses_raster_cap_for_non_svg_content`: write 4.5 MB of valid PNG bytes to a temp file named `large.svg`, pass to the load future, and confirm `Ok` (the content peek does not match SVG, the 64 MB raster cap applies, and the file passes despite the misleading extension). This is the symmetric assertion that content-keying does not over-tighten on non-SVG inputs whose extensions look SVG-shaped.

`crates/warpui_core/src/image_cache.rs` (new test module section):

- `decode_static_rejects_dimensions_over_cap`: a synthesized PNG header declaring `20000 × 20000` returns `Err`.
- `decode_static_rejects_pixels_over_cap`: a synthesized PNG header declaring `9000 × 8000` (72 megapixels, above `MAX_DECODE_PIXELS = 67M`) returns `Err`.
- `decode_static_accepts_normal_photo`: a 4000×3000 PNG decodes successfully.
- `decode_animated_rejects_too_many_frames`: a fixture animated GIF declaring `MAX_ANIMATED_FRAMES + 1` frames returns `Err` after iterating the cap.
- `decode_animated_rejects_total_pixel_budget`: a fixture animated WebP whose frames sum above `MAX_ANIMATED_TOTAL_PIXELS` returns `Err` and bails partway through iteration (the test asserts the error fires before the full frame set is materialized).
- `decode_animated_constructs_bitmap_for_legitimate_input`: a small animated GIF with N frames returns `Ok(AnimatedBitmap)` with N frames preserved (regression check; asserts change 4 did not change frame-collection behavior and the changelog continues to animate).
- `decode_animated_constructs_bitmap_for_legitimate_webp`: same shape for animated WebP.
- `decode_svg_rejects_intrinsic_dimensions_over_cap`: an SVG of `<svg width="200000" height="200000">...</svg>` parses then bails at the intrinsic-dimension cap.
- `decode_svg_accepts_normal_icon`: a 256×256 SVG fixture renders successfully.
- `decode_svg_rejects_non_xml_prefix`: a 1 KB byte buffer that starts with `\x00\x00\x00` (or any non-XML prefix) but is fed through the SVG path returns `Err` from the content-sanity check before `usvg::Tree::from_data` is invoked. Asserts the prefix check fires.
- `decode_svg_accepts_xml_prelude_with_bom_and_whitespace`: a buffer that starts with the UTF-8 BOM (`\xEF\xBB\xBF`), then `\n  `, then `<?xml version="1.0"?><svg ...>`, passes the content-sanity check.
- `decode_svg_accepts_doctype_prelude`: a buffer that starts with `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">\n<svg xmlns="http://www.w3.org/2000/svg" ...>` passes the content-sanity check (asserts the DOCTYPE prelude form is recognized so legitimate SVGs declaring a DTD are not rejected).
- `decode_svg_accepts_xml_comment_prelude`: a buffer that starts with `<!-- generated by Inkscape -->\n<svg xmlns="http://www.w3.org/2000/svg" ...>` passes the content-sanity check (asserts the XML comment prelude form is recognized so SVGs from common authoring tools are not rejected).

`crates/ui_components/src/lightbox.rs` (new test module section):

- `lightbox_renders_error_variant_with_filename_and_message`: snapshot or property check that the Error arm renders both the description and the message.

`app/src/workspace/lightbox_view.rs` (new test module section):

- `post_load_callback_rewrites_failed_to_load_to_error`: simulate `AssetState::FailedToLoad` and assert the entry's source becomes `Error` with a sanitized message.
- `post_load_callback_rewrites_unrecognized_to_error`: simulate `AssetState::Loaded { ImageType::Unrecognized }` and assert the entry's source becomes `Error { "could not detect image format" }`.

### Manual validation

Behavior-to-step mapping (numbered against the product spec's Success criteria):

1. **Open each format**: click each of `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg` from the file tree. Confirm the Lightbox opens, no new tab is created, no session-restore artifact appears on relaunch. Open the same image via tree-arrows + Enter.
2. **Modality**: with the Lightbox open on `screenshot.png`, click anywhere in the File Tree, click a terminal pane, click a Code Editor tab, click the tab bar. Confirm none of these clicks have any effect: the Lightbox stays on `screenshot.png`, no other file opens, no focus moves.
3. **Dismiss paths and focus**: dismiss via Escape, scrim click, and × button; in each case focus returns to the previously-active tab pane.
4. **Read / decode failure**: open a corrupt PNG fixture and confirm the Lightbox shows the per-entry error with the filename. Delete the file from outside Warp between dispatch and read; confirm the same error path is taken.
5. **Pre-read size cap**: create a 100 MB `.png` (`truncate -s 100M huge.png`) and click it. Confirm the Lightbox opens directly into the per-entry error state, and confirm via Activity Monitor / Process Memory that no 100 MB allocation occurred. The error message is the sanitized constant; no path or OS error string appears.
6. **Symlink to special file**: create `ln -s /dev/zero zero.png` in a project directory and click it. Confirm the Lightbox opens directly into the per-entry error state ("not a regular file") and that no read is attempted; verify that no memory growth occurs.
7. **TOCTOU growth**: pass the metadata stat with a small file, then grow the file beyond the cap before the asset cache reads (race window is short; can be reproduced with a wrapper script that grows the file in a tight loop). Confirm the bounded read in change 5 detects the over-cap byte count and dispatches Error.
7a. **TOCTOU file-type swap**: pass change 2's `is_file()` stat with a regular file, then race-replace the path with a FIFO (or character device, or directory) before the asset cache opens it. Confirm the post-open `is_file()` check on the opened descriptor in change 5 rejects the read before any byte is consumed (no hang, no OOM, error variant rendered in the Lightbox).
8. **Decoder dimension and pixel caps**: open a 10000×10000 PNG. Confirm the Lightbox shows the per-entry error citing the size, not a partial render or an OOM. Repeat with a 5000×5000 PNG (below the cap, 25 megapixels) and confirm it renders normally.
9. **Animated formats**: open an animated GIF and an animated WebP from the file tree. Confirm each renders its first frame statically (no continuous playback in the Lightbox). Then navigate to the changelog page in the Resource Center and confirm changelog GIFs/WebPs continue to animate (regression check on change 4).
10. **Pathological animated input**: open an animated GIF with 500 frames (above `MAX_ANIMATED_FRAMES`). Confirm the per-entry error state. Open an animated WebP whose frames sum above `MAX_ANIMATED_TOTAL_PIXELS` and confirm the same; observe via memory inspection that the error fires before the full frame set is materialized.
11. **SVG**: open a small well-formed SVG and confirm it renders (not raw XML). Open `<svg width="200000" height="200000">...</svg>` and confirm the per-entry error state ("svg dimensions exceed render budget") rather than a multi-GB pixmap allocation.
11a. **SVG byte-cap and parser DoS**: generate a `deeply_nested.svg` whose total byte size is **above** `MAX_SVG_BYTES = 4 MB` — for example 4.5 MB containing roughly 50000 opened `<g>` elements (`<?xml ...?><svg xmlns=...>` followed by enough `<g/>` repetitions to push the file past 4.5 MB). Open it from the file tree and confirm the per-entry error fires from the **content-keyed asset-cache cap** before `usvg::Tree::from_data` is ever invoked: the 1 KB peek matches `looks_like_svg_xml`, the bounded read selects 4 MB, the read returns `Err` past the cap, and no parse is attempted. Expected outcome: per-entry error state in the Lightbox, no parse run. With Activity Monitor / `time` / RSS observation, confirm the workspace foreground thread does not spike for hundreds of milliseconds, and total resident memory does not climb to multi-hundred-MB during the dispatch. Then repeat the same payload renamed `evil.png` (still 4.5 MB of SVG XML) and confirm the same outcome: the cap is content-keyed, so the rejection fires regardless of extension. Then **add the under-cap counter-fixture**: generate a 100 KB well-formed SVG with normal structure (e.g. a typical icon or diagram) and confirm it parses normally and renders in the Lightbox — this verifies content-keying does not over-tighten on legitimate SVGs. Then generate a 5 MB binary blob renamed `data.svg` (e.g. `dd if=/dev/urandom of=data.svg bs=1M count=5`) and open it; confirm the per-entry error fires either from the SVG byte cap (if the random first 1 KB happens to peek as XML and is over 4 MB) or, more typically, from the content-sanity prefix check after a 64 MB-ceilinged read (random bytes do not match `looks_like_svg_xml`), in either case before the parser runs.
11b. **FIFO open does not hang** (POSIX-only): in a project directory, run `mkfifo /tmp/preview-fifo.png` (no writer attached), then click the path from the file tree. Confirm the Lightbox shows the per-entry error within a fraction of a second, not a multi-second or indefinite hang. This validates the `O_NONBLOCK` defense: without `O_NONBLOCK`, the asset-cache `open()` call would block on `read()` of the FIFO from a non-existent writer until interrupted.
12. **Mislabeled file**: create a `.png` whose contents are a tarball or random bytes. Click it. Confirm the per-entry error state ("could not detect image format") rather than a permanent spinner. Click again; confirm the error state appears immediately (the cached `Unrecognized` is recognized by the post-load callback).
13. **Telemetry**: with telemetry inspection enabled, click an image and confirm `CodePanelsFileOpened` fires with `target` set to the new variant's serialized form.
14. **No regression for non-image binaries**: click `.zip`, `.mp3`, `.exe`, `.pdf` files. They open in the OS default app (`SystemGeneric`) exactly as today.

### Runtime checks

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --tests -- -D warnings`
- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`

## Follow-ups

In rough priority order; none are required for v1.

- **Accessibility plumbing on the GPUI Image element**. Add an `accessibility(...)` builder on `crates/warpui_core/src/elements/image.rs` and wire its paint pass to register an AX node, then expose the Lightbox image's filename as the AX label. This is the v1.1 deliverable on the accessibility axis; v1 ships with the filename rendered as the visible `description` slot but without an a11y label on the image element itself.
- **Sibling navigation**. Left/Right arrow keys step through other supported image files in the same directory. Requires: a sibling-listing helper with a directory-size cap, a bounded preload window so parallel decodes are bounded, lazy promotion of `Loading` to `Resolved` on navigation, and a `path` carrier on `LightboxImage`. Also requires deciding the swap-while-open interaction (the post-v1 design will revisit the modality contract). This is the next biggest user win after v1.
- **Move decode and metadata stat to the background executor**. Decode in `AssetCache::load_asynchronously` runs on the foreground executor; the change-2 `std::fs::metadata` does too. Moving both off-thread affects every Lightbox call site and every other asset-cache consumer. Should land before the Lightbox is used for any larger inputs than v1 allows or in any context where stalled-FS responsiveness matters more than v1's posture.
- **Convert `ImageType::Unrecognized` to `Err` globally** with an audit of every `try_from_bytes` caller. Closes the mislabeled-file rough edge for surfaces other than the Lightbox file-tree path (which v1 already closes locally).
- **Adopt `LightboxImageSource::Error` at the artifacts call site** (`app/src/ai/artifacts/mod.rs:362-365`) so screenshot fetch failures use `Error` instead of the `Loading + "Failed to load"` description workaround.
- **Animated GIF / WebP continuous playback in the Lightbox**. Wire `Image::enable_animation_with_start_time(Instant)` into the Lightbox image element and drive a per-frame redraw loop on the focused entry. **Play/pause control** is the next layer on top.
- **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).
- **Status footer** (filename, dimensions, file size, format string): extend `lightbox::Params` with an optional metadata strip rendered below the image.
- **EXIF orientation and ICC color profile**: extend the agent-mode decoder in `app/src/util/image.rs` and wire into `ImageType::try_from_bytes`.
- **Visible thumbnail strip**: only relevant once sibling navigation lands.
- **Additional raster formats** (HEIC, HEIF, AVIF, BMP, TIFF, ICO): depends on backend `image`-crate features and decoder availability; reclassify in `is_supported_image_file` and re-test the cap behavior.
- **Magic-byte content sniffing**: extend `crates/warp_util/src/file_type.rs` to read the first N bytes when the extension claims an image; route mismatches as a non-blocking warning rather than a permanent spinner.
- **Right-click context menu**: wire `Copy Image`, `Copy file path` (relative/absolute), `Reveal in Finder/Files`, `Attach as Agent context` on the Lightbox image surface.
- **Drag-out to attach as Agent context**: share the payload type used by `app/src/terminal/input.rs::handle_pasted_or_dragdropped_image_filepaths`.
- **Disk-backed thumbnail cache and size-cap setting**: only relevant once the visible thumbnail strip lands.
- **SVG `size_in_bytes`** currently returns 0 (`image_cache.rs:370`), so SVGs do not count against the asset-cache eviction budget. Compute a reasonable proxy (e.g. `data.len()` or rasterized pixmap size) so the cache can evict them.
- **Adopt a `usvg`/`resvg` parse-budget API** if and when one is exposed (parse-time CPU budget, max element count, max nesting depth). When that lands it becomes the right primary defense for SVG parser DoS and the v1 `MAX_SVG_BYTES = 4 MB` byte cap can relax to match the raster cap.
- **Parameterize `AssetSource::LocalFile` with optional `max_bytes`**. v1 applies a single global cap (64 MB raster / 4 MB SVG content-keyed) to every `LocalFile` consumer; the per-consumer survey in change 5 confirmed no current caller routinely exceeds those caps. If a future consumer (video-preview surface, large bundled-export viewer, etc.) genuinely needs to load a larger file via `LocalFile`, extend the variant to `LocalFile { path, max_bytes: Option<u64> }` with `None` preserving the unbounded legacy behavior and image-preview opting into `Some(MAX_ASSET_LOCAL_FILE_BYTES)`. This is documented as a follow-up rather than implemented in v1 because there is no current beneficiary.
- **Image diff across git revisions**: render two `Lightbox`-style panes side by side, tied into the existing diff infrastructure.
- **Slideshow / fullscreen mode**: auto-advance with a configurable interval. Depends on sibling navigation.
- **RAW formats** (CR2, NEF, ARW, DNG): pulls in a much larger decoder dependency; gate behind a feature flag.
- **Remote URL preview**: open `https://...` images directly from clipboard or terminal hyperlink without a local file round-trip.
