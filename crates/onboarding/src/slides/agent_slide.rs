use ai::LLMId;
use pathfinder_color::ColorU;
use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui_core::elements::{
    AnchorPair, Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Dismiss, Empty, Flex, FormattedTextElement, Hoverable,
    Icon as WarpUiIcon, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    OffsetType, ParentElement, ParentOffsetBounds, PositioningAxis, Radius, SavePosition,
    ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Stack, Text, XAxisAnchor, YAxisAnchor,
};
use warpui_core::fonts::{Properties, Weight};
use warpui_core::keymap::Keystroke;
use warpui_core::platform::Cursor;
use warpui_core::scene::DropShadow;
use warpui_core::text_layout::TextAlignment;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

use super::two_line_button::{render_two_line_button, TwoLineButtonSpec};
use super::OnboardingSlide;
use crate::model::{OnboardingStateEvent, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};
use crate::visuals::agent_visual;

/// Information about a model displayed on the onboarding slide.
#[derive(Clone, Debug)]
pub struct OnboardingModelInfo {
    pub id: LLMId,
    pub title: String,
    pub icon: Icon,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentAutonomy {
    Full,
    #[default]
    Partial,
    None,
}

impl std::fmt::Display for AgentAutonomy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentAutonomy::Full => write!(f, "full"),
            AgentAutonomy::Partial => write!(f, "partial"),
            AgentAutonomy::None => write!(f, "none"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDevelopmentSettings {
    /// The selected model's ID.
    pub selected_model_id: LLMId,
    pub autonomy: Option<AgentAutonomy>,
    /// Whether the CLI agent toolbar is enabled (maps to `should_render_cli_agent_footer`).
    pub cli_agent_toolbar_enabled: bool,
    /// The default session mode chosen during onboarding.
    pub session_default: crate::SessionDefault,
    /// Whether the user chose to disable the Oz AI assistant.
    pub disable_oz: bool,
    /// Whether agent notifications (mailbox button, toasts, notification items) are shown.
    pub show_agent_notifications: bool,
}

impl AgentDevelopmentSettings {
    pub fn new(default_model_id: LLMId) -> Self {
        Self {
            selected_model_id: default_model_id,
            autonomy: Some(AgentAutonomy::default()),
            cli_agent_toolbar_enabled: true,
            session_default: crate::SessionDefault::Agent,
            disable_oz: false,
            show_agent_notifications: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AgentSlideAction {
    /// Select model by its ID. When the picker is expanded this also collapses it.
    SelectModel(LLMId),
    /// Toggle the expanded state of the collapsed picker chip.
    ToggleModelListExpanded,
    /// Update the keyboard/hover highlight cursor to the given model id.
    /// Dispatched from hover handlers on enabled rows.
    HighlightModel(LLMId),
    SelectAutonomy(AgentAutonomy),
    ToggleDisableOz,
    BackClicked,
    NextClicked,
}

pub struct AgentSlide {
    onboarding_state: warpui_core::ModelHandle<OnboardingStateModel>,

    /// Mouse state handles for each model row.
    model_mouse_states: Vec<MouseStateHandle>,

    /// Mouse state handle for the collapsed model-picker chip (closed-state click target).
    chip_mouse_state: MouseStateHandle,

    autonomy_full_mouse_state: MouseStateHandle,
    autonomy_partial_mouse_state: MouseStateHandle,
    autonomy_none_mouse_state: MouseStateHandle,

    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
    dropdown_scroll_state: ClippedScrollStateHandle,
    is_model_list_expanded: bool,
    highlighted_model_id: Option<LLMId>,
}

/// Produces the `SavePosition` id for the model row at `index` in the
/// dropdown list. Used by `scroll_to_position` to scroll a specific row into
/// view as the keyboard highlight moves.
fn model_row_position_id(index: usize) -> String {
    format!("agent_slide_model_row_{index}")
}

impl AgentSlide {
    pub(crate) fn new(
        onboarding_state: warpui_core::ModelHandle<OnboardingStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model_count = onboarding_state.as_ref(ctx).models().len();
        let model_mouse_states = (0..model_count)
            .map(|_| MouseStateHandle::default())
            .collect();

        ctx.subscribe_to_model(&onboarding_state, |me, model, event, ctx| {
            match event {
                OnboardingStateEvent::ModelsUpdated => {
                    let state = model.as_ref(ctx);
                    // Clear the highlight if its id is no longer in the list.
                    let still_present = me
                        .highlighted_model_id
                        .as_ref()
                        .is_some_and(|id| state.models().iter().any(|m| &m.id == id));
                    if !still_present {
                        me.highlighted_model_id = None;
                    }
                    let model_count = state.models().len();
                    me.ensure_mouse_states_for_models(model_count, ctx);
                }
                OnboardingStateEvent::AuthStateChanged
                | OnboardingStateEvent::SelectedSlideChanged
                | OnboardingStateEvent::IntentionChanged
                | OnboardingStateEvent::Completed
                | OnboardingStateEvent::UpgradeRequested
                | OnboardingStateEvent::NoAiConfirmationChanged => {}
            }
        });

        Self {
            onboarding_state,
            model_mouse_states,
            chip_mouse_state: MouseStateHandle::default(),
            autonomy_full_mouse_state: MouseStateHandle::default(),
            autonomy_partial_mouse_state: MouseStateHandle::default(),
            autonomy_none_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            dropdown_scroll_state: ClippedScrollStateHandle::new(),
            is_model_list_expanded: false,
            highlighted_model_id: None,
        }
    }

    /// Ensures we have enough mouse state handles for the given model count.
    fn ensure_mouse_states_for_models(&mut self, model_count: usize, ctx: &mut ViewContext<Self>) {
        if self.model_mouse_states.len() < model_count {
            self.model_mouse_states.extend(
                (self.model_mouse_states.len()..model_count).map(|_| MouseStateHandle::default()),
            );
        }
        ctx.notify();
    }

    fn agent_settings<'a>(&self, app: &'a AppContext) -> &'a AgentDevelopmentSettings {
        self.onboarding_state.as_ref(app).agent_settings()
    }

    fn workspace_enforces_autonomy(&self, app: &AppContext) -> bool {
        self.onboarding_state
            .as_ref(app)
            .workspace_enforces_autonomy()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        settings: &AgentDevelopmentSettings,
        workspace_enforces_autonomy: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // The collapsed slide content is rendered unconditionally — the expanded
        // state is a floating overlay (built in `View::render`) that sits *on top
        // of* this content, so the underlying layout never shifts between the two
        // states. That keeps the header + picker chip pinned in place.
        let bottom_nav = self.render_bottom_nav(appearance, app);
        slide_content::onboarding_slide_content(
            vec![
                self.render_header(appearance),
                self.render_sections(appearance, settings, workspace_enforces_autonomy, app),
            ],
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .paragraph("Customize your Warp Agent")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Select your Warp Agent's defaults.",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_sub(
            appearance.theme(),
            appearance.theme().background().into_solid(),
        ))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.0)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_sections(
        &self,
        appearance: &Appearance,
        settings: &AgentDevelopmentSettings,
        workspace_enforces_autonomy: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let model_section = self.render_model_section(appearance, settings, app);
        let autonomy_section = if workspace_enforces_autonomy {
            self.render_autonomy_workspace_enforced(appearance)
        } else {
            self.render_autonomy_section(appearance, settings)
        };

        let upper_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(model_section)
            .with_child(
                Container::new(autonomy_section)
                    .with_margin_top(24.)
                    .finish(),
            );

        // Apply a semi-transparent overlay to visually disable the upper sections
        // when the "Disable Oz" checkbox is checked.
        let upper_sections: Box<dyn Element> = if settings.disable_oz {
            let bg = appearance.theme().background().into_solid();
            let overlay_color = ColorU::new(bg.r, bg.g, bg.b, 128);
            Container::new(upper_col.finish())
                .with_foreground_overlay(overlay_color)
                .finish()
        } else {
            upper_col.finish()
        };

        let col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(upper_sections);

        Container::new(col.finish()).with_margin_top(40.).finish()
    }

    fn render_section_header(
        &self,
        title: &'static str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(title)
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_model_section(
        &self,
        appearance: &Appearance,
        settings: &AgentDevelopmentSettings,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let header = self.render_section_header("Default model", appearance);

        let expanded = self.is_model_list_expanded;
        let chip = self.render_collapsed_model_chip(appearance, settings, app, expanded);

        // Wrap the chip in a `Stack` so the floating dropdown overlay can live
        // inline here (as a child of the left column) and inherit the chip's
        // full column width via `ParentOffsetBounds::ParentBySize`. When the
        // picker is collapsed the Stack has just the chip as its single child
        // and lays out exactly like a bare chip would.
        let mut chip_stack = Stack::new().with_child(chip);
        if expanded {
            let positioning = OffsetPositioning::from_axes(
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::ParentBySize,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(4.),
                    AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                ),
            );
            chip_stack.add_positioned_overlay_child(
                self.render_model_list_overlay(appearance, app),
                positioning,
            );
        }

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(
                Container::new(chip_stack.finish())
                    .with_margin_top(12.)
                    .finish(),
            )
            .finish()
    }

    /// Renders the single-row collapsed picker button: provider icon, selected title,
    /// and trailing chevron. `expanded` controls whether the chip draws its blue
    /// "focused" border (when the full-list view is active) or the neutral border.
    fn render_collapsed_model_chip(
        &self,
        appearance: &Appearance,
        settings: &AgentDevelopmentSettings,
        app: &AppContext,
        expanded: bool,
    ) -> Box<dyn Element> {
        const CHIP_HEIGHT: f32 = 48.;
        const CHIP_RADIUS: f32 = 8.;

        let theme = appearance.theme();
        let background_for_text = theme.background().into_solid();
        let ui_font_family = appearance.ui_font_family();

        let models = self.onboarding_state.as_ref(app).models();
        let selected = models
            .iter()
            .find(|m| m.id == settings.selected_model_id)
            .or_else(|| models.first());

        let (title_text, icon) = match selected {
            Some(model) => (model.title.clone(), Some(model.icon)),
            None => ("".to_string(), None),
        };
        let is_disabled = settings.disable_oz;

        let title_color: ColorU = if is_disabled {
            internal_colors::text_disabled(theme, background_for_text)
        } else {
            internal_colors::text_main(theme, background_for_text)
        };

        let border_color = if expanded && !is_disabled {
            theme.accent()
        } else {
            Fill::Solid(internal_colors::neutral_4(theme))
        };

        let mouse_state = self.chip_mouse_state.clone();

        let hoverable = Hoverable::new(mouse_state, move |_| {
            let title_el = Text::new(title_text.clone(), ui_font_family, 14.0)
                .with_color(title_color)
                .with_style(Properties {
                    weight: Weight::Normal,
                    ..Default::default()
                })
                .with_line_height_ratio(1.0)
                .finish();

            let title_row: Box<dyn Element> = if let Some(icon) = icon {
                const ICON_SIZE: f32 = 14.;
                let icon_el =
                    ConstrainedBox::new(Box::new(icon.to_warpui_icon(title_color.into())))
                        .with_width(ICON_SIZE)
                        .with_height(ICON_SIZE)
                        .finish();
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icon_el)
                    .with_child(Container::new(title_el).with_margin_left(8.).finish())
                    .finish()
            } else {
                title_el
            };

            // Trailing chevron icon.
            let chevron = ConstrainedBox::new(Box::new(WarpUiIcon::new(
                "bundled/svg/chevron-down.svg",
                title_color,
            )))
            .with_width(14.)
            .with_height(14.)
            .finish();

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(title_row)
                .with_child(chevron)
                .finish();

            ConstrainedBox::new(
                Container::new(row)
                    .with_horizontal_padding(16.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CHIP_RADIUS)))
                    .with_border(Border::all(1.).with_border_fill(border_color))
                    .finish(),
            )
            .with_min_height(CHIP_HEIGHT)
            .finish()
        });

