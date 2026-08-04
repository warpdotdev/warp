use pathfinder_color::ColorU;
use ui_components::{Component as _, Options as _, button};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Expanded, Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Stack,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::Cursor;
use warpui_core::prelude::Align;
use warpui_core::text_layout::TextAlignment;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use super::upgrade_auth_prompt::render_upgrade_auth_prompt_bar;
use crate::model::{CreditPackOption, CreditPurchaseState, OnboardingStateModel};
use crate::slides::{layout, slide_content};
use crate::telemetry::OnboardingEvent;

/// Upper bound on rendered credit packs. The server offers four today; the cap
/// keeps a fixed pool of mouse states (hover tracking needs stable handles)
/// without capping what the server may add later in any meaningful way.
const MAX_CREDIT_PACKS: usize = 8;

/// Gap between credit pack tiles, matching the Billing & Usage page's add-on
/// credit denominations row.
const CREDIT_PACK_TILE_SPACING: f32 = 8.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferVariant {
    HeadStart,
    ChooseHowToStart,
}

impl OfferVariant {
    pub(crate) fn title(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "You've got a head start",
            OfferVariant::ChooseHowToStart => "Choose how to start",
        }
    }

    pub(crate) fn subtitle(self) -> Option<&'static str> {
        match self {
            OfferVariant::HeadStart => {
                Some("Your account includes AI usage to help you get started.")
            }
            OfferVariant::ChooseHowToStart => None,
        }
    }

    pub(crate) fn primary_label(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Unlock the full AI experience",
            // Two of the three options are ways to use Warp with AI, so this
            // card is named for what actually distinguishes it: the plan.
            OfferVariant::ChooseHowToStart => "Subscribe to a Warp plan",
        }
    }

    /// `shows_credit_packs` is the same condition that decides whether the
    /// buy-credits card renders (see [`OfferSlide::shows_credit_packs`]) — the
    /// add-on savings line only makes sense next to the packs it refers to, so
    /// without them the card keeps its original copy.
    pub(crate) fn primary_description(self, shows_credit_packs: bool) -> &'static str {
        match self {
            OfferVariant::HeadStart => {
                "Get more monthly usage, expanded cloud agent access, and collaboration features."
            }
            OfferVariant::ChooseHowToStart if shows_credit_packs => {
                "Warp Agent works locally or in the cloud with frontier and OSS models. Get monthly credits at the best value, and save 20% on add-on credits with any Build plan."
            }
            OfferVariant::ChooseHowToStart => {
                "Warp Agent works locally or in the cloud with frontier and OSS models. Proactively fix terminal errors, implement changes, and ship verified code."
            }
        }
    }

    pub(crate) fn secondary_label(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "Start with included AI",
            OfferVariant::ChooseHowToStart => "Set up AI later",
        }
    }

    pub(crate) fn secondary_description(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => {
                "Explore with the AI usage included with your account and upgrade to add more anytime."
            }
            OfferVariant::ChooseHowToStart => {
                "Explore the terminal, bring your own inference, or use another CLI agent. Add AI usage and features anytime."
            }
        }
    }

    /// Whether this offer includes the one-time credit-pack option. Only the
    /// free-standard offer does: the head-start offer already ships with
    /// included AI usage, so a pack purchase isn't the decision being made.
    pub(crate) fn supports_credit_packs(self) -> bool {
        matches!(self, OfferVariant::ChooseHowToStart)
    }

    pub(crate) fn credits_label(self) -> &'static str {
        "Buy AI credits"
    }

    pub(crate) fn credits_description(self) -> &'static str {
        "Best for trying Warp without a subscription. Buy a one-time credit pack and start using the Warp Agent right away."
    }

    fn credits_action(self) -> &'static str {
        "buy_ai_credits"
    }

    pub(crate) fn included_features(self) -> &'static [&'static str] {
        match self {
            OfferVariant::HeadStart => &[
                "Limited monthly AI usage for occasional tasks",
                "Access to premium and open-source models",
                "Use the Warp Agent locally and in the cloud",
            ],
            OfferVariant::ChooseHowToStart => &[],
        }
    }

    pub(crate) fn slide_name(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "head_start",
            OfferVariant::ChooseHowToStart => "choose_how_to_start",
        }
    }

    pub(crate) fn account_class(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "free_icp",
            OfferVariant::ChooseHowToStart => "free_standard",
        }
    }

    fn primary_action(self) -> &'static str {
        match self {
            OfferVariant::HeadStart => "get_more_ai",
            // Telemetry identifier, not user-facing copy: kept stable across
            // the card's rename to "Subscribe to a Warp plan" so existing
            // dashboards don't lose continuity.
            OfferVariant::ChooseHowToStart => "use_warp_with_ai",
        }
    }
}

