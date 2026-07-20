//! Shared orchestration controls reused by the `RunAgentsCardView`
//! confirmation card editor and the plan-card
//! `OrchestrationConfigBlockView`.
//!
//! The generic parameter `A` is the parent view's typed action — both
//! consumers impl [`OrchestrationControlAction`] to provide the mapping
//! from field-change events to their own action enum.

use ai::agent::action::RunAgentsExecutionMode;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    Point, Radius, Text,
};
use warpui::event::DispatchedEvent;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, View, ViewContext, ViewHandle,
};

use crate::LLMPreferences;
use crate::ai::blocklist::inline_action::host_picker::HostPicker;
use crate::ai::execution_profiles::model_menu_items::available_model_menu_items;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::harness_display;
use crate::ai::orchestration::{
    AUTH_SECRET_INHERIT_LABEL, OptionBadge, OptionFooter, OptionRow, OptionSnapshot,
    OptionSourceStatus, api_key_snapshot, build_runner_snapshot, environment_snapshot,
    harness_snapshot, host_snapshot, model_snapshot, persist_auth_secret_selection,
};
pub use crate::ai::orchestration::{
    AuthSecretSelection, ORCHESTRATION_WARP_WORKER_HOST, OrchestrationConfigState,
    OrchestrationEditState, accept_disabled_reason_with_auth, empty_env_recommendation_message,
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, should_show_auth_secret_picker,
};
use crate::appearance::Appearance;
use crate::menu::{MenuItem, MenuItemFields};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::FilterableDropdown;
use crate::view_components::dropdown::{
    Dropdown, DropdownAction, DropdownItemAction, DropdownStyle,
};

// ── Shared constants ────────────────────────────────────────────────

pub const ORCHESTRATION_PICKER_HEIGHT: f32 = 36.;
pub const ORCHESTRATION_PICKER_BORDER_WIDTH: f32 = 1.;
pub const ORCHESTRATION_PICKER_FONT_SIZE: f32 = 14.;
pub const ORCHESTRATION_PICKER_RADIUS: f32 = 4.;
pub const ORCHESTRATION_PICKER_MAX_WIDTH: f32 = 205.;

const ORCHESTRATION_SEGMENTED_CONTROL_PADDING: f32 = 4.;
const ORCHESTRATION_SEGMENT_VERTICAL_PADDING: f32 = 4.;

/// Label for the auth secret column.
pub const AUTH_SECRET_COLUMN_LABEL: &str = "API key";
const AUTH_SECRET_CREATE_NEW_LABEL: &str = "New API key…";

// ── Action trait ────────────────────────────────────────────────────

/// Trait that both `RunAgentsCardViewAction` and
/// `OrchestrationConfigBlockAction` implement so the shared picker
/// creation and render helpers can produce the correct action variant.
pub trait OrchestrationControlAction: DropdownItemAction + Clone {
    fn execution_mode_toggled(is_remote: bool) -> Self;
    fn model_changed(model_id: String) -> Self;
    fn harness_changed(harness_type: String) -> Self;
    fn environment_changed(environment_id: String) -> Self;
    fn create_environment_requested() -> Self;
    /// Runner UID selected in the Runner dropdown; empty clears the
    /// override ("Use environment default").
    fn runner_changed(runner_id: String) -> Self;
    /// `None` means Inherit; `Some(name)` means a named managed secret.
    fn auth_secret_changed(name: Option<String>) -> Self;
    /// User picked the "New API key…" item; opens the workspace create modal.
    fn create_new_auth_secret_requested() -> Self;
}

// ── Picker handles ──────────────────────────────────────────────────

/// Picker view handles shared between card editor and plan-card config
/// block. Generic over the action type `A`.
#[derive(Clone)]
pub struct OrchestrationPickerHandles<A: OrchestrationControlAction> {
    pub model_picker: Option<ViewHandle<FilterableDropdown<A>>>,
    pub harness_picker: Option<ViewHandle<Dropdown<A>>>,
    pub environment_picker: Option<ViewHandle<FilterableDropdown<A>>>,
    /// Runner picker for the Cloud variant (gated on `CloudAgentRunners`).
    /// `None` until built; runners are fetched via `FactoryClient::get_runners`.
    pub runner_picker: Option<ViewHandle<FilterableDropdown<A>>>,
    pub host_picker: Option<ViewHandle<HostPicker>>,
    /// Picker for the managed auth secret used by non-Oz cloud children.
    /// `None` when the picker hasn't been built yet (e.g. harness is Oz or
    /// execution mode is Local), or when the harness has no supported
    /// auth-secret types.
    pub auth_secret_picker: Option<ViewHandle<Dropdown<A>>>,
    pub local_toggle: MouseStateHandle,
    pub cloud_toggle: MouseStateHandle,
}

impl<A: OrchestrationControlAction> Default for OrchestrationPickerHandles<A> {
    fn default() -> Self {
        Self {
            model_picker: None,
            harness_picker: None,
            environment_picker: None,
            runner_picker: None,
            host_picker: None,
            auth_secret_picker: None,
            local_toggle: MouseStateHandle::default(),
            cloud_toggle: MouseStateHandle::default(),
        }
    }
}

// ── Picker styling ──────────────────────────────────────────────────

