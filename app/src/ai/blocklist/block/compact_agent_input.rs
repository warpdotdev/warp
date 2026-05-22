//! Compact free-form text input used by inline AI block actions.
use warpui::presenter::ChildView;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey,
    PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, TextOptions,
};

/// Wraps an [`EditorView`] for inline prompts that need a lightweight text input.
///
/// Enter submits trimmed non-empty text and clears the buffer. Escape is emitted for the parent
/// view to handle.
pub struct CompactAgentInput {
    editor: ViewHandle<EditorView>,
}

/// Events emitted by [`CompactAgentInput`].
pub enum CompactAgentInputEvent {
    /// The user pressed Enter with non-empty trimmed contents.
    Submit(String),
    /// The user pressed Escape while the input was focused.
    Escape,
}

impl CompactAgentInput {
    /// Creates a compact AI input view backed by an autogrowing, soft-wrapping editor.
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let text_options = TextOptions::ui_text(None, Appearance::as_ref(ctx));
        let editor = ctx.add_view(|ctx| {
            let options = EditorOptions {
                autogrow: true,
                soft_wrap: true,
                text: text_options,
                include_ai_context_menu: true,
                propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::AtBoundary,
                ..Default::default()
            };
            let mut editor = EditorView::new(options, ctx);
            editor.set_is_ai_input(true, ctx);
            editor
        });

        ctx.subscribe_to_view(&editor, Self::handle_editor_event);

        Self { editor }
    }

    /// Sets the placeholder shown while the input buffer is empty.
    pub fn set_placeholder_text(&self, text: impl Into<String>, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(text, ctx);
        });
    }

    /// Returns the underlying editor handle for integrations that need direct editor access.
    pub fn editor(&self) -> &ViewHandle<EditorView> {
        &self.editor
    }

    /// Replaces the current buffer contents.
    pub fn set_text(&self, text: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(text, ctx);
        });
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Enter => {
                let content = self
                    .editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
                if !content.is_empty() {
                    self.editor
                        .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                    ctx.emit(CompactAgentInputEvent::Submit(content));
                }
            }
            EditorEvent::Escape => {
                ctx.emit(CompactAgentInputEvent::Escape);
            }
            _ => {}
        }
    }
}

impl View for CompactAgentInput {
    fn ui_name() -> &'static str {
        "CompactAgentInput"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.editor).finish()
    }
}

impl Entity for CompactAgentInput {
    type Event = CompactAgentInputEvent;
}
