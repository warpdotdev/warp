use warp_editor::{editor::NavigationKey, model::RichTextEditorModel, render::model::RenderState};
use warpui::{
    elements::{
        AnchorPair, Container, Flex, MouseStateHandle, OffsetPositioning, OffsetType,
        ParentElement, PositionedElementOffsetBounds, PositioningAxis, XAxisAnchor, YAxisAnchor,
    },
    fonts::Weight,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, BlurContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    editor::{
        EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextOptions,
    },
};

use super::model::NotebooksEditorModel;

const EDITOR_WIDTH: f32 = 368.;
const EDITOR_VERTICAL_PADDING: f32 = 12.;
const EDITOR_MARGIN: f32 = 16.;
const BETWEEN_EDITOR_MARGIN: f32 = 8.;

pub enum LinkEditorEvent {
    Close,
}

#[derive(Debug, Clone)]
pub enum LinkEditorAction {
    ApplyLink,
}

pub struct LinkEditor {
    model: ModelHandle<NotebooksEditorModel>,
    tag_editor: ViewHandle<EditorView>,
    url_editor: ViewHandle<EditorView>,
    apply_link_mouse_state: MouseStateHandle,
}

impl LinkEditor {
    pub fn new(model: ModelHandle<NotebooksEditorModel>, ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let editor_options = SingleLineEditorOptions {
            text: TextOptions::ui_text(None, appearance),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
            ..Default::default()
        };

        let tag_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(editor_options.clone(), ctx);
            editor.set_placeholder_text(crate::i18n::tr_static(ctx, "Text"), ctx);
            editor
        });

        ctx.subscribe_to_view(&tag_editor, |notebook, _, event, ctx| {
            notebook.handle_tag_editor_event(event, ctx);
        });

        let url_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(editor_options.clone(), ctx);
            editor.set_placeholder_text(crate::i18n::tr_static(ctx, "Link (web or file)"), ctx);
            editor
        });

        ctx.subscribe_to_view(&url_editor, |notebook, _, event, ctx| {
            notebook.handle_url_editor_event(event, ctx);
        });

        LinkEditor {
            model,
            tag_editor,
            url_editor,
            apply_link_mouse_state: Default::default(),
        }
    }

    pub fn editors_focused(&self, app: &AppContext) -> bool {
        self.tag_editor.is_focused(app) || self.url_editor.is_focused(app)
    }

    /// Focus the URL editor.
    pub fn focus_url_editor(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.url_editor);
    }

    #[cfg(test)]
    pub(super) fn url_editor(&self) -> &ViewHandle<EditorView> {
        &self.url_editor
    }

    #[cfg(test)]
    pub(super) fn tag_editor(&self) -> &ViewHandle<EditorView> {
        &self.tag_editor
    }

    /// Populate the link editor with the state of the active selection.
    pub fn populate(&mut self, ctx: &mut ViewContext<Self>) {
        let buffer_model = self.model.as_ref(ctx);
        let selected_content = buffer_model.selected_text(ctx);
        let url_at_selection = buffer_model.link_at_selection_head(ctx);
        self.tag_editor.update(ctx, |view, ctx| {
            view.clear_buffer_and_reset_undo_stack(ctx);
            view.set_buffer_text(&selected_content, ctx);
        });

        self.url_editor.update(ctx, |view, ctx| {
            view.clear_buffer_and_reset_undo_stack(ctx);

            if let Some(url) = &url_at_selection {
                view.set_buffer_text(url, ctx);
            }
        });
    }

    fn handle_tag_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Enter
            | EditorEvent::Navigate(NavigationKey::Tab | NavigationKey::ShiftTab) => {
                ctx.focus(&self.url_editor)
            }
            EditorEvent::Escape => ctx.emit(LinkEditorEvent::Close),
            _ => (),
        }
    }

    fn handle_url_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Enter => self.apply_link(ctx),
            EditorEvent::Navigate(NavigationKey::Tab | NavigationKey::ShiftTab) => {
                ctx.focus(&self.tag_editor)
            }
            EditorEvent::Escape => ctx.emit(LinkEditorEvent::Close),
            _ => (),
        }
    }

    /// Whether or not the link editor is in a valid state that can be applied.
    fn is_valid(&self, ctx: &AppContext) -> bool {
        !self.tag_editor.as_ref(ctx).is_empty(ctx) && !self.url_editor.as_ref(ctx).is_empty(ctx)
    }

    /// Apply the current link tag and url to the selected text and close the link editor.
    fn apply_link(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_valid(ctx) {
            return;
        }

        let tag = self.tag_editor.as_ref(ctx).buffer_text(ctx);
        let url = self.url_editor.as_ref(ctx).buffer_text(ctx);

        self.model.update(ctx, |model, ctx| {
            model.set_link(tag, url, ctx);
        });

        ctx.emit(LinkEditorEvent::Close);
    }

    pub fn positioning(render_state: &RenderState) -> OffsetPositioning {
        let selection_position = render_state.saved_positions().text_selection_id();

        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
            )
            .with_conditional_anchor(),
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(4.),
                AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
            )
            .with_conditional_anchor(),
        )
    }
}

impl Entity for LinkEditor {
    type Event = LinkEditorEvent;
}

impl View for LinkEditor {
    fn ui_name() -> &'static str {
        "LinkEditor"
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            ctx.emit(LinkEditorEvent::Close);
        }
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut editors = Flex::column();

        editors.add_child(
            appearance
                .ui_builder()
                .text_input(self.tag_editor.clone())
                .with_style(UiComponentStyles {
                    width: Some(EDITOR_WIDTH),
                    padding: Some(Coords {
                        top: EDITOR_VERTICAL_PADDING,
                        bottom: EDITOR_VERTICAL_PADDING,
                        left: EDITOR_MARGIN,
                        right: EDITOR_MARGIN,
                    }),
                    margin: Some(Coords {
                        top: EDITOR_MARGIN,
                        bottom: BETWEEN_EDITOR_MARGIN,
                        left: EDITOR_MARGIN,
                        right: EDITOR_MARGIN,
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        editors.add_child(
            appearance
                .ui_builder()
                .text_input(self.url_editor.clone())
                .with_style(UiComponentStyles {
                    width: Some(EDITOR_WIDTH),
                    padding: Some(Coords {
                        top: EDITOR_VERTICAL_PADDING,
                        bottom: EDITOR_VERTICAL_PADDING,
                        left: EDITOR_MARGIN,
                        right: EDITOR_MARGIN,
                    }),
                    margin: Some(Coords {
                        left: EDITOR_MARGIN,
                        right: EDITOR_MARGIN,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        let mut link_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.apply_link_mouse_state.clone())
            .with_centered_text_label(crate::i18n::tr_static(app, "Apply link").to_string());

        // Disable the link button if either of the editors are empty.
        if !self.is_valid(app) {
            link_button = link_button.disabled();
        };

        editors.add_child(
            link_button
                .with_style(UiComponentStyles {
                    width: Some(EDITOR_WIDTH),
                    margin: Some(Coords::uniform(EDITOR_MARGIN)),
                    font_weight: Some(Weight::Bold),
                    padding: Some(Coords {
                        left: EDITOR_VERTICAL_PADDING,
                        right: EDITOR_VERTICAL_PADDING,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(LinkEditorAction::ApplyLink))
                .finish(),
        );

        Container::new(editors.finish())
            .with_background(appearance.theme().surface_2())
            .finish()
    }
}

impl TypedActionView for LinkEditor {
    type Action = LinkEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if matches!(action, LinkEditorAction::ApplyLink) {
            self.apply_link(ctx);
        }
    }
}
