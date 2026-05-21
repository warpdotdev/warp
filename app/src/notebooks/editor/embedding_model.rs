use std::{borrow::Cow, mem};

use string_offset::CharOffset;
use warp_editor::{
    content::{anchor::Anchor, selection_model::BufferSelectionModel},
    editor::EmbeddedItemModel,
};
use warpui::{
    elements::{Border, Container, MouseStateHandle},
    platform::Cursor,
    ui_components::{button::ButtonVariant, components::UiComponent},
    AppContext, Element, Entity, ModelAsRef, ModelContext, ModelHandle, SingletonEntity,
};

use crate::appearance::Appearance;

use super::{model::ChildModelHandle, view::EditorViewAction, NotebookWorkflow};

#[derive(Default)]
struct MouseStateHandles {
    remove_embedding_button_state: MouseStateHandle,
}

pub struct NotebookEmbed {
    start: Anchor,
    hashed_id: String,
    is_selected: bool,
    selection_model: ModelHandle<BufferSelectionModel>,
    mouse_state_handles: MouseStateHandles,
}

impl NotebookEmbed {
    pub fn new(
        start: CharOffset,
        hashed_id: String,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let start = selection_model.update(ctx, |selection_model, ctx| {
            selection_model.anchor(start, ctx)
        });

        Self {
            start,
            hashed_id,
            selection_model,
            is_selected: false,
            mouse_state_handles: Default::default(),
        }
    }

    pub fn hashed_id(&self) -> &str {
        self.hashed_id.as_str()
    }

    pub fn start_offset(&self, ctx: &impl ModelAsRef) -> Option<CharOffset> {
        self.selection_model.as_ref(ctx).resolve_anchor(&self.start)
    }

    fn selectable(&self) -> bool {
        false
    }
}

impl Entity for NotebookEmbed {
    type Event = ();
}

impl EmbeddedItemModel for NotebookEmbed {
    fn render_item_footer(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn border(&self, app: &AppContext) -> Option<Border> {
        if self.is_selected {
            let border_fill = Appearance::as_ref(app).theme().accent();
            Some(Border::all(3.).with_border_fill(border_fill))
        } else {
            None
        }
    }

    fn render_remove_embedding_button(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let offset = self.start_offset(ctx)?;
        Some(
            Container::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Text,
                        self.mouse_state_handles
                            .remove_embedding_button_state
                            .clone(),
                    )
                    .with_text_label("Remove".to_string())
                    .build()
                    .with_cursor(Cursor::Arrow)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(EditorViewAction::RemoveEmbeddingAt(offset));
                    })
                    .finish(),
            )
            .with_margin_right(12.)
            .finish(),
        )
    }
}

impl ChildModelHandle for ModelHandle<NotebookEmbed> {
    fn start_offset(&self, app: &AppContext) -> Option<CharOffset> {
        self.as_ref(app).start_offset(app)
    }

    fn end_offset(&self, app: &AppContext) -> Option<CharOffset> {
        // Embedding should always take one character offset.
        self.as_ref(app).start_offset(app).map(|offset| offset + 1)
    }

    fn selectable(&self, app: &AppContext) -> bool {
        self.as_ref(app).selectable()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executable_workflow(&self, _: &AppContext) -> Option<NotebookWorkflow> {
        None
    }

    fn executable_command<'a>(&'a self, _: &'a AppContext) -> Option<Cow<'a, str>> {
        None
    }

    fn selected(&self, app: &AppContext) -> bool {
        self.as_ref(app).is_selected
    }

    fn set_selected(&self, selected: bool, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, _ctx| {
            mem::replace(&mut model.is_selected, selected)
        })
    }

    fn clone_boxed(&self) -> Box<dyn ChildModelHandle> {
        Box::new(self.clone())
    }
}
