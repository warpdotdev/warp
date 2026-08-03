use std::ops::Range;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use crate::EventContext;
use crate::elements::{Point, ZIndex};
use crate::keymap::Keystroke;
use crate::platform::keyboard::KeyCode;
use crate::zoom::{Scale, ZoomFactor};

#[derive(Debug)]
pub struct DispatchedEvent {
    event: Event,
}

impl DispatchedEvent {
    /// Filters out event types that most-likely shouldn't be handled if this
    /// event is being received by an element at the given z-index
    pub fn at_z_index(&self, z_index: ZIndex, ctx: &EventContext) -> Option<&Event> {
        match self.event {
            Event::KeyDown { .. } => Some(&self.event),
            Event::ScrollWheel { position, .. }
            | Event::LeftMouseDown { position, .. }
            | Event::LeftMouseUp { position, .. }
            | Event::LeftMouseDragged { position, .. }
            | Event::MiddleMouseDown { position, .. }
            | Event::RightMouseDown { position, .. }
            | Event::BackMouseDown { position, .. }
            | Event::ForwardMouseDown { position, .. } => {
                if !ctx.is_covered(Point::from_vec2f(position, z_index)) {
                    Some(&self.event)
                } else {
                    None
                }
            }
            Event::MouseMoved { .. } => Some(&self.event),
            Event::ModifierStateChanged { .. } => Some(&self.event),
            Event::ModifierKeyChanged { .. } => Some(&self.event),
            Event::TypedCharacters { .. } => Some(&self.event),
            Event::DragAndDropFiles { .. } => Some(&self.event),
            Event::DragFiles { .. } => Some(&self.event),
            Event::DragFileExit => Some(&self.event),
            Event::SetMarkedText { .. } => Some(&self.event),
            Event::ClearMarkedText => Some(&self.event),
        }
    }

    /// Returns the raw event - note that an element at a higher z-index
    /// may already have handled it.
    pub fn raw_event(&self) -> &Event {
        &self.event
    }
}

impl From<Event> for DispatchedEvent {
    fn from(event: Event) -> Self {
        Self { event }
    }
}

