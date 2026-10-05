use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use serde_json::Value;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::send_telemetry_from_ctx;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Expanded, Flex, MainAxisSize, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::llms::CHATGPT_USAGE_URL;
use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};

const MODAL_WIDTH: f32 = 420.;
const LOGO_SIZE: f32 = 32.;

fn modal_background(appearance: &Appearance) -> Fill {
    appearance.theme().surface_3()
}

fn modal_text_main(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .main_text_color(modal_background(appearance))
        .into_solid()
}

fn modal_text_sub(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .sub_text_color(modal_background(appearance))
        .into_solid()
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ChatGPTPlanModalAction::Close,
        id!(ChatGPTPlanModal::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum ChatGPTPlanModalAction {
    Close,
    ManageUsage,
}

#[derive(Clone, Copy, Debug)]
pub enum ChatGPTPlanModalEvent {
    Close,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        hovered.then(|| appearance.theme().surface_overlay_1())
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        modal_text_sub(appearance)
    }
}

struct ManageUsageButtonTheme;

impl ActionButtonTheme for ManageUsageButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        hovered.then(|| appearance.theme().surface_overlay_2())
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        modal_text_main(appearance)
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(appearance.theme().outline().into_solid())
    }
}

struct CtaButtonTheme;

impl ActionButtonTheme for CtaButtonTheme {
    fn background(&self, _hovered: bool, appearance: &Appearance) -> Option<Fill> {
        Some(Fill::Solid(appearance.theme().foreground().into_solid()))
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().background().into_solid()
    }
}

/// One-time notice, required by OpenAI's Sign in with ChatGPT guidelines, telling the user that
/// eligible requests now bill to their ChatGPT plan and where to manage that usage.
pub struct ChatGPTPlanModal {
    close_button: ViewHandle<ActionButton>,
    manage_usage_button: ViewHandle<ActionButton>,
    got_it_button: ViewHandle<ActionButton>,
}

impl ChatGPTPlanModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(ChatGPTPlanModalAction::Close))
        });

        let manage_usage_button = ctx.add_view(|_ctx| {
            ActionButton::new("Manage usage", ManageUsageButtonTheme)
                .with_icon(Icon::LinkExternal)
                .with_full_width(true)
                .on_click(|ctx| ctx.dispatch_typed_action(ChatGPTPlanModalAction::ManageUsage))
        });

        let got_it_button = ctx.add_view(|_ctx| {
            ActionButton::new("Got it", CtaButtonTheme)
                .with_full_width(true)
                .on_click(|ctx| ctx.dispatch_typed_action(ChatGPTPlanModalAction::Close))
        });

        Self {
            close_button,
            manage_usage_button,
            got_it_button,
        }
    }

    fn render_logo(appearance: &Appearance) -> Box<dyn Element> {
        ConstrainedBox::new(
            Icon::OpenAILogo
                .to_warpui_icon(Fill::Solid(modal_text_main(appearance)))
                .finish(),
        )
        .with_width(LOGO_SIZE)
        .with_height(LOGO_SIZE)
        .finish()
    }

    fn render_title(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "You're using your ChatGPT plan",
            appearance.ui_font_family(),
            20.,
        )
        .with_color(modal_text_main(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish()
    }

    fn render_description(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "Eligible requests in Warp will use your ChatGPT plan. You can manage usage for Warp \
             in your ChatGPT settings.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(modal_text_sub(appearance))
        .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                Expanded::new(1., ChildView::new(&self.manage_usage_button).finish()).finish(),
            )
            .with_child(Expanded::new(1., ChildView::new(&self.got_it_button).finish()).finish())
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(Self::render_logo(appearance))
                .with_child(
                    Container::new(
                        Flex::column()
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_spacing(8.)
                            .with_child(Self::render_title(appearance))
                            .with_child(Self::render_description(appearance))
                            .finish(),
                    )
                    .with_margin_top(16.)
                    .finish(),
                )
                .with_child(Container::new(footer).with_margin_top(32.).finish())
                .finish(),
        )
        .with_horizontal_padding(32.)
        .with_vertical_padding(32.)
        .finish()
    }
}

impl Entity for ChatGPTPlanModal {
    type Event = ChatGPTPlanModalEvent;
}

impl View for ChatGPTPlanModal {
    fn ui_name() -> &'static str {
        "ChatGPTPlanModal"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let close_el = Container::new(ChildView::new(&self.close_button).finish())
            .with_uniform_padding(4.)
            .with_padding_right(2.)
            .finish();

        let mut card_stack = Stack::new();
        card_stack.add_child(self.render_body(appearance));
        card_stack.add_positioned_child(
            close_el,
            OffsetPositioning::offset_from_parent(
                vec2f(-4., 4.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        let card = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(card_stack.finish())
                    .finish(),
            )
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for ChatGPTPlanModal {
    type Action = ChatGPTPlanModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ChatGPTPlanModalAction::Close => {
                send_telemetry_from_ctx!(ChatGPTPlanModalTelemetryEvent::Dismissed, ctx);
                ctx.emit(ChatGPTPlanModalEvent::Close);
            }
            ChatGPTPlanModalAction::ManageUsage => {
                send_telemetry_from_ctx!(ChatGPTPlanModalTelemetryEvent::ManageUsageClicked, ctx);
                ctx.open_url(CHATGPT_USAGE_URL);
            }
        }
    }
}

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum ChatGPTPlanModalTelemetryEvent {
    Shown,
    Dismissed,
    ManageUsageClicked,
}

impl TelemetryEvent for ChatGPTPlanModalTelemetryEvent {
    fn name(&self) -> &'static str {
        ChatGPTPlanModalTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::Shown | Self::Dismissed | Self::ManageUsageClicked => None,
        }
    }

    fn description(&self) -> &'static str {
        ChatGPTPlanModalTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        ChatGPTPlanModalTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::Shown | Self::Dismissed | Self::ManageUsageClicked => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for ChatGPTPlanModalTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Shown => "ChatGPTPlanModal.Shown",
            Self::Dismissed => "ChatGPTPlanModal.Dismissed",
            Self::ManageUsageClicked => "ChatGPTPlanModal.ManageUsageClicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Shown => "The \"You're using your ChatGPT plan\" modal was shown to the user",
            Self::Dismissed => "The user dismissed the \"You're using your ChatGPT plan\" modal",
            Self::ManageUsageClicked => {
                "The user opened ChatGPT usage settings from the \"You're using your ChatGPT plan\" modal"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::Shown | Self::Dismissed | Self::ManageUsageClicked => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(ChatGPTPlanModalTelemetryEvent);
