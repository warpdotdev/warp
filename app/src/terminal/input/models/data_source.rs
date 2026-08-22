use fuzzy_match::{FuzzyMatchResult, match_indices_case_insensitive};
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use ordered_float::OrderedFloat;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, Empty, FormattedTextElement, Highlight,
    HighlightedHyperlink, MouseStateHandle, Radius, Text,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::keymap::Keystroke;
use warpui::platform::{Cursor, OperatingSystem};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _,
    WeakViewHandle,
};

use super::model_spec_scores::{
    CUSTOM_MODEL_ROUTER_DESCRIPTION, CUSTOM_MODEL_ROUTER_TITLE, CostRow, MODEL_SPECS_DESCRIPTION,
    MODEL_SPECS_TITLE, ModelSpecScoresLayout, REASONING_LEVEL_DESCRIPTION, REASONING_LEVEL_TITLE,
    render_model_spec_header, render_model_spec_scores,
};
use super::view::InlineModelSelectorView;
use crate::ai::custom_model_routers::is_custom_router_id;
use crate::ai::execution_profiles::model_menu_items::is_auto;
use crate::ai::llms::{
    ByoKeySource, DisableReason, LLMId, LLMInfo, LLMPreferences, LLMProvider, ModelIconFlags,
    byo_key_source_for_model_for_render_context, effective_model_disable_reason,
    effective_model_disable_reason_for_render_context, model_leading_icon,
    should_show_bedrock_icon_for_model_for_render_context,
    should_show_gemini_enterprise_agent_platform_icon_for_model_for_render_context,
};
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::{SearchItem, SyncDataSource};
use crate::settings_view::SettingsSection;
use crate::terminal::input::inline_menu::{
    DetailsRenderConfig, InlineMenuAction, InlineMenuMessageArgs, InlineMenuType,
    default_navigation_message_items, styles as inline_styles,
};
use crate::terminal::input::message_bar::{Message, MessageItem};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::{TeamContextForOperation, UserWorkspaces};

/// Auto models pick their concrete model server-side, so the cost line names the
/// class of inference rather than a host the request may never reach.
const AUTO_HOSTED_INFERENCE_LABEL: &str = "Inference may use your hosted inference";

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
/// Frontend-neutral model picker result shared by GUI and TUI surfaces.
#[derive(Clone, Debug)]
pub struct ModelPickerChoice {
    pub llm: LLMInfo,
    pub disable_reason: Option<DisableReason>,
    pub name_match_result: Option<FuzzyMatchResult>,
    pub score: OrderedFloat<f64>,
}

impl ModelPickerChoice {
    pub fn is_selectable(&self) -> bool {
        self.disable_reason.is_none()
    }

    fn priority_tier(&self) -> u8 {
        if self.is_selectable() { 0 } else { 1 }
    }
}