/// Additional metadata about key events.
#[derive(Clone, Debug, Default)]
pub struct KeyEventDetails {
    // allows distinguishing between left and right alt
    pub left_alt: bool,
    pub right_alt: bool,
    /// The key that would have been produced without any modifiers (including Shift).
    pub key_without_modifiers: Option<String>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct ModifiersState {
    pub alt: bool,
    pub cmd: bool,
    pub shift: bool,
    pub ctrl: bool,
    /// The function key, often labeled as "fn" on keyboards.
    /// We use "func" to avoid clashing with the `fn` keyword.
    /// Note this is NOT fully implemented for non-Mac platforms yet.
    pub func: bool,
}

#[derive(Copy, Clone, Debug)]
pub enum KeyState {
    Pressed,
    Released,
}

/// TODO: for the events that have modifiers (e.g. cmd, shift), we should
/// combine these into a Modifiers struct and pass these along from fn to fn.
#[derive(Clone, Debug, strum_macros::EnumDiscriminants)]
pub enum Event {
    /// Gets fired when a key is pressed. The keystroke attribute contains the raw
    /// key code and its modifiers.
    KeyDown {
        keystroke: Keystroke,
        chars: String,
        details: KeyEventDetails,
        is_composing: bool,
    },
    ScrollWheel {
        position: Vector2F,
        delta: Vector2F,
        precise: bool,
        modifiers: ModifiersState,
    },
    LeftMouseDown {
        position: Vector2F,
        modifiers: ModifiersState,
        click_count: u32,
        /// Whether this is the first mouse down event on an inactive window
        /// that is causing the window to activate.
        is_first_mouse: bool,
    },
    LeftMouseUp {
        position: Vector2F,
        modifiers: ModifiersState,
    },
    LeftMouseDragged {
        position: Vector2F,
        modifiers: ModifiersState,
    },
    MiddleMouseDown {
        position: Vector2F,
        cmd: bool,
        shift: bool,
        click_count: u32,
    },
    RightMouseDown {
        position: Vector2F,
        cmd: bool,
        shift: bool,
        click_count: u32,
    },
    /// One of the side buttons often found on external mice.
    /// It cycles forward between tabs by default.
    ForwardMouseDown {
        position: Vector2F,
        cmd: bool,
        shift: bool,
        click_count: u32,
    },
    /// One of the side buttons often found on external mice.
    /// It cycles backward between tabs by default.
    BackMouseDown {
        position: Vector2F,
        cmd: bool,
        shift: bool,
        click_count: u32,
    },
    MouseMoved {
        position: Vector2F,
        cmd: bool,
        shift: bool,
        /// Whether this mouse event was initiated by the user or by Warp synthetically.
        /// We create such synthetic mouse events for certain behaviors behaviors within
        /// Warp, such as triggering the correct tab to be "hovered" when the user closes
        /// a tab to the left. In this case, there's no user-initiated mouse event, however,
        /// we create a synthetic mouse event so that we carry over the "hovered" state from
        /// the old tab (closed) to the new tab (shifted). Certain elements within Warp,
        /// such as the alt-screen, may not want such synthetic mouse events since it can
        /// interfere with actions such as mouse dragging.
        is_synthetic: bool,
    },
    /// Gets fired when the modifier flag states changed -- this could happen either
    /// when a user presses down on or releases a modifier key.
    ModifierStateChanged {
        // Note that in web framework modifier keypresses do not contain mouse
        // position information. But we also have cases where modifier flag state
        // is closely coupled with mouse position for determine whether certain events
        // should be fired. This position meta data will be kept in the event for now,
        // we could always remove it in the future if this does not fit.
        mouse_position: Vector2F,
        modifiers: ModifiersState,
        /// The specific key code for the event. Can be used to identify which modifier key
        /// was pressed or released.
        key_code: Option<KeyCode>,
    },
    /// Gets fired when a modifier key is pressed/released. Contains details on whether the
    /// key was pressed or released and the key code.
    ModifierKeyChanged {
        key_code: KeyCode,
        state: KeyState,
    },
    /// Gets fired when a printable character is produced by the text input system and
    /// the corresponding KeyDown event is not handled. This event is not dispatched
    /// when intermediary keys are pressed (e.g. dead keys or key presses in the IME).
    /// TypedCharacter needs to be of type String because CJK languages represent words
    /// with a set of characters and thus the output from IME could be more than one single character.
    TypedCharacters {
        chars: String,
    },
    /// Gets fired when user drags a file or folder into Warp. Note that there could exist
    /// multiple file paths in one event as user could drag and drop multiple targets.
    DragAndDropFiles {
        paths: Vec<String>,
        location: Vector2F,
    },

    DragFiles {
        location: Vector2F,
    },

    DragFileExit,

    SetMarkedText {
        marked_text: String,
        selected_range: Range<usize>,
    },

    ClearMarkedText,
}

impl Event {
    /// Returns the mouse-down position of the event,
    /// iff the event is one of the many `*MouseDown` events.
    pub fn mouse_down_position(&self) -> Option<Vector2F> {
        match self {
            Self::LeftMouseDown { position, .. }
            | Self::RightMouseDown { position, .. }
            | Self::MiddleMouseDown { position, .. }
            | Self::ForwardMouseDown { position, .. }
            | Self::BackMouseDown { position, .. } => Some(*position),
            _ => None,
        }
    }

