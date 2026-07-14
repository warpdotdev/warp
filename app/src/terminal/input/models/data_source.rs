use std::collections::HashSet;

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use indexmap::IndexMap;
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use ordered_float::OrderedFloat;
use parking_lot::Mutex;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, FormattedTextElement, Highlight, HighlightedHyperlink,
    Hoverable, MouseStateHandle, Radius, Text,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::keymap::Keystroke;
use warpui::platform::{Cursor, OperatingSystem};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _,
    WeakModelHandle,
};

use super::model_spec_scores::{
    render_model_spec_header, render_model_spec_scores, CostRow, CostRowTooltip,
    ModelSpecScoresLayout, CUSTOM_MODEL_ROUTER_DESCRIPTION, CUSTOM_MODEL_ROUTER_TITLE,
    MODEL_SPECS_DESCRIPTION, MODEL_SPECS_TITLE, REASONING_LEVEL_DESCRIPTION, REASONING_LEVEL_TITLE,
};
use crate::ai::custom_model_routers::is_custom_router_id;
use crate::ai::execution_profiles::model_menu_items::is_auto;
use crate::ai::llms::{
    byo_key_source_for_model, should_show_bedrock_icon_for_model, should_show_key_icon_for_model,
    ByoKeySource, DisableReason, LLMId, LLMInfo, LLMPreferences, LLMProvider, LLMSpec,
};
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::{SearchItem, SyncDataSource};
use crate::settings_view::SettingsSection;
use crate::terminal::input::inline_menu::{
    default_navigation_message_items, styles as inline_styles, DetailsRenderConfig,
    InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction, InlineMenuType,
};
use crate::terminal::input::message_bar::{Message, MessageItem};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;

const AUTO_BEDROCK_TOOLTIP: &str = "Warp uses Bedrock when the model Auto selects supports it; otherwise it may use Warp-hosted inference.";

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

/// A single reasoning-level variant within a collapsed model group.
///
/// `level` is the display label (e.g. `"low"`, `"medium"`); `id` is the concrete
/// `LLMId` that selecting this level resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReasoningVariant {
    pub(super) level: String,
    pub(super) id: LLMId,
}

/// A collapsed group of model choices, mirroring the dropdown
/// `ProfileModelSelector` collapse logic.
///
/// Reasoning variants sharing a `base_model_name` collapse into one group keyed
/// by that base name; all `auto` models collapse under the `"auto"` key; every
/// other model (non-reasoning, custom-router, custom-endpoint) forms its own
/// single-member group keyed by its id. The group carries enough data for the
/// data source to build one `ModelSearchItem` (the representative variant) and
/// for the view to drive the reasoning sidecar (the ordered variants, the
/// active variant, and the variant that accept-on-row resolves to).
#[derive(Clone, Debug)]
pub(super) struct ModelGroup {
    /// Collapse key: `"auto"`, `base_model_name`, or the model `id`.
    #[allow(dead_code)]
    key: String,
    /// Row label shown in the results list.
    base_name: String,
    /// Text the fuzzy search matches against (the collapsed label for reasoning
    /// and auto groups, the per-model display name otherwise).
    search_label: String,
    /// The representative variant's id (the active variant when the family is
    /// selected, otherwise the deterministic default).
    representative_id: LLMId,
    /// Reasoning variants in server order. Empty for non-reasoning and auto groups.
    pub(super) variants: Vec<ReasoningVariant>,
    /// Index in `variants` of the currently-active variant, if the active model
    /// is one of this group's variants.
    active_variant_index: Option<usize>,
    /// Index in `variants` that accepting the collapsed row resolves to (the
    /// active variant when the family is selected, otherwise the first level).
    pub(super) target_variant_index: usize,
    /// True when the group renders a selectable reasoning sidecar (a reasoning
    /// family with more than one variant).
    pub(super) has_reasoning_sidecar: bool,
    /// True when the active model is a member of this group.
    is_active: bool,
    /// True for the collapsed `auto` group.
    is_auto: bool,
}

