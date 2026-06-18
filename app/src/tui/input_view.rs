//! [`TuiInputView`]: the prototype's editable prompt. It owns a single-line
//! draft plus a cursor and turns key input into typed editing actions, emitting
//! [`InputEvent::Submitted`] when the user presses Enter on a non-empty draft.
//!
//! The view holds a plain `String` + char-index cursor rather than reusing the
//! GUI `EditorModel`, whose CRDT/selection/display-map machinery is coupled to
//! GUI text layout and disproportionate for a single-line prototype.

use warpui_core::elements::tui::{
    Color, TuiContainer, TuiElement, TuiEventHandler, TuiInputLine, TuiStyle,
};
use warpui_core::{AppContext, Entity, Event, TuiView, TypedActionView, ViewContext};

/// Placeholder shown when the draft is empty (mirrors the Figma mock).
const PLACEHOLDER: &str =
    "Warp anything e.g. Create an agent that summarizes repository activity each day";

/// Near-white draft text (`#f1f1f1`).
const TEXT_COLOR: Color = Color::Rgb(0xf1, 0xf1, 0xf1);
/// Dim gray placeholder text (`#8e8e8e`).
const PLACEHOLDER_COLOR: Color = Color::Rgb(0x8e, 0x8e, 0x8e);

/// Events emitted by [`TuiInputView`] for ancestors to observe.
#[derive(Debug)]
pub enum InputEvent {
    /// The user submitted a draft. The text is emitted verbatim; only
    /// whitespace-only drafts are suppressed (never emitted).
    Submitted(String),
}

/// The typed editing actions the input dispatches from its key handlers.
#[derive(Debug)]
pub enum InputAction {
    Insert(String),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Submit,
}

#[derive(Default)]
pub struct TuiInputView {
    draft: String,
    /// Cursor position as a char index into `draft`, in `[0, draft char count]`.
    cursor: usize,
}

impl TuiInputView {
    fn char_count(&self) -> usize {
        self.draft.chars().count()
    }

    /// The byte offset of char index `char_idx` (or the string length when the
    /// index is at the end).
    fn byte_index(&self, char_idx: usize) -> usize {
        self.draft
            .char_indices()
            .nth(char_idx)
            .map_or(self.draft.len(), |(byte, _)| byte)
    }

    fn insert(&mut self, text: &str) {
        let byte = self.byte_index(self.cursor);
        self.draft.insert_str(byte, text);
        self.cursor += text.chars().count();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            let start = self.byte_index(self.cursor - 1);
            let end = self.byte_index(self.cursor);
            self.draft.replace_range(start..end, "");
            self.cursor -= 1;
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.char_count() {
            let start = self.byte_index(self.cursor);
            let end = self.byte_index(self.cursor + 1);
            self.draft.replace_range(start..end, "");
        }
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_count());
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.char_count();
    }

    /// Takes the draft for submission, clearing it and resetting the cursor.
    /// Returns `None` (leaving the draft untouched) when it is whitespace-only.
    fn take_submission(&mut self) -> Option<String> {
        if self.draft.trim().is_empty() {
            return None;
        }
        self.cursor = 0;
        Some(std::mem::take(&mut self.draft))
    }
}

impl Entity for TuiInputView {
    type Event = InputEvent;
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        let mut input_line = TuiInputLine::new(self.draft.clone(), self.cursor)
            .with_style(TuiStyle::default().fg(TEXT_COLOR));
        if self.draft.is_empty() {
            input_line =
                input_line.with_placeholder(PLACEHOLDER, TuiStyle::default().fg(PLACEHOLDER_COLOR));
        }

        let frame = TuiContainer::new(input_line).with_rounded_border();

        let handler = TuiEventHandler::new(frame)
            .on_key("enter", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Submit)
            })
            .on_key("backspace", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Backspace)
            })
            .on_key("delete", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Delete)
            })
            .on_key("left", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Left)
            })
            .on_key("right", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Right)
            })
            .on_key("home", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::Home)
            })
            .on_key("end", |_, ctx, _| {
                ctx.dispatch_typed_action(InputAction::End)
            })
            .on_key_fallback(|event, ctx, _| {
                let Event::KeyDown {
                    keystroke, chars, ..
                } = event
                else {
                    return false;
                };
                // Only consume unmodified, printable input; let chorded keys
                // (Ctrl-C, etc.) propagate to the runtime/ancestors.
                if keystroke.ctrl || keystroke.alt || keystroke.cmd || keystroke.meta {
                    return false;
                }
                if chars.is_empty() || chars.chars().any(char::is_control) {
                    return false;
                }
                ctx.dispatch_typed_action(InputAction::Insert(chars.clone()));
                true
            });

        Box::new(handler)
    }
}

impl TypedActionView for TuiInputView {
    type Action = InputAction;

    fn handle_action(&mut self, action: &InputAction, ctx: &mut ViewContext<Self>) {
        match action {
            InputAction::Insert(text) => self.insert(text),
            InputAction::Backspace => self.backspace(),
            InputAction::Delete => self.delete(),
            InputAction::Left => self.move_left(),
            InputAction::Right => self.move_right(),
            InputAction::Home => self.move_home(),
            InputAction::End => self.move_end(),
            InputAction::Submit => {
                if let Some(text) = self.take_submission() {
                    ctx.emit(InputEvent::Submitted(text));
                }
            }
        }
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "input_view_tests.rs"]
mod tests;
