use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use ordered_float::OrderedFloat;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, FormattedTextElement, Highlight, HighlightedHyperlink,
    MouseStateHandle, Radius, Text,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, EntityId, SingletonEntity as _};

use crate::ai::execution_profiles::model_menu_items::is_auto;
use crate::ai::llms::{
    is_using_api_key_for_provider, DisableReason, LLMId, LLMInfo, LLMPreferences, LLMProvider,
    LLMSpec,
};
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::{SearchItem, SyncDataSource};
use crate::settings_view::SettingsSection;
use crate::terminal::input::inline_menu::{
    default_navigation_message_items, InlineMenuAction, InlineMenuMessageArgs, InlineMenuType,
};
use crate::terminal::input::inline_menu::{styles as inline_styles, DetailsRenderConfig};
use crate::terminal::input::message_bar::{Message, MessageItem};
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;
use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;

use super::model_spec_scores::{
    render_model_spec_header, render_model_spec_scores, CostRow, ModelSpecScoresLayout,
    MODEL_SPECS_DESCRIPTION, MODEL_SPECS_TITLE, REASONING_LEVEL_DESCRIPTION, REASONING_LEVEL_TITLE,
};

#[derive(Clone, Debug)]
pub struct AcceptModel {
    pub id: LLMId,
}

impl InlineMenuAction for AcceptModel {
    const MENU_TYPE: InlineMenuType = InlineMenuType::ModelSelector;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        if !FeatureFlag::InlineMenuHeaders.is_enabled() {
            return Some(Message::new(default_navigation_message_items(&args)));
        }

        let mut items = vec![
            MessageItem::keystroke(Keystroke {
                key: "enter".to_owned(),
                ..Default::default()
            }),
            MessageItem::text(" to select"),
            MessageItem::keystroke(if OperatingSystem::get().is_mac() {
                Keystroke {
                    key: "enter".to_owned(),
                    cmd: true,
                    ..Default::default()
                }
            } else {
                Keystroke {
                    key: "enter".to_owned(),
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                }
            }),
            MessageItem::text(" select and save to profile"),
        ];

        if args.inline_menu_model.tab_configs().len() > 1 {
            items.push(MessageItem::keystroke(Keystroke {
                key: "tab".to_owned(),
                shift: true,
                ..Default::default()
            }));
            items.push(MessageItem::text(" to cycle tabs"));
        }

        items.push(MessageItem::clickable(
            vec![
                MessageItem::keystroke(Keystroke {
                    key: "escape".to_owned(),
                    ..Default::default()
                }),
                MessageItem::text(" to dismiss"),
            ],
            |ctx| {
                ctx.dispatch_typed_action(
                    crate::terminal::input::inline_menu::InlineMenuRowAction::<Self>::Dismiss,
                );
            },
            args.inline_menu_model.mouse_states().dismiss.clone(),
        ));

        Some(Message::new(items))
    }

    fn details_render_config(app: &AppContext) -> Option<DetailsRenderConfig> {
        let appearance = Appearance::as_ref(app);
        let max_item_width = app.font_cache().em_width(
            appearance.ui_font_family(),
            inline_styles::font_size(appearance),
        ) * 40.;
        Some(DetailsRenderConfig {
            min_required_details_width: Some(model_specs_width(app)),
            max_result_width: Some(max_item_width),
        })
    }
}

fn model_specs_width(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    app.font_cache().em_width(
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    ) * 34.
}

pub struct ModelSelectorDataSource {
    terminal_view_id: EntityId,
}

impl ModelSelectorDataSource {
    pub fn new(terminal_view_id: EntityId) -> Self {
        Self { terminal_view_id }
    }

    fn order_model_choices<'a>(
        llm_preferences: &LLMPreferences,
        choices: Vec<&'a LLMInfo>,
    ) -> Vec<&'a LLMInfo> {
        let mut auto_choices = Vec::new();
        let mut custom_choices = Vec::new();
        let mut other_choices = Vec::new();

        for llm in choices {
            if is_auto(llm) {
                auto_choices.push(llm);
            } else if llm_preferences.custom_llm_info_for_id(&llm.id).is_some() {
                custom_choices.push(llm);
            } else {
                other_choices.push(llm);
            }
        }

        auto_choices
            .into_iter()
            .chain(custom_choices)
            .chain(other_choices)
            .collect()
    }
}

