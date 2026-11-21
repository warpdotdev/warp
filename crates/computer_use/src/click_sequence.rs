use crate::{Action, MouseButton, Target, Vector2I};

#[derive(Default)]
pub(crate) struct ClickSequence {
    previous_press: Option<(Target, MouseButton, Option<Vector2I>)>,
    count: i64,
    released: bool,
}

impl ClickSequence {
    pub fn next(&mut self, target: Target, action: &Action) -> i64 {
        match action {
            Action::MouseDown { button, at } => {
                let press = (target, *button, *at);
                self.count = if self.released && self.previous_press == Some(press) {
                    self.count.saturating_add(1)
                } else {
                    1
                };
                self.previous_press = Some(press);
                self.released = false;
                self.count
            }
            Action::MouseUp { button } => {
                if !self.released
                    && self
                        .previous_press
                        .is_some_and(|(press_target, press_button, _)| {
                            press_target == target && press_button == *button
                        })
                {
                    self.released = true;
                    self.count
                } else {
                    *self = Self::default();
                    1
                }
            }
            Action::Wait(_)
            | Action::MouseMove { .. }
            | Action::MouseWheel { .. }
            | Action::TypeText { .. }
            | Action::KeyDown { .. }
            | Action::KeyUp { .. } => {
                *self = Self::default();
                0
            }
        }
    }
}

#[cfg(test)]
#[path = "click_sequence_tests.rs"]
mod tests;