impl ModelGroup {
    /// The concrete `LLMId` that accepting this collapsed row resolves to.
    fn target_id(&self) -> LLMId {
        self.variants
            .get(self.target_variant_index)
            .map(|variant| variant.id.clone())
            .unwrap_or_else(|| self.representative_id.clone())
    }
}

/// Collapse a flat list of model choices into one [`ModelGroup`] per base model,
/// mirroring the dropdown `ProfileModelSelector::refresh_model_menu` keying:
/// `"auto"` for auto models, `base_model_name()` for reasoning families, and the
/// model `id` for everything else.
///
/// `custom_endpoint_ids` lists custom-endpoint model ids so they are never
/// grouped by base name (they render one row each, as they do today). The helper
/// is pure and free of `AppContext` so it is unit-testable with `LLMInfo` fixtures.
fn collapse_reasoning_variants(
    choices: &[&LLMInfo],
    active_llm_id: &LLMId,
    custom_endpoint_ids: &HashSet<LLMId>,
) -> Vec<ModelGroup> {
    // Group by collapse key, preserving the server-provided order (the choices
    // are already ordered by `order_model_choices`).
    let mut groups: IndexMap<String, Vec<&LLMInfo>> = IndexMap::new();
    for llm in choices {
        let key = if is_custom_router_id(llm.id.as_str()) || custom_endpoint_ids.contains(&llm.id) {
            llm.id.to_string()
        } else if is_auto(llm) {
            "auto".to_string()
        } else if llm.has_reasoning_level() {
            llm.base_model_name().to_string()
        } else {
            llm.id.to_string()
        };
        groups.entry(key).or_default().push(*llm);
    }

    groups
        .into_iter()
        .map(|(key, llms)| {
            let representative = llms.first().expect("group has at least one member");
            let is_auto_group =
                is_auto(representative) && !is_custom_router_id(representative.id.as_str());
            let is_custom = is_custom_router_id(representative.id.as_str())
                || custom_endpoint_ids.contains(&representative.id);
            let is_reasoning_family =
                !is_auto_group && !is_custom && representative.has_reasoning_level();

            if is_auto_group {
                let active_member = llms
                    .iter()
                    .find(|llm| &llm.id == active_llm_id)
                    .unwrap_or(representative);
                ModelGroup {
                    key,
                    base_name: "auto".to_string(),
                    search_label: "auto".to_string(),
                    representative_id: active_member.id.clone(),
                    variants: Vec::new(),
                    active_variant_index: None,
                    target_variant_index: 0,
                    has_reasoning_sidecar: false,
                    is_active: llms.iter().any(|llm| &llm.id == active_llm_id),
                    is_auto: true,
                }
            } else if is_reasoning_family {
                let variants: Vec<ReasoningVariant> = llms
                    .iter()
                    .map(|llm| ReasoningVariant {
                        level: llm.reasoning_level().unwrap_or_default(),
                        id: llm.id.clone(),
                    })
                    .collect();
                let active_variant_index = variants
                    .iter()
                    .position(|variant| &variant.id == active_llm_id);
                // Accept-on-row resolves to the active variant when the family is
                // selected, otherwise the first listed level (a deterministic
                // default; there is no per-family server default level today).
                let target_variant_index = active_variant_index.unwrap_or(0);
                ModelGroup {
                    key,
                    base_name: representative.base_model_name().to_string(),
                    search_label: representative.base_model_name().to_string(),
                    representative_id: variants
                        .get(target_variant_index)
                        .map(|variant| variant.id.clone())
                        .unwrap_or_else(|| representative.id.clone()),
                    variants,
                    active_variant_index,
                    target_variant_index,
                    has_reasoning_sidecar: false,
                    is_active: active_variant_index.is_some(),
                    is_auto: false,
                }
            } else {
                // Non-reasoning, custom-router, or custom-endpoint model: one row each.
                ModelGroup {
                    key,
                    base_name: representative.display_name.clone(),
                    search_label: representative.display_name.clone(),
                    representative_id: representative.id.clone(),
                    variants: Vec::new(),
                    active_variant_index: None,
                    target_variant_index: 0,
                    has_reasoning_sidecar: false,
                    is_active: &representative.id == active_llm_id,
                    is_auto: false,
                }
            }
        })
        .map(|mut group| {
            // The sidecar is only useful when there is more than one level to choose.
            group.has_reasoning_sidecar = group.variants.len() > 1;
            group
        })
        .collect()
}

