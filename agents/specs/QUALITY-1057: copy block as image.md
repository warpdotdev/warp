# Spec: Copy block as image (QUALITY-1057)

Linear: [QUALITY-1057](https://linear.app/warpdotdev/issue/QUALITY-1057/implement-copy-block-as-image)
Context issue: [warpdotdev/warp#3065](https://github.com/warpdotdev/warp/issues/3065) ·
Originating request: [warpdotdev/warp-server#12687](https://github.com/warpdotdev/warp-server/issues/12687)
Target repo: **`warpdotdev/warp`** (client). All block rendering, the block
context menu, copy actions, and clipboard access live here. `warp-server` is
**not** modified.
Commit-pinned refs below are against `master @ 0b5adc0536e4ce42642c72c6a3b0a75012280994`.

## == PRODUCT ==

### Summary
Add a **"Copy as image"** action for a terminal block that renders the block's
command+output to a PNG raster — preserving theme colors and syntax
highlighting — and writes it to the system clipboard. This sits alongside the
existing text copy affordances ("Copy", "Copy command(s)", "Copy output") so a
user can paste a faithful picture of a block into Slack, a doc, a bug report, or
a presentation without taking a manual screenshot (the exact ask in #3065).

### Key design choices
1. **Client-side local render, not server reuse.** The image is produced on the
   client by rasterizing the block through the existing renderer + frame-capture
   primitives, **not** by round-tripping to `warp-server`'s block-sharing
   preview-image pipeline. Rationale: block-sharing requires login, a network
   round-trip, and **uploads the block** (see `app/src/terminal/share_block_modal.rs`,
   which is entirely server-backed and only ever copies a *link* or an HTML embed
   snippet to the clipboard — never local image bytes). "Copy as image" must be an
   instant, offline, no-upload local action that matches the "copy" mental model.
   Server reuse is explicitly rejected for the primary path.
2. **Whole block (command + output) by default.** The action mirrors the existing
   top-level "Copy" (`BlockEntity::CommandAndOutput`). Output-only / command-only
   image variants are a possible follow-on, not in this change.
3. **PNG to the system clipboard is the destination.** Populate
   `ClipboardContent.images` with an encoded PNG. "Save block image to a file" is
   an explicit **follow-on**, out of scope here.
4. **Gated behind a feature flag** (`FeatureFlag::CopyBlockAsImage`), dogfood-first,
   per the repo's feature-flag conventions — the menu item and action are hidden
   when the flag is off.
5. **Cross-platform via the two existing render backends.** macOS renders through
   Metal; Linux and Windows share the wgpu backend. The core new work is (a) an
   **offscreen single-block render** and (b) **Linux/Windows clipboard image
   *write*** (macOS already writes images; Linux/Windows currently do not — see
   Tech/Context). Factory verification runs on a **Linux** cloud build, so Linux
   is the visually-verified platform; Windows shares the same wgpu render + arboard
   write path and is covered by unit tests.

### Behavior (numbered, testable invariants)
1. **Menu entry (flag on).** Right-clicking a single selected block shows a
   **"Copy as image"** item in the block context menu, adjacent to "Copy" /
   "Copy command" / "Copy output". When `FeatureFlag::CopyBlockAsImage` is off, the
   item is absent and the action is not registered.
2. **Clipboard image write.** Invoking "Copy as image" places a **PNG raster of
   the block's rendered command+output** on the system clipboard
   (`ClipboardContent.images` populated with `mime_type: "image/png"`).
3. **Paste fidelity.** Pasting into an image-accepting target (e.g. Slack, an
   image editor, a doc) yields the block image with theme background, foreground
   colors, and syntax highlighting matching what is rendered on screen.
4. **Full content, scroll-independent.** The captured image contains the block's
   full command+output laid out at the terminal's column width and full height —
   **independent of scroll position or partial visibility**. A block taller than
   the viewport, or scrolled partly off-screen, still produces a complete image
   (this is why an offscreen render is required rather than cropping the visible
   window).
5. **Disabled/no-op states.** For an empty block (no command and no output) the
   item is disabled, consistent with how "Copy output" is disabled today
   (`app/src/terminal/view.rs:16850`). Multi-block selection: the item is disabled
   or hidden (single-selection only) for this change.
6. **Secret obfuscation is respected.** If the block contains obfuscated secrets,
   the image reflects the same obfuscation the block uses on screen / in block
   sharing (`get_secret_obfuscation_mode`, `crates/.../safe_mode_settings`). Secrets
   must not be revealed in the image.
7. **Command-palette discoverability.** A `CustomAction::CopyBlockAsImage` is
   registered so the action is reachable from the Command Palette (no default
   keybinding is required; one may be added). Per AGENTS.md, a new user-facing
   action gets a Command Palette entry.
8. **Failure handling.** If rendering or PNG encoding fails, the action logs and
   `report_error!`s, does **not** panic, and does **not** mutate the clipboard;
   an optional failure toast may be shown. On success an optional confirmation
   toast ("Copied block image.") may be shown, consistent with other copy
   affordances.

## == TECH ==

### Context (how the area works today)
Text copy is fully wired; the only missing piece is turning a block into a bitmap
and (on Linux/Windows) writing image bytes to the clipboard.

Copy action + menu plumbing (all in `warpdotdev/warp @ 0b5adc0`):
- Actions enum: `CustomAction::{CopyBlock, CopyBlockCommand, CopyBlockOutput}` —
  `app/src/util/bindings.rs:82` (variants) and `:356`/`:371` (keystrokes).
- Context-menu actions: `ContextMenuAction::{CopyBlocks, CopyBlockCommands,
  CopyBlockOutputs, CopyBlockFilteredOutputs}` — `app/src/terminal/view.rs:1344`.
- Menu item construction for a block: `app/src/terminal/view.rs (16746-16895)`
  (the "Copy" / "Copy commands" / "Share..." / "Copy output" items).
- Dispatch: `TerminalView::context_menu_action` — `app/src/terminal/view.rs:24613`
  (matches each `ContextMenuAction` to a handler).
- Handlers: `context_menu_copy_block_outputs` → `copy_blocks(BlockEntity::…)` →
  `ctx.clipboard().write(ClipboardContent::plain_text(...))` —
  `app/src/terminal/view.rs (20973-21051)`.

Clipboard model (already image-aware):
- `ClipboardContent { plain_text, paths, html, images: Option<Vec<ImageData>> }`
  and `ImageData { data: Vec<u8>, mime_type: String, filename: Option<String> }` —
  `crates/warpui_core/src/clipboard.rs:29,46`.
- **macOS write supports images** (encoded bytes → pasteboard `public.png` etc.):
  `crates/warpui/src/platform/mac/clipboard.rs:54-69`.
- **Linux write does NOT write images** — only `html`/`plain_text`:
  `crates/warpui/src/windowing/winit/linux/clipboard.rs (188-199)`
  (`write_to_specific_clipboard`).
- **Windows write does NOT write images** — only `html`/`text`:
  `crates/warpui/src/windowing/winit/windows/clipboard.rs:20-35`.
- `arboard` already has the `image-data` feature enabled on Linux/Windows —
  `crates/warpui/Cargo.toml:175-179` — so `arboard`'s `set().image(...)` is
  available; the warpui write impls simply don't call it yet. Note: `arboard`'s
  `ImageData` takes **raw RGBA** (`{ width, height, bytes }`), whereas the macOS
  pasteboard takes **encoded PNG bytes**.

Render-to-bitmap primitives (the reusable core):
- `CapturedFrame { width, height, data, format: Rgba|Bgra }` with `ensure_rgba()` —
  `crates/warpui_core/src/platform/mod.rs:504`.
- `WindowContext::request_frame_capture(callback)` — `crates/warpui_core/src/platform/mod.rs:492`;
  Metal impl `crates/warpui/src/platform/mac/window.rs:1210`, wgpu impl
  `crates/warpui/src/windowing/winit/window.rs:1742`. **This captures the whole
  window frame**, not a single block.
- Metal readback `capture_frame(...)` and offscreen render target
  `create_capture_texture(...)` (currently `#[allow(dead_code)]`, "kept for future
  headless capture") — `crates/warpui/src/platform/mac/rendering/metal/frame_capture.rs:30,97`.
- wgpu frame capture needs `COPY_SRC` on the surface, and today that is enabled
  **only under the `integration_tests` cargo feature** —
  `crates/warpui/src/rendering/wgpu/resources.rs:863`. Production wgpu builds
  therefore cannot capture the swapchain as-is.
- PNG encoding pattern (`image` crate `PngEncoder`) demonstrated in
  `crates/warpui/examples/frame-capture-test/root_view.rs:80-98`. `image` is a
  workspace dependency with the `png` feature — `Cargo.toml:175`.

**Net gap:** there is no path to render *one block* to a bitmap. This is the core
new work. Cropping the visible window frame is rejected: it fails invariant #4
(tall / scrolled blocks) and is test-gated on wgpu.

### Proposed changes
1. **Feature flag.** Add `FeatureFlag::CopyBlockAsImage` (per the `add-feature-flag`
   skill), default-on for dogfood; gate the menu item and action registration on it.
2. **Action + menu wiring** (client, `app/`):
   - Add `CustomAction::CopyBlockAsImage` to `app/src/util/bindings.rs` (no default
     keystroke required; include a Command Palette description).
   - Add `ContextMenuAction::CopyBlockAsImage { block_index }` to
     `app/src/terminal/view.rs:1344` (+ its `Debug` arm at `:1470`).
   - Add the "Copy as image" `MenuItemFields` in the block context-menu builder
     (`app/src/terminal/view.rs (16845-16882)`, near "Copy output"), gated by the
     flag, single-selection, and non-empty content.
   - Add the dispatch arm in `context_menu_action` (`:24613`) calling a new handler.
3. **Render-block-to-image routine** (the core new work). Add a routine that:
   - Builds a `Scene` containing just the selected block, laid out at the terminal
     column width and the block's **full unclipped height** (reuse the block/grid
     element construction that `TerminalView` already uses for on-screen rendering;
     factor out a single-block element path). Render at the window backing scale
     factor for crisp output.
   - Renders that scene to an **offscreen texture** and reads it back to a
     `CapturedFrame`:
     - macOS: use `create_capture_texture` + `capture_frame`
       (`crates/warpui/src/platform/mac/rendering/metal/frame_capture.rs`), promoting
       the currently-dead code to a real path.
     - Linux/Windows (wgpu): render to an offscreen wgpu texture created with
       `COPY_SRC` usage (**independent of the swapchain**, so no change to the
       production swapchain config and no perf regression to normal rendering),
       then read back. Generalize the existing test-only capture into a reusable
       offscreen-capture helper.
   - Calls `CapturedFrame::ensure_rgba()`, then encodes PNG via the `image` crate
     (`PngEncoder`, as in the frame-capture example) to `Vec<u8>`, and also retains
     the raw RGBA for the arboard path.
   - Respects the block's secret-obfuscation mode.
   - Caps very tall blocks at the platform max texture dimension
     (`WindowContext::max_texture_dimension_2d`, 8192 on Metal) — either cap height
     with a documented behavior or tile; state and test the chosen behavior.
4. **Clipboard image write on Linux + Windows.** Extend the `Clipboard::write`
   impls to write `contents.images`:
   - Linux `crates/warpui/src/windowing/winit/linux/clipboard.rs`: in
     `write_to_specific_clipboard`, when `contents.images` is present call
     `self.inner.set().clipboard(kind).image(arboard::ImageData { width, height,
     bytes })` using **raw RGBA** (decode the PNG or pass the retained RGBA).
   - Windows `crates/warpui/src/windowing/winit/windows/clipboard.rs`: same via
     `self.inner.set().image(...)`.
   - macOS needs no change (already writes encoded PNG via
     `ClipboardContent.images`).
   - The handler builds `ClipboardContent { images: Some(vec![ImageData { data:
     png_bytes, mime_type: "image/png", filename: Some("warp-block.png") }]),
     ..Default::default() }` and calls `ctx.clipboard().write(...)`.
5. **Telemetry.** Add a copy-as-image telemetry event mirroring
   `TelemetryEvent::ContextMenuCopy` (`app/src/terminal/view.rs:21034`).

### Tradeoffs / risks
- **Offscreen single-block scene construction is the main complexity.** The block's
  normal render is embedded in `TerminalView`'s viewport-relative layout; producing
  a standalone, full-height single-block scene requires factoring the block element
  path out of the viewport render. This is the largest and riskiest part; the
  implementation may refine the exact mechanism, but the observable contract
  (invariants #2–#4) is fixed.
- **Rejected alternative — crop the visible window frame** via `request_frame_capture`:
  fails invariant #4 (tall/scrolled blocks, overlapping UI) and is test-gated on
  wgpu (`resources.rs:863`).
- **Rejected alternative — server block-sharing image**: requires login + network +
  block upload; wrong semantics for a local copy (see Key design choice #1).
- **Windows cannot be visually verified** in the factory's Linux cloud env; it
  shares the wgpu render + arboard write path with Linux and is covered by unit
  tests.
- **Image size**: very tall blocks may exceed max texture dimensions; behavior must
  be defined and tested.

## Validation & verification criteria (must ALL pass before merge)

1. **Feature behavior (primary, user-facing — computer-use proof required).** On a
   **Linux** dogfood build with `FeatureFlag::CopyBlockAsImage` on: right-click a
   block → the **"Copy as image"** item is present; invoking it and pasting into an
   image-accepting surface yields a PNG of the block's rendered command+output with
   correct theme colors and syntax highlighting. Captured as computer-use
   screenshots (menu item + paste result) attached to the ticket and PR per
   `factory-verification` / `factory-ui-verification`. Verifies invariants #1–#3.
2. **Full-content / scroll independence.** A block taller than the viewport (or
   scrolled partly off-screen) produces a complete image containing the entire
   command+output. Checked via the render routine's output dimensions in a test and
   via a computer-use paste of a tall block. Verifies invariant #4.
3. **Render-routine unit test.** A new test drives the render-block-to-image routine
   on a known block and asserts it returns `Ok` with a valid PNG (PNG magic bytes
   `89 50 4E 47`) of the expected (non-zero, width≈columns·cell·scale) dimensions.
   Fails before the routine exists, passes after. Place next to the routine per repo
   test conventions (`${file}_tests.rs`).
4. **Clipboard image-write unit tests (Linux + Windows).** New tests assert that
   `Clipboard::write` on Linux and Windows now forwards `contents.images` to the
   underlying arboard `set().image(...)` (extend
   `crates/warpui/src/windowing/winit/{linux,windows}/clipboard_tests.rs`). Fails
   before the change (images ignored), passes after. Verifies the platform gap is
   closed for invariant #2.
5. **End-to-end action test.** An integration/UI test invoking
   `ContextMenuAction::CopyBlockAsImage` results in `ClipboardContent.images`
   populated with a PNG (extend the block-copy coverage in
   `crates/integration/.../shell_integration_tests.rs` / `ui_tests.rs`). Verifies
   the menu→handler→clipboard wiring.
6. **Feature-flag gating test.** With `CopyBlockAsImage` off, the "Copy as image"
   menu item is absent and the action unregistered. Verifies invariant #1.
7. **Secret obfuscation.** A block with obfuscated secrets produces an image with
   the secrets obfuscated (assert via the render routine honoring the obfuscation
   mode, plus visual spot-check). Verifies invariant #6.
8. **No collateral damage.** Existing "Copy", "Copy command(s)", "Copy output", and
   text paste still work — confirmed by the existing block-copy integration tests
   and a text-paste check (image-only clipboard content does not corrupt text paste
   targets, consistent with `should_insert_text_on_paste`,
   `crates/warpui_core/src/clipboard.rs:99`).
9. **Presubmit gate.** `./script/format` and `cargo clippy` (the presubmit
   versions) are clean and **`./script/presubmit` passes** unconditionally.
10. **Visual proof validated against this spec.** The attached computer-use
    screenshots demonstrate criteria #1–#2 (and #7 where applicable); proof that
    shows the wrong surface or omits a listed criterion is treated as missing proof.

---
_This spec is committed to the shared `factory/copy-block-as-image` branch; once
approved, the implementation is added to **this same PR** (its title/description
rewritten to describe the shipped change). GitHub is the source of truth for this
file — edit it here to request changes._