/// Constructs the shared `UiComponentStyles` for orchestration pickers.
pub fn picker_styles(appearance: &Appearance) -> (UiComponentStyles, PickerColors) {
    let theme = appearance.theme();
    let padding = Coords {
        top: 8.,
        bottom: 8.,
        left: 12.,
        right: 12.,
    };
    let corner_radius = CornerRadius::with_all(Radius::Pixels(ORCHESTRATION_PICKER_RADIUS));
    // The picker bg is a translucent overlay (surface_overlay_1 =
    // fg at 5%). It must stay translucent so that the accent-tinted
    // card background in the config block shows through, and so that
    // gradient-background themes render correctly.
    let background_fill: Fill = theme.surface_overlay_1();
    let background: warpui::elements::Fill = background_fill.into();
    // Border and font colors are intentionally left to the dropdown's
    // default ButtonVariant::Secondary styling, which uses
    // theme.outline() and theme.main_text_color() — both are
    // contrast-aware and adapt correctly to all themes.

    let styles = UiComponentStyles {
        height: Some(ORCHESTRATION_PICKER_HEIGHT),
        background: Some(background),
        border_width: Some(ORCHESTRATION_PICKER_BORDER_WIDTH),
        border_radius: Some(corner_radius),
        font_size: Some(ORCHESTRATION_PICKER_FONT_SIZE),
        padding: Some(padding),
        ..Default::default()
    };
    let colors = PickerColors {
        padding,
        corner_radius,
        background,
    };
    (styles, colors)
}

#[derive(Clone)]
pub struct PickerColors {
    pub padding: Coords,
    pub corner_radius: CornerRadius,
    pub background: warpui::elements::Fill,
}

// ── Picker creation (generic over action type) ──────────────────────

/// Creates a standard dropdown with the shared orchestration picker
/// chrome (border, radius, background, font).
pub fn new_standard_picker_dropdown<A: OrchestrationControlAction, V: View>(
    colors: &PickerColors,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<Dropdown<A>> {
    let padding = colors.padding;
    let corner_radius = colors.corner_radius;
    let background = colors.background;
    ctx.add_typed_action_view(move |ctx_dropdown| {
        let mut dropdown = Dropdown::<A>::new(ctx_dropdown);
        dropdown.set_use_overlay_layer(false, ctx_dropdown);
        dropdown.set_match_menu_width_to_top_bar(true, ctx_dropdown);
        dropdown.set_main_axis_size(MainAxisSize::Max, ctx_dropdown);
        dropdown.set_style(DropdownStyle::ActionButtonSecondary, ctx_dropdown);
        dropdown.set_top_bar_height(ORCHESTRATION_PICKER_HEIGHT, ctx_dropdown);
        dropdown.set_top_bar_max_width(f32::INFINITY);
        dropdown.set_padding(padding, ctx_dropdown);
        dropdown.set_border_radius(corner_radius, ctx_dropdown);
        dropdown.set_background(background, ctx_dropdown);
        dropdown.set_border_width(ORCHESTRATION_PICKER_BORDER_WIDTH, ctx_dropdown);
        dropdown.set_font_size(ORCHESTRATION_PICKER_FONT_SIZE, ctx_dropdown);
        dropdown
    })
}

/// Creates a searchable dropdown with the shared orchestration picker
/// chrome (border, radius, background, font).
pub fn new_standard_filterable_picker_dropdown<A: OrchestrationControlAction, V: View>(
    styles: &UiComponentStyles,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<FilterableDropdown<A>> {
    let styles = *styles;
    ctx.add_typed_action_view(move |ctx_dropdown| {
        let mut dropdown = FilterableDropdown::<A>::new(ctx_dropdown);
        dropdown.set_use_overlay_layer(false, ctx_dropdown);
        dropdown.set_match_menu_width_to_top_bar(true, ctx_dropdown);
        dropdown.set_main_axis_size(MainAxisSize::Max, ctx_dropdown);
        dropdown.set_button_variant(ButtonVariant::Secondary);
        dropdown.set_style(styles);
        dropdown.set_top_bar_height(ORCHESTRATION_PICKER_HEIGHT, ctx_dropdown);
        dropdown.set_top_bar_max_width(f32::INFINITY);
        dropdown
    })
}

/// Execution mode for the placeholder states the `populate_*` helpers
/// build for the snapshot builders: only the Local/Cloud distinction
/// matters to the builders, so the Remote fields are left empty.
fn snapshot_execution_mode(is_local: bool) -> RunAgentsExecutionMode {
    if is_local {
        RunAgentsExecutionMode::Local
    } else {
        RunAgentsExecutionMode::Remote {
            environment_id: String::new(),
            worker_host: String::new(),
            computer_use_enabled: false,
            runner_id: String::new(),
        }
    }
}

/// Label of the snapshot row matching `selected_id`, if any.
fn selected_row_label(snapshot: &OptionSnapshot) -> Option<String> {
    snapshot.selected_id.as_ref().and_then(|id| {
        snapshot
            .rows
            .iter()
            .find(|row| &row.id == id)
            .map(|row| row.label.clone())
    })
}

/// Rich menu items for Oz model rows. The snapshot owns inclusion,
/// ordering, and selection; this maps each row id back to its `LLMInfo`
/// and renders through [`available_model_menu_items`] so Oz rows keep
/// provider/credential icons and disabled gating — GUI rendering
/// concerns that cannot live in the frontend-neutral snapshot layer.
fn oz_model_menu_items<A: OrchestrationControlAction, V: View>(
    rows: &[OptionRow],
    ctx: &mut ViewContext<V>,
) -> Vec<MenuItem<DropdownAction>> {
    let llm_prefs = LLMPreferences::as_ref(ctx);
    let all_choices: Vec<_> = llm_prefs.get_base_llm_choices_for_agent_mode(ctx).collect();
    let ordered_choices: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            all_choices
                .iter()
                .copied()
                .find(|llm| llm.id.to_string() == row.id)
        })
        .collect();
    available_model_menu_items(
        ordered_choices,
        move |llm| DropdownAction::select_action_and_close(A::model_changed(llm.id.to_string())),
        None,
        None,
        false,
        false,
        ctx,
    )
}