impl SyncDataSource for ModelSelectorDataSource {
    type Action = AcceptModel;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let llm_preferences = LLMPreferences::as_ref(app);
        let is_full_terminal = query.filters.contains(&QueryFilter::FullTerminalUseModels);

        let active_llm_id = if is_full_terminal {
            llm_preferences
                .get_active_cli_agent_model(app, Some(self.terminal_view_id))
                .id
                .clone()
        } else {
            llm_preferences
                .get_active_base_model(app, Some(self.terminal_view_id))
                .id
                .clone()
        };

        let choices = if is_full_terminal {
            llm_preferences.get_cli_agent_llm_choices(app).collect_vec()
        } else {
            llm_preferences
                .get_base_llm_choices_for_agent_mode(app)
                .collect_vec()
        };
        let choices = Self::order_model_choices(llm_preferences, choices);

        let query_text = query.text.trim().to_lowercase();

        if query_text.is_empty() {
            return Ok(choices
                .into_iter()
                .map(|llm| QueryResult::from(ModelSearchItem::new(llm, &active_llm_id, app)))
                .collect());
        }

        Ok(choices
            .into_iter()
            .filter_map(|llm| {
                let match_result = match_indices_case_insensitive(
                    llm.display_name.to_lowercase().as_str(),
                    query_text.as_str(),
                )?;

                // Avoid spamming results with extremely weak matches.
                if query_text.len() > 1 && match_result.score < 10 {
                    return None;
                }

                Some(QueryResult::from(
                    ModelSearchItem::new(llm, &active_llm_id, app)
                        .with_name_match_result(Some(match_result.clone()))
                        .with_score(OrderedFloat(match_result.score as f64)),
                ))
            })
            .collect())
    }
}

impl Entity for ModelSelectorDataSource {
    type Event = ();
}

#[derive(Clone)]
struct ModelSearchItem {
    id: LLMId,
    provider: LLMProvider,
    spec: Option<LLMSpec>,
    provider_icon: Option<Icon>,
    display_text: String,
    is_selected: bool,
    is_custom_endpoint: bool,
    disable_reason: Option<DisableReason>,
    name_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
    manage_api_key_mouse_state: MouseStateHandle,
    reasoning_level: Option<String>,
    discount_percentage: Option<f32>,
}

impl ModelSearchItem {
    fn new(llm: &LLMInfo, active_llm_id: &LLMId, app: &AppContext) -> Self {
        // If the model requires an upgrade but the user already has a BYOK key
        // for this provider, treat it as enabled by clearing the disable reason.
        let disable_reason = if llm.disable_reason == Some(DisableReason::RequiresUpgrade)
            && is_using_api_key_for_provider(&llm.provider, app)
        {
            None
        } else {
            llm.disable_reason.clone()
        };
        let is_custom_endpoint = LLMPreferences::as_ref(app)
            .custom_llm_info_for_id(&llm.id)
            .is_some();
        Self {
            id: llm.id.clone(),
            provider: llm.provider.clone(),
            spec: llm.spec.clone(),
            provider_icon: llm.provider.icon(),
            display_text: llm.display_name.clone(),
            is_selected: &llm.id == active_llm_id,
            is_custom_endpoint,
            disable_reason,
            name_match_result: None,
            score: OrderedFloat(f64::MIN),
            manage_api_key_mouse_state: Default::default(),
            reasoning_level: llm.reasoning_level(),
            discount_percentage: llm.discount_percentage,
        }
    }

    fn with_name_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.name_match_result = result;
        self
    }

    fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }
}

