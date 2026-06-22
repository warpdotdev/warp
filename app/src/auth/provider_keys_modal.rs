use pathfinder_color::ColorU;
use ui_components::{button, Component as _, Options as _};
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Fill,
    Flex, FormattedTextElement, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    Radius, Shrinkable, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};

const MODAL_WIDTH: f32 = 460.;
const INPUT_BORDER_RADIUS: Radius = Radius::Pixels(4.);

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ProviderKeysModalAction::Cancel,
        id!(ProviderKeysModalView::ui_name()),
    )]);
}

#[derive(Clone, Copy, Debug)]
pub enum ProviderKeysModalAction {
    Save,
    Cancel,
}

#[derive(Clone, Debug)]
pub enum ProviderKeysModalEvent {
    Cancelled,
    Save {
        openai: Option<String>,
        anthropic: Option<String>,
        google: Option<String>,
    },
}

pub struct ProviderKeysModalView {
    openai_input: ViewHandle<EditorView>,
    anthropic_input: ViewHandle<EditorView>,
    google_input: ViewHandle<EditorView>,
    cancel_button: button::Button,
    add_button: button::Button,
    close_mouse_state: MouseStateHandle,
}

impl ProviderKeysModalView {
    pub fn new(
        openai: Option<String>,
        anthropic: Option<String>,
        google: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let openai_input = Self::make_key_editor("sk-...", openai, ctx);
        let anthropic_input = Self::make_key_editor("sk-ant-...", anthropic, ctx);
        let google_input = Self::make_key_editor("AIzaSy...", google, ctx);

        Self {
            openai_input,
            anthropic_input,
            google_input,
            cancel_button: button::Button::default(),
            add_button: button::Button::default(),
            close_mouse_state: MouseStateHandle::default(),
        }
    }