/// Populates the model picker from [`model_snapshot`] for the active
/// harness (Warp LLM catalog for Oz, "Default model" for local Codex,
/// "Default model" plus the server-provided catalog otherwise).
pub fn populate_model_picker_for_harness<A: OrchestrationControlAction, V: View>(
    dropdown: &ViewHandle<FilterableDropdown<A>>,
    initial_model_id: &str,
    harness_type: &str,
    is_local: bool,
    ctx: &mut ViewContext<V>,
) {
    let state = OrchestrationConfigState::from_run_agents_fields(
        Some(initial_model_id),
        Some(harness_type),
        &snapshot_execution_mode(is_local),
    );
    let is_oz = matches!(
        Harness::parse_orchestration_harness(harness_type),
        Some(Harness::Oz) | None
    );
    dropdown.update(ctx, |dropdown, ctx_dropdown| {
        let snapshot = model_snapshot(&state, ctx_dropdown);
        let selected_label = selected_row_label(&snapshot);
        let items = if is_oz {
            oz_model_menu_items::<A, _>(&snapshot.rows, ctx_dropdown)
        } else {
            snapshot
                .rows
                .into_iter()
                .map(|row| {
                    MenuItem::Item(MenuItemFields::new(&row.label).with_on_select_action(
                        DropdownAction::select_action_and_close(A::model_changed(row.id)),
                    ))
                })
                .collect()
        };
        dropdown.set_rich_items(items, ctx_dropdown);
        if let Some(label) = &selected_label {
            dropdown.set_selected_by_name(label, ctx_dropdown);
        }
    });
}

/// Populates the harness picker from [`harness_snapshot`], mapping rows
/// to menu items (icon/brand color from the row's harness, disabled
/// reason to a disabled item with a tooltip).
pub fn populate_harness_picker<A: OrchestrationControlAction, V: View>(
    dropdown: &ViewHandle<Dropdown<A>>,
    initial_harness: &str,
    is_local: bool,
    ctx: &mut ViewContext<V>,
) {
    let state = OrchestrationConfigState::from_run_agents_fields(
        None,
        Some(initial_harness),
        &snapshot_execution_mode(is_local),
    );
    dropdown.update(ctx, |dropdown, ctx_dropdown| {
        let snapshot = harness_snapshot(&state, ctx_dropdown);
        let selected_label = selected_row_label(&snapshot);
        let items: Vec<MenuItem<DropdownAction>> = snapshot
            .rows
            .into_iter()
            .map(|row| {
                let mut fields = MenuItemFields::new(&row.label);
                if let Some(harness) = row.harness {
                    fields = fields.with_icon(harness_display::icon_for(harness));
                    if let Some(color) = harness_display::brand_color(harness) {
                        fields = fields.with_override_icon_color(Fill::from(color));
                    }
                }
                match row.disabled_reason {
                    Some(reason) => {
                        fields = fields.with_disabled(true).with_tooltip(reason);
                    }
                    None => {
                        fields = fields.with_on_select_action(
                            DropdownAction::select_action_and_close(A::harness_changed(row.id)),
                        );
                    }
                }
                MenuItem::Item(fields)
            })
            .collect();
        dropdown.set_rich_items(items, ctx_dropdown);
        if let Some(label) = &selected_label {
            dropdown.set_selected_by_name(label, ctx_dropdown);
        }
    });
}

