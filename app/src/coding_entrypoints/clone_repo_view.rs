use warp_core::send_telemetry_from_ctx;
use warpui::elements::{ChildView, Flex, ParentElement as _};
use warpui::{
    AppContext, Element, Entity, FocusContext, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::coding_entrypoints::glowing_editor::{GlowingEditor, GlowingEditorEvent};
use crate::TelemetryEvent;

pub struct CloneRepoView {
    editor: ViewHandle<GlowingEditor>,
    is_ftux: bool,
}

pub enum CloneRepoEvent {
    SubmitPrompt(String),
    Cancel,
}

impl CloneRepoView {
    pub fn new(is_ftux: bool, ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
            GlowingEditor::new(
                "Provide a repository URL e.g. \"git@github.com:username/project.git\"",
                ctx,
            )
        });

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        Self { editor, is_ftux }
    }

    fn handle_editor_event(&mut self, event: &GlowingEditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            GlowingEditorEvent::Submit(prompt) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::CloneRepoPromptSubmitted {
                        is_ftux: self.is_ftux
                    },
                    ctx
                );
                ctx.emit(CloneRepoEvent::SubmitPrompt(prompt.clone()))
            }
            GlowingEditorEvent::Cancel => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
                ctx.emit(CloneRepoEvent::Cancel)
            }
        }
    }
}

impl Entity for CloneRepoView {
    type Event = CloneRepoEvent;
}

impl TypedActionView for CloneRepoView {
    type Action = ();
}

impl View for CloneRepoView {
    fn ui_name() -> &'static str {
        "CloneRepoView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Flex::column()
            .with_child(ChildView::new(&self.editor).finish())
            .finish()
    }
}
