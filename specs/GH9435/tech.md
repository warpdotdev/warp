# GH9435: Tech Spec — Windows corner resize hit target

## Context
`specs/GH9435/product.md` defines the desired user-facing behavior: Windows users should be able to acquire diagonal resize from Warp window corners without hunting for a tiny hotspot.

Warp creates normal app windows with `hide_title_bar: true`, so Windows windows usually use Warp's custom undecorated frame rather than the native titlebar and resize border (`crates/warpui_core/src/core/app.rs:2319`). The relevant custom hit testing lives in the winit platform layer:

- `crates/warpui/src/windowing/winit/window.rs:64` defines `DRAG_RESIZE_MARGIN` as `4.0` logical pixels.
- `crates/warpui/src/windowing/winit/window.rs:912` updates drag-resize state from the last cursor position when a window is not maximized.
- `crates/warpui/src/windowing/winit/window.rs:947` starts the OS resize operation with `winit::window::Window::drag_resize_window`.
- `crates/warpui/src/windowing/winit/window.rs:958` classifies the cursor position by checking whether the pointer is inside the same fixed margin from each edge.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs:1167` stores cursor movement in logical pixels.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs:1582` updates the custom resize state only for undecorated windows.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs:1591` intercepts left mouse down to start drag-resizing before dispatching the click into Warp UI.

The likely root cause is the single fixed margin: a diagonal corner is returned only when the cursor is within `4.0` logical pixels of both adjacent edges, so each corner is effectively a very small square. Increasing the global margin would also widen side-edge hit targets everywhere, which risks stealing content clicks near all window edges. The safer fix is to keep edge margins small while adding a larger corner-specific target on Windows.

## Proposed changes
1. Split drag-resize hit-test sizing into edge and corner margins.
   - Keep the current edge margin as the default side-edge target.
   - Add a larger Windows-only corner margin for diagonal resize. Start with `12.0` logical pixels for Windows custom undecorated windows; this is 3x the current margin and remains small enough to avoid broad content interception.
   - Keep non-Windows behavior unchanged by defaulting the non-Windows corner margin to the existing edge margin.

2. Replace `drag_resize_direction_at_position(window_size, cursor_position, margin)` with a helper that accepts explicit hit-test parameters.
   - Suggested shape:
     - `edge_margin: f32`
     - `corner_margin: f32`
   - Use inclusive comparisons at boundaries (`<=` / `>=`) to avoid one-pixel dead bands at exact margin values.
   - Compute whether the cursor is in the north, south, west, or east corner bands using `corner_margin`.
   - Return a diagonal `ResizeDirection` first when the cursor is in both a horizontal and vertical corner band for the same corner.
   - If no diagonal corner matched, compute side-edge resize directions using `edge_margin`.
   - Return `None` when neither corner nor edge rules match.

3. Keep `update_drag_resize_state` responsible for platform and window-state gating.
   - Continue returning `None` when the window is maximized.
   - Continue calling the hit-test helper with the current inner window size converted to logical pixels.
   - Use a small `drag_resize_hit_test_config()` helper so platform constants are centralized and testable:
     - Windows: `edge_margin = 4.0`, `corner_margin = 12.0`.
     - Other platforms: `edge_margin = 4.0`, `corner_margin = 4.0`.

4. Do not change the event-loop ordering.
   - `MouseMoved` should still refresh the active drag-resize direction before the next click.
   - `LeftMouseDown` should still call `try_drag_resize()` before dispatching into Warp UI.
   - Touch input should remain excluded from `drag_resize_window` exactly as it is today.

5. Avoid adding a visible resize grip in this implementation.
   - The product spec intentionally scopes v1 to the invisible hit target because there is no visual design mock.
   - A visible grip can be explored later in app-level UI code if product/design wants an explicit affordance.

6. Keep the change in the winit windowing layer unless implementation discovers a platform limitation.
   - The problem is localized to cursor-position classification before `drag_resize_window`.
   - No app workspace, pane, tab, or settings state should be required.
   - No telemetry is needed for this bug fix.

## Testing and validation
1. Add unit coverage for the pure resize-direction helper in `crates/warpui/src/windowing/winit/window.rs`.
   - With Windows-style config (`edge_margin = 4.0`, `corner_margin = 12.0`), positions inside all four `12x12` logical corner targets return the correct diagonal `ResizeDirection`.
   - Positions near an edge but outside the corner target return the correct side-edge direction.
   - Positions outside both edge and corner targets return `None`.
   - Boundary positions at exactly the configured margin classify consistently and do not create dead zones.
   - Non-Windows/default config preserves existing `4.0` logical-pixel behavior.

2. Run the targeted Rust unit test for the `warpui` crate after implementation.
   - Preferred command: `cargo test -p warpui drag_resize`
   - If the exact test names differ, run the smallest `cargo test -p warpui ...` filter that covers the new helper tests.

3. Run formatting and an appropriate compile check.
   - `cargo fmt --check`
   - `cargo check -p warpui`

4. Manually validate on Windows 11 against product behavior invariants 1-13.
   - Restored window: hover each corner and verify a diagonal resize cursor appears over a noticeably larger target than before.
   - Drag each corner and verify both dimensions resize in the expected direction.
   - Hover side edges away from corners and verify horizontal/vertical resize still works.
   - Hover and click app content just outside the resize target near the window edge and verify normal content interactions still work.
   - Move between corner, edge, and content regions and verify cursor updates without flicker.
   - Verify the top titlebar drag region still moves the window away from the top corners.
   - Maximize, restore, then verify corner resizing works again.
   - Check 100%, 125%, 150%, and 200% display scaling when available.
   - Check a secondary monitor when available.

5. Regression-check non-Windows behavior if the shared helper changes.
   - On macOS/Linux, verify the existing corner and edge resize behavior is unchanged for undecorated windows.
   - If only unit tests are available on non-Windows, confirm the default config matches the old `4.0` margin semantics.

## Risks and mitigations
1. An expanded invisible corner could steal clicks from content near the window corners. Mitigate by limiting the expanded target to Windows and to a small corner square, while preserving the narrow side-edge margin.

2. Too-small a corner margin may not fix the reported issue at high DPI. Mitigate with manual validation at common Windows display scaling values and adjust the Windows corner margin before shipping if `12.0` logical pixels still feels too narrow.

3. Cursor ownership can be subtle because Warp UI also sets cursors for app content. Mitigate by preserving the existing cursor reset path when leaving resize regions and manually checking cursor transitions during validation.

## Parallelization
Parallel implementation is not especially beneficial. The code change is localized to one helper and its unit tests, while Windows manual validation depends on a single built app artifact.

## Follow-ups
1. If users still miss the target after the invisible hit-target fix, consider a separate product/design pass for a visible resize grip or Windows-specific corner affordance.
2. If Windows users report side-edge acquisition problems separately from diagonal corners, consider a separate issue for side-edge margin tuning rather than broadening this bug fix.