#[derive(Clone, Debug)]
pub enum OfferSlideAction {
    SelectPrimary,
    SelectBuyCredits,
    SelectSetUpLater,
    SelectCreditPack(usize),
    Back,
    GetWarping,
    CopyUpgradeUrl,
    PasteAuthTokenFromClipboard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OfferChoice {
    #[default]
    Primary,
    /// Buy a one-time credit pack instead of subscribing.
    BuyCredits,
    SetUpLater,
}

#[derive(Clone, Debug)]
pub enum OfferSlideEvent {
    SetUpLaterSelected { variant: OfferVariant },
    CopyUpgradeUrlRequested,
    PasteAuthTokenFromClipboardRequested,
}

pub struct OfferSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    primary_mouse_state: MouseStateHandle,
    buy_credits_mouse_state: MouseStateHandle,
    secondary_mouse_state: MouseStateHandle,
    /// One hover handle per rendered credit pack row. Allocated up front so
    /// each row keeps a stable handle across renders.
    credit_pack_mouse_states: [MouseStateHandle; MAX_CREDIT_PACKS],
    back_button: button::Button,
    get_warping_button: button::Button,
    selected_choice: OfferChoice,
    scroll_state: ClippedScrollStateHandle,
    show_auth_prompt_bar: bool,
    copy_url_mouse_state: MouseStateHandle,
    paste_token_mouse_state: MouseStateHandle,
}

