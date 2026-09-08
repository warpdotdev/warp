use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex,
    MainAxisAlignment, ParentElement as _, Radius, Shrinkable,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{BorderStyle, Coords, UiComponent as _, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};

const PROMPT_INPUT_HEIGHT: f32 = 56.;
const ICON_MARGIN_LEFT: f32 = 12.;
const ICON_MARGIN_RIGHT: f32 = 6.;

pub struct GlowingEditor {
    editor: ViewHandle<EditorView>,
    /// A closure that returns if the current editor content are valid and can be submitted.
    validator: Box<dyn Fn(&str) -> bool>,
    /// Whether or not the last submission attempt failed validation.
    has_error: bool,
}

impl GlowingEditor {
    pub fn new(placeholder: impl Into<String>, ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size() + 2.;

        let editor = ctx.add_typed_action_view(|ctx| {
            let options = EditorOptions {
                soft_wrap: true,
                text: TextOptions {
                    font_size_override: Some(font_size),
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text(placeholder, ctx);
            editor
        });

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        Self {
            editor,
            validator: Box::new(|_| true),
            has_error: false,
        }
    }

    /// Validates the input contents using the provided `validator` whenever a submit action is
    /// attempted.
    #[expect(dead_code, reason = "Nothing needs validation currently.")]
    pub fn with_validator<F: Fn(&str) -> bool + 'static>(mut self, validator: F) -> Self {
        self.validator = Box::new(validator);
        self
    }

    pub fn clear_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                let prompt = self
                    .editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
                if prompt.is_empty() {
                    return;
                }

                if !(self.validator)(&prompt) {
                    self.has_error = true;
                    ctx.notify();
                } else {
                    self.has_error = false;
                    self.clear_buffer(ctx);
                    ctx.emit(GlowingEditorEvent::Submit(prompt));
                }
            }
            EditorEvent::Escape => ctx.emit(GlowingEditorEvent::Cancel),
            // Clear error state when user types (since this is submit-only validation)
            EditorEvent::Edited(_) => {
                if self.has_error {
                    self.has_error = false;
                    ctx.notify();
                }
            }
            _ => (),
        }
    }
}

pub enum GlowingEditorEvent {
    Submit(String),
    Cancel,
}

impl Entity for GlowingEditor {
    type Event = GlowingEditorEvent;
}

impl TypedActionView for GlowingEditor {
    type Action = ();
}

impl View for GlowingEditor {
    fn ui_name() -> &'static str {
        "GlowingEditor"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let input_box = Shrinkable::new(
            1.,
            Align::new(
                appearance
                    .ui_builder()
                    .text_input(self.editor.clone())
                    .with_style(UiComponentStyles {
                        font_weight: Some(Weight::Semibold),
                        border_style: Some(BorderStyle::None),
                        border_width: Some(0.),
                        background: Some(ColorU::transparent_black().into()),
                        padding: Some(Coords::uniform(10.).left(0.)),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        )
        .finish();

        let font_size = appearance.ui_font_size() + 2.;
        let agent_icon = Container::new(
            ConstrainedBox::new(
                Icon::AgentMode
                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                    .finish(),
            )
            .with_height(font_size)
            .with_width(font_size)
            .finish(),
        )
        .with_margin_left(ICON_MARGIN_LEFT)
        .with_margin_right(ICON_MARGIN_RIGHT)
        .finish();

        let editor_content = ConstrainedBox::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .with_children([agent_icon, input_box])
                .finish(),
        )
        .with_min_height(PROMPT_INPUT_HEIGHT);

        let border_fill = if self.has_error {
            theme.ui_error_color()
        } else {
            theme.outline().into_solid()
        };

        let shadow_color = if self.has_error {
            ColorU::new(255, 0, 0, 100) // Red shadow with higher opacity for error
        } else {
            ColorU::new(255, 143, 253, 15) // Default purple shadow
        };

        Container::new(editor_content.finish())
            .with_border(border_fill)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_drop_shadow(
                DropShadow::new_with_standard_offset_and_spread(shadow_color)
                    .with_offset(Vector2F::zero()),
            )
            .with_background(theme.background().into_solid())
            .finish()
    }
}