impl SearchItem for ModelSearchItem {
    type Action = AcceptModel;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &crate::appearance::Appearance,
    ) -> Box<dyn Element> {
        let icon_size = inline_styles::font_size(appearance);
        let icon_color = inline_styles::icon_color(appearance);

        let icon = self
            .provider_icon
            .unwrap_or(Icon::Oz)
            .to_warpui_icon(icon_color)
            .finish();

        Container::new(
            ConstrainedBox::new(icon)
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(inline_styles::ICON_MARGIN)
        .finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use warpui::elements::{Flex, ParentElement as _};
        use warpui::prelude::CrossAxisAlignment;

        let appearance = crate::appearance::Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);
        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_text_color =
            inline_styles::secondary_text_color(theme, background_color.into());

        let name_text_color = if self.is_disabled() {
            secondary_text_color
        } else {
            primary_text_color
        };

        let mut text = Text::new_inline(
            self.display_text.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(name_text_color.into())
        .with_clip(ClipConfig::ellipsis());

        if let Some(name_match) = &self.name_match_result {
            if !name_match.matched_indices.is_empty() {
                text = text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    name_match.matched_indices.clone(),
                );
            }
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(text.finish());

        if self.is_custom_endpoint || is_using_api_key_for_provider(&self.provider, app) {
            let key_icon =
                ConstrainedBox::new(Icon::Key.to_warpui_icon(secondary_text_color).finish())
                    .with_width(font_size)
                    .with_height(font_size)
                    .finish();
            row = row.with_child(Container::new(key_icon).with_margin_left(6.).finish());
        }

        if self.is_selected {
            let selected_label = "(selected)";
            let selected_text = Text::new_inline(
                selected_label.to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(secondary_text_color.into())
            .with_single_highlight(
                Highlight::new().with_properties(Properties {
                    style: Style::Italic,
                    ..Default::default()
                }),
                (0..selected_label.len()).collect(),
            )
            .finish();
            row = row.with_child(Container::new(selected_text).with_margin_left(6.).finish());
        }

        if self.is_disabled() {
            let disabled_label = "(disabled)";
            let disabled_text = Text::new_inline(
                disabled_label.to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(secondary_text_color.into())
            .with_single_highlight(
                Highlight::new().with_properties(Properties {
                    style: Style::Italic,
                    ..Default::default()
                }),
                (0..disabled_label.len()).collect(),
            )
            .finish();
            row = row.with_child(Container::new(disabled_text).with_margin_left(6.).finish());
        }

        if should_show_discount_chip(
            self.discount_percentage,
            is_using_api_key_for_provider(&self.provider, app),
        ) {
            let discount_percentage = self.discount_percentage.unwrap_or(0.);
            let chip = Container::new(
                Text::new_inline(
                    format!("{}% off!", discount_percentage.round() as u32),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_green())
                .finish(),
            )
            .with_padding_left(4.)
            .with_padding_right(4.)
            .with_background(theme.green_overlay_1())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_left(6.)
            .finish();
            row = row.with_child(chip);
        }

        row.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &crate::appearance::Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    fn render_details(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        use warpui::elements::{Flex, ParentElement as _};

        let appearance = crate::appearance::Appearance::as_ref(app);
        let theme = appearance.theme();

        let (title, description) = if self.reasoning_level.is_some() {
            (REASONING_LEVEL_TITLE, REASONING_LEVEL_DESCRIPTION)
        } else {
            (MODEL_SPECS_TITLE, MODEL_SPECS_DESCRIPTION)
        };
        let header = render_model_spec_header(title, description, app);

        let is_using_api_key =
            self.is_custom_endpoint || is_using_api_key_for_provider(&self.provider, app);
        let cost_row = if is_using_api_key {
            let manage_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Outlined,
                    self.manage_api_key_mouse_state.clone(),
                )
                .with_text_label("Manage".to_string())
                .with_style(UiComponentStyles {
                    height: Some(24.),
                    padding: Some(Coords {
                        top: 2.,
                        bottom: 2.,
                        left: 4.,
                        right: 4.,
                    }),
                    ..Default::default()
                })
                .with_cursor(Some(Cursor::PointingHand))
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                        search_query: "api".to_string(),
                        section: Some(SettingsSection::WarpAgent),
                    });
                })
                .finish();

            CostRow::BilledToApi {
                manage_button: Container::new(manage_button).finish(),
            }
        } else {
            CostRow::Bar {
                value: self.spec.as_ref().map(|spec| spec.cost),
            }
        };

        let scores = render_model_spec_scores(
            self.spec.as_ref(),
            cost_row,
            ModelSpecScoresLayout {
                bg_bar_color: internal_colors::neutral_3(theme),
            },
            app,
        );

        let mut column = Flex::column()
            .with_child(Container::new(header).with_margin_bottom(12.).finish())
            .with_child(scores);

        if self.disable_reason.as_ref() == Some(&DisableReason::RequiresUpgrade) {
            let upgrade_url = if let Some(team) = UserWorkspaces::as_ref(app).current_team() {
                UserWorkspaces::upgrade_link_for_team(team.uid)
            } else {
                let user_id = AuthStateProvider::as_ref(app)
                    .get()
                    .user_id()
                    .unwrap_or_default();
                UserWorkspaces::upgrade_link(user_id)
            };

            let mut display_name = self.display_text.clone();
            if let Some(first) = display_name.get_mut(..1) {
                first.make_ascii_uppercase();
            }

            // Show a BYOK option when the user's tier supports it and the provider
            // is one that accepts user-supplied API keys.
            let byok_available = UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app)
                && matches!(
                    self.provider,
                    LLMProvider::OpenAI | LLMProvider::Anthropic | LLMProvider::Google
                );

            let mut text_fragments = vec![
                FormattedTextFragment::plain_text(format!(
                    "{display_name} is not available for free users. "
                )),
                FormattedTextFragment::hyperlink("Upgrade", upgrade_url),
            ];

            if byok_available {
                text_fragments.push(FormattedTextFragment::plain_text(" or ".to_string()));
                text_fragments.push(FormattedTextFragment::hyperlink_action(
                    "bring your own key",
                    WorkspaceAction::ShowSettingsPageWithSearch {
                        search_query: "api".to_string(),
                        section: Some(SettingsSection::WarpAgent),
                    },
                ));
            }

            let upgrade_text = FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(text_fragments)]),
                inline_styles::font_size(appearance),
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                theme.disabled_ui_text_color().into_solid(),
                HighlightedHyperlink::default(),
            )
            .with_hyperlink_font_color(theme.accent().into_solid())
            .register_default_click_handlers_with_action_support(|hyperlink_lens, event, ctx| {
                match hyperlink_lens {
                    warpui::elements::HyperlinkLens::Url(url) => {
                        ctx.open_url(url);
                    }
                    warpui::elements::HyperlinkLens::Action(action_ref) => {
                        if let Some(action) = action_ref.as_any().downcast_ref::<WorkspaceAction>()
                        {
                            event.dispatch_typed_action(action.clone());
                        }
                    }
                }
            })
            .finish();

            column = column.with_child(Container::new(upgrade_text).with_margin_top(12.).finish());
        }

        Some(
            ConstrainedBox::new(column.finish())
                .with_width(model_specs_width(app))
                .finish(),
        )
    }

    fn priority_tier(&self) -> u8 {
        if self.is_disabled() {
            1
        } else {
            0
        }
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        AcceptModel {
            id: self.id.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn is_disabled(&self) -> bool {
        self.disable_reason.is_some()
    }

    fn tooltip(&self) -> Option<String> {
        self.disable_reason
            .as_ref()
            .map(|reason| reason.tooltip_text().to_string())
    }

    fn accessibility_label(&self) -> String {
        let mut label = format!("Model: {}", self.display_text);
        if self.is_selected {
            label.push_str(" (selected)");
        }
        if self.is_disabled() {
            label.push_str(" (disabled)");
        }
        label
    }
}

/// Returns true when a promo discount chip should be shown for a model.
/// Discounts only apply when the user is billing through Warp credits,
/// so we suppress the chip when the user is routing through their own API key.
fn should_show_discount_chip(discount_percentage: Option<f32>, is_using_byok: bool) -> bool {
    discount_percentage.is_some_and(|p| p > 0.) && !is_using_byok
}
