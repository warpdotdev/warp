# Viewer-Driven Terminal Sizing for Remote Control Sessions

## Summary
When a Claude Code session is viewed on a mobile web browser, the terminal is super wide because the sharer's desktop terminal width dictates the PTY size. We allow the viewer to report its own terminal size back to the sharer, so the sharer can resize its PTY to fit the viewer's viewport.

The sharer honors the viewer's reported size when all of the following are true:
- The `viewer_driven_sizing_enabled` setting is true (killswitch — defaults to true)
- The session has exactly 1 viewer (`present_viewer_count() == 1`)
- The viewer is the same user as the sharer (same `firebase_uid`), **or** the session is a cloud agent session (`SessionSourceType::AmbientAgent`)

On the viewer side, the viewer reports its size when the sharer is the same user (matching `firebase_uid`), or when the session is a cloud agent session. No platform gating (web vs native) is applied.

## Relevant Code

### Resize flow (sharer → viewer)
- `app/src/terminal/terminal_size_element.rs` — `TerminalSizeElement::after_layout()` sends rendered size through `resize_tx`
- `app/src/terminal/model/terminal_model.rs:1968-1980` — `TerminalModel::resize()` sends `OrderedTerminalEventType::Resize` through `ordered_terminal_events_for_shared_session_tx`
- `app/src/terminal/shared_session/sharer/network.rs:1156` — sharer's `Network` sends `UpstreamMessage::OrderedTerminalEvent` to the server
- `app/src/terminal/shared_session/viewer/event_loop.rs:242` — viewer processes `Resize` event, calls `resize_from_sharer_update()`
- `app/src/terminal/view/shared_session/view_impl.rs:1622` — `resize_from_sharer_update()` stores sharer size and calls `resize_internal()`
- `app/src/terminal/view.rs:1201-1236` — `SizeUpdateBuilder::build()` takes the MAX of sharer's size and viewer's pane size (unless viewer-driven sizing is active, in which case the viewer's natural size is used directly)

### Viewer → sharer communication (WriteToPty as reference pattern)
- `session-sharing-server/protocol/src/viewer.rs:316` — `viewer::UpstreamMessage` enum (new: `ReportTerminalSize` at line 389)
- `session-sharing-server/server/src/sessions/manager/viewer.rs:40` — `handle_viewer_upstream_message()` dispatches viewer messages (new: `ReportTerminalSize` handler at line 313)
- `session-sharing-server/protocol/src/sharer.rs:339` — `sharer::DownstreamMessage` enum (new: `ViewerTerminalSizeReported` at line 454)
- `app/src/terminal/shared_session/sharer/network.rs:1037-1051` — sharer's `Network` handles `DownstreamMessage::ViewerTerminalSizeReported`, emits `NetworkEvent::ViewerTerminalSizeReported`
- `app/src/terminal/local_tty/terminal_manager.rs:1945-1967` — sharer's terminal manager handles `NetworkEvent::ViewerTerminalSizeReported`

### Participant identity
- `app/src/terminal/shared_session/presence_manager.rs:313` — `PresenceManager::firebase_uid()` returns our own uid
- `app/src/terminal/shared_session/presence_manager.rs:338-341` — `present_viewer_count()` returns the number of present viewers
- `app/src/terminal/shared_session/presence_manager.rs:737-751` — `viewer_firebase_uid()` looks up a viewer's firebase uid by participant id
- `session-sharing-server/protocol/src/common/participant.rs:44` — `ProfileData` includes `firebase_uid`

### Viewer sizing
- `app/src/terminal/view.rs:1190-1199` — `create_size_info()` computes `new_size.rows`/`new_size.columns` from the viewer's actual pane dimensions
- `app/src/terminal/view/shared_session/viewer.rs:25-31` — client-side `Viewer` struct stores `sharer_size` and `last_reported_natural_size` for deduplication
- `app/src/terminal/view/shared_session/view_impl.rs:1652-1686` — `is_viewer_driven_sizing_eligible()` checks eligibility (same-user or ambient agent)
- `app/src/terminal/shared_session/settings.rs:39-46` — `viewer_driven_sizing_enabled` killswitch setting

### PTY resize
- `app/src/terminal/writeable_pty/terminal_manager_util.rs:67-71` — `view::Event::Resize` wired to `PtyController::resize_pty()`
- `app/src/terminal/writeable_pty/pty_controller.rs:598` — `resize_pty()` sends `Message::Resize` to the event loop (also triggers on `rows_or_columns_changed()` for `ViewerSizeReported`)
- `app/src/terminal/local_tty/unix.rs:538` — `on_resize()` calls `ioctl(TIOCSWINSZ)` on the PTY fd

### Server relay and throttling
- `session-sharing-server/server/src/sessions/manager/viewer_terminal_size.rs` — `relay_viewer_terminal_size()` publishes to event bus, `process_viewer_terminal_size_report()` delivers to the sharer
- `session-sharing-server/server/src/sessions/manager/event_bus/mod.rs:161-166` — `InvalidationTopicMessage::ViewerTerminalSizeReported` variant
- `session-sharing-server/server/src/sessions/network/viewer/join.rs:261-282` — throttled channel setup using `INVALIDATIONS_THROTTLE_PERIOD`

## Changes

### 1. Protocol Layer (`session-sharing-server/protocol/`)

**Viewer upstream message** — added to `viewer::UpstreamMessage` in `protocol/src/viewer.rs:389`:
```rust
ReportTerminalSize { window_size: WindowSize },
```

**Sharer downstream message** — added to `sharer::DownstreamMessage` in `protocol/src/sharer.rs:454`:
```rust
ViewerTerminalSizeReported {
    participant_id: ParticipantId,
    window_size: WindowSize,
},
```

### 2. Server Message Handling (`session-sharing-server/server/`)

**Handler** — in `server/src/sessions/manager/viewer.rs:313`, `ReportTerminalSize` sends the window size into a throttled unbounded channel (`terminal_size_tx`) on the `ViewerMessageContext`.

**Throttled relay** — in `server/src/sessions/network/viewer/join.rs:261-282`, when a viewer joins, a background task consumes the channel through the `throttle()` utility (reusing `INVALIDATIONS_THROTTLE_PERIOD`). The throttle coalesces rapid size updates so the event bus sees at most one publish per period. The task calls `relay_viewer_terminal_size()`, which publishes an `InvalidationTopicMessage::ViewerTerminalSizeReported` to the event bus.

**Invalidation processing** — in `server/src/sessions/manager/invalidations.rs:309`, the invalidation handler calls `process_viewer_terminal_size_report()` which looks up the session, finds the sharer's `downstream_tx`, and sends `sharer::DownstreamMessage::ViewerTerminalSizeReported`. This event-bus approach ensures cross-instance delivery (the viewer and sharer may be connected to different server instances).

**New module** — `server/src/sessions/manager/viewer_terminal_size.rs` contains `relay_viewer_terminal_size()` and `process_viewer_terminal_size_report()`.

### 3. Client: Viewer Reports Its Size (`warp-internal/app/`)

**Reporting via `resize_internal`** — viewer size reporting is handled inside `TerminalView::resize_internal()` at `app/src/terminal/view.rs:13763-13797` (`maybe_report_viewer_terminal_size`). After every resize, the method checks the `SizeUpdateReason`: if it's `SharerSizeChanged`, reporting is skipped (prevents loops). Otherwise, if the viewer is eligible, it compares the natural rows/cols (stored in `SizeUpdate::natural_rows`/`natural_cols`, captured before shared-session clamping in `SizeUpdateBuilder::build()`) against `last_reported_natural_size` for deduplication, and emits `Event::ReportViewerTerminalSize` when changed.

**Loop prevention** — because the report check is inside `resize_internal`, a `SharerSizeChanged` resize (from `resize_from_sharer_update`) is never re-reported to the sharer. This is strictly more reliable than the previous flag-based approach since it relies on the resize reason, not a flag that must be set/cleared across layout passes.

**Eligibility** — `is_viewer_driven_sizing_eligible(is_sharer, ctx)` at `app/src/terminal/view/shared_session/view_impl.rs:1652-1686` checks viewer count and, for non-ambient-agent sessions, same-user identity. For ambient agent sessions (`SessionSourceType::AmbientAgent`), the `firebase_uid` check is skipped since the agent and user have different UIDs. For the viewer side (`is_sharer=false`): must be the only viewer, and either ambient agent or sharer UID matches ours. For the sharer side (`is_sharer=true`): `present_viewer_count() == 1`, and either ambient agent or viewer UID matches ours.

**Viewer terminal manager wiring** — at `app/src/terminal/shared_session/viewer/terminal_manager.rs:1277-1281`, `TerminalViewEvent::ReportViewerTerminalSize` calls `network.send_report_terminal_size(window_size)`. The `send_report_terminal_size()` method at `viewer/network.rs:938-940` sends `UpstreamMessage::ReportTerminalSize`.

**Viewer-side sizing override** — in `SizeUpdateBuilder::build()` at `app/src/terminal/view.rs:1211-1236`, when the viewer has `last_reported_natural_size.is_some()` (viewer-driven sizing is active), the MAX with the sharer's size is skipped — the viewer uses its own natural size directly. This prevents the viewer from inflating its model size to the sharer's dimensions when the sharer PTY is already sized to the viewer's viewport.

### 4. Client: Sharer Honors Viewer's Size (`warp-internal/app/`)

**New downstream message handling** — in the sharer's `Network::process_websocket_message()` at `app/src/terminal/shared_session/sharer/network.rs:1040-1047`, `DownstreamMessage::ViewerTerminalSizeReported` emits `NetworkEvent::ViewerTerminalSizeReported { window_size }`. The `participant_id` from the protocol message is not propagated since the centralized eligibility check doesn't need it.

**Killswitch** — the sharer's handler for `NetworkEvent::ViewerTerminalSizeReported` at `app/src/terminal/local_tty/terminal_manager.rs:1969-1979` first checks the `viewer_driven_sizing_enabled` setting (`SharedSessionSettings`). If disabled, the report is ignored entirely.

**Decision logic on sharer's terminal_manager** — if the killswitch is enabled, the handler calls `is_viewer_driven_sizing_eligible(true, ctx)`. If eligible, calls `resize_from_viewer_report()`.

**Full-pipeline resize** — `resize_from_viewer_report()` at `view_impl.rs:1693-1704` stores the viewer's reported size in `TerminalView::active_viewer_driven_size`, then builds a `SizeUpdate` with `SizeUpdateReason::ViewerSizeReported` and calls `resize_internal()`. This goes through the normal view/model/PTY pipeline. The `active_viewer_driven_size` field ensures `AfterLayout` uses the viewer's dimensions (not the sharer's natural pane size). The field is cleared by `restore_pty_to_sharer_size()` when viewer-driven sizing becomes ineligible.

**New `SizeUpdateReason::ViewerSizeReported`** — added to `app/src/terminal/mod.rs` alongside `SharerSizeChanged`. Uses the viewer's reported size directly (floored at 1). Old blocks are not reflowed for this reason (in `TerminalModel::resize()`), since the viewer's size is transient.

## Risks and Mitigations

### Sharer's local terminal renders narrow content
When the PTY is resized to the mobile viewer's dimensions, the sharer's desktop terminal will render narrow content. This is acceptable because viewer-driven sizing only activates for self-viewing remote control sessions (same user, 1 viewer) where the sharer is typically not actively looking at their desktop terminal.

### Race between viewer join and size report
The viewer may not have its layout computed when it first joins. The first `after_terminal_view_layout()` pass that computes a natural size will report it; subsequent layout passes correct it as the view settles.

### Multiple rapid resize events
Server-side throttling (via the `throttle()` utility with `TERMINAL_SIZE_THROTTLE_PERIOD`) coalesces rapid viewer size reports. The sharer applies the latest received size.

## Testing and Validation
- Verify that a viewer's terminal size is reported to the sharer on layout and on browser resize.
- Verify that the sharer resizes its PTY when conditions are met (1 viewer, same user).
- Verify no resize happens when the viewer is a different user.
- Verify no resize happens when there are more than 1 viewer.
- Verify no regressions in normal shared session resize behavior.
- Verify that the viewer skips the sharer-size MAX when viewer-driven sizing is active.
- Verify no resize loop when the viewer resizes (sharer echoes back a resize, viewer suppresses re-report).
- Verify that `restore_pty_to_sharer_size()` correctly restores the sharer's natural dimensions when the viewer disconnects.