pub fn create_environment_picker<A: OrchestrationControlAction, V: View>(
    initial_env_id: &str,
    styles: &UiComponentStyles,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<FilterableDropdown<A>> {
    let initial_env = initial_env_id.to_string();
    let styles = *styles;
    let footer_mouse_state = MouseStateHandle::default();
    let dropdown_handle = ctx.add_typed_action_view(move |ctx_dropdown| {
        let mut dropdown = FilterableDropdown::<A>::new(ctx_dropdown);
        dropdown.set_use_overlay_layer(false, ctx_dropdown);
        dropdown.set_match_menu_width_to_top_bar(true, ctx_dropdown);
        dropdown.set_main_axis_size(MainAxisSize::Max, ctx_dropdown);
        dropdown.set_button_variant(ButtonVariant::Secondary);
        dropdown.set_style(styles);
        dropdown.set_top_bar_height(ORCHESTRATION_PICKER_HEIGHT, ctx_dropdown);
        dropdown.set_top_bar_max_width(f32::INFINITY);
        dropdown
    });
    dropdown_handle.update(ctx, |dropdown, ctx_dropdown| {
        let footer_mouse_state = footer_mouse_state.clone();
        dropdown.set_footer(
            move |app| render_new_environment_footer::<A>(footer_mouse_state.clone(), app),
            ctx_dropdown,
        );
    });
    populate_environment_picker(&dropdown_handle, &initial_env, ctx);
    dropdown_handle
}

/// Populates the environment picker from [`environment_snapshot`]
/// ("Empty environment" plus existing environments sorted by name).
pub fn populate_environment_picker<A: OrchestrationControlAction, V: View>(
    dropdown_handle: &ViewHandle<FilterableDropdown<A>>,
    initial_env_id: &str,
    ctx: &mut ViewContext<V>,
) {
    let state = OrchestrationConfigState::from_run_agents_fields(
        None,
        None,
        &RunAgentsExecutionMode::Remote {
            environment_id: initial_env_id.to_string(),
            worker_host: String::new(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    );
    dropdown_handle.update(ctx, |dropdown, ctx_dropdown| {
        let snapshot = environment_snapshot(&state, ctx_dropdown);
        let selected_label = selected_row_label(&snapshot);
        let items = snapshot
            .rows
            .into_iter()
            .map(|row| {
                MenuItem::Item(MenuItemFields::new(&row.label).with_on_select_action(
                    DropdownAction::select_action_and_close(A::environment_changed(row.id)),
                ))
            })
            .collect();
        dropdown.set_rich_items(items, ctx_dropdown);
        if let Some(label) = &selected_label {
            dropdown.set_selected_by_name(label, ctx_dropdown);
        }
    });
}

/// Creates the Runner picker dropdown with the shared orchestration
/// chrome, then populates it from the supplied runners list. Runners are
/// not cached client-side (unlike environments), so the owning view
/// fetches them via `FactoryClient::get_runners` and passes them in;
/// `loading` renders the picker in its loading state until they arrive.
pub fn create_runner_picker<A: OrchestrationControlAction, V: View>(
    initial_runner_id: &str,
    runners: &[(String, String)],
    loading: bool,
    styles: &UiComponentStyles,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<FilterableDropdown<A>> {
    let styles = *styles;
    let dropdown_handle = ctx.add_typed_action_view(move |ctx_dropdown| {
        let mut dropdown = FilterableDropdown::<A>::new(ctx_dropdown);
        dropdown.set_use_overlay_layer(false, ctx_dropdown);
        dropdown.set_match_menu_width_to_top_bar(true, ctx_dropdown);
        dropdown.set_main_axis_size(MainAxisSize::Max, ctx_dropdown);
        dropdown.set_button_variant(ButtonVariant::Secondary);
        dropdown.set_style(styles);
        dropdown.set_top_bar_height(ORCHESTRATION_PICKER_HEIGHT, ctx_dropdown);
        dropdown.set_top_bar_max_width(f32::INFINITY);
        dropdown
    });
    populate_runner_picker(&dropdown_handle, runners, initial_runner_id, loading, ctx);
    dropdown_handle
}

/// Populates the runner picker from [`build_runner_snapshot`] ("Use
/// environment default" plus the supplied runners).
pub fn populate_runner_picker<A: OrchestrationControlAction, V: View>(
    dropdown_handle: &ViewHandle<FilterableDropdown<A>>,
    runners: &[(String, String)],
    current_runner_id: &str,
    loading: bool,
    ctx: &mut ViewContext<V>,
) {
    let snapshot = build_runner_snapshot(runners.to_vec(), current_runner_id, loading);
    let selected_label = selected_row_label(&snapshot);
    dropdown_handle.update(ctx, |dropdown, ctx_dropdown| {
        let items = snapshot
            .rows
            .into_iter()
            .map(|row| {
                MenuItem::Item(MenuItemFields::new(&row.label).with_on_select_action(
                    DropdownAction::select_action_and_close(A::runner_changed(row.id)),
                ))
            })
            .collect();
        dropdown.set_rich_items(items, ctx_dropdown);
        if let Some(label) = &selected_label {
            dropdown.set_selected_by_name(label, ctx_dropdown);
        }
    });
}

fn render_new_environment_footer<A: OrchestrationControlAction>(
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let is_hovered = mouse_state.lock().unwrap().is_hovered();
    let bg = if is_hovered {
        theme.surface_3()
    } else {
        theme.surface_2()
    };
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size();
    let text_color = theme.active_ui_text_color();
    let icon_size = font_size;
    let mouse_state = mouse_state.clone();

    Hoverable::new(mouse_state, move |_| {
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(
                    ConstrainedBox::new(Icon::Plus.to_warpui_icon(text_color).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                )
                .with_child(
                    Text::new_inline("New environment", font_family, font_size)
                        .with_color(text_color.into())
                        .finish(),
                )
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .with_background(bg)
        .with_border(Border::top(1.).with_border_fill(theme.outline()))
        .finish()
    })
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(A::create_environment_requested());
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}
/// Repopulates the host picker rows from [`host_snapshot`] (workspace
/// default, connected workers, recent custom slug), then sets the
/// current selection to `initial_host`.
pub fn populate_host_picker<V: View>(
    picker: &ViewHandle<HostPicker>,
    initial_host: &str,
    ctx: &mut ViewContext<V>,
) {
    let state = OrchestrationConfigState::from_run_agents_fields(
        None,
        None,
        &RunAgentsExecutionMode::Remote {
            environment_id: String::new(),
            worker_host: initial_host.to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    );
    let snapshot = host_snapshot(&state, ctx);
    let selected = snapshot
        .selected_id
        .unwrap_or_else(|| ORCHESTRATION_WARP_WORKER_HOST.to_string());
    let mut default_host = None;
    let mut recent_host = None;
    let mut connected_hosts = Vec::new();
    for row in snapshot.rows {
        match row.badge {
            Some(OptionBadge::Default) => default_host = Some(row.id),
            Some(OptionBadge::Recent) => recent_host = Some(row.id),
            Some(OptionBadge::Connected) => connected_hosts.push(row.id),
            // The unbadged "warp" row is built into the HostPicker itself.
            None => {}
        }
    }
    picker.update(ctx, |picker, picker_ctx| {
        picker.set_options(default_host, recent_host, connected_hosts, picker_ctx);
        picker.set_selected(&selected, picker_ctx);
    });
}

// ── Auth secret helpers ──────────────────────────────────

/// Trigger label for the auth-secret dropdown. `Unset` falls back to
/// "+ New API key…" rather than auto-picking the first loaded key.
fn auth_secret_trigger_label(selection: &AuthSecretSelection, supports_create_new: bool) -> String {
    match selection {
        AuthSecretSelection::Named(name) => name.clone(),
        AuthSecretSelection::Inherit => AUTH_SECRET_INHERIT_LABEL.to_string(),
        AuthSecretSelection::CreatingNew => AUTH_SECRET_CREATE_NEW_LABEL.to_string(),
        AuthSecretSelection::Unset if supports_create_new => {
            AUTH_SECRET_CREATE_NEW_LABEL.to_string()
        }
        AuthSecretSelection::Unset => AUTH_SECRET_INHERIT_LABEL.to_string(),
    }
}

/// Populates the auth secret picker from [`api_key_snapshot`]: Inherit,
/// loaded managed secrets, then a "+ New API key…" entry for harnesses
/// with managed-secret types. Also kicks off a lazy fetch so subsequent
/// paints replace "Loading…" with real entries.
pub fn populate_auth_secret_picker_for_harness<A: OrchestrationControlAction, V: View>(
    dropdown: &ViewHandle<Dropdown<A>>,
    selection: &AuthSecretSelection,
    harness_type: &str,
    ctx: &mut ViewContext<V>,
) {
    let Some(harness) = Harness::parse_orchestration_harness(harness_type) else {
        return;
    };
    if harness == Harness::Oz {
        return;
    }
    // Trigger lazy fetch so the next paint shows real entries.
    HarnessAvailabilityModel::handle(ctx).update(ctx, |model, ctx| {
        model.ensure_auth_secrets_fetched(harness, ctx);
    });

    let mut state = OrchestrationConfigState::from_run_agents_fields(
        None,
        Some(harness_type),
        &RunAgentsExecutionMode::Local,
    );
    state.auth_secret_selection = selection.clone();
    dropdown.update(ctx, |dropdown, ctx_dropdown| {
        let snapshot = api_key_snapshot(&state, ctx_dropdown);
        let supports_create_new =
            matches!(snapshot.footer, Some(OptionFooter::CreateNewAuthSecret));
        let mut items: Vec<MenuItem<DropdownAction>> = snapshot
            .rows
            .into_iter()
            .map(|row| {
                // An empty row id is the Inherit entry; others are named
                // managed secrets.
                let name = (!row.id.is_empty()).then_some(row.id);
                MenuItem::Item(MenuItemFields::new(&row.label).with_on_select_action(
                    DropdownAction::select_action_and_close(A::auth_secret_changed(name)),
                ))
            })
            .collect();
        match snapshot.status {
            OptionSourceStatus::Loading => items.push(MenuItem::Item(
                MenuItemFields::new("Loading…").with_disabled(true),
            )),
            OptionSourceStatus::Failed { message } => items.push(MenuItem::Item(
                MenuItemFields::new(&message).with_disabled(true),
            )),
            OptionSourceStatus::Ready | OptionSourceStatus::Empty { .. } => {}
        }
        if supports_create_new {
            items.push(MenuItem::Separator);
            items.push(MenuItem::Item(
                MenuItemFields::new(AUTH_SECRET_CREATE_NEW_LABEL).with_on_select_action(
                    DropdownAction::select_action_and_close(A::create_new_auth_secret_requested()),
                ),
            ));
        }
        let final_selection =
            auth_secret_trigger_label(&state.auth_secret_selection, supports_create_new);
        dropdown.set_rich_items(items, ctx_dropdown);
        dropdown.set_selected_by_name(&final_selection, ctx_dropdown);
    });
}

/// Marks `CreatingNew` (not re-seeded from settings, so a background refresh
/// can't restore a stale selection mid-create). Used by both card views.
pub fn apply_create_new_auth_secret_requested<V: View>(
    state: &mut OrchestrationConfigState,
    _ctx: &mut ViewContext<V>,
) {
    state.select_create_new_auth_secret();
}

/// Adopts a freshly-created secret as the active selection when its
/// harness matches the card's current harness. Returns `true` on mutation.
pub fn apply_created_auth_secret_if_matches<V: View>(
    state: &mut OrchestrationConfigState,
    created_harness: Harness,
    created_name: &str,
    ctx: &mut ViewContext<V>,
) -> bool {
    let Some(card_harness) = Harness::parse_orchestration_harness(&state.harness_type) else {
        return false;
    };
    if card_harness != created_harness {
        return false;
    }
    if matches!(&state.auth_secret_selection, AuthSecretSelection::Named(n) if n == created_name) {
        return false;
    }
    state.auth_secret_selection = AuthSecretSelection::Named(created_name.to_string());
    persist_auth_secret_selection(&state.harness_type, &state.auth_secret_selection, ctx);
    true
}

// ── Shared action helpers ───────────────────────────────────

/// Worker host to display for the current execution mode (Local always
/// shows the Warp host).
fn current_worker_host(state: &OrchestrationConfigState) -> &str {
    match &state.execution_mode {
        RunAgentsExecutionMode::Remote { worker_host, .. } => worker_host.as_str(),
        RunAgentsExecutionMode::Local => ORCHESTRATION_WARP_WORKER_HOST,
    }
}

/// Handles a harness change for both card views: applies the shared
/// [`OrchestrationEditState::apply_harness_change`] transition, then
/// repopulates the affected pickers.
///
/// Does NOT re-enter the harness picker that dispatched this action
/// (unless local sanitization changed the harness out from under it).
pub fn apply_harness_change<A: OrchestrationControlAction, V: View>(
    orchestration_edit_state: &mut OrchestrationEditState,
    handles: &OrchestrationPickerHandles<A>,
    new_harness_type: &str,
    fallback_base_model_id: Option<String>,
    ctx: &mut ViewContext<V>,
) {
    orchestration_edit_state.apply_harness_change(new_harness_type, fallback_base_model_id, ctx);
    let state = &orchestration_edit_state.orchestration_config_state;
    let is_local = !state.execution_mode.is_remote();
    if is_local
        && state.harness_type != new_harness_type
        && let Some(handle) = &handles.harness_picker
    {
        populate_harness_picker(handle, &state.harness_type, true, ctx);
    }
    if let Some(handle) = &handles.model_picker {
        populate_model_picker_for_harness(
            handle,
            &state.model_id,
            &state.harness_type,
            is_local,
            ctx,
        );
    }
    if let Some(handle) = &handles.auth_secret_picker {
        populate_auth_secret_picker_for_harness(
            handle,
            &state.auth_secret_selection,
            new_harness_type,
            ctx,
        );
    }
}

/// Handles an execution-mode toggle for both card views: applies the
/// shared [`OrchestrationConfigState::apply_execution_mode_change`]
/// transition, then repopulates the affected pickers and syncs all
/// picker selections.
pub fn apply_execution_mode_change<A: OrchestrationControlAction, V: View>(
    state: &mut OrchestrationConfigState,
    handles: &OrchestrationPickerHandles<A>,
    is_remote: bool,
    fallback_base_model_id: Option<String>,
    ctx: &mut ViewContext<V>,
) {
    state.apply_execution_mode_change(is_remote, fallback_base_model_id, ctx);
    let is_local = !state.execution_mode.is_remote();
    if let Some(handle) = &handles.harness_picker {
        populate_harness_picker(handle, &state.harness_type, is_local, ctx);
    }
    if let Some(handle) = &handles.model_picker {
        populate_model_picker_for_harness(
            handle,
            &state.model_id,
            &state.harness_type,
            is_local,
            ctx,
        );
    }
    if let Some(handle) = &handles.host_picker {
        populate_host_picker(handle, current_worker_host(state), ctx);
    }
    sync_picker_selections(state, handles, ctx);
}

// ── Picker repopulation + selection sync ──

/// Revalidates the edit state against the latest catalogs via
/// [`OrchestrationConfigState::revalidate_after_catalog_change`], then
/// repopulates every picker from the current server-provided data and
/// re-syncs dropdown selections.
pub fn repopulate_all_pickers<A: OrchestrationControlAction, V: View>(
    state: &mut OrchestrationConfigState,
    handles: &OrchestrationPickerHandles<A>,
    ctx: &mut ViewContext<V>,
) {
    state.revalidate_after_catalog_change(ctx);
    let is_local = !state.execution_mode.is_remote();
    if let Some(handle) = &handles.harness_picker {
        populate_harness_picker(handle, &state.harness_type, is_local, ctx);
    }
    if let Some(handle) = &handles.model_picker {
        populate_model_picker_for_harness(
            handle,
            &state.model_id,
            &state.harness_type,
            is_local,
            ctx,
        );
    }
    if let Some(handle) = &handles.auth_secret_picker {
        populate_auth_secret_picker_for_harness(
            handle,
            &state.auth_secret_selection,
            &state.harness_type,
            ctx,
        );
    }
    if let Some(handle) = &handles.host_picker {
        populate_host_picker(handle, current_worker_host(state), ctx);
    }
    sync_picker_selections(state, handles, ctx);
}

pub fn sync_picker_selections<A: OrchestrationControlAction, V: View>(
    state: &OrchestrationConfigState,
    handles: &OrchestrationPickerHandles<A>,
    ctx: &mut ViewContext<V>,
) {
    if let Some(model_picker) = handles.model_picker.clone() {
        let snapshot = model_snapshot(state, ctx);
        if let Some(label) = selected_row_label(&snapshot) {
            model_picker.update(ctx, |dropdown, ctx_dropdown| {
                dropdown.set_selected_by_name(&label, ctx_dropdown);
            });
        }
    }
    if let Some(harness_picker) = handles.harness_picker.clone() {
        let harness_type = state.harness_type.clone();
        harness_picker.update(ctx, |dropdown, ctx_dropdown| {
            let target = Harness::parse_orchestration_harness(&harness_type).unwrap_or(Harness::Oz);
            // Use the server-provided display_name from HarnessAvailabilityModel
            // so the selection matches the labels (which also use display_name).
            let display = HarnessAvailabilityModel::as_ref(ctx_dropdown)
                .display_name_for(target)
                .to_string();
            dropdown.set_selected_by_name(&display, ctx_dropdown);
        });
    }
    if let Some(environment_picker) = handles.environment_picker.clone() {
        let snapshot = environment_snapshot(state, ctx);
        if let Some(label) = selected_row_label(&snapshot) {
            environment_picker.update(ctx, |dropdown, ctx_dropdown| {
                dropdown.set_selected_by_name(&label, ctx_dropdown);
            });
        }
    }
    if let Some(host_picker) = handles.host_picker.clone() {
        let worker_host = current_worker_host(state).to_string();
        host_picker.update(ctx, |picker, picker_ctx| {
            picker.set_selected(&worker_host, picker_ctx);
        });
    }
    if let Some(auth_secret_picker) = handles.auth_secret_picker.clone() {
        let supports_create_new = matches!(
            api_key_snapshot(state, ctx).footer,
            Some(OptionFooter::CreateNewAuthSecret)
        );
        let label = auth_secret_trigger_label(&state.auth_secret_selection, supports_create_new);
        auth_secret_picker.update(ctx, |dropdown, ctx_dropdown| {
            dropdown.set_selected_by_name(&label, ctx_dropdown);
        });
    }
}

// ── Adaptive picker layout ──────────────────────────────────────────

/// Lays out children horizontally at a fixed width when they all fit,
/// otherwise stacks them vertically at full available width.
///
/// Switches to vertical when `n * picker_width + (n-1) * spacing` exceeds
/// the available width from the incoming size constraint.
struct AdaptivePickerRow {
    children: Vec<Box<dyn Element>>,
    picker_width: f32,
    spacing: f32,
    is_vertical: bool,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl AdaptivePickerRow {
    fn new(picker_width: f32, spacing: f32) -> Self {
        Self {
            children: Vec::new(),
            picker_width,
            spacing,
            is_vertical: false,
            size: None,
            origin: None,
        }
    }

    fn add_child(&mut self, child: Box<dyn Element>) {
        self.children.push(child);
    }

    fn finish(self) -> Box<dyn Element> {
        Box::new(self)
    }
}

impl Element for AdaptivePickerRow {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let n = self.children.len();
        if n == 0 {
            self.size = Some(Vector2F::zero());
            return Vector2F::zero();
        }

        let total_horizontal =
            self.picker_width * n as f32 + self.spacing * n.saturating_sub(1) as f32;

        self.is_vertical = total_horizontal > constraint.max.x();

        if self.is_vertical {
            let width = constraint.max.x();
            let mut total_height = 0.0f32;
            for (i, child) in self.children.iter_mut().enumerate() {
                if i > 0 {
                    total_height += self.spacing;
                }
                let child_constraint =
                    SizeConstraint::new(vec2f(width, 0.), vec2f(width, f32::INFINITY));
                let child_size = child.layout(child_constraint, ctx, app);
                total_height += child_size.y();
            }
            let size = vec2f(width, total_height);
            self.size = Some(size);
            size
        } else {
            let mut max_height = 0.0f32;
            for child in self.children.iter_mut() {
                let child_constraint = SizeConstraint::new(
                    vec2f(self.picker_width, 0.),
                    vec2f(self.picker_width, f32::INFINITY),
                );
                let child_size = child.layout(child_constraint, ctx, app);
                max_height = max_height.max(child_size.y());
            }
            let size = vec2f(total_horizontal, max_height);
            self.size = Some(size);
            size
        }
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let mut current = origin;
        if self.is_vertical {
            for (i, child) in self.children.iter_mut().enumerate() {
                if i > 0 {
                    current += vec2f(0., self.spacing);
                }
                child.paint(current, ctx, app);
                if let Some(size) = child.size() {
                    current += vec2f(0., size.y());
                }
            }
        } else {
            for (i, child) in self.children.iter_mut().enumerate() {
                if i > 0 {
                    current += vec2f(self.spacing, 0.);
                }
                child.paint(current, ctx, app);
                let advance = child.size().map_or(self.picker_width, |s| s.x());
                current += vec2f(advance, 0.);
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let mut handled = false;
        for child in &mut self.children {
            handled |= child.dispatch_event(event, ctx, app);
        }
        handled
    }
}

// ── Render helpers ──────────────────────────────────────────────────

pub fn render_mode_toggle<A: OrchestrationControlAction>(
    is_remote: bool,
    handles: &OrchestrationPickerHandles<A>,
    appearance: &Appearance,
    active_segment_bg: Option<Fill>,
    full_width: bool,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label = Text::new(
        "Agent location".to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 1.,
    )
    .with_color(blended_colors::text_disabled(theme, theme.surface_1()))
    .finish();

    let local_segment = render_segment_button::<A>(
        "Local",
        !is_remote,
        A::execution_mode_toggled(false),
        handles.local_toggle.clone(),
        appearance,
        active_segment_bg,
    );
    let cloud_segment = render_segment_button::<A>(
        "Cloud",
        is_remote,
        A::execution_mode_toggled(true),
        handles.cloud_toggle.clone(),
        appearance,
        active_segment_bg,
    );

    let segment_outer_bg = warp_core::ui::theme::color::internal_colors::fg_overlay_2(theme);
    let segments_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(Expanded::new(1.0, cloud_segment).finish())
        .with_child(Expanded::new(1.0, local_segment).finish())
        .finish();
    let segmented_control = Container::new(segments_row)
        .with_padding_top(ORCHESTRATION_SEGMENTED_CONTROL_PADDING)
        .with_padding_bottom(ORCHESTRATION_SEGMENTED_CONTROL_PADDING)
        .with_padding_left(ORCHESTRATION_SEGMENTED_CONTROL_PADDING)
        .with_padding_right(ORCHESTRATION_SEGMENTED_CONTROL_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_background(segment_outer_bg)
        .finish();
    let segmented_control =
        ConstrainedBox::new(segmented_control).with_height(ORCHESTRATION_PICKER_HEIGHT);
    let segmented_control = if full_width {
        segmented_control.finish()
    } else {
        segmented_control
            .with_width(ORCHESTRATION_PICKER_MAX_WIDTH)
            .finish()
    };

    let cross_axis = if full_width {
        CrossAxisAlignment::Stretch
    } else {
        CrossAxisAlignment::Start
    };
    Flex::column()
        .with_cross_axis_alignment(cross_axis)
        .with_child(Container::new(label).with_margin_bottom(6.).finish())
        .with_child(segmented_control)
        .finish()
}

fn render_segment_button<A: OrchestrationControlAction>(
    label: &str,
    is_active: bool,
    on_click: A,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
    active_bg_override: Option<Fill>,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label_owned = label.to_string();
    let font_family = appearance.ui_font_family();
    let font_size = ORCHESTRATION_PICKER_FONT_SIZE;
    let active_text_color = blended_colors::text_main(theme, theme.surface_1());
    let inactive_text_color = blended_colors::text_disabled(theme, theme.surface_1());
    let segment_active_bg = active_bg_override
        .unwrap_or_else(|| warp_core::ui::theme::color::internal_colors::fg_overlay_4(theme));
    Hoverable::new(mouse_state, move |_| {
        let text = Text::new(label_owned.clone(), font_family, font_size)
            .with_color(if is_active {
                active_text_color
            } else {
                inactive_text_color
            })
            .finish();
        let centered = warpui::elements::Align::new(text).finish();
        let mut container = Container::new(centered)
            .with_vertical_padding(ORCHESTRATION_SEGMENT_VERTICAL_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if is_active {
            container = container.with_background(segment_active_bg);
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(on_click.clone());
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

pub fn render_picker_row<A: OrchestrationControlAction>(
    state: &OrchestrationConfigState,
    handles: &OrchestrationPickerHandles<A>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_picker_row_with_layout(state, handles, appearance, false)
}

/// Renders pickers vertically at full width when `vertical` is true,
/// or in the original horizontal layout when false.
pub fn render_picker_row_with_layout<A: OrchestrationControlAction>(
    state: &OrchestrationConfigState,
    handles: &OrchestrationPickerHandles<A>,
    appearance: &Appearance,
    vertical: bool,
) -> Box<dyn Element> {
    let is_remote = state.execution_mode.is_remote();
    let show_auth_picker = should_show_auth_secret_picker(state);

    if vertical {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(12.);

        let add = |col: &mut Flex, label: &str, picker: Option<Box<dyn Element>>| {
            col.add_child(render_picker_column(label, picker, appearance));
        };

        // Plan-card ordering groups harness-scoped pickers (harness + API
        // key) before host/environment/model so the API key sits directly
        // under the harness selector and does not split the model picker
        // from the "Primary model…" subtext that follows the picker row.
        add(
            &mut column,
            "Agent harness",
            handles
                .harness_picker
                .as_ref()
                .map(|p| ChildView::new(p).finish()),
        );
        if show_auth_picker {
            add(
                &mut column,
                AUTH_SECRET_COLUMN_LABEL,
                handles
                    .auth_secret_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
        }
        if is_remote {
            add(
                &mut column,
                "Host",
                handles
                    .host_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
            add(
                &mut column,
                "Environment",
                handles
                    .environment_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
            if FeatureFlag::CloudAgentRunners.is_enabled() {
                add(
                    &mut column,
                    "Runner",
                    handles
                        .runner_picker
                        .as_ref()
                        .map(|p| ChildView::new(p).finish()),
                );
            }
        }
        add(
            &mut column,
            "Base model",
            handles
                .model_picker
                .as_ref()
                .map(|p| ChildView::new(p).finish()),
        );

        Container::new(column.finish())
            .with_margin_top(12.)
            .finish()
    } else {
        let mut row = AdaptivePickerRow::new(ORCHESTRATION_PICKER_MAX_WIDTH, 12.);

        let add_picker =
            |row: &mut AdaptivePickerRow, label: &str, picker: Option<Box<dyn Element>>| {
                let col = render_picker_column(label, picker, appearance);
                row.add_child(col);
            };

        add_picker(
            &mut row,
            "Agent harness",
            handles
                .harness_picker
                .as_ref()
                .map(|p| ChildView::new(p).finish()),
        );
        if is_remote {
            add_picker(
                &mut row,
                "Host",
                handles
                    .host_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
            add_picker(
                &mut row,
                "Environment",
                handles
                    .environment_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
            if FeatureFlag::CloudAgentRunners.is_enabled() {
                add_picker(
                    &mut row,
                    "Runner",
                    handles
                        .runner_picker
                        .as_ref()
                        .map(|p| ChildView::new(p).finish()),
                );
            }
        }
        add_picker(
            &mut row,
            "Base model",
            handles
                .model_picker
                .as_ref()
                .map(|p| ChildView::new(p).finish()),
        );
        if show_auth_picker {
            add_picker(
                &mut row,
                AUTH_SECRET_COLUMN_LABEL,
                handles
                    .auth_secret_picker
                    .as_ref()
                    .map(|p| ChildView::new(p).finish()),
            );
        }

        Container::new(row.finish()).with_margin_top(12.).finish()
    }
}

pub fn render_picker_column(
    label: &str,
    picker: Option<Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label_el = Text::new(
        label.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 1.,
    )
    .with_color(blended_colors::text_disabled(theme, theme.surface_1()))
    .finish();

    let body: Box<dyn Element> = picker.unwrap_or_else(|| Empty::new().finish());
    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(label_el)
        .with_child(body)
        .finish()
}

pub fn render_validation_error(
    reason: impl Into<String>,
    color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Container::new(
        Text::new(
            reason.into(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(color)
        .finish(),
    )
    .with_margin_bottom(8.)
    .finish()
}