        if is_disabled {
            hoverable.finish()
        } else {
            hoverable
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(AgentSlideAction::ToggleModelListExpanded);
                })
                .finish()
        }
    }

    /// Renders the vertical list of model rows shown inside the floating dropdown
    /// overlay. Each row shows a provider icon + title on the left, with a
    /// "Recommended" pill on the right for the default model.
    fn render_model_list_rows(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        const ROW_HEIGHT: f32 = 48.;
        const ROW_GAP: f32 = 2.;

        let state = self.onboarding_state.as_ref(app);
        let highlighted_id = self.highlighted_model_id.clone();
        let selected_id = state.agent_settings().selected_model_id.clone();
        let models = state.models();

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (index, model) in models.iter().enumerate() {
            let mouse_state = self
                .model_mouse_states
                .get(index)
                .cloned()
                .unwrap_or_default();

            let is_highlighted = highlighted_id.as_ref() == Some(&model.id)
                || (highlighted_id.is_none() && model.id == selected_id);
            let row =
                self.render_model_row(appearance, model, is_highlighted, mouse_state, ROW_HEIGHT);
            // Wrap each row in `SavePosition` so the scrollable can scroll
            // the keyboard-highlighted row into view (see
            // `advance_highlighted_model`). Mirrors the pattern in
            // `VerticalTabsPanelState::scroll_to_tab`.
            let row = SavePosition::new(row, &model_row_position_id(index)).finish();

            let margin_top = if index == 0 { 0. } else { ROW_GAP };
            col = col.with_child(Container::new(row).with_margin_top(margin_top).finish());
        }
        col.finish()
    }

    fn render_model_list_overlay(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        const OVERLAY_RADIUS: f32 = 8.;
        const OVERLAY_PADDING: f32 = 4.;
        const OVERLAY_MAX_HEIGHT: f32 = 400.;

        let theme = appearance.theme();
        let list = self.render_model_list_rows(appearance, app);

        // Wrap the list in a vertical `ClippedScrollable` so rows scroll when
        // they exceed `OVERLAY_MAX_HEIGHT`.
        let scrollable = ClippedScrollable::vertical(
            self.dropdown_scroll_state.clone(),
            list,
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.surface_1()).into(),
            theme.main_text_color(theme.surface_1()).into(),
            theme.surface_1().into(),
        )
        .with_overlayed_scrollbar()
        .finish();

        let card = ConstrainedBox::new(
            Container::new(scrollable)
                .with_background(theme.surface_1())
                .with_border(Border::all(1.).with_border_color(internal_colors::neutral_4(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(OVERLAY_RADIUS)))
                .with_uniform_padding(OVERLAY_PADDING)
                .with_drop_shadow(DropShadow::default())
                .finish(),
        )
        .with_max_height(OVERLAY_MAX_HEIGHT)
        .finish();

        Dismiss::new(card)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(AgentSlideAction::ToggleModelListExpanded);
            })
            .finish()
    }

    fn render_model_row(
        &self,
        appearance: &Appearance,
        model: &OnboardingModelInfo,
        is_highlighted: bool,
        mouse_state: MouseStateHandle,
        height: f32,
    ) -> Box<dyn Element> {
        const ROW_RADIUS: f32 = 6.;

        let theme = appearance.theme();
        let background_for_text = theme.background().into_solid();
        let ui_font_family = appearance.ui_font_family();

        let title_color: ColorU = internal_colors::text_main(theme, background_for_text);

        let row_id = model.id.clone();
        let title = model.title.clone();
        let icon = model.icon;
        let is_default = model.is_default;

        let hoverable_body = Hoverable::new(mouse_state, move |_| {
            let title_el = Text::new(title.clone(), ui_font_family, 14.0)
                .with_color(title_color)
                .with_style(Properties {
                    weight: Weight::Normal,
                    ..Default::default()
                })
                .with_line_height_ratio(1.0)
                .finish();

            const ICON_SIZE: f32 = 14.;
            let icon_el = ConstrainedBox::new(Box::new(icon.to_warpui_icon(title_color.into())))
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish();
            let title_row: Box<dyn Element> = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(icon_el)
                .with_child(Container::new(title_el).with_margin_left(8.).finish())
                .finish();

            // "Recommended" pill on the server-designated default model.
            let make_pill = |label: &'static str| -> Box<dyn Element> {
                let badge = Text::new(label.to_string(), ui_font_family, 11.0)
                    .with_color(internal_colors::text_sub(theme, background_for_text))
                    .with_style(Properties {
                        weight: Weight::Normal,
                        ..Default::default()
                    })
                    .with_line_height_ratio(1.0)
                    .finish();
                Container::new(badge)
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .with_padding_top(4.)
                    .with_padding_bottom(4.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_background(Fill::Solid(internal_colors::neutral_3(theme)))
                    .finish()
            };

            let trailing: Box<dyn Element> = if is_default {
                make_pill("Recommended")
            } else {
                Empty::new().finish()
            };

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(title_row)
                .with_child(trailing)
                .finish();

            let background = if is_highlighted {
                Some(Fill::Solid(internal_colors::neutral_2(theme)))
            } else {
                None
            };

            let mut container = Container::new(row)
                .with_horizontal_padding(12.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));
            if let Some(bg) = background {
                container = container.with_background(bg);
            }

            ConstrainedBox::new(container.finish())
                .with_min_height(height)
                .finish()
        });

        let click_id = row_id.clone();
        let hover_id = row_id;
        hoverable_body
            .with_cursor(Cursor::PointingHand)
            .on_hover(move |is_hovered, ctx, _, _| {
                if is_hovered {
                    ctx.dispatch_typed_action(AgentSlideAction::HighlightModel(hover_id.clone()));
                }
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AgentSlideAction::SelectModel(click_id.clone()));
            })
            .finish()
    }

    fn render_autonomy_workspace_enforced(&self, appearance: &Appearance) -> Box<dyn Element> {
        let header = self.render_section_header("Autonomy", appearance);

        let theme = appearance.theme();
        let background_for_text = theme.background().into_solid();
        let ui_font_family = appearance.ui_font_family();

        let title_color = internal_colors::text_main(theme, background_for_text);
        let subtitle_color = internal_colors::text_sub(theme, background_for_text);

        let title_el = Text::new("Set by Team Workspace", ui_font_family, 14.0)
            .with_color(title_color)
            .with_style(Properties {
                weight: Weight::Normal,
                ..Default::default()
            })
            .with_line_height_ratio(1.0)
            .finish();

        let subtitle_el = Text::new(
            "Autonomy settings are configured as part of your team workspace.",
            ui_font_family,
            12.0,
        )
        .with_color(subtitle_color)
        .with_style(Properties {
            weight: Weight::Normal,
            ..Default::default()
        })
        .with_line_height_ratio(1.0)
        .finish();

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title_el)
            .with_child(Container::new(subtitle_el).with_margin_top(8.).finish())
            .finish();

        let message_box = Container::new(content)
            .with_uniform_padding(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_color(internal_colors::neutral_4(theme)))
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Container::new(message_box).with_margin_top(12.).finish())
            .finish()
    }

    fn render_autonomy_section(
        &self,
        appearance: &Appearance,
        settings: &AgentDevelopmentSettings,
    ) -> Box<dyn Element> {
        let header = self.render_section_header("Autonomy", appearance);

        // The rows now take the full column width (vs. the previous three-across layout),
        // so they no longer need the extra height that came from cramped subtitle wrapping.
        const OPTION_HEIGHT: f32 = 72.;
        const OPTION_GAP: f32 = 8.;

        let theme = appearance.theme();
        let background_for_text = theme.background().into_solid();
        let text_main = internal_colors::text_main(theme, background_for_text);
        let text_sub = internal_colors::text_sub(theme, background_for_text);

        let autonomy_options: [(AgentAutonomy, &str, &str, MouseStateHandle); 3] = [
            (
                AgentAutonomy::Full,
                "Full",
                "Warp Agent runs commands, writes code, and reads files without asking.",
                self.autonomy_full_mouse_state.clone(),
            ),
            (
                AgentAutonomy::Partial,
                "Partial",
                "Warp Agent can plan, read files, and execute low-risk commands. Asks before making any changes or executing sensitive commands.",
                self.autonomy_partial_mouse_state.clone(),
            ),
            (
                AgentAutonomy::None,
                "None",
                "Warp Agent takes no actions without your approval.",
                self.autonomy_none_mouse_state.clone(),
            ),
        ];

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (idx, (autonomy, title, subtitle, mouse_state)) in
            autonomy_options.into_iter().enumerate()
        {
            let is_selected = !settings.disable_oz && settings.autonomy == Some(autonomy);
            let (title_color, subtitle_color) = if is_selected {
                (text_main.into(), text_main.into())
            } else {
                (text_sub.into(), text_sub.into())
            };

            let button = render_two_line_button(
                appearance,
                TwoLineButtonSpec {
                    is_selected,
                    title: title.to_string(),
                    subtitle: subtitle.to_string(),
                    height: OPTION_HEIGHT,
                    mouse_state,
                    click_action: AgentSlideAction::SelectAutonomy(autonomy),
                    subtitle_font_size: 12.0,
                    title_color,
                    subtitle_color,
                    icon: None,
                    disabled_badge: None,
                    is_disabled: settings.disable_oz,
                },
            );

            let margin_top = if idx == 0 { 0. } else { OPTION_GAP };
            col = col.with_child(Container::new(button).with_margin_top(margin_top).finish());
        }

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Container::new(col.finish()).with_margin_top(12.).finish())
            .finish()
    }

    fn render_bottom_nav(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AgentSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Next".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AgentSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let (step_index, step_count) = self.onboarding_state.as_ref(app).progress();
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    fn render_visual(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            let use_vertical = self
                .onboarding_state
                .as_ref(app)
                .ui_customization()
                .use_vertical_tabs;
            let path = if use_vertical {
                "async/png/onboarding/agent_intention/customize_vertical_tabs.png"
            } else {
                "async/png/onboarding/agent_intention/customize_horizontal_tabs.png"
            };
            layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_WIDE)
        } else {
            let panel_background = internal_colors::neutral_2(theme);
            let neutral = internal_colors::neutral_4(theme);

            let blue = theme.ansi_fg_blue();
            let green = theme.ansi_fg_green();
            let yellow = theme.ansi_fg_yellow();

            Container::new(agent_visual(panel_background, neutral, blue, green, yellow))
                .with_background_color(internal_colors::neutral_1(theme))
                .finish()
        }
    }
}