/// Applies the GUI model picker's ordering, fuzzy filtering, and effective disabled state.
pub fn query_model_picker_choices<'a>(
    llm_preferences: &LLMPreferences,
    choices: impl IntoIterator<Item = &'a LLMInfo>,
    query_text: &str,
    team_context: &TeamContextForOperation,
    app: &AppContext,
) -> Vec<ModelPickerChoice> {
    let choices = ModelSelectorDataSource::order_model_choices(
        llm_preferences,
        choices.into_iter().collect(),
    );
    let query_text = query_text.trim().to_lowercase();
    let mut results = choices
        .into_iter()
        .filter_map(|llm| {
            let name_match_result = if query_text.is_empty() {
                None
            } else {
                let result = match_indices_case_insensitive(
                    llm.display_name.to_lowercase().as_str(),
                    query_text.as_str(),
                )?;
                if query_text.len() > 1 && result.score < 10 {
                    return None;
                }
                Some(result)
            };
            let disable_reason = effective_model_disable_reason(llm, team_context, app);
            Some(ModelPickerChoice {
                llm: llm.clone(),
                disable_reason,
                score: OrderedFloat(
                    name_match_result
                        .as_ref()
                        .map_or(f64::MIN, |result| result.score as f64),
                ),
                name_match_result,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by_key(|choice| (choice.priority_tier(), choice.score));
    results
}

pub(crate) fn query_model_picker_catalog_choices<'a>(
    llm_preferences: &LLMPreferences,
    choices: impl IntoIterator<Item = &'a LLMInfo>,
    query_text: &str,
) -> Vec<ModelPickerChoice> {
    let choices = ModelSelectorDataSource::order_model_choices(
        llm_preferences,
        choices.into_iter().collect(),
    );
    let query_text = query_text.trim().to_lowercase();
    let mut results = choices
        .into_iter()
        .filter_map(|llm| {
            let name_match_result = if query_text.is_empty() {
                None
            } else {
                let result = match_indices_case_insensitive(
                    llm.display_name.to_lowercase().as_str(),
                    query_text.as_str(),
                )?;
                if query_text.len() > 1 && result.score < 10 {
                    return None;
                }
                Some(result)
            };
            Some(ModelPickerChoice {
                llm: llm.clone(),
                disable_reason: llm.disable_reason.clone(),
                score: OrderedFloat(
                    name_match_result
                        .as_ref()
                        .map_or(f64::MIN, |result| result.score as f64),
                ),
                name_match_result,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by_key(|choice| (choice.priority_tier(), choice.score));
    results
}

pub struct ModelSelectorDataSource {
    terminal_view_id: EntityId,
    owner_view: WeakViewHandle<InlineModelSelectorView>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
}

impl ModelSelectorDataSource {
    pub fn new(
        terminal_view_id: EntityId,
        owner_view: WeakViewHandle<InlineModelSelectorView>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    ) -> Self {
        Self {
            terminal_view_id,
            owner_view,
            ambient_agent_view_model,
        }
    }

    /// Attaches an ambient agent view model after construction so the picker treats this pane as a
    /// cloud pane, which changes the listed models (custom-endpoint models are suppressed; see
    /// [`Self::include_model_in_picker`]). Used on the shared-session viewer path where the model
    /// is created lazily at `SessionJoined`. Idempotent: a no-op when a model is already set. The
    /// next `run_query` (menu open / typing) picks up the new value.
    pub fn set_ambient_agent_view_model(
        &mut self,
        ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ambient_agent_view_model.is_some() {
            return;
        }
        self.ambient_agent_view_model = Some(ambient_agent_view_model);
        ctx.notify();
    }

    /// Returns whether a model should appear in the inline picker.
    /// Custom-endpoint models are suppressed in Oz cloud agent panes because
    /// they cannot route through Warp's cloud inference infrastructure.
    pub(crate) fn include_model_in_picker(is_cloud_pane: bool, is_custom_endpoint: bool) -> bool {
        !is_cloud_pane || !is_custom_endpoint
    }

    fn order_model_choices<'a>(
        llm_preferences: &LLMPreferences,
        choices: Vec<&'a LLMInfo>,
    ) -> Vec<&'a LLMInfo> {
        let mut auto_choices = Vec::new();
        let mut custom_router_choices = Vec::new();
        let mut custom_choices = Vec::new();
        let mut other_choices = Vec::new();

        for llm in choices {
            // Check custom router before is_auto because custom router ids contain
            // "auto" and would otherwise land in auto_choices.
            if is_custom_router_id(llm.id.as_str()) {
                custom_router_choices.push(llm);
            } else if is_auto(llm) {
                auto_choices.push(llm);
            } else if llm_preferences.custom_llm_info_for_id(&llm.id).is_some() {
                custom_choices.push(llm);
            } else {
                other_choices.push(llm);
            }
        }

        auto_choices
            .into_iter()
            .chain(custom_router_choices)
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

        let is_cloud_pane = self.ambient_agent_view_model.is_some();
        let choices = if is_full_terminal {
            llm_preferences
                .get_cli_agent_llm_choices_catalog()
                .filter(|llm| {
                    let is_custom = llm_preferences.custom_llm_info_for_id(&llm.id).is_some();
                    Self::include_model_in_picker(is_cloud_pane, is_custom)
                })
                .collect_vec()
        } else {
            llm_preferences
                .get_base_llm_choices_for_agent_mode_catalog()
                .filter(|llm| {
                    let is_custom = llm_preferences.custom_llm_info_for_id(&llm.id).is_some();
                    Self::include_model_in_picker(is_cloud_pane, is_custom)
                })
                .collect_vec()
        };
        Ok(
            query_model_picker_catalog_choices(llm_preferences, choices, &query.text)
                .into_iter()
                .map(|choice| {
                    QueryResult::from(ModelSearchItem::new(
                        choice,
                        self.terminal_view_id,
                        self.owner_view.clone(),
                        is_full_terminal,
                        is_cloud_pane,
                    ))
                })
                .collect(),
        )
    }
}

impl Entity for ModelSelectorDataSource {
    type Event = ();
}

#[derive(Clone)]
struct ModelSearchItem {
    id: LLMId,
    terminal_view_id: EntityId,
    owner_view: WeakViewHandle<InlineModelSelectorView>,
    is_full_terminal: bool,
    is_cloud_pane: bool,
    catalog_disable_reason: Option<DisableReason>,
    name_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
    manage_api_key_mouse_state: MouseStateHandle,
}

impl ModelSearchItem {
    fn new(
        choice: ModelPickerChoice,
        terminal_view_id: EntityId,
        owner_view: WeakViewHandle<InlineModelSelectorView>,
        is_full_terminal: bool,
        is_cloud_pane: bool,
    ) -> Self {
        Self {
            id: choice.llm.id,
            terminal_view_id,
            owner_view,
            is_full_terminal,
            is_cloud_pane,
            catalog_disable_reason: choice.disable_reason,
            name_match_result: choice.name_match_result,
            score: choice.score,
            manage_api_key_mouse_state: Default::default(),
        }
    }

    fn presentation(&self, app: &AppContext) -> Option<ModelSearchPresentation> {
        self.owner_view.upgrade(app)?;
        let workspaces = UserWorkspaces::as_ref(app);
        let team_render_context = workspaces.team_context(&self.owner_view, app);
        let preferences = LLMPreferences::as_ref(app);
        let llm = preferences.get_llm_info(&self.id)?.clone();
        let is_custom_endpoint = preferences.custom_llm_info_for_id(&llm.id).is_some();
        let is_visible = ModelSelectorDataSource::include_model_in_picker(
            self.is_cloud_pane,
            is_custom_endpoint,
        ) && if self.is_full_terminal {
            preferences
                .get_cli_agent_llm_choices_for_render_context(team_render_context.as_ref(), app)
                .any(|choice| choice.id == llm.id)
        } else {
            preferences
                .get_base_llm_choices_for_agent_mode_for_render_context(
                    team_render_context.as_ref(),
                    app,
                )
                .any(|choice| choice.id == llm.id)
        };
        let active_id = if self.is_full_terminal {
            &preferences
                .get_active_cli_agent_model_for_render_context(
                    Some(self.terminal_view_id),
                    team_render_context.as_ref(),
                    app,
                )
                .id
        } else {
            &preferences
                .get_active_base_model_for_render_context(
                    Some(self.terminal_view_id),
                    team_render_context.as_ref(),
                    app,
                )
                .id
        };
        let is_using_bedrock = should_show_bedrock_icon_for_model_for_render_context(
            &llm,
            team_render_context.as_ref(),
            app,
        );
        let is_using_gemini_enterprise_agent_platform =
            should_show_gemini_enterprise_agent_platform_icon_for_model_for_render_context(
                &llm,
                team_render_context.as_ref(),
                app,
            );
        let byo_key_source =
            byo_key_source_for_model_for_render_context(&llm, team_render_context.as_ref(), app);
        let is_custom_router = is_custom_router_id(llm.id.as_str());
        let is_auto = is_auto(&llm);
        let leading_icon = model_leading_icon(
            &llm,
            ModelIconFlags {
                is_custom_router,
                is_auto,
                is_using_bedrock,
                is_using_gemini_enterprise: is_using_gemini_enterprise_agent_platform,
            },
        );
        let is_using_cloud_host = is_using_bedrock || is_using_gemini_enterprise_agent_platform;
        let credential_icon =
            (!is_using_cloud_host && byo_key_source.is_some()).then_some(Icon::Key);
        let disable_reason = effective_model_disable_reason_for_render_context(
            &llm,
            team_render_context.as_ref(),
            app,
        );
        let user_id = AuthStateProvider::as_ref(app)
            .get()
            .user_id()
            .unwrap_or_default();
        let upgrade_url =
            UserWorkspaces::upgrade_link_for_render_context(team_render_context.as_ref(), user_id);
        Some(ModelSearchPresentation {
            is_visible,
            is_selected: active_id == &llm.id,
            llm,
            leading_icon,
            credential_icon,
            byo_key_source,
            disable_reason,
            is_custom_router,
            is_auto,
            is_using_bedrock,
            is_using_gemini_enterprise_agent_platform,
            upgrade_url,
        })
    }
}

struct ModelSearchPresentation {
    is_visible: bool,
    is_selected: bool,
    llm: LLMInfo,
    leading_icon: Icon,
    credential_icon: Option<Icon>,
    byo_key_source: Option<ByoKeySource>,
    disable_reason: Option<DisableReason>,
    is_custom_router: bool,
    is_auto: bool,
    is_using_bedrock: bool,
    is_using_gemini_enterprise_agent_platform: bool,
    upgrade_url: String,
}

impl SearchItem for ModelSearchItem {
    type Action = AcceptModel;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &crate::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(presentation) = self.presentation(app) else {
            return Empty::new().finish();
        };
        let icon_size = inline_styles::font_size(appearance);
        let icon_color = inline_styles::icon_color(appearance);
        let icon = presentation
            .leading_icon
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
        let Some(presentation) = self.presentation(app) else {
            return Empty::new().finish();
        };

        let appearance = crate::appearance::Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);
        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_text_color =
            inline_styles::secondary_text_color(theme, background_color.into());

        let name_text_color = if presentation.disable_reason.is_some() {
            secondary_text_color
        } else {
            primary_text_color
        };

        let mut text = Text::new_inline(
            presentation.llm.display_name.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(name_text_color.into())
        .with_clip(ClipConfig::ellipsis());

        if let Some(name_match) = &self.name_match_result
            && !name_match.matched_indices.is_empty()
        {
            text = text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                name_match.matched_indices.clone(),
            );
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(text.finish());
        if let Some(icon) = presentation.credential_icon {
            let credential_icon =
                ConstrainedBox::new(icon.to_warpui_icon(secondary_text_color).finish())
                    .with_width(font_size)
                    .with_height(font_size)
                    .finish();
            row = row.with_child(
                Container::new(credential_icon)
                    .with_margin_left(6.)
                    .finish(),
            );
        }

        if presentation.is_selected {
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

        if presentation.disable_reason.is_some() {
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
            presentation.llm.discount_percentage,
            presentation.credential_icon.is_some()
                || presentation.is_using_bedrock
                || presentation.is_using_gemini_enterprise_agent_platform,
        ) {
            let discount_percentage = presentation.llm.discount_percentage.unwrap_or(0.);
            let chip = Container::new(
                Text::new_inline(
                    format!("{}% off", discount_percentage.round() as u32),
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
        let presentation = self.presentation(app)?;

        let appearance = crate::appearance::Appearance::as_ref(app);
        let theme = appearance.theme();
        if presentation.is_custom_router {
            let header = render_model_spec_header(
                CUSTOM_MODEL_ROUTER_TITLE,
                CUSTOM_MODEL_ROUTER_DESCRIPTION,
                app,
            );
            let source_text = Text::new(
                presentation
                    .llm
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_string(),
                appearance.ui_font_family(),
                inline_styles::font_size(appearance),
            )
            .with_color(theme.disabled_ui_text_color().into())
            .finish();
            let column = Flex::column()
                .with_child(Container::new(header).with_margin_bottom(12.).finish())
                .with_child(source_text)
                .finish();
            return Some(
                ConstrainedBox::new(column)
                    .with_width(model_specs_width(app))
                    .finish(),
            );
        }

        let (title, description) = if presentation.llm.reasoning_level().is_some() {
            (REASONING_LEVEL_TITLE, REASONING_LEVEL_DESCRIPTION)
        } else {
            (MODEL_SPECS_TITLE, MODEL_SPECS_DESCRIPTION)
        };
        let header = render_model_spec_header(title, description, app);

        let uses_external_inference = presentation.is_using_bedrock
            || presentation.is_using_gemini_enterprise_agent_platform
            || presentation.byo_key_source.is_some();
        let cost_row = if uses_external_inference {
            let search_query = if presentation.is_using_bedrock {
                "bedrock"
            } else if presentation.is_using_gemini_enterprise_agent_platform {
                "gemini enterprise"
            } else {
                "api"
            }
            .to_string();
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
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                        search_query: search_query.clone(),
                        section: Some(SettingsSection::WarpAgent),
                    });
                })
                .finish();
            CostRow::BilledToProvider {
                label: if presentation.is_auto
                    && (presentation.is_using_bedrock
                        || presentation.is_using_gemini_enterprise_agent_platform)
                {
                    AUTO_HOSTED_INFERENCE_LABEL
                } else if presentation.is_using_bedrock {
                    "Inference via Bedrock"
                } else if presentation.is_using_gemini_enterprise_agent_platform {
                    "Inference via Gemini Enterprise Agent Platform"
                } else if let Some(source) = presentation.byo_key_source {
                    source.inference_label()
                } else {
                    "Inference via API key"
                },
                manage_button: Container::new(manage_button).finish(),
            }
        } else {
            CostRow::Bar {
                value: presentation.llm.spec.as_ref().map(|spec| spec.cost),
            }
        };

        let scores = render_model_spec_scores(
            presentation.llm.spec.as_ref(),
            cost_row,
            ModelSpecScoresLayout {
                bg_bar_color: internal_colors::neutral_3(theme),
            },
            app,
        );

        let mut column = Flex::column()
            .with_child(Container::new(header).with_margin_bottom(12.).finish())
            .with_child(scores);
        if presentation.disable_reason.as_ref() == Some(&DisableReason::RequiresUpgrade) {
            let mut display_name = presentation.llm.display_name.clone();
            if let Some(first) = display_name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            let byok_available = UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app)
                && matches!(
                    presentation.llm.provider,
                    LLMProvider::OpenAI | LLMProvider::Anthropic | LLMProvider::Google
                );

            let mut text_fragments = vec![
                FormattedTextFragment::plain_text(format!(
                    "{display_name} is not available for free users. "
                )),
                FormattedTextFragment::hyperlink("Upgrade", presentation.upgrade_url),
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
        if self.catalog_disable_reason.is_some() {
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
        self.catalog_disable_reason.is_some()
    }

    fn is_visible_for_context(&self, app: &AppContext) -> bool {
        self.presentation(app)
            .is_some_and(|presentation| presentation.is_visible)
    }

    fn is_disabled_for_context(&self, app: &AppContext) -> bool {
        self.presentation(app)
            .is_none_or(|presentation| presentation.disable_reason.is_some())
    }

    fn tooltip(&self) -> Option<String> {
        self.catalog_disable_reason
            .as_ref()
            .map(|reason| reason.tooltip_text().to_string())
    }
    fn tooltip_for_context(&self, app: &AppContext) -> Option<String> {
        self.presentation(app)
            .and_then(|presentation| presentation.disable_reason)
            .map(|reason| reason.tooltip_text().to_string())
    }

    fn accessibility_label(&self) -> String {
        format!("Model: {}", self.id)
    }

    fn accessibility_label_for_context(&self, app: &AppContext) -> String {
        let Some(presentation) = self.presentation(app) else {
            return self.accessibility_label();
        };
        let mut label = format!("Model: {}", presentation.llm.display_name);
        if presentation.is_selected {
            label.push_str(" (selected)");
        }
        if presentation.disable_reason.is_some() {
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
