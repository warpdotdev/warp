#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]

use crate::appearance::Appearance;
use crate::code::editor::find::view::{FIND_BAR_PADDING, FIND_EDITOR_BORDER_RADIUS};
use crate::editor::{
    EditorView, Event as EditorEvent, InteractionState, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::i18n::{self, I18nKey};
use warpui::{
    elements::{
        Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, DropShadow, Flex,
        ParentElement, Radius, Text,
    },
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const GOTO_LINE_WIDTH: f32 = 300.;
const GOTO_LINE_LABEL_FONT_SIZE: f32 = 12.;
const GOTO_LINE_EDITOR_FONT_SIZE: f32 = 12.;
const GOTO_LINE_ERROR_FONT_SIZE: f32 = 11.;
const GOTO_LINE_EDITOR_PADDING: f32 = 6.;
const GOTO_LINE_EDITOR_BORDER_WIDTH: f32 = 1.;
const GOTO_LINE_ROW_SPACING: f32 = 6.;

#[derive(Debug)]
pub enum Event {
    Close,
    Confirm { input: String },
}

pub struct GoToLineView {
    line_editor: ViewHandle<EditorView>,
    is_open: bool,
    error_message: Option<String>,
}

impl GoToLineView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let line_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(GOTO_LINE_EDITOR_FONT_SIZE), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: false,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor
                .set_placeholder_text(i18n::tr(ctx, I18nKey::CodeLineNumberColumnPlaceholder), ctx);
            editor
        });

        ctx.subscribe_to_view(&line_editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let appearance_handle = Appearance::handle(ctx);
        ctx.observe(&appearance_handle, |_, _, ctx| {
            ctx.notify();
        });

        Self {
            line_editor,
            is_open: false,
            error_message: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn open(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_open = true;
        self.error_message = None;
        self.line_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
            editor.clear_buffer(ctx);
        });
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_open = false;
        self.error_message = None;
        ctx.notify();
    }

    pub fn set_error(&mut self, message: String, ctx: &mut ViewContext<Self>) {
        self.error_message = Some(message);
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                let input = self.line_editor.as_ref(ctx).buffer_text(ctx);
                ctx.emit(Event::Confirm { input });
            }
            EditorEvent::Escape => {
                ctx.emit(Event::Close);
            }
            _ => {}
        }
    }
}

impl Entity for GoToLineView {
    type Event = Event;
}

impl TypedActionView for GoToLineView {
    type Action = ();

    fn handle_action(&mut self, _action: &(), _ctx: &mut ViewContext<Self>) {}
}

impl View for GoToLineView {
    fn ui_name() -> &'static str {
        "GoToLineView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.line_editor);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let label = Text::new_inline(
            "Go to line",
            appearance.ui_font_family(),
            GOTO_LINE_LABEL_FONT_SIZE,
        )
        .with_color(theme.active_ui_text_color().into())
        .finish();

        let input_field = Container::new(ChildView::new(&self.line_editor).finish())
            .with_padding_left(8.)
            .with_padding_right(4.)
            .with_padding_top(GOTO_LINE_EDITOR_PADDING)
            .with_padding_bottom(GOTO_LINE_EDITOR_PADDING)
            .with_background(theme.surface_1())
            .with_border(
                Border::all(GOTO_LINE_EDITOR_BORDER_WIDTH).with_border_fill(theme.surface_3()),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                FIND_EDITOR_BORDER_RADIUS,
            )))
            .finish();

        let mut content = Flex::column().with_child(
            Container::new(label)
                .with_margin_bottom(GOTO_LINE_ROW_SPACING)
                .finish(),
        );
        content.add_child(input_field);

        if let Some(error) = &self.error_message {
            let error_text = Text::new_inline(
                error.clone(),
                appearance.ui_font_family(),
                GOTO_LINE_ERROR_FONT_SIZE,
            )
            .with_color(theme.ui_error_color())
            .finish();
            content.add_child(
                Container::new(error_text)
                    .with_margin_top(GOTO_LINE_ROW_SPACING)
                    .finish(),
            );
        }

        let panel = Container::new(
            ConstrainedBox::new(
                Container::new(content.finish())
                    .with_background(theme.surface_2())
                    .finish(),
            )
            .with_width(GOTO_LINE_WIDTH)
            .finish(),
        )
        .with_uniform_padding(FIND_BAR_PADDING)
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            FIND_EDITOR_BORDER_RADIUS,
        )))
        .with_drop_shadow(DropShadow::default())
        .finish();

        Align::new(panel).top_center().finish()
    }
}