impl OfferSlide {
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] =
        &["async/png/onboarding/welcome_agent.png"];

    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            primary_mouse_state: MouseStateHandle::default(),
            buy_credits_mouse_state: MouseStateHandle::default(),
            secondary_mouse_state: MouseStateHandle::default(),
            credit_pack_mouse_states: std::array::from_fn(|_| MouseStateHandle::default()),
            back_button: button::Button::default(),
            get_warping_button: button::Button::default(),
            selected_choice: OfferChoice::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            show_auth_prompt_bar: false,
            copy_url_mouse_state: MouseStateHandle::default(),
            paste_token_mouse_state: MouseStateHandle::default(),
        }
    }

    fn variant(&self, app: &AppContext) -> Option<OfferVariant> {
        self.onboarding_state.as_ref(app).offer_variant()
    }

    /// The credit packs to render, capped at [`MAX_CREDIT_PACKS`]. Empty when
    /// the offer doesn't include the option or pricing hasn't arrived yet, in
    /// which case the buy-credits card is not shown at all.
    fn credit_packs<'a>(
        &self,
        variant: OfferVariant,
        app: &'a AppContext,
    ) -> &'a [CreditPackOption] {
        if !variant.supports_credit_packs() {
            return &[];
        }
        let packs = self.onboarding_state.as_ref(app).credit_pack_options();
        &packs[..packs.len().min(MAX_CREDIT_PACKS)]
    }

    fn shows_credit_packs(&self, variant: OfferVariant, app: &AppContext) -> bool {
        !self.credit_packs(variant, app).is_empty()
    }

    /// The selectable options, top to bottom. Also the order the arrow keys
    /// move through.
    fn choices(&self, variant: OfferVariant, app: &AppContext) -> Vec<OfferChoice> {
        let mut choices = vec![OfferChoice::Primary];
        if self.shows_credit_packs(variant, app) {
            choices.push(OfferChoice::BuyCredits);
        }
        choices.push(OfferChoice::SetUpLater);
        choices
    }

    /// The selected option, falling back to the subscribe option if the
    /// buy-credits option was selected and has since disappeared (e.g. pricing
    /// went away on a refresh).
    fn effective_choice(&self, variant: OfferVariant, app: &AppContext) -> OfferChoice {
        if self.selected_choice == OfferChoice::BuyCredits && !self.shows_credit_packs(variant, app)
        {
            return OfferChoice::Primary;
        }
        self.selected_choice
    }

    fn credit_purchase_state(&self, app: &AppContext) -> CreditPurchaseState {
        self.onboarding_state.as_ref(app).credit_purchase_state()
    }

    /// Whether the pack at `index` renders as selected. Which pack is selected
    /// is a choice made *inside* the buy-credits option, so it is only shown
    /// while that option is the chosen one — otherwise the default pack index
    /// would accent a tile inside a card the user hasn't picked.
    fn credit_pack_is_selected(
        &self,
        variant: OfferVariant,
        index: usize,
        ctx: &AppContext,
    ) -> bool {
        self.effective_choice(variant, ctx) == OfferChoice::BuyCredits
            && index
                == self
                    .onboarding_state
                    .as_ref(ctx)
                    .selected_credit_pack_index()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        variant: OfferVariant,
        app: &AppContext,
    ) -> Box<dyn Element> {
        slide_content::onboarding_slide_content(
            vec![
                Align::new(Self::render_header(appearance, variant))
                    .left()
                    .finish(),
                self.render_options(appearance, variant, app),
            ],
            self.render_bottom_nav(appearance, variant, app),
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(appearance: &Appearance, variant: OfferVariant) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title = appearance
            .ui_builder()
            .paragraph(variant.title())
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();
        let mut header = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title);
        if let Some(subtitle) = variant.subtitle() {
            let subtitle =
                FormattedTextElement::from_str(subtitle, appearance.ui_font_family(), 16.)
                    .with_color(internal_colors::text_sub(
                        theme,
                        theme.background().into_solid(),
                    ))
                    .with_weight(Weight::Normal)
                    .with_alignment(TextAlignment::Left)
                    .with_line_height_ratio(1.0)
                    .finish();
            header = header.with_child(Container::new(subtitle).with_margin_top(8.).finish());
        }
        let features = variant.included_features();
        if !features.is_empty() {
            let green = theme.ansi_fg_green();
            let mut feature_list = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(10.);
            for feature in features {
                let check = ConstrainedBox::new(Box::new(
                    Icon::CheckSkinny.to_warpui_icon(Fill::Solid(green)),
                ))
                .with_width(14.)
                .with_height(14.)
                .finish();
                let text = appearance
                    .ui_builder()
                    .paragraph(*feature)
                    .with_style(UiComponentStyles {
                        font_size: Some(13.),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                feature_list.add_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(check)
                        .with_child(Container::new(text).with_margin_left(6.).finish())
                        .finish(),
                );
            }
            header = header.with_child(
                Container::new(feature_list.finish())
                    .with_margin_top(32.)
                    .finish(),
            );
        }
        header.finish()
    }

    fn render_options(
        &self,
        appearance: &Appearance,
        variant: OfferVariant,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let selected_choice = self.effective_choice(variant, app);
        let shows_credit_packs = self.shows_credit_packs(variant, app);
        let primary = Self::render_option_card(
            appearance,
            variant.primary_label(),
            variant.primary_description(shows_credit_packs),
            selected_choice == OfferChoice::Primary,
            Some("Recommended"),
            self.primary_mouse_state.clone(),
            OfferSlideAction::SelectPrimary,
            None,
        );
        let secondary = Self::render_option_card(
            appearance,
            variant.secondary_label(),
            variant.secondary_description(),
            selected_choice == OfferChoice::SetUpLater,
            None,
            self.secondary_mouse_state.clone(),
            OfferSlideAction::SelectSetUpLater,
            None,
        );

        let mut options = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(primary).with_margin_bottom(12.).finish());

        if shows_credit_packs {
            let buy_credits = Self::render_option_card(
                appearance,
                variant.credits_label(),
                variant.credits_description(),
                selected_choice == OfferChoice::BuyCredits,
                None,
                self.buy_credits_mouse_state.clone(),
                OfferSlideAction::SelectBuyCredits,
                Some(self.render_credit_packs(appearance, variant, app)),
            );
            options =
                options.with_child(Container::new(buy_credits).with_margin_bottom(12.).finish());
        }

        options = options.with_child(secondary);

        if let Some(status) = self.render_purchase_status(appearance, app) {
            options = options.with_child(Container::new(status).with_margin_top(12.).finish());
        }

        Container::new(options.finish())
            .with_margin_top(38.)
            .finish()
    }

    /// The selectable credit packs, laid out as a single horizontal row of
    /// equal-width tiles so the whole slide fits without the onboarding
    /// container scrolling. Mirrors the Billing & Usage page's add-on credit
    /// denominations row (`Wrap::row` of compact credit chips, 8px apart);
    /// tiles are `Expanded` here so the packs always stay on one line rather
    /// than wrapping onto a second.
    fn render_credit_packs(
        &self,
        appearance: &Appearance,
        variant: OfferVariant,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let packs = self.credit_packs(variant, app);
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(CREDIT_PACK_TILE_SPACING);
        for (index, pack) in packs.iter().enumerate() {
            row.add_child(
                Expanded::new(
                    1.,
                    Self::render_credit_pack_tile(
                        appearance,
                        *pack,
                        self.credit_pack_is_selected(variant, index, app),
                        self.credit_pack_mouse_states[index].clone(),
                        index,
                    ),
                )
                .finish(),
            );
        }
        row.finish()
    }

    /// One pack tile: the credit count (with the same credits icon the Billing
    /// & Usage denominations use), the premium-adjusted price, and the volume
    /// savings badge. Stacked vertically so four tiles fit across the card.
    fn render_credit_pack_tile(
        appearance: &Appearance,
        pack: CreditPackOption,
        selected: bool,
        mouse_state: MouseStateHandle,
        index: usize,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let border = if selected {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };
        let text_main = internal_colors::text_main(theme, bg_solid);
        let text_sub = internal_colors::text_sub(theme, bg_solid);

        let credits_icon = ConstrainedBox::new(Box::new(
            Icon::Credits.to_warpui_icon(Fill::Solid(text_main)),
        ))
        .with_width(13.)
        .with_height(13.)
        .finish();
        let credits = appearance
            .ui_builder()
            .paragraph(pack.credits_label())
            .with_style(UiComponentStyles {
                font_size: Some(15.),
                font_weight: Some(Weight::Semibold),
                font_color: Some(text_main),
                ..Default::default()
            })
            .build()
            .finish();
        let credits_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(credits_icon)
            .with_child(Container::new(credits).with_margin_left(5.).finish())
            .finish();

        let price = appearance
            .ui_builder()
            .paragraph(pack.price_label())
            .with_style(UiComponentStyles {
                font_size: Some(13.),
                font_color: Some(text_sub),
                ..Default::default()
            })
            .build()
            .finish();

        // Every tile renders a badge slot so all four are the same height. The
        // smallest pack has no volume discount, so its slot lays out the same
        // text fully transparent: that reserves exactly the right line box
        // without a fixed-height constant and without drawing a "Save 0%" that
        // would read as a real discount.
        let green = theme.ansi_fg_green();
        let has_savings = pack.savings_percent > 0;
        let badge_percent = if has_savings { pack.savings_percent } else { 0 };
        let mut badge = Container::new(
            appearance
                .ui_builder()
                .paragraph(format!("Save {badge_percent}%"))
                .with_style(UiComponentStyles {
                    font_size: Some(11.),
                    font_color: Some(if has_savings {
                        green
                    } else {
                        ColorU::transparent_black()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_horizontal_padding(6.)
        .with_vertical_padding(1.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)));
        if has_savings {
            badge = badge.with_background(Fill::Solid(green).with_opacity(10));
        }

        let tile = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(credits_row)
            .with_child(Container::new(price).with_margin_top(4.).finish())
            .with_child(Container::new(badge.finish()).with_margin_top(6.).finish())
            .finish();
        let background = selected.then(|| internal_colors::accent_overlay_1(theme));

        Hoverable::new(mouse_state, move |_| {
            let mut container = Container::new(tile)
                .with_horizontal_padding(8.)
                .with_vertical_padding(10.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(1.).with_border_fill(border));
            if let Some(background) = background {
                container = container.with_background(background);
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(OfferSlideAction::SelectCreditPack(index));
        })
        .finish()
    }

    /// Inline status for a *failed* credit purchase only. The in-flight states
    /// deliberately render nothing: the "Waiting for checkout\u{2026}" button label
    /// already says everything the user needs, so a second running commentary
    /// under the cards would be noise. A rejection is not transient — without
    /// this line the purchase would fail silently.
    fn render_purchase_status(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        match self.credit_purchase_state(app) {
            CreditPurchaseState::Idle
            | CreditPurchaseState::Purchasing
            | CreditPurchaseState::AwaitingCheckout => return None,
            CreditPurchaseState::Failed => {}
        }
        Some(
            appearance
                .ui_builder()
                .paragraph(
                    "We couldn't start that purchase. Try again, or choose \"Set up AI later\" to continue.",
                )
                .with_style(UiComponentStyles {
                    font_size: Some(13.),
                    font_color: Some(theme.ansi_fg_red()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        variant: OfferVariant,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let back = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::Back);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );
        let enter = Keystroke::parse("enter").unwrap_or_default();
        // A purchase in flight owns the primary action until it resolves; the
        // user can still pick "Set up AI later" to leave without buying.
        let purchase_in_flight = self.effective_choice(variant, app) == OfferChoice::BuyCredits
            && self.credit_purchase_state(app).is_in_flight();
        let get_warping = self.get_warping_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(if purchase_in_flight {
                    "Waiting for checkout\u{2026}".into()
                } else {
                    "Get Warping".into()
                }),
                theme: &button::themes::Primary,
                options: button::Options {
                    disabled: purchase_in_flight,
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(OfferSlideAction::GetWarping);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(back)
            .with_child(get_warping)
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_option_card(
        appearance: &Appearance,
        label: &'static str,
        description: &'static str,
        selected: bool,
        badge_label: Option<&'static str>,
        mouse_state: MouseStateHandle,
        action: OfferSlideAction,
        extra_content: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = selected.then(|| internal_colors::accent_overlay_1(theme));
        let border = if selected {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };
        let label = appearance
            .ui_builder()
            .paragraph(label)
            .with_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Semibold),
                ..Default::default()
            })
            .build()
            .finish();
        let mut header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label);
        if let Some(badge_label) = badge_label {
            let green = theme.ansi_fg_green();
            let badge = Container::new(
                appearance
                    .ui_builder()
                    .paragraph(badge_label)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        font_color: Some(green),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(11.)))
            .with_background(Fill::Solid(green).with_opacity(10))
            .finish();
            header = header.with_child(badge);
        }
        let description = appearance
            .ui_builder()
            .paragraph(description)
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_color: Some(internal_colors::text_sub(
                    theme,
                    theme.background().into_solid(),
                )),
                ..Default::default()
            })
            .build()
            .finish();
        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header.finish())
            .with_child(Container::new(description).with_margin_top(8.).finish());
        if let Some(extra_content) = extra_content {
            column = column.with_child(Container::new(extra_content).with_margin_top(16.).finish());
        }
        let content = column.finish();

        Hoverable::new(mouse_state, move |_| {
            let mut card = Container::new(content)
                .with_uniform_padding(24.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.).with_border_fill(border));
            if let Some(background) = background {
                card = card.with_background(background);
            }
            card.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_visual(&self) -> Box<dyn Element> {
        layout::onboarding_right_panel_with_bg(
            Self::VISUAL_IMAGE_PATHS[0],
            layout::FOREGROUND_LAYOUT_DEFAULT,
        )
    }

    fn send_action(&self, variant: OfferVariant, action: &str, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            OnboardingEvent::OnboardingAction {
                slide_name: variant.slide_name().to_string(),
                action: action.to_string(),
                account_class: Some(variant.account_class().to_string()),
            },
            ctx
        );
    }

    fn request_upgrade(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        self.send_action(variant, variant.primary_action(), ctx);
        self.show_auth_prompt_bar = true;
        self.onboarding_state.update(ctx, |model, ctx| {
            model.request_upgrade(ctx);
        });
        ctx.notify();
    }

    fn set_up_later(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        self.send_action(variant, "set_up_later", ctx);
        ctx.emit(OfferSlideEvent::SetUpLaterSelected { variant });
    }

    /// Starts a one-time credit-pack purchase. The app crate performs the
    /// purchase and reports back; onboarding stays on this slide until the
    /// credits actually land, so abandoning checkout doesn't advance anyone.
    fn buy_credits(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        if self.credit_purchase_state(ctx).is_in_flight() {
            return;
        }
        self.send_action(variant, variant.credits_action(), ctx);
        self.onboarding_state.update(ctx, |model, ctx| {
            model.request_credit_purchase(ctx);
        });
        ctx.notify();
    }

    fn select_choice(&mut self, choice: OfferChoice, ctx: &mut ViewContext<Self>) {
        if self.selected_choice == choice {
            return;
        }
        self.selected_choice = choice;
        ctx.notify();
    }

    /// Moves the selection `delta` positions through the options that are
    /// actually on screen, clamped at both ends.
    fn move_selection(&mut self, delta: isize, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        let choices = self.choices(variant, ctx);
        let current = self.effective_choice(variant, ctx);
        let current_index = choices
            .iter()
            .position(|choice| *choice == current)
            .unwrap_or(0) as isize;
        let next_index = (current_index + delta).clamp(0, choices.len() as isize - 1) as usize;
        self.select_choice(choices[next_index], ctx);
    }

    fn select_credit_pack(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.select_choice(OfferChoice::BuyCredits, ctx);
        self.onboarding_state.update(ctx, |model, ctx| {
            model.select_credit_pack(index, ctx);
        });
        ctx.notify();
    }

    fn get_warping(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(variant) = self.variant(ctx) else {
            return;
        };
        match self.effective_choice(variant, ctx) {
            OfferChoice::Primary => self.request_upgrade(ctx),
            OfferChoice::BuyCredits => self.buy_credits(ctx),
            OfferChoice::SetUpLater => self.set_up_later(ctx),
        }
    }

    fn back(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.back(ctx);
        });
    }
}