impl Entity for AgentSlide {
    type Event = ();
}

impl View for AgentSlide {
    fn ui_name() -> &'static str {
        "AgentSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let settings = self.agent_settings(app);
        let workspace_enforces_autonomy = self.workspace_enforces_autonomy(app);

        // The floating dropdown overlay is built inside `render_model_section`
        // so it inherits the column width naturally. Here we only need the
        // base two-column layout.
        layout::static_left(
            || self.render_content(appearance, settings, workspace_enforces_autonomy, app),
            || self.render_visual(appearance, app),
        )
    }
}

impl AgentSlide {
    fn select_model(&mut self, model_id: LLMId, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |state, ctx| {
            state.on_user_selected_model(model_id, ctx);
        });
        ctx.notify();
    }

    fn set_model_list_expanded(&mut self, expanded: bool, ctx: &mut ViewContext<Self>) {
        if self.is_model_list_expanded == expanded {
            return;
        }
        self.is_model_list_expanded = expanded;
        if expanded {
            // Seed the highlight from the current selection so keyboard nav
            // starts on the selected row.
            let state = self.onboarding_state.as_ref(ctx);
            let selected_id = state.agent_settings().selected_model_id.clone();
            if let Some(index) = state.models().iter().position(|m| m.id == selected_id) {
                self.dropdown_scroll_state.scroll_to_position(ScrollTarget {
                    position_id: model_row_position_id(index),
                    mode: ScrollToPositionMode::FullyIntoView,
                });
            }
            self.highlighted_model_id = Some(selected_id);
        }
        ctx.notify();
    }

    /// Advances the highlight cursor to the next/previous model, wrapping. The
    /// origin of the walk is the currently-highlighted id (if any), else the
    /// currently-selected id. Also scrolls the dropdown so the newly-highlighted
    /// row stays visible — same `SavePosition` + `scroll_to_position` pattern
    /// used by `VerticalTabsPanelState::scroll_to_tab`.
    fn advance_highlighted_model(&mut self, forward: bool, ctx: &mut ViewContext<Self>) {
        let (model_ids, selected_id) = {
            let state = self.onboarding_state.as_ref(ctx);
            let ids: Vec<LLMId> = state.models().iter().map(|m| m.id.clone()).collect();
            (ids, state.agent_settings().selected_model_id.clone())
        };
        let count = model_ids.len();
        if count == 0 {
            return;
        }
        let start_index = self
            .highlighted_model_id
            .as_ref()
            .and_then(|id| model_ids.iter().position(|m| m == id))
            .or_else(|| model_ids.iter().position(|m| *m == selected_id))
            .unwrap_or(0);
        let next_index = if forward {
            (start_index + 1) % count
        } else {
            (start_index + count - 1) % count
        };
        self.highlighted_model_id = Some(model_ids[next_index].clone());
        // Scroll the dropdown so the new highlight is visible. `FullyIntoView`
        // is a no-op when the row is already fully in view, otherwise it
        // scrolls the minimum amount to show it.
        self.dropdown_scroll_state.scroll_to_position(ScrollTarget {
            position_id: model_row_position_id(next_index),
            mode: ScrollToPositionMode::FullyIntoView,
        });
        ctx.notify();
    }

    fn select_autonomy(&mut self, autonomy: AgentAutonomy, ctx: &mut ViewContext<Self>) {
        if self.workspace_enforces_autonomy(ctx) {
            return;
        }
        self.onboarding_state.update(ctx, |state, ctx| {
            state.set_agent_autonomy(autonomy, ctx);
        });
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |state, ctx| {
            state.next(ctx);
        });
    }
}

