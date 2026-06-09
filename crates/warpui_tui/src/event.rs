//! TUI event plumbing.
//!
//! The runtime converts raw crossterm events into the shared
//! [`warpui_core::Event`] vocabulary (so element/view dispatch is identical to
//! the GUI), then walks the rendered element tree handing each element the
//! event plus a [`TuiEventContext`] it can use to queue app updates and typed
//! actions back into the shared core.
//!
//! This module freezes the event *types* and the *signature* of
//! [`crossterm_event_to_warp_event`]. The crossterm→warp mapping itself, along
//! with any scroll/mouse normalization helpers, is implemented by the renderer/
//! runtime layer (task 3.4).

use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use warpui_core::event::{KeyEventDetails, ModifiersState};
use warpui_core::geometry::vector::{vec2f, Vector2F};
use warpui_core::keymap::Keystroke;
use warpui_core::{Action, App, EntityId, Event};

/// Whether an element that handled an event wants its ancestors to keep seeing
/// it. Returned by event-aware elements during dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiDispatchEventResult {
    /// Continue offering the event to ancestor elements.
    PropagateToParent,
    /// Consume the event; ancestors do not see it.
    StopPropagation,
}

/// The outcome of dispatching an event through a rendered tree: whether any
/// element handled it (e.g. to decide if a redraw is warranted).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiEventDispatchResult {
    pub handled: bool,
}

type TuiAppUpdate = Box<dyn FnOnce(&mut App)>;

/// Collects the side effects an element requests while handling an event:
/// deferred mutations of the [`App`] and typed actions to dispatch through the
/// shared core. The runtime drains these after dispatch and applies them on the
/// main thread, mirroring how GUI event handlers defer work via the app context.
#[derive(Default)]
pub struct TuiEventContext {
    updates: Vec<TuiAppUpdate>,
    typed_actions: Vec<TuiDispatchedAction>,
    origin_view_id: Option<EntityId>,
}

// Consumed by the runtime's dispatch loop (task 3.4); inert until then.
#[allow(dead_code)]
pub(crate) struct TuiDispatchedAction {
    pub(crate) origin_view_id: EntityId,
    pub(crate) action: Box<dyn Action>,
}

impl TuiEventContext {
    /// Queues a closure to run against the [`App`] once dispatch completes.
    pub fn dispatch_app_update<F>(&mut self, update: F)
    where
        F: 'static + FnOnce(&mut App),
    {
        self.updates.push(Box::new(update));
    }

    /// Queues a typed action to dispatch from the view currently being
    /// processed. Panics if called outside of view event processing, where
    /// there is no origin view to attribute the action to.
    pub fn dispatch_typed_action(&mut self, action: impl Action) {
        let origin_view_id = self
            .origin_view_id
            .expect("typed actions can only be dispatched while processing a rendered TUI view");
        self.typed_actions.push(TuiDispatchedAction {
            origin_view_id,
            action: Box::new(action),
        });
    }

    // The `take_*`/`set_origin_view` plumbing is drained by the runtime's
    // dispatch loop (task 3.4); it is inert within the foundation crate alone.
    #[allow(dead_code)]
    pub(crate) fn take_updates(&mut self) -> Vec<TuiAppUpdate> {
        std::mem::take(&mut self.updates)
    }

    #[allow(dead_code)]
    pub(crate) fn take_typed_actions(&mut self) -> Vec<TuiDispatchedAction> {
        std::mem::take(&mut self.typed_actions)
    }

    /// Sets the view that subsequently dispatched actions are attributed to,
    /// returning the previous origin so callers can restore it when leaving the
    /// view's subtree.
    #[allow(dead_code)]
    pub(crate) fn set_origin_view(&mut self, view_id: Option<EntityId>) -> Option<EntityId> {
        std::mem::replace(&mut self.origin_view_id, view_id)
    }
}

/// Converts a raw crossterm event into the shared [`warpui_core::Event`]
/// vocabulary, or `None` if the event has no warp equivalent.
///
/// FROZEN SIGNATURE. The mapping is implemented by the renderer/runtime layer
/// (task 3.4); the signature is fixed here so sibling tasks can build against
/// it.
pub fn crossterm_event_to_warp_event(event: CrosstermEvent) -> Option<Event> {
    match event {
        CrosstermEvent::Key(key_event) => key_event_to_warp_event(key_event),
        CrosstermEvent::Mouse(mouse_event) => mouse_event_to_warp_event(mouse_event),
        CrosstermEvent::FocusGained
        | CrosstermEvent::FocusLost
        | CrosstermEvent::Paste(_)
        | CrosstermEvent::Resize(_, _) => None,
    }
}

