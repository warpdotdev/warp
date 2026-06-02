use warpui_core::event::ModifiersState;

use super::indexing::Point;

#[derive(Debug, Copy, Clone)]
pub enum MouseButton {
    Left,
    Right,
    Wheel,
    LeftDrag,
    Move, // Used for mouse hover events (when cursor is moving)
}
#[derive(Debug, Copy, Clone)]
pub enum MouseAction {
    Pressed,
    Released,
    Scrolled { delta: i32 },
}

#[derive(Debug, Copy, Clone)]
pub struct MouseState {
    button: MouseButton,
    action: MouseAction,
    point: Option<Point>,
    modifiers: ModifiersState,
}

impl MouseState {
    pub fn new(button: MouseButton, action: MouseAction, modifiers: ModifiersState) -> Self {
        Self {
            button,
            action,
            point: None,
            modifiers,
        }
    }

    pub fn set_point(mut self, p: Point) -> Self {
        self.point = Some(p);
        self
    }

    pub fn button(&self) -> &MouseButton {
        &self.button
    }
    pub fn action(&self) -> &MouseAction {
        &self.action
    }

    pub fn maybe_point(&self) -> Option<Point> {
        self.point
    }

    pub fn modifiers(&self) -> &ModifiersState {
        &self.modifiers
    }
}