pub struct ModelSelectorDataSource {
    terminal_view_id: EntityId,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    /// Weak handle to this data source, baked into each collapsed reasoning
    /// `ModelSearchItem` so `render_details` can read the live reasoning-sidecar
    /// state (focus + highlighted level) without rebuilding the results. Set
    /// once after construction via [`Self::set_self_weak`].
    self_weak: Option<WeakModelHandle<ModelSelectorDataSource>>,
    /// Whether the reasoning sidecar currently has keyboard focus (vs. the model
    /// list). Owned here so `run_query` can bake it into items for rendering.
    sidecar_focused: bool,
    /// The keyboard-highlighted reasoning level within the focused item's
    /// variants. Read live by the sidecar renderer via `self_weak`.
    sidecar_highlighted_level: usize,
    /// The groups produced by the last `run_query`, cached so the view can look
    /// up the selected item's sidecar variants and target level without
    /// recomputing the collapse. Behind a `Mutex` because `SyncDataSource::run_query`
    /// takes `&self`.
    last_groups: Mutex<Vec<ModelGroup>>,
}

impl ModelSelectorDataSource {
    pub fn new(
        terminal_view_id: EntityId,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    ) -> Self {
        Self {
            terminal_view_id,
            ambient_agent_view_model,
            self_weak: None,
            sidecar_focused: false,
            sidecar_highlighted_level: 0,
            last_groups: Mutex::new(Vec::new()),
        }
    }

    /// Set the weak self-handle used to read live sidecar state in `render_details`.
    /// Called once after the model is created.
    pub(crate) fn set_self_weak(&mut self, weak: WeakModelHandle<ModelSelectorDataSource>) {
        self.self_weak = Some(weak);
    }

    pub(crate) fn set_sidecar_focused(&mut self, focused: bool) {
        self.sidecar_focused = focused;
    }

    pub(crate) fn set_sidecar_highlighted_level(&mut self, level: usize) {
        self.sidecar_highlighted_level = level;
    }

    pub(crate) fn sidecar_focused(&self) -> bool {
        self.sidecar_focused
    }

    pub(crate) fn sidecar_highlighted_level(&self) -> usize {
        self.sidecar_highlighted_level
    }