    /// Returns a copy of the event marked as synthetic, if this is a
    /// `MouseMoved` event, otherwise returns [`None`].
    pub fn to_synthetic_mouse_move_event(&self) -> Option<Self> {
        if let Event::MouseMoved {
            cmd,
            shift,
            position,
            is_synthetic: _,
        } = self
        {
            Some(Event::MouseMoved {
                cmd: *cmd,
                shift: *shift,
                position: *position,
                is_synthetic: true,
            })
        } else {
            None
        }
    }
}

pub trait InBoundsExt {
    /// Check whether something occurred within the given bounding box.
    fn in_bounds(&self, bounds: RectF) -> bool;
}

impl InBoundsExt for Event {
    fn in_bounds(&self, bounds: RectF) -> bool {
        // This trait is meant to check whether the Self is within the bounds,
        // however in this implementation Self is Event that may not always be related to the
        // mouse - so for all such cases, lets just return true.
        match self {
            Event::ScrollWheel { position, .. }
            | Event::LeftMouseDown { position, .. }
            | Event::LeftMouseUp { position, .. }
            | Event::LeftMouseDragged { position, .. }
            | Event::MiddleMouseDown { position, .. }
            | Event::RightMouseDown { position, .. }
            | Event::ForwardMouseDown { position, .. }
            | Event::BackMouseDown { position, .. }
            | Event::MouseMoved { position, .. } => bounds.contains_point(*position),
            Event::ModifierStateChanged { mouse_position, .. } => {
                bounds.contains_point(*mouse_position)
            }
            Event::DragAndDropFiles { location, .. } | Event::DragFiles { location } => {
                bounds.contains_point(*location)
            }
            Event::KeyDown { .. }
            | Event::ModifierKeyChanged { .. }
            | Event::TypedCharacters { .. }
            | Event::DragFileExit
            | Event::SetMarkedText { .. }
            | Event::ClearMarkedText => true,
        }
    }
}

impl Scale for Event {
    fn scale_up(self, zoom_factor: ZoomFactor) -> Self {
        match self {
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers,
            } => Event::ScrollWheel {
                position: position.scale_up(zoom_factor),
                delta,
                precise,
                modifiers,
            },
            Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
            } => Event::LeftMouseDown {
                position: position.scale_up(zoom_factor),
                modifiers,
                click_count,
                is_first_mouse,
            },
            Event::LeftMouseUp {
                position,
                modifiers,
            } => Event::LeftMouseUp {
                position: position.scale_up(zoom_factor),
                modifiers,
            },
            Event::LeftMouseDragged {
                position,
                modifiers,
            } => Event::LeftMouseDragged {
                position: position.scale_up(zoom_factor),
                modifiers,
            },
            Event::MiddleMouseDown {
                position,
                cmd,
                shift,
                click_count,
            } => Event::MiddleMouseDown {
                position: position.scale_up(zoom_factor),
                cmd,
                shift,
                click_count,
            },
            Event::RightMouseDown {
                position,
                cmd,
                shift,
                click_count,
            } => Event::RightMouseDown {
                position: position.scale_up(zoom_factor),
                cmd,
                shift,
                click_count,
            },
            Event::ForwardMouseDown {
                position,
                cmd,
                shift,
                click_count,
            } => Event::ForwardMouseDown {
                position: position.scale_up(zoom_factor),
                cmd,
                shift,
                click_count,
            },
            Event::BackMouseDown {
                position,
                cmd,
                shift,
                click_count,
            } => Event::BackMouseDown {
                position: position.scale_up(zoom_factor),
                cmd,
                shift,
                click_count,
            },
            Event::MouseMoved {
                position,
                cmd,
                shift,
                is_synthetic,
            } => Event::MouseMoved {
                position: position.scale_up(zoom_factor),
                cmd,
                shift,
                is_synthetic,
            },
            Event::ModifierStateChanged {
                mouse_position,
                modifiers,
                key_code,
            } => Event::ModifierStateChanged {
                mouse_position: mouse_position.scale_up(zoom_factor),
                modifiers,
                key_code,
            },
            Event::DragAndDropFiles { paths, location } => Event::DragAndDropFiles {
                paths,
                location: location.scale_up(zoom_factor),
            },
            Event::DragFiles { location } => Event::DragFiles {
                location: location.scale_up(zoom_factor),
            },
            Event::DragFileExit => Event::DragFileExit,
            Event::KeyDown {
                keystroke,
                chars,
                details,
                is_composing,
            } => Event::KeyDown {
                keystroke,
                chars,
                details,
                is_composing,
            },
            Event::ModifierKeyChanged { key_code, state } => {
                Event::ModifierKeyChanged { key_code, state }
            }
            Event::TypedCharacters { chars } => Event::TypedCharacters { chars },
            Event::SetMarkedText {
                marked_text,
                selected_range,
            } => Event::SetMarkedText {
                marked_text,
                selected_range,
            },
            Event::ClearMarkedText => Event::ClearMarkedText,
        }
    }
}