impl Entity for OfferSlide {
    type Event = OfferSlideEvent;
}

impl View for OfferSlide {
    fn ui_name() -> &'static str {
        "OfferSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(variant) = self.variant(app) else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let slide = layout::static_left(
            || self.render_content(appearance, variant, app),
            || self.render_visual(),
        );
        if !self.show_auth_prompt_bar {
            return slide;
        }

        let auth_prompt_bar = render_upgrade_auth_prompt_bar(
            appearance,
            self.copy_url_mouse_state.clone(),
            self.paste_token_mouse_state.clone(),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::CopyUpgradeUrl);
            }),
            Box::new(|ctx| {
                ctx.dispatch_typed_action(OfferSlideAction::PasteAuthTokenFromClipboard);
            }),
        );

        Stack::new()
            .with_child(slide)
            .with_child(Align::new(auth_prompt_bar).bottom_center().finish())
            .finish()
    }
}

impl OnboardingSlide for OfferSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.move_selection(-1, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.move_selection(1, ctx);
    }
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.get_warping(ctx);
    }
}

impl TypedActionView for OfferSlide {
    type Action = OfferSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OfferSlideAction::SelectPrimary => self.select_choice(OfferChoice::Primary, ctx),
            OfferSlideAction::SelectBuyCredits => {
                self.select_choice(OfferChoice::BuyCredits, ctx);
            }
            OfferSlideAction::SelectSetUpLater => {
                self.select_choice(OfferChoice::SetUpLater, ctx);
            }
            OfferSlideAction::SelectCreditPack(index) => self.select_credit_pack(*index, ctx),
            OfferSlideAction::Back => self.back(ctx),
            OfferSlideAction::GetWarping => self.get_warping(ctx),
            OfferSlideAction::CopyUpgradeUrl => {
                ctx.emit(OfferSlideEvent::CopyUpgradeUrlRequested);
            }
            OfferSlideAction::PasteAuthTokenFromClipboard => {
                ctx.emit(OfferSlideEvent::PasteAuthTokenFromClipboardRequested);
            }
        }
    }
}

#[cfg(test)]
#[path = "offer_slide_tests.rs"]
mod tests;
