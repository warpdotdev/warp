# Shared session QR code — Tech Spec
Product spec: `specs/qr-code/PRODUCT.md`
## Context
`PRODUCT.md` defines the user-visible behavior for adding a QR-code affordance to the live-session sharing dialog. The implementation should extend the existing session sharing dialog rather than adding another sharing surface.
The relevant current code paths are:
- `app/src/drive/sharing/mod.rs:34` defines `ShareableObject::Session`, carrying a terminal view handle, `SessionId`, and `started_at`.
- `app/src/drive/sharing/mod.rs:45` implements `ShareableObject::link`; the session branch returns `join_link(session_id)`, which should be the sole source of truth for QR payloads.
- `app/src/terminal/shared_session/mod.rs:285` implements `join_link`, including staging/native-intent and preview-channel behavior.
- `app/src/terminal/view/shared_session/view_impl.rs:591` sets the pane header shareable object to `ShareableObject::Session` when a share starts and opens the sharing dialog unless the flow explicitly skips it.
- `app/src/terminal/view/shared_session/view_impl.rs:408` refreshes the same shareable object from active shared-session state when roles or session state change.
- `app/src/workspace/view.rs:4281` listens for `ManagerEvent::StartedShare`, copies the remote-control link, and shows the `Remote control link copied.` toast.
- `app/src/terminal/shared_session/manager.rs:135` emits `ManagerEvent::StartedShare` with the new shared-session id and window id.
- `app/src/drive/sharing/dialog/mod.rs:69` defines the generic `SharingDialog` state and `UiStateHandles`.
- `app/src/drive/sharing/dialog/mod.rs:149` defines `SharingDialogAction`; it currently has `CopyLink` and permission actions but no QR actions.
- `app/src/drive/sharing/dialog/mod.rs:332` resets the dialog target through `set_target`.
- `app/src/drive/sharing/dialog/mod.rs:368` exposes `has_shared_session_target`, which already distinguishes session targets from other shareable object types.
- `app/src/drive/sharing/dialog/mod.rs:428` treats sessions as editable so the sharing dialog opens for session targets.
- `app/src/drive/sharing/dialog/mod.rs:905` copies the target URL and sends `CopiedSharedSessionLink` telemetry for session targets.
- `app/src/drive/sharing/dialog/mod.rs:2372` renders the footer link and `Copy link` button in `render_object_link`.
- `app/src/drive/sharing/dialog/mod.rs:2505` composes the full dialog in `render`.
- `app/src/drive/sharing/style.rs:15` contains sharing-dialog layout constants and color helpers.
- `crates/warpui_core/src/platform/file_picker.rs:127` defines `SaveFilePickerConfiguration`, and `crates/warpui_core/src/core/view/context.rs:311` exposes `open_save_file_picker` for save-file flows.
- `crates/warpui_core/src/clipboard.rs:29` defines `ClipboardContent`, which the existing link-copy action already uses for plain-text URLs.
The app icon inventory now exposes the QR flow controls used by this surface, including `Icon::Download`, `Icon::Copy`, and `Icon::QrCode`, with the QR asset mapped alongside the other share/link icons.
## Proposed changes
### 1. Add the toast entry point shown in the mocks
Extend the existing shared-session start toast in `app/src/workspace/view.rs` so it renders:
- message: `Remote control link copied.`
- inline action: `View QR code`
The action should carry the new `SessionId` from `ManagerEvent::StartedShare` and route back to the matching shared terminal view. Add a workspace action dedicated to opening the QR flow for that session id instead of relying on whichever pane is currently focused.
Add a lookup helper on the shared-session manager that resolves a shared session id back to its terminal view. From there, dispatch into the terminal/pane-header sharing flow so the right pane opens its existing sharing overlay in QR mode.
If the standard live-session sharing dialog is already open, switch that existing overlay into QR mode. If it is closed, open it directly in QR mode. Back should always return to the access-management panel for the same session target.
### 2. Add QR mode to `SharingDialog`
Add a dialog mode enum in `app/src/drive/sharing/dialog/mod.rs`, for example:
- `SharingDialogMode::Access`
- `SharingDialogMode::QrCode`
Store it on `SharingDialog`, defaulting to `Access`. Reset it to `Access` in `set_target` so a recycled dialog does not show QR content for a different target.
Extend `UiStateHandles` with mouse states for:
- the footer QR button;
- the QR dialog back button;
- the QR dialog copy-link button;
- the QR dialog download button.
Extend `SharingDialogAction` with:
- `ShowQrCode`
- `BackToAccessDialog`
- `DownloadQrCode`
Keep `SharingDialogAction::Close` behavior unchanged: it should still close the overlay and reset editable state.
### 3. Render the QR entry point only for session targets
Extract a small helper such as `target_link(&self, app: &AppContext) -> Option<String>` so `render_object_link`, copy, and QR actions all use the same URL lookup.
Modify `render_object_link` so that when `self.target` is `Some(ShareableObject::Session { .. })`, the footer row includes an icon-only QR button between the link field and the existing `Copy link` button. For non-session targets, keep the current link + copy button layout.
The QR button should:
- use the same button/style system as the existing footer controls;
- dispatch `SharingDialogAction::ShowQrCode`;
- be disabled or omitted when `target_link` returns `None`;
- show a tooltip equivalent to `Show QR code`.
Add a bundled QR icon if needed:
- Add the SVG asset under the existing bundled SVG asset location.
- Add a variant such as `Icon::QrCode`.
- Map it to the bundled path near the other share/link icons.
### 4. Render the QR-code view inside `SharingDialog`
In `View for SharingDialog`, branch on `self.mode`:
- `Access` renders the existing dialog contents.
- `QrCode` renders a compact QR view for session targets.
Keep the same outer `Dismiss` behavior and border/background styling so the QR view remains part of the pane-header sharing overlay. Use the Figma dimensions as targets rather than introducing a new modal system:
- width around 400px;
- header height around 48px;
- body centered around a 192px QR card;
- two 32px icon buttons beneath the QR card.
The QR branch must preserve the sharing dialog's compact intrinsic overlay layout. Render it with minimum-height column sizing rather than max-height stretching so opening QR mode replaces the dialog contents instead of producing a workspace-height panel.
The QR view should render:
- a header row with back button, `Share session QR code`, `ESC`, and close button;
- a QR code card;
- copy-link and download-image icon buttons.
If `self.target` is no longer a session target or `target_link` is missing, render a compact error body with the same header and a message equivalent to `Unable to create QR code for this session link.`
### 5. Generate QR data with a small pure helper
Add a small QR helper module, for example `app/src/drive/sharing/qr_code.rs`, with pure functions:
- `qr_matrix_for_url(url: &str) -> Result<QrMatrix, QrCodeError>`
- `qr_png_for_url(url: &str, pixel_size: u32) -> Result<Vec<u8>, QrCodeError>`
Add a workspace dependency on a QR encoder crate such as `qrcode` in the root `Cargo.toml` and `app/Cargo.toml`. Prefer a dependency that can produce a boolean module matrix without pulling in a large image stack; use the existing workspace `image` crate for PNG encoding because it is already present.
Use the same QR helper for on-screen rendering and PNG generation so scanning behavior cannot drift between the two paths.
For on-screen rendering, prefer drawing the QR matrix directly with Warp UI rectangles rather than feeding a generated PNG back through the image cache. This keeps the view deterministic, avoids temporary files, and makes sizing straightforward. The helper should expose module count and module values; the view computes cell size and quiet-zone padding inside the 160px visual target.
For PNG export, generate a black-on-white PNG with a quiet zone. Use a larger export size than the on-screen display, such as 512px or 1024px, so downloaded images remain scannable when printed or projected.
### 6. Copy the session link from QR mode
Wire the QR dialog's copy button to the existing `SharingDialogAction::CopyLink` behavior so it:
- resolves `target_link`;
- writes the same plain session URL that the access dialog footer copies;
- preserves the existing shared-session link-copy telemetry and toast feedback;
- behaves consistently across platforms without depending on image clipboard support.
### 7. Download QR image
Implement `SharingDialogAction::DownloadQrCode` by:
- generating the same PNG bytes;
- opening `ctx.open_save_file_picker` with `SaveFilePickerConfiguration::new().with_default_filename(default_qr_filename(...))`;
- writing the bytes to the selected path on the background executor or through an existing async file-write helper;
- showing a success toast when the file is written;
- showing a failure toast on write or generation errors;
- doing nothing when the picker returns `None`.
Default filename suggestion: `warp-session-qr-code-<session-id>.png` for session targets. If the session id is unavailable in a future target shape, fall back to `warp-session-qr-code.png`.
The download action can be compiled only for local filesystem builds if needed. If save-file picker or filesystem writes are unavailable for a target platform, disable or hide the download button there rather than showing a broken control.
### 8. Preserve existing sharing behavior
Do not change:
- `ShareableObject::Session.link`;
- `join_link`;
- `SharingDialogAction::CopyLink`;
- ACL update actions;
- session invite flow;
- pane-header sharing dialog toggling.
The QR flow should be additive. Existing tests for session permissions and link-copy behavior should keep passing without expected-output changes except where snapshots explicitly include the new QR button.
### 9. Telemetry
Keep existing link-copy telemetry unchanged. Add telemetry only if the product/event taxonomy already has an appropriate place for it:
- `OpenedSharedSessionQrCode`
- `DownloadedSharedSessionQrCode`
If new telemetry is added, include the same action source where available or derive it from the sharing dialog context. Avoid error-level logs for QR generation or export failures; use user-visible toasts and at most warn-level diagnostic logging.
## Testing and validation
Map validation to `PRODUCT.md` behavior:
- Behavior 1-5: workspace/session tests verify the shared-session toast includes `View QR code`, that it targets the newly-started session id, that clicking it opens QR mode on the correct pane even when the access dialog was not already open, and that the QR state remains the same compact sharing overlay rather than a full-height panel.
- Behavior 5-7: unit or view tests verify that `render_object_link` includes the QR button only for `ShareableObject::Session`, and non-session targets still render the existing footer.
- Behavior 9-12: view/action tests verify `ShowQrCode`, `BackToAccessDialog`, `Close`, and Escape transition between access mode, QR mode, and closed overlay as expected.
- Behavior 13, 15, 18, 29: pure QR helper tests verify that the generated matrix/PNG encodes the exact URL passed in and does not add extra query parameters or payload data.
- Behavior 14: screenshot or integration verification compares the QR view against the Figma layout in dark theme, including header, centered QR card, and copy/download buttons.
- Behavior 16-17: tests update the target/session link while QR mode is active and verify the rendered/exported QR payload follows the current `ShareableObject::link` result; missing links render the error state.
- Behavior 19-21: clipboard tests verify the QR dialog copy button routes through the plain-link copy path and does not attempt image clipboard writes.
- Behavior 22-24: save-file tests cover default filename, cancel behavior, successful write, and write failure.
- Behavior 25: existing `CopyLink` tests or new targeted tests verify plain link copying and toast behavior remain unchanged.
- Behavior 26: manual verification confirms the QR view copy/download icon buttons show the expected tooltips.
- Behavior 28: unit tests construct two session targets with different ids and verify each QR helper call uses that target's own link.
Suggested targeted commands after implementation:
- `cargo nextest run --no-fail-fast --workspace drive::sharing::dialog`
- `cargo nextest run --no-fail-fast --workspace drive::sharing::qr_code`
- `cargo check`
If the implementation changes Rust files, follow the repository convention of adding tests in a sibling `_tests.rs` file and importing it with a `#[cfg(test)]` directive from the implementation module.
## Parallelization
Do not split this implementation across parallel agents. The work is concentrated in the sharing dialog, icon plumbing, QR generation helper, and adjacent tests; parallel edits would likely collide in the same files and add coordination overhead.
## Risks and mitigations
- QR scan reliability: mitigate by keeping black-on-white rendering, including a quiet zone, and validating with an actual phone camera during manual QA.
- Platform clipboard differences: mitigate by reusing the existing plain-link clipboard path shared by supported platforms.
- Platform save-file differences: mitigate by using the existing `open_save_file_picker` abstraction and hiding/disabling download where local filesystem support is unavailable.
- Link drift: mitigate by deriving all QR payloads from `ShareableObject::link` at action/render time rather than storing a separate URL string.
