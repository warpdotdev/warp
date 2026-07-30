//! Generic focusable TUI text field over the shared editor model and element.
//!
//! Unlike [`crate::input::TuiInputView`], this view owns no prompt submission,
//! input-mode, inline-menu, or form policy. Embedding views provide that chrome
//! and behavior while reusing model-backed editing and focus handling.

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{TuiElement, TuiHoverable};
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext,
};

use crate::editor_element::{TuiEditorAction, TuiEditorElement};
use crate::editor_interaction::{
    TuiEditorBehavior, TuiEditorCommand, TuiEditorInteractionOutcome, TuiEditorState,
    apply_editor_action, apply_editor_clipboard_action, follow_editor_cursor,
};

#[derive(Clone, Copy)]
pub(crate) enum TuiEditorVerticalDirection {
    Up,
    Down,
}

impl From<TuiEditorVerticalDirection> for TuiEditorCommand {
    fn from(direction: TuiEditorVerticalDirection) -> Self {
        match direction {
            TuiEditorVerticalDirection::Up => Self::MoveUp,
            TuiEditorVerticalDirection::Down => Self::MoveDown,
        }
    }
}

/// Events emitted when the editor content changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiEditorViewEvent {
    Changed(String),
}

/// Actions raised by the shared editor element or editor chrome.
#[derive(Clone, Debug)]
pub(crate) enum TuiEditorViewAction {
    FocusRequested,
    Editor(TuiEditorAction),
    Command(TuiEditorCommand),
}

/// A reusable single-line editor view.
pub(crate) struct TuiEditorView {
    model: ModelHandle<CodeEditorModel>,
    editor_state: TuiEditorState,
    editor_behavior: TuiEditorBehavior,
    focused: bool,
    mouse_state: MouseStateHandle,
}

impl TuiEditorView {
    /// Creates an empty single-line editor backed by a char-cell model.
    pub(crate) fn single_line(ctx: &mut ViewContext<Self>) -> Self {
        let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(1, ctx));
        ctx.subscribe_to_model(&model, |editor, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            let text = editor.text(ctx);
            ctx.emit(TuiEditorViewEvent::Changed(text));
            ctx.notify();
        });
        Self {
            model,
            editor_state: TuiEditorState::default(),
            editor_behavior: TuiEditorBehavior::single_line(),
            focused: false,
            mouse_state: MouseStateHandle::default(),
        }
    }

    /// Creates an empty multiline editor with a bounded viewport.
    pub(crate) fn multiline(viewport_rows: u32, ctx: &mut ViewContext<Self>) -> Self {
        let mut view = Self::single_line(ctx);
        view.editor_behavior = TuiEditorBehavior::multiline(viewport_rows);
        view
    }

    /// Returns the current editor text.
    pub(crate) fn text(&self, ctx: &AppContext) -> String {
        let model = self.model.as_ref(ctx);
        let buffer = model.content().as_ref(ctx);
        if buffer.is_empty() {
            String::new()
        } else {
            buffer.text().into_string()
        }
    }

    /// Returns whether the editor owns focus.
    pub(crate) fn is_focused(&self) -> bool {
        self.focused
    }

    /// Replaces editor content without emitting `Changed`.
    pub(crate) fn set_text(&mut self, text: impl Into<String>, ctx: &mut ViewContext<Self>) {
        let text = text.into();
        let text = self.editor_behavior.normalize_text(&text).to_string();
        if self.text(ctx) == text {
            return;
        }
        self.model.update(ctx, |model, ctx| {
            model.reset_content(InitialBufferState::plain_text(&text), ctx);
        });
        ctx.notify();
    }

    /// Renders the shared editor configured as a one-row field.
    fn render_editor(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        TuiEditorElement::new(&self.model, ctx)
            .with_view_focused(self.focused)
            .editable()
            .with_viewport_rows(self.editor_behavior.viewport_rows())
            .on_action(|action, event_ctx| {
                event_ctx.dispatch_typed_action(TuiEditorViewAction::Editor(action));
            })
            .finish()
    }

    /// Applies an editor action using the same model operations as `TuiInputView`.
    fn handle_editor_action(
        &mut self,
        action: &TuiEditorAction,
        ctx: &mut ViewContext<Self>,
    ) -> TuiEditorInteractionOutcome {
        if matches!(
            action,
            TuiEditorAction::SelectionStartAt { .. }
                | TuiEditorAction::SelectionExtendTo { .. }
                | TuiEditorAction::SelectWordAt { .. }
                | TuiEditorAction::SelectLineAt { .. }
        ) {
            ctx.focus_self();
        }
        apply_editor_action(&self.model, action, self.editor_behavior, ctx)
    }

    /// Applies a keybound editor command to the shared editor model.
    fn handle_command(
        &mut self,
        command: TuiEditorCommand,
        ctx: &mut ViewContext<Self>,
    ) -> TuiEditorInteractionOutcome {
        self.editor_state
            .apply_command(&self.model, command, self.editor_behavior, ctx)
    }

    pub(crate) fn can_move_vertically(
        &self,
        direction: TuiEditorVerticalDirection,
        ctx: &AppContext,
    ) -> bool {
        let model = self.model.as_ref(ctx);
        let cursor_offset = model
            .selection_model()
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default();
        let render = model.render_state().as_ref(ctx);
        let Some(char_cell) = render.char_cell() else {
            return false;
        };
        let cursor_offset = CharOffset::from(cursor_offset.as_usize().saturating_sub(1));
        let hidden = char_cell.hidden_line_ranges(ctx);
        let lattice = char_cell.display_lattice(&hidden);
        let Some(cursor) = lattice.offset_to_display_point(cursor_offset) else {
            return false;
        };
        match direction {
            TuiEditorVerticalDirection::Up => cursor.row > 0,
            TuiEditorVerticalDirection::Down => cursor.row + 1 < lattice.rows().len(),
        }
    }
}

impl Entity for TuiEditorView {
    type Event = TuiEditorViewEvent;
}

impl TuiView for TuiEditorView {
    fn ui_name() -> &'static str {
        "TuiEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let editor = self.render_editor(app);
        TuiHoverable::new(self.mouse_state.clone(), editor)
            .on_click(|event_ctx, _| {
                event_ctx.dispatch_typed_action(TuiEditorViewAction::FocusRequested);
            })
            .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }
}

impl TypedActionView for TuiEditorView {
    type Action = TuiEditorViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let outcome = match action {
            TuiEditorViewAction::FocusRequested => {
                ctx.focus_self();
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiEditorViewAction::Editor(action) => self.handle_editor_action(action, ctx),
            TuiEditorViewAction::Command(command) => self.handle_command(*command, ctx),
        };
        let outcome = match outcome {
            TuiEditorInteractionOutcome::Clipboard(action) => {
                if let Err(error) = apply_editor_clipboard_action(&self.model, action, ctx) {
                    log::error!("Failed to copy TUI editor selection: {error}");
                }
                TuiEditorInteractionOutcome::FollowCursor
            }
            outcome => outcome,
        };
        if outcome == TuiEditorInteractionOutcome::FollowCursor {
            follow_editor_cursor(&self.model, self.editor_behavior, ctx);
        }
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "editor_view_tests.rs"]
mod tests;