impl OnboardingSlide for AgentSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.agent_settings(ctx).disable_oz {
            return;
        }
        // While the picker is expanded, arrow keys move the highlight cursor
        // instead of cycling autonomy.
        if self.is_model_list_expanded {
            self.advance_highlighted_model(/* forward */ false, ctx);
            return;
        }
        if self.workspace_enforces_autonomy(ctx) {
            return;
        }
        let Some(autonomy) = self.agent_settings(ctx).autonomy else {
            return;
        };
        let up_autonomy = match autonomy {
            AgentAutonomy::Full => AgentAutonomy::None,
            AgentAutonomy::Partial => AgentAutonomy::Full,
            AgentAutonomy::None => AgentAutonomy::Partial,
        };

        self.select_autonomy(up_autonomy, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.agent_settings(ctx).disable_oz {
            return;
        }
        if self.is_model_list_expanded {
            self.advance_highlighted_model(/* forward */ true, ctx);
            return;
        }
        if self.workspace_enforces_autonomy(ctx) {
            return;
        }
        let Some(autonomy) = self.agent_settings(ctx).autonomy else {
            return;
        };
        let down_autonomy = match autonomy {
            AgentAutonomy::Full => AgentAutonomy::Partial,
            AgentAutonomy::Partial => AgentAutonomy::None,
            AgentAutonomy::None => AgentAutonomy::Full,
        };

        self.select_autonomy(down_autonomy, ctx);
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        // If the picker is expanded: Enter selects the highlighted row (if any)
        // and collapses the list. Does NOT advance to the next slide.
        if self.is_model_list_expanded {
            if let Some(id) = self.highlighted_model_id.clone() {
                self.select_model(id, ctx);
            }
            self.set_model_list_expanded(false, ctx);
            return;
        }
        self.next(ctx);
    }

    fn on_escape(&mut self, ctx: &mut ViewContext<Self>) {
        // Escape closes the expanded picker without mutating selection.
        if self.is_model_list_expanded {
            self.set_model_list_expanded(false, ctx);
        }
    }
}

