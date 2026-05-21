use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme};
use asset_macro::bundled_or_fetched_asset;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, CacheOption, ChildAnchor, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DropShadow, Flex, FormattedTextElement, Image, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

/// White button theme for the Codex modal CTA.
struct WhiteButtonTheme;

impl ActionButtonTheme for WhiteButtonTheme {
    fn background(&self, hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(Fill::Solid(ColorU::new(230, 230, 230, 255)))
        } else {
            Some(Fill::Solid(ColorU::new(255, 255, 255, 255)))
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        ColorU::new(0, 0, 0, 255)
    }
}

const BUTTON_DIAMETER: f32 = 20.;
const MODAL_HEIGHT: f32 = 395.;
const LEFT_PANEL_WIDTH: f32 = 330.;
const RIGHT_PANEL_WIDTH: f32 = 325.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        CodexModalAction::Close,
        id!("CodexModal"),
    )]);
}

#[derive(Default)]
struct StateHandles {
    close_button: MouseStateHandle,
}

pub struct CodexModal {
    state_handles: StateHandles,
    cta_button: ViewHandle<ActionButton>,
}

impl CodexModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cta_button = ctx.add_view(|ctx| {
            ActionButton::new(
                crate::i18n::tr_static(ctx, "Use latest codex model"),
                WhiteButtonTheme,
            )
            .with_icon(Icon::OpenAILogo)
            .with_full_width(true)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(CodexModalAction::UseCodex);
            })
        });

        CodexModal {
            state_handles: Default::default(),
            cta_button,
        }
    }

    fn render_new_badge(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        // Magenta/pink color for the badge
        let magenta: ColorU = theme.terminal_colors().normal.magenta.into();
        Container::new(
            Text::new("New", appearance.ui_font_family(), 12.)
                .with_color(magenta)
                .finish(),
        )
        .with_vertical_padding(4.)
        .with_horizontal_padding(10.)
        .with_background(Fill::Solid(magenta).with_opacity(15))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
        .finish()
    }

    fn render_left_panel(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // "New" badge
        let new_badge = self.render_new_badge(appearance);

        // Title
        let title = FormattedTextElement::from_str(
            "Use Codex models in Warp",
            appearance.ui_font_family(),
            24.,
        )
        .with_color(blended_colors::text_main(
            theme,
            blended_colors::neutral_1(theme),
        ))
        .with_weight(Weight::Bold)
        .finish();

        // Description - first paragraph
        let description_1 = FormattedTextElement::from_str(
            "Codex is OpenAI's most advanced agentic coding model for real-world engineering.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(blended_colors::text_sub(
            theme,
            blended_colors::neutral_1(theme),
        ))
        .finish();

        // Description - second paragraph
        let description_2 = FormattedTextElement::from_str(
            "Use Codex directly in Oz and leverage \
            features like in-app code review, agent session sharing and file editing.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(blended_colors::text_sub(
            theme,
            blended_colors::neutral_1(theme),
        ))
        .finish();

        // Left panel content
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(Container::new(new_badge).with_margin_bottom(16.).finish())
                        .with_child(Container::new(title).with_margin_bottom(16.).finish())
                        .with_child(
                            Container::new(description_1)
                                .with_margin_bottom(12.)
                                .finish(),
                        )
                        .with_child(description_2)
                        .finish(),
                )
                .with_child(ChildView::new(&self.cta_button).finish())
                .finish(),
        )
        .with_background_color(blended_colors::neutral_1(theme))
        .with_corner_radius(CornerRadius::with_left(Radius::Pixels(10.)))
        .with_uniform_padding(24.)
        .finish()
    }

    fn render_right_panel(&self) -> Box<dyn Element> {
        ConstrainedBox::new(
            Image::new(
                bundled_or_fetched_asset!("png/codex_integration.png"),
                CacheOption::BySize,
            )
            .with_corner_radius(CornerRadius::with_right(Radius::Pixels(10.)))
            .finish(),
        )
        .with_width(RIGHT_PANEL_WIDTH)
        .with_height(MODAL_HEIGHT)
        .finish()
    }
}

impl Entity for CodexModal {
    type Event = CodexModalEvent;
}

impl View for CodexModal {
    fn ui_name() -> &'static str {
        "CodexModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Close button
        let close_button = appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.state_handles.close_button.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CodexModalAction::Close))
            .finish();

        // Modal with two panels
        let mut modal = Stack::new();
        modal.add_child(
            Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            ConstrainedBox::new(self.render_left_panel(app))
                                .with_width(LEFT_PANEL_WIDTH)
                                .with_height(MODAL_HEIGHT)
                                .finish(),
                        )
                        .with_child(self.render_right_panel())
                        .finish(),
                )
                .with_width(LEFT_PANEL_WIDTH + RIGHT_PANEL_WIDTH)
                .with_height(MODAL_HEIGHT)
                .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        );
        modal.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                vec2f(-8., 8.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        // Center the modal in the window
        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal.finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        // Background overlay
        Container::new(Align::new(stack.finish()).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for CodexModal {
    type Action = CodexModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodexModalAction::Close => {
                ctx.emit(CodexModalEvent::Close);
            }
            CodexModalAction::UseCodex => {
                ctx.emit(CodexModalEvent::UseCodex);
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum CodexModalEvent {
    Close,
    UseCodex,
}

#[derive(Copy, Clone, Debug)]
pub enum CodexModalAction {
    Close,
    UseCodex,
}