fn key_event_to_warp_event(event: KeyEvent) -> Option<Event> {
    // Only key presses map to a warp `KeyDown`; repeats/releases are ignored so
    // dispatch matches the GUI's press-driven keystroke model.
    if event.kind != KeyEventKind::Press {
        return None;
    }

    let key = key_name(event.code, event.modifiers)?;
    let chars = match event.code {
        KeyCode::Char(char) => char.to_string(),
        _ => String::new(),
    };

    Some(Event::KeyDown {
        keystroke: Keystroke {
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
            cmd: event.modifiers.contains(KeyModifiers::SUPER),
            meta: event.modifiers.contains(KeyModifiers::META),
            key,
        },
        chars,
        details: KeyEventDetails {
            key_without_modifiers: key_without_modifiers(event.code),
            ..Default::default()
        },
        is_composing: false,
    })
}

/// The warp keystroke `key` name for a crossterm key code, or `None` for keys
/// with no warp equivalent (pure modifiers, lock keys, media keys, etc.).
fn key_name(code: KeyCode, modifiers: KeyModifiers) -> Option<String> {
    match code {
        KeyCode::Backspace => Some("backspace".to_owned()),
        KeyCode::Enter => Some("enter".to_owned()),
        KeyCode::Left => Some("left".to_owned()),
        KeyCode::Right => Some("right".to_owned()),
        KeyCode::Up => Some("up".to_owned()),
        KeyCode::Down => Some("down".to_owned()),
        KeyCode::Home => Some("home".to_owned()),
        KeyCode::End => Some("end".to_owned()),
        KeyCode::PageUp => Some("pageup".to_owned()),
        KeyCode::PageDown => Some("pagedown".to_owned()),
        KeyCode::Tab | KeyCode::BackTab => Some("\t".to_owned()),
        KeyCode::Delete => Some("delete".to_owned()),
        KeyCode::Insert => Some("insert".to_owned()),
        KeyCode::Esc => Some("escape".to_owned()),
        KeyCode::F(number) if number <= 20 => Some(format!("f{number}")),
        KeyCode::Char(' ') => Some(" ".to_owned()),
        KeyCode::Char(char) if modifiers.contains(KeyModifiers::SHIFT) => Some(char.to_string()),
        KeyCode::Char(char) => Some(char.to_lowercase().to_string()),
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_)
        | KeyCode::F(_) => None,
    }
}

fn key_without_modifiers(code: KeyCode) -> Option<String> {
    match code {
        KeyCode::Char(char) => Some(char.to_lowercase().to_string()),
        _ => None,
    }
}

fn mouse_event_to_warp_event(event: MouseEvent) -> Option<Event> {
    let position = vec2f(f32::from(event.column), f32::from(event.row));
    let modifiers = modifiers_state(event.modifiers);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Event::LeftMouseDown {
            position,
            modifiers,
            click_count: 1,
            is_first_mouse: false,
        }),
        MouseEventKind::Up(MouseButton::Left) => Some(Event::LeftMouseUp {
            position,
            modifiers,
        }),
        MouseEventKind::Drag(MouseButton::Left) => Some(Event::LeftMouseDragged {
            position,
            modifiers,
        }),
        MouseEventKind::Down(MouseButton::Middle) => Some(Event::MiddleMouseDown {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            click_count: 1,
        }),
        MouseEventKind::Down(MouseButton::Right) => Some(Event::RightMouseDown {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            click_count: 1,
        }),
        MouseEventKind::Moved => Some(Event::MouseMoved {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            is_synthetic: false,
        }),
        MouseEventKind::ScrollUp => Some(scroll_wheel_event(position, modifiers, vec2f(0.0, 1.0))),
        MouseEventKind::ScrollDown => {
            Some(scroll_wheel_event(position, modifiers, vec2f(0.0, -1.0)))
        }
        MouseEventKind::ScrollLeft => {
            Some(scroll_wheel_event(position, modifiers, vec2f(-1.0, 0.0)))
        }
        MouseEventKind::ScrollRight => {
            Some(scroll_wheel_event(position, modifiers, vec2f(1.0, 0.0)))
        }
        MouseEventKind::Up(MouseButton::Middle | MouseButton::Right)
        | MouseEventKind::Drag(MouseButton::Middle | MouseButton::Right) => None,
    }
}

fn scroll_wheel_event(position: Vector2F, modifiers: ModifiersState, delta: Vector2F) -> Event {
    Event::ScrollWheel {
        position,
        delta,
        precise: false,
        modifiers,
    }
}

fn modifiers_state(modifiers: KeyModifiers) -> ModifiersState {
    ModifiersState {
        alt: modifiers.contains(KeyModifiers::ALT),
        cmd: modifiers.contains(KeyModifiers::SUPER),
        shift: modifiers.contains(KeyModifiers::SHIFT),
        ctrl: modifiers.contains(KeyModifiers::CONTROL),
        func: false,
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