impl TypedActionView for AgentSlide {
    type Action = AgentSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentSlideAction::SelectModel(model_id) => {
                if !self.agent_settings(ctx).disable_oz {
                    self.select_model(model_id.clone(), ctx);
                    // If the picker is open, clicking a row collapses it after selecting.
                    if self.is_model_list_expanded {
                        self.set_model_list_expanded(false, ctx);
                    }
                }
            }
            AgentSlideAction::ToggleModelListExpanded => {
                if self.agent_settings(ctx).disable_oz {
                    return;
                }
                self.set_model_list_expanded(!self.is_model_list_expanded, ctx);
            }
            AgentSlideAction::HighlightModel(model_id) => {
                if self.highlighted_model_id.as_ref() != Some(model_id) {
                    self.highlighted_model_id = Some(model_id.clone());
                    ctx.notify();
                }
            }
            AgentSlideAction::SelectAutonomy(autonomy) => {
                if !self.agent_settings(ctx).disable_oz {
                    self.select_autonomy(*autonomy, ctx);
                }
            }
            AgentSlideAction::ToggleDisableOz => {
                let current = self
                    .onboarding_state
                    .as_ref(ctx)
                    .agent_settings()
                    .disable_oz;
                self.onboarding_state.update(ctx, |state, ctx| {
                    state.set_disable_oz(!current, ctx);
                });
                ctx.notify();
            }
            AgentSlideAction::BackClicked => {
                self.onboarding_state.update(ctx, |state, ctx| {
                    state.back(ctx);
                });
            }
            AgentSlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