    /// Returns the cached group whose collapsed row resolves to `target_id`
    /// (i.e. the group that the selected item represents), or `None` when the
    /// selection is stale relative to the last query.
    pub(super) fn group_for_target(&self, target_id: &LLMId) -> Option<ModelGroup> {
        self.last_groups
            .lock()
            .iter()
            .find(|group| &group.target_id() == target_id)
            .cloned()
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

        let is_cloud_pane = self.ambient_agent_view_model.is_some();
        let choices = if is_full_terminal {
            llm_preferences
                .get_cli_agent_llm_choices(app)
                .filter(|llm| {
                    let is_custom = llm_preferences.custom_llm_info_for_id(&llm.id).is_some();
                    Self::include_model_in_picker(is_cloud_pane, is_custom)
                })
                .collect_vec()
        } else {
            llm_preferences
                .get_base_llm_choices_for_agent_mode(app)
                .filter(|llm| {
                    let is_custom = llm_preferences.custom_llm_info_for_id(&llm.id).is_some();
                    Self::include_model_in_picker(is_cloud_pane, is_custom)
                })
                .collect_vec()
        };
        let choices = Self::order_model_choices(llm_preferences, choices);

        // Collapse per-reasoning-level variants into one entry per base model,
        // mirroring the dropdown `ProfileModelSelector`. Custom-endpoint models
        // are excluded from base-name grouping so they keep rendering one row each.
        let custom_endpoint_ids: HashSet<LLMId> = llm_preferences
            .custom_llm_choices(app)
            .map(|info| info.id.clone())
            .collect();
        let groups = collapse_reasoning_variants(&choices, &active_llm_id, &custom_endpoint_ids);
        // Cache the groups so the view can look up the selected item's sidecar
        // variants and target level without recomputing the collapse.
        *self.last_groups.lock() = groups.clone();

        let self_weak = self.self_weak.clone();
        let build_item = |group: &ModelGroup| {
            let representative = choices
                .iter()
                .find(|llm| llm.id == group.representative_id)
                .copied()
                .unwrap_or_else(|| {
                    choices
                        .first()
                        .copied()
                        .expect("choices is non-empty when groups are non-empty")
                });
            ModelSearchItem::new_from_group(
                group,
                representative,
                &active_llm_id,
                self_weak.clone(),
                app,
            )
        };

        let query_text = query.text.trim().to_lowercase();

        if query_text.is_empty() {
            return Ok(groups
                .iter()
                .map(build_item)
                .map(QueryResult::from)
                .collect());
        }

        Ok(groups
            .iter()
            .filter_map(|group| {
                // Match collapsed reasoning/auto groups against their base label
                // (e.g. `terra` matches the single `gpt-5.6-terra` row, not N
                // per-level rows); non-collapsed items still match on their display name.
                let match_result = match_indices_case_insensitive(
                    group.search_label.to_lowercase().as_str(),
                    query_text.as_str(),
                )?;

                // Avoid spamming results with extremely weak matches.
                if query_text.len() > 1 && match_result.score < 10 {
                    return None;
                }

                Some(QueryResult::from(
                    build_item(group)
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
    /// The concrete `LLMId` that accepting this row resolves to: the targeted
    /// reasoning variant for a collapsed reasoning family, otherwise the model id.
    id: LLMId,
    provider: LLMProvider,
    spec: Option<LLMSpec>,
    leading_icon: Icon,
    credential_icon: Option<Icon>,
    byo_key_source: Option<ByoKeySource>,
    display_text: String,
    is_selected: bool,
    is_custom_router: bool,
    /// Source/routing description for custom model routers (from `LLMInfo.description`).
    description: Option<String>,
    disable_reason: Option<DisableReason>,
    is_auto: bool,
    is_using_bedrock: bool,
    name_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
    manage_api_key_mouse_state: MouseStateHandle,
    cost_row_tooltip_mouse_state: MouseStateHandle,
    reasoning_level: Option<String>,
    discount_percentage: Option<f32>,
    /// Reasoning-level variants exposed in the sidecar (server order). Empty for
    /// non-reasoning and auto groups, and for single-variant reasoning groups.
    variants: Vec<ReasoningVariant>,
    /// Index in `variants` of the currently-active variant (matches the active
    /// model), if any. Used to render the sidecar checkmark.
    active_variant_index: Option<usize>,
    /// Index in `variants` that accepting the collapsed row (without entering the
    /// sidecar) resolves to. `accept_result` returns this variant's id.
    target_variant_index: usize,
    /// Whether this row renders a selectable reasoning sidecar.
    has_reasoning_sidecar: bool,
    /// Weak handle to the data source, used by `render_details` to read the live
    /// sidecar focus + highlighted level. `None` in unit tests.
    data_source_weak: Option<WeakModelHandle<ModelSelectorDataSource>>,
    /// One stable mouse-state handle per sidecar row, so hover state persists
    /// across re-renders. Created at construction (one per variant).
    sidecar_row_mouse_states: Vec<MouseStateHandle>,
}

impl ModelSearchItem {
    /// Build a `ModelSearchItem` from a collapsed [`ModelGroup`] and its
    /// representative `LLMInfo`. The representative supplies the provider, spec,
    /// icons, and credential/disable metadata; the group supplies the collapsed
    /// label, the reasoning variants, and the target variant that accept resolves to.
    fn new_from_group(
        group: &ModelGroup,
        representative: &LLMInfo,
        _active_llm_id: &LLMId,
        data_source_weak: Option<WeakModelHandle<ModelSelectorDataSource>>,
        app: &AppContext,
    ) -> Self {
        // If the model requires an upgrade but the user already has a BYOK key
        // for this provider, treat it as enabled by clearing the disable reason.
        let disable_reason = if representative.disable_reason
            == Some(DisableReason::RequiresUpgrade)
            && should_show_key_icon_for_model(representative, app)
        {
            None
        } else {
            representative.disable_reason.clone()
        };
        let is_custom_router = is_custom_router_id(representative.id.as_str());
        let is_auto = group.is_auto;
        let is_using_bedrock = should_show_bedrock_icon_for_model(representative, app);
        let byo_key_source = byo_key_source_for_model(representative, app);
        let leading_icon = if is_using_bedrock {
            Icon::Aws
        } else if is_custom_router {
            Icon::Dataflow
        } else {
            representative.provider.icon().unwrap_or(Icon::Oz)
        };
        let credential_icon = if !is_using_bedrock && byo_key_source.is_some() {
            Some(Icon::Key)
        } else {
            None
        };
        let sidecar_row_mouse_states = group
            .variants
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();
        Self {
            id: group.target_id(),
            provider: representative.provider.clone(),
            spec: representative.spec.clone(),
            leading_icon,
            credential_icon,
            byo_key_source,
            display_text: group.base_name.clone(),
            is_selected: group.is_active,
            is_custom_router,
            description: representative.description.clone(),
            disable_reason,
            is_auto,
            is_using_bedrock,
            name_match_result: None,
            score: OrderedFloat(f64::MIN),
            manage_api_key_mouse_state: Default::default(),
            cost_row_tooltip_mouse_state: Default::default(),
            reasoning_level: representative.reasoning_level(),
            discount_percentage: representative.discount_percentage,
            variants: group.variants.clone(),
            active_variant_index: group.active_variant_index,
            target_variant_index: group.target_variant_index,
            has_reasoning_sidecar: group.has_reasoning_sidecar,
            data_source_weak,
            sidecar_row_mouse_states,
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

    /// Reads the live reasoning-sidecar state (focus + highlighted level) from the
    /// data source via the weak self-handle baked in at query time. Falls back to
    /// (not focused, the targeted level) when the handle is unavailable (e.g. in
    /// unit tests), so the sidecar still renders with the active variant checked.
    fn live_sidecar_state(&self, app: &AppContext) -> (bool, usize) {
        let Some(weak) = self.data_source_weak.as_ref() else {
            return (false, self.target_variant_index);
        };
        let Some(handle) = weak.upgrade(app) else {
            return (false, self.target_variant_index);
        };
        let data_source = handle.as_ref(app);
        (
            data_source.sidecar_focused(),
            data_source.sidecar_highlighted_level(),
        )
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

        let icon = self.leading_icon.to_warpui_icon(icon_color).finish();

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
        if let Some(icon) = self.credential_icon {
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
            self.credential_icon.is_some() || self.is_using_bedrock,
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

        // Custom auto models get an informational blurb instead of spec bars.
        if self.is_custom_router {
            let header = render_model_spec_header(
                CUSTOM_MODEL_ROUTER_TITLE,
                CUSTOM_MODEL_ROUTER_DESCRIPTION,
                app,
            );
            let source_text = Text::new(
                self.description.as_deref().unwrap_or("").to_string(),
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

        // Collapsed reasoning family: render a selectable reasoning-level sidecar
        // (one row per level, the active variant checked, the keyboard-highlighted
        // level emphasized when the sidecar has focus) instead of the read-only spec panel.
        if self.has_reasoning_sidecar {
            let (sidecar_focused, highlighted_level) = self.live_sidecar_state(app);
            return Some(render_reasoning_sidecar(
                &self.variants,
                self.active_variant_index,
                sidecar_focused,
                highlighted_level,
                &self.sidecar_row_mouse_states,
                app,
            ));
        }

        let (title, description) = if self.reasoning_level.is_some() {
            (REASONING_LEVEL_TITLE, REASONING_LEVEL_DESCRIPTION)
        } else {
            (MODEL_SPECS_TITLE, MODEL_SPECS_DESCRIPTION)
        };
        let header = render_model_spec_header(title, description, app);

        let cost_row = if self.is_using_bedrock || self.byo_key_source.is_some() {
            let search_query = if self.is_using_bedrock {
                "bedrock"
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
                label: if self.is_using_bedrock && self.is_auto {
                    "Inference may use Bedrock"
                } else if self.is_using_bedrock {
                    "Inference via Bedrock"
                } else if let Some(source) = self.byo_key_source {
                    source.inference_label()
                } else {
                    "Inference via API key"
                },
                tooltip: if self.is_using_bedrock && self.is_auto {
                    Some(CostRowTooltip {
                        text: AUTO_BEDROCK_TOOLTIP,
                        mouse_state: self.cost_row_tooltip_mouse_state.clone(),
                    })
                } else {
                    None
                },
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

/// Renders the selectable reasoning-level sidecar for a collapsed reasoning
/// family: a header followed by one row per level. The active variant is marked
/// with a checkmark; the keyboard-highlighted level (when the sidecar has focus)
/// and the mouse-hovered row get a highlight background. Clicking a row accepts
/// that variant's `LLMId` through the inline-menu accept pipeline.
#[allow(clippy::too_many_arguments)]
fn render_reasoning_sidecar(
    variants: &[ReasoningVariant],
    active_variant_index: Option<usize>,
    sidecar_focused: bool,
    highlighted_level: usize,
    row_mouse_states: &[MouseStateHandle],
    app: &AppContext,
) -> Box<dyn Element> {
    use warpui::elements::{DispatchEventResult, EventHandler, Flex, ParentElement as _};
    use warpui::prelude::{CrossAxisAlignment, Empty};

    let appearance = Appearance::as_ref(app);
    let font_size = inline_styles::font_size(appearance);
    let bg_color = inline_styles::menu_background_color(app);
    let primary_text_color = inline_styles::primary_text_color(appearance.theme(), bg_color.into());
    let checkmark_color = inline_styles::icon_color(appearance);
    let row_height = font_size + 8.;

    let header = render_model_spec_header(REASONING_LEVEL_TITLE, REASONING_LEVEL_DESCRIPTION, app);

    let mut column =
        Flex::column().with_child(Container::new(header).with_margin_bottom(12.).finish());

    for (idx, variant) in variants.iter().enumerate() {
        let is_active = active_variant_index == Some(idx);
        let is_highlighted = sidecar_focused && idx == highlighted_level;
        let mouse_state = row_mouse_states.get(idx).cloned().unwrap_or_default();

        let check_slot = if is_active {
            ConstrainedBox::new(Icon::Check.to_warpui_icon(checkmark_color).finish())
                .with_width(font_size)
                .with_height(font_size)
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(font_size)
                .with_height(font_size)
                .finish()
        };

        let row_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(check_slot)
                    .with_margin_right(inline_styles::ICON_MARGIN)
                    .finish(),
            )
            .with_child(
                Text::new_inline(
                    variant.level.clone(),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(primary_text_color.into())
                .finish(),
            )
            .finish();

        let row_hoverable = Hoverable::new(mouse_state, move |mouse_state| {
            let highlight = ItemHighlightState::new(is_highlighted, mouse_state);
            let background = inline_styles::item_background(highlight, appearance);
            let row = Container::new(row_content)
                .with_padding_left(inline_styles::ITEM_HORIZONTAL_PADDING)
                .with_padding_right(inline_styles::ITEM_HORIZONTAL_PADDING)
                .with_padding_top(4.)
                .with_padding_bottom(4.);
            if let Some(background) = background {
                row.with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        inline_styles::ITEM_CORNER_RADIUS,
                    )))
                    .finish()
            } else {
                row.finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let variant_id = variant.id.clone();
        let row_element = EventHandler::new(row_hoverable)
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .on_left_mouse_up(move |ctx, _, _| {
                ctx.dispatch_typed_action(InlineMenuRowAction::<AcceptModel>::Accept {
                    item: AcceptModel {
                        id: variant_id.clone(),
                    },
                    cmd_or_ctrl_enter: false,
                });
                DispatchEventResult::StopPropagation
            })
            .finish();

        column = column.with_child(
            ConstrainedBox::new(Container::new(row_element).with_margin_bottom(4.).finish())
                .with_height(row_height)
                .finish(),
        );
    }

    ConstrainedBox::new(column.finish())
        .with_width(model_specs_width(app))
        .finish()
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
