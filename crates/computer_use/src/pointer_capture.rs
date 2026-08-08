//! Recording of resolved pointer events into the active recording's
//! [`PointerSink`], shared by every actor that supports overlay burn-in.
//!
//! Actors dispatch pointer actions in two coordinate spaces — the window-local
//! pixels a caller addressed and the resolved global pixels the event was
//! actually posted at — and the recording keeps whichever one matches its own
//! capture space.

use instant::Instant;
use pathfinder_geometry::vector::Vector2I;

use crate::overlay::{PointerEvent, PointerEventKind};
use crate::{MouseButton, PointerSink, Target};

/// Resolves an action's coordinate into the recording's capture-space pixels, or
/// `None` when the action's surface does not match the recording (so the event
/// is omitted rather than drawn at the wrong place).
pub(crate) fn resolve_capture_point(
    recording_target: Target,
    action_target: Target,
    local: Vector2I,
    global: Vector2I,
) -> Option<Vector2I> {
    match recording_target {
        // Full-screen capture: everything maps to global screen pixels.
        Target::Screen => Some(global),
        // Window capture: only actions on the recorded window resolve, using the
        // window-local pixels that match the captured window's frame.
        Target::Window {
            window_id: recorded,
            ..
        } => match action_target {
            Target::Window { window_id, .. } if window_id == recorded => Some(local),
            _ => None,
        },
    }
}

/// Records a resolved coordinate-carrying pointer event (a press, move, or
/// scroll position sample) into the pointer sink, updating the recording-scoped
/// pointer session so a later release (which carries no coordinate) can reuse
/// the last point — even when that release arrives in a later `UseComputer`
/// call. An event whose surface does not match the recording clears the session
/// so a following release is not recorded at a stale coordinate.
pub(crate) fn record_positioned_event(
    pointer_sink: Option<&PointerSink>,
    kind: PointerEventKind,
    button: Option<MouseButton>,
    action_target: Target,
    local: Vector2I,
    global: Vector2I,
) {
    let Some(sink) = pointer_sink else {
        return;
    };
    match resolve_capture_point(sink.recording_target, action_target, local, global) {
        Some(point) => {
            sink.session.record_press_or_move(kind, button, point);
            push_pointer_event(sink, point, kind, button);
        }
        None => {
            // Any unmatched coordinate-carrying event (a surface that isn't the
            // recorded one) invalidates the active pointer state, so a following
            // release is not recorded at a stale in-frame coordinate.
            sink.session.clear();
        }
    }
}

/// Records a release at the session's last resolved point (a release carries no
/// coordinate of its own), but only when the released button matches the active
/// press. Omitted when there is no matching active press — for example when the
/// press was on a non-recorded surface, in a failed/cancelled call whose session
/// was reset, or for a button that was never pressed.
pub(crate) fn record_up(pointer_sink: Option<&PointerSink>, button: MouseButton) {
    let Some(sink) = pointer_sink else {
        return;
    };
    if let Some(point) = sink.session.record_release(button) {
        push_pointer_event(sink, point, PointerEventKind::Up, Some(button));
    }
}

fn push_pointer_event(
    sink: &PointerSink,
    point: Vector2I,
    kind: PointerEventKind,
    button: Option<MouseButton>,
) {
    let offset = Instant::now()
        .checked_duration_since(sink.started_at)
        .unwrap_or_default();
    if let Ok(mut events) = sink.events.lock() {
        events.push(PointerEvent {
            offset,
            kind,
            button,
            point,
        });
    }
}

#[cfg(test)]
#[path = "pointer_capture_tests.rs"]
mod tests;