    fn make_key_editor(
        placeholder: &str,
        initial: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView> {
        let placeholder = placeholder.to_string();
        ctx.add_typed_action_view(move |ctx| {
            let appearance = Appearance::as_ref(ctx);
            let text_colors = crate::settings_view::editor_text_colors(appearance);
            let options = SingleLineEditorOptions {
                is_password: true,
                text: TextOptions {
                    font_family_override: Some(appearance.ui_font_family()),
                    text_colors_override: Some(text_colors),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(&placeholder, ctx);
            if let Some(initial) = initial {
                editor.set_buffer_text(&initial, ctx);
            }
            editor
        })
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let read = |handle: &ViewHandle<EditorView>, ctx: &ViewContext<Self>| {
            let text = handle.as_ref(ctx).buffer_text(ctx);
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let openai = read(&self.openai_input, ctx);
        let anthropic = read(&self.anthropic_input, ctx);
        let google = read(&self.google_input, ctx);
        ctx.emit(ProviderKeysModalEvent::Save {
            openai,
            anthropic,
            google,
        });
    }

    fn render_field(
        &self,
        appearance: &Appearance,
        label: &'static str,
        editor: ViewHandle<EditorView>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let dialog_surface_solid = theme.surface_1().into_solid();
        let input_bg = theme.surface_2();
        let input_bg_solid = input_bg.into_solid();
        let input_text_color: ColorU = internal_colors::text_main(theme, input_bg_solid);
        let border_color = internal_colors::neutral_4(theme);

        let label_el = FormattedTextElement::from_str(label, appearance.ui_font_family(), 12.)
            .with_color(internal_colors::text_main(theme, dialog_surface_solid))
            .with_weight(Weight::Normal)
            .with_alignment(TextAlignment::Left)
            .with_line_height_ratio(1.0)
            .finish();

        let input = appearance
            .ui_builder()
            .text_input(editor)
            .with_style(UiComponentStyles {
                background: Some(input_bg.into()),
                border_width: Some(1.),
                border_color: Some(Fill::Solid(border_color)),
                border_radius: Some(CornerRadius::with_all(INPUT_BORDER_RADIUS)),
                font_color: Some(input_text_color),
                padding: Some(Coords {
                    top: 10.,
                    bottom: 10.,
                    left: 16.,
                    right: 16.,
                }),
                ..Default::default()
            })
            .build()
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(label_el)
            .with_child(Container::new(input).with_margin_top(8.).finish())
            .finish()
    }
}

impl Entity for ProviderKeysModalView {
    type Event = ProviderKeysModalEvent;
}

impl View for ProviderKeysModalView {
    fn ui_name() -> &'static str {
        "ProviderKeysModalView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.openai_input);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let dialog_surface = theme.surface_1();
        let dialog_surface_solid = dialog_surface.into_solid();
        let border_color = internal_colors::neutral_4(theme);
        let ui_builder = appearance.ui_builder();

        let title = FormattedTextElement::from_str("Add API key", appearance.ui_font_family(), 16.)
            .with_color(internal_colors::text_main(theme, dialog_surface_solid))
            .with_weight(Weight::Bold)
            .with_line_height_ratio(1.25)
            .finish();

        let close_button = ui_builder
            .close_button(24., self.close_mouse_state.clone())
            .build()
            .on_click(|ctx: &mut warpui::EventContext, _, _| {
                ctx.dispatch_typed_action(ProviderKeysModalAction::Cancel);
            })
            .finish();

        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(close_button)
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Use your own API keys from model providers for Warp Agent.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(internal_colors::text_sub(theme, dialog_surface_solid))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(subtitle).with_margin_bottom(16.).finish())
            .with_child(self.render_field(appearance, "OpenAI API key", self.openai_input.clone()))
            .with_child(
                Container::new(self.render_field(
                    appearance,
                    "Anthropic API key",
                    self.anthropic_input.clone(),
                ))
                .with_margin_top(16.)
                .finish(),
            )
            .with_child(
                Container::new(self.render_field(
                    appearance,
                    "Google API key",
                    self.google_input.clone(),
                ))
                .with_margin_top(16.)
                .finish(),
            )
            .finish();

        let cancel_button = self.cancel_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Cancel".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ProviderKeysModalAction::Cancel);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let add_button = self.add_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Add keys".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ProviderKeysModalAction::Save);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let footer = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(cancel_button)
                .with_child(Container::new(add_button).with_margin_left(8.).finish())
                .finish(),
        )
        .with_border(Border::top(1.).with_border_color(border_color))
        .with_horizontal_padding(24.)
        .with_vertical_padding(12.)
        .finish();

        let dialog = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(title_row)
                    .with_horizontal_padding(24.)
                    .with_padding_top(24.)
                    .with_padding_bottom(12.)
                    .finish(),
            )
            .with_child(
                Container::new(body)
                    .with_horizontal_padding(24.)
                    .with_padding_bottom(16.)
                    .finish(),
            )
            .with_child(footer)
            .finish();

        let modal = ConstrainedBox::new(
            Container::new(dialog)
                .with_background(dialog_surface)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        let mut stack = Stack::new();
        stack.add_child(
            Container::new(warpui::elements::Empty::new().finish())
                .with_background_color(ColorU::new(0, 0, 0, 179))
                .finish(),
        );
        stack.add_child(
            Dismiss::new(Align::new(modal).finish())
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(ProviderKeysModalAction::Cancel);
                })
                .finish(),
        );
        stack.finish()
    }
}

impl TypedActionView for ProviderKeysModalView {
    type Action = ProviderKeysModalAction;

    fn handle_action(&mut self, action: &ProviderKeysModalAction, ctx: &mut ViewContext<Self>) {
        match action {
            ProviderKeysModalAction::Save => {
                self.submit(ctx);
            }
            ProviderKeysModalAction::Cancel => {
                ctx.emit(ProviderKeysModalEvent::Cancelled);
            }
        }
    }
}
