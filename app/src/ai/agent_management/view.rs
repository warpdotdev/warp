use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use fuzzy_match::match_indices_case_insensitive;
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use siphasher::sip::SipHasher;
use warp_core::features::FeatureFlag;
use warpui::scene::DropShadow;
use warpui::ui_components::button::ButtonVariant;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationNavigationSubject,
    AgentConversationsModel, AgentConversationsModelEvent, AgentManagementFilters, ArtifactFilter,
    ConversationUpdateKind, CreatedOnFilter, CreatorFilter, EnvironmentFilter, HarnessFilter,
    OwnerFilter, SessionStatus, SourceFilter, StatusFilter,
};
use crate::ai::agent_management::agent_type_selector::{
    AgentType, AgentTypeSelector, AgentTypeSelectorEvent,
};
use crate::ai::agent_management::cloud_setup_guide_view::{
    CloudSetupGuideEvent, CloudSetupGuideView,
};
use crate::ai::agent_management::details_action_buttons::{
    ActionButtonsConfig, AgentDetailsButtonEvent, ConversationActionButtonsRow,
};
use crate::ai::agent_management::telemetry::{
    AgentManagementTelemetryEvent, ArtifactType, FilterType, OpenedFrom,
};
use crate::ai::ambient_agents::{cancel_task_with_toast, AgentSource};
use crate::ai::artifacts::{Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent};
use crate::ai::blocklist::format_credits;
use crate::ai::conversation_details_panel::{
    ConversationDetailsData, ConversationDetailsPanel, ConversationDetailsPanelEvent,
};
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::harness_display;
use crate::app_state::PersistedAgentManagementFilters;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    PropagateHorizontalNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::menu::{MenuItem, MenuItemFields};
use crate::notebooks::NotebookId;
use crate::settings::ai::AISettings;
use crate::ui_components::agent_icon::agent_conversation_entry_icon_variant;
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::ui_components::icon_with_status::render_icon_with_status;
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, NakedTheme, PrimaryTheme, SecondaryTheme,
};
use crate::view_components::compactible_action_button::{
    CompactibleActionButton, MEDIUM_SIZE_SWITCH_THRESHOLD,
};
use crate::view_components::dropdown::{Dropdown, DropdownAction, DropdownStyle};
use crate::view_components::DismissibleToast;
use crate::view_components::FilterableDropdown;
use crate::workflows::WorkflowType;
use crate::workspace::{ForkedConversationDestination, ToastStack};
use crate::workspace::{RestoreConversationLayout, WorkspaceAction};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{send_telemetry_from_ctx, AgentModeEntrypoint};
use pathfinder_geometry::vector::vec2f;
use settings::Setting;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::clipboard::ClipboardContent;
use warpui::elements::new_scrollable::{
    NewScrollableElement, ScrollableAppearance, SingleAxisConfig,
};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Empty, Expanded, Flex, Hoverable, List, ListState, MainAxisSize,
    MouseStateHandle, NewScrollable, OffsetPositioning, Padding, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Rect, ScrollStateHandle, ScrollbarWidth, Shrinkable,
    SizeConstraintCondition, SizeConstraintSwitch, Stack, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::components::UiComponentStyles;
use warpui::{
    keymap::FixedBinding, Action, AppContext, Entity, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle, WeakViewHandle,
};

lazy_static! {
    static ref HASHER: SipHasher = SipHasher::new_with_keys(0, 0);
}

const MANAGEMENT_PANEL_WIDTH: f32 = 400.;
// Vertical margin for filter row elements to align with dropdown buttons
const FILTER_ROW_VERTICAL_MARGIN: f32 = 6.;

// Environment IDs are a fixed-length ServerId (22 chars), so keep this dropdown compact.
const ENV_DROPDOWN_WIDTH: f32 = 190.;

const CARD_ROW_SPACING: f32 = 8.;
const CARD_CONTENT_PADDING: f32 = 12.;
const CARD_BORDER_RADIUS: f32 = 4.;
const CARD_MARGIN_BOTTOM: f32 = 8.;

const BUTTON_SIZE: f32 = 20.;
/// Total size of the agent icon-with-status component rendered in each card's header row.
const CARD_AGENT_ICON_SIZE: f32 = 24.;
const CREATOR_AVATAR_FONT_SIZE: f32 = 10.;

const SESSION_EXPIRED_TEXT: &str = "Sessions expire after one week and cannot be opened.";

pub fn init(app: &mut AppContext) {
    use crate::util::bindings::cmd_or_ctrl_shift;

    app.register_fixed_bindings([FixedBinding::new(
        cmd_or_ctrl_shift("f"),
        AgentManagementViewAction::FocusSearch,
        warpui::keymap::macros::id!(AgentManagementView::ui_name()),
    )]);
}

fn should_show_artifacts(artifacts: &[Artifact]) -> bool {
    !artifacts.is_empty() && FeatureFlag::ConversationArtifacts.is_enabled()
}

pub type ManagementCardItemId = AgentConversationEntryId;

/// Store state for a given task row
struct CardState {
    hover_state: MouseStateHandle,
    avatar_hover_state: MouseStateHandle,
    session_status_hover_state: MouseStateHandle,
    artifact_buttons_view: Option<ViewHandle<ArtifactButtonsRow>>,
    action_buttons_hover_state: MouseStateHandle,
    action_buttons_view: ViewHandle<ConversationActionButtonsRow>,
    /// Use this ID to look up the full data from the model
    item_id: ManagementCardItemId,
}

pub struct AgentManagementView {
    list_state: ListState<()>,
    loading_icon_mouse_state: MouseStateHandle,
    scroll_state: ScrollStateHandle,

    /// Store the most recent requested set of ConversationOrTasks on the view
    items: Vec<CardState>,

    /// Store filters on the data
    filters: AgentManagementFilters,

    /// Search query for filtering by title
    search_query: String,
    search_editor: ViewHandle<EditorView>,

    /// Whether the user has dismissed the setup guide
    has_dismissed_setup_guide: bool,
    /// Whether the user is viewing the setup guide (toggled via button)
    is_viewing_setup_guide: bool,
    setup_guide_button: CompactibleActionButton,
    new_agent_button: CompactibleActionButton,
    view_agents_button: ViewHandle<ActionButton>,

    /// Agent type selector modal
    agent_type_selector: ViewHandle<AgentTypeSelector>,
    is_agent_type_selector_open: bool,

    cloud_setup_guide_view: ViewHandle<CloudSetupGuideView>,

    all_filter_button: ViewHandle<ActionButton>,
    personal_filter_button: ViewHandle<ActionButton>,
    status_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>,
    source_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>,
    created_on_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>,
    artifact_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>,
    harness_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>,
    environment_dropdown: ViewHandle<FilterableDropdown<AgentManagementViewAction>>,
    creator_dropdown: ViewHandle<FilterableDropdown<AgentManagementViewAction>>,
    clear_all_filters_button: ViewHandle<ActionButton>,
    no_filter_results_button: ViewHandle<ActionButton>,

    /// Details panel for showing task/conversation metadata
    details_panel: ViewHandle<ConversationDetailsPanel>,
    /// Currently selected item ID (for rendering details)
    selected_item_id: Option<ManagementCardItemId>,
}

/// Enum to track the state of the view, based on what tasks we have visible
enum ViewState {
    /// The model is still loading in data
    Loading,
    /// Showing the setup guide (this is the zero state when has_tasks is false)
    SetupGuide { has_items: bool },
    /// We have tasks, but currently have a filter applied that matches none of them
    NoFilterMatches,
    /// We have tasks that should be shown to the user
    HasTasks,
}

impl AgentManagementView {
    pub fn new(
        persisted_filters: Option<PersistedAgentManagementFilters>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &AgentConversationsModel::handle(ctx),
            Self::handle_agent_management_model_event,
        );

        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, _event, ctx| {
                me.update_harness_dropdown(ctx);
            },
        );

        let list_state = Self::construct_fresh_list_state(ctx.handle());

        let all_filter_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("All", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_tooltip("View your agent tasks plus all shared team tasks")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::SetOwnerFilter(
                        OwnerFilter::All,
                    ))
                })
        });

        let personal_filter_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Personal", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_tooltip("View agent tasks you created")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::SetOwnerFilter(
                        OwnerFilter::PersonalOnly,
                    ))
                })
        });

        let setup_guide_button = CompactibleActionButton::new(
            "Get started".to_string(),
            None,
            ButtonSize::Small,
            AgentManagementViewAction::ToggleSetupGuide,
            Icon::HelpCircle,
            Arc::new(SecondaryTheme),
            ctx,
        );

        let view_agents_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("View Agents", NakedTheme)
                .with_size(ButtonSize::Small)
                .with_icon(Icon::ArrowLeft)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::ToggleSetupGuide)
                })
        });

        // Set up dropdowns
        let status_dropdown = ctx.add_typed_action_view(Self::create_status_dropdown);
        let source_dropdown = ctx.add_typed_action_view(Self::create_source_dropdown);
        let created_on_dropdown = ctx.add_typed_action_view(Self::create_created_on_dropdown);
        let artifact_dropdown = ctx.add_typed_action_view(Self::create_artifact_dropdown);
        let harness_dropdown = ctx.add_typed_action_view(Self::create_harness_dropdown);
        let environment_dropdown = ctx.add_typed_action_view(Self::create_environment_dropdown);
        let creator_dropdown = ctx.add_typed_action_view(Self::create_creator_dropdown);

        let no_filter_results_button = ctx.add_typed_action_view(move |_ctx| {
            ActionButton::new("Clear filters", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::ClearFilters)
                })
        });

        let clear_all_filters_button = ctx.add_typed_action_view(move |_ctx| {
            ActionButton::new("Clear all", NakedTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::ClearFilters)
                })
        });

        let cloud_setup_guide_view = ctx.add_typed_action_view(CloudSetupGuideView::new);
        ctx.subscribe_to_view(&cloud_setup_guide_view, |_, _, event, ctx| match event {
            CloudSetupGuideEvent::OpenNewTabAndInsertWorkflow(workflow) => {
                ctx.emit(AgentManagementViewEvent::OpenNewTabAndRunWorkflow(
                    Box::new(workflow.clone()),
                ));
            }
        });

        let search_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::handle(ctx).as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("Search", ctx);
            editor
        });
        ctx.subscribe_to_view(&search_editor, |me, _handle, event, ctx| {
            me.handle_search_editor_event(event, ctx);
        });

        let new_agent_button = CompactibleActionButton::new(
            "New agent".to_string(),
            None,
            ButtonSize::Small,
            AgentManagementViewAction::ShowAgentTypeSelector,
            Icon::Plus,
            Arc::new(PrimaryTheme),
            ctx,
        );

        let agent_type_selector = ctx.add_typed_action_view(AgentTypeSelector::new);
        ctx.subscribe_to_view(&agent_type_selector, Self::handle_agent_type_selector_event);

        let has_dismissed_setup_guide = *AISettings::as_ref(ctx).did_dismiss_cloud_setup_guide;

        let filters = persisted_filters.map(|p| p.filters).unwrap_or_default();

        let details_panel: ViewHandle<ConversationDetailsPanel> =
            ctx.add_typed_action_view(|ctx| {
                ConversationDetailsPanel::new(true, MANAGEMENT_PANEL_WIDTH, ctx)
            });

        ctx.subscribe_to_view(&details_panel, Self::handle_details_panel_event);

        let mut view = Self {
            list_state,
            scroll_state: ScrollStateHandle::default(),
            items: Vec::new(),
            filters,
            search_query: String::new(),
            search_editor,
            has_dismissed_setup_guide,
            is_viewing_setup_guide: false,
            setup_guide_button,
            view_agents_button,
            cloud_setup_guide_view,
            loading_icon_mouse_state: MouseStateHandle::default(),
            all_filter_button,
            personal_filter_button,
            status_dropdown,
            source_dropdown,
            created_on_dropdown,
            artifact_dropdown,
            harness_dropdown,
            environment_dropdown,
            creator_dropdown,
            clear_all_filters_button,
            no_filter_results_button,
            new_agent_button,
            agent_type_selector,
            is_agent_type_selector_open: false,
            details_panel,
            selected_item_id: None,
        };

        view.update_filter_buttons(ctx);
        view.sync_with_loaded_filters(ctx);
        view.update_creator_dropdown(ctx);
        view.update_environment_dropdown(ctx);

        // Trigger server fetch if persisted filters differ from defaults
        // (team tasks are not loaded at startup, so we need to fetch them)
        if view.filters != AgentManagementFilters::default() {
            view.trigger_filter_fetch(ctx);
        }

        view.get_tasks_from_model(ctx);
        view
    }

    fn get_view_state(&self, app: &AppContext) -> ViewState {
        let model = AgentConversationsModel::as_ref(app);
        let has_items = model.has_items();

        // If loading with zero items, show skeleton cards
        // If loading with items, show list of interactive conversations (with loading indicator in header)
        if model.is_loading() && !has_items {
            return ViewState::Loading;
        }

        // Show setup guide if: no items (zero state) or user clicked button to toggle on the guide
        if !has_items || self.is_viewing_setup_guide {
            return ViewState::SetupGuide { has_items };
        }

        if self.items.is_empty() {
            return ViewState::NoFilterMatches;
        }

        ViewState::HasTasks
    }

    fn update_filter_buttons(&mut self, ctx: &mut ViewContext<Self>) {
        let owners = self.filters.owners;
        self.all_filter_button.update(ctx, |button, ctx| {
            button.set_active(owners == OwnerFilter::All, ctx);
        });
        self.personal_filter_button.update(ctx, |button, ctx| {
            button.set_active(owners == OwnerFilter::PersonalOnly, ctx);
        });
    }

    /// Sync the UI to match the loaded filter states.
    fn sync_with_loaded_filters(&mut self, ctx: &mut ViewContext<Self>) {
        self.status_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetStatusFilter(self.filters.status),
                ctx,
            );
        });

        self.source_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetSourceFilter(self.filters.source.clone()),
                ctx,
            );
        });

        self.created_on_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetCreatedOnFilter(self.filters.created_on),
                ctx,
            );
        });

        self.artifact_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetArtifactFilter(self.filters.artifact),
                ctx,
            );
        });

        self.harness_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetHarnessFilter(self.filters.harness),
                ctx,
            );
        });
    }

    /// Get current filters for persistence.
    pub fn get_filters(&self) -> PersistedAgentManagementFilters {
        PersistedAgentManagementFilters {
            filters: self.filters.clone(),
        }
    }

    fn construct_fresh_list_state(handle: WeakViewHandle<Self>) -> ListState<()> {
        ListState::new(move |index, _scroll_offset, app| {
            let Some(view_handle) = handle.upgrade(app) else {
                return Empty::new().finish();
            };
            view_handle.as_ref(app).render_card_at_index(index, app)
        })
    }

    fn create_status_dropdown(
        ctx: &mut ViewContext<Dropdown<AgentManagementViewAction>>,
    ) -> Dropdown<AgentManagementViewAction> {
        let (magenta, green, red) = {
            let theme = Appearance::as_ref(ctx).theme();
            (
                theme.ansi_fg_magenta(),
                theme.ansi_fg_green(),
                theme.ansi_fg_red(),
            )
        };

        let mut dropdown = Dropdown::new(ctx);
        Self::setup_filter_menu(&mut dropdown, "Status", ctx);

        // Use this helper to make dropdown items with status icons
        let make_status_option =
            |label: &str, action: AgentManagementViewAction, icon_data: Option<(Icon, Fill)>| {
                let mut fields = MenuItemFields::new(label)
                    .with_on_select_action(DropdownAction::SelectActionAndClose(action));
                if let Some((icon, color)) = icon_data {
                    fields = fields.with_icon(icon).with_override_icon_color(color);
                }
                MenuItem::Item(fields)
            };

        let items = vec![
            make_status_option(
                "All",
                AgentManagementViewAction::SetStatusFilter(StatusFilter::All),
                None,
            ),
            make_status_option(
                "Working",
                AgentManagementViewAction::SetStatusFilter(StatusFilter::Working),
                Some((Icon::ClockLoader, Fill::from(magenta))),
            ),
            make_status_option(
                "Done",
                AgentManagementViewAction::SetStatusFilter(StatusFilter::Done),
                Some((Icon::Check, Fill::from(green))),
            ),
            make_status_option(
                "Failed",
                AgentManagementViewAction::SetStatusFilter(StatusFilter::Failed),
                Some((Icon::X, Fill::from(red))),
            ),
        ];

        dropdown.set_rich_items(items, ctx);
        dropdown.set_selected_by_index(0, ctx);
        dropdown
    }

    fn create_source_dropdown(
        ctx: &mut ViewContext<Dropdown<AgentManagementViewAction>>,
    ) -> Dropdown<AgentManagementViewAction> {
        let mut dropdown = Dropdown::new(ctx);
        Self::setup_filter_menu(&mut dropdown, "Source", ctx);
        // Set a max height so we can fit all of the source options without scrolling
        dropdown.set_menu_max_height(200., ctx);

        let items = Self::build_source_dropdown_items();
        dropdown.set_rich_items(items, ctx);
        dropdown.set_selected_by_index(0, ctx);
        dropdown
    }

    /// Build the list of source filter items.
    fn build_source_dropdown_items() -> Vec<MenuItem<DropdownAction<AgentManagementViewAction>>> {
        // Build up the sources list
        let mut sources = vec![
            AgentSource::WebApp,
            AgentSource::CloudMode,
            AgentSource::AgentWebhook,
            AgentSource::Cli,
        ];
        if FeatureFlag::InteractiveConversationManagementView.is_enabled() {
            sources.push(AgentSource::Interactive)
        }
        sources.push(AgentSource::Linear);
        sources.push(AgentSource::Slack);
        if FeatureFlag::ScheduledAmbientAgents.is_enabled() {
            sources.push(AgentSource::ScheduledAgent);
        }

        let mut items = vec![MenuItem::Item(
            MenuItemFields::new("All").with_on_select_action(DropdownAction::SelectActionAndClose(
                AgentManagementViewAction::SetSourceFilter(SourceFilter::All),
            )),
        )];
        for source in sources {
            items.push(MenuItem::Item(
                MenuItemFields::new(source.display_name()).with_on_select_action(
                    DropdownAction::SelectActionAndClose(
                        AgentManagementViewAction::SetSourceFilter(SourceFilter::Specific(source)),
                    ),
                ),
            ));
        }

        items
    }

    /// Update the source dropdown items when tasks change.
    fn update_source_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let items = Self::build_source_dropdown_items();
        self.source_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(items, ctx);
            dropdown.set_selected_by_action(
                AgentManagementViewAction::SetSourceFilter(self.filters.source.clone()),
                ctx,
            );
        });
    }

    fn create_created_on_dropdown(
        ctx: &mut ViewContext<Dropdown<AgentManagementViewAction>>,
    ) -> Dropdown<AgentManagementViewAction> {
        let mut dropdown = Dropdown::new(ctx);
        Self::setup_filter_menu(&mut dropdown, "Created on", ctx);

        let items = vec![
            MenuItem::Item(MenuItemFields::new("All").with_on_select_action(
                DropdownAction::SelectActionAndClose(
                    AgentManagementViewAction::SetCreatedOnFilter(CreatedOnFilter::All),
                ),
            )),
            MenuItem::Item(MenuItemFields::new("Last 24 hours").with_on_select_action(
                DropdownAction::SelectActionAndClose(
                    AgentManagementViewAction::SetCreatedOnFilter(CreatedOnFilter::Last24Hours),
                ),
            )),
            MenuItem::Item(MenuItemFields::new("Past 3 days").with_on_select_action(
                DropdownAction::SelectActionAndClose(
                    AgentManagementViewAction::SetCreatedOnFilter(CreatedOnFilter::Past3Days),
                ),
            )),
            MenuItem::Item(MenuItemFields::new("Last week").with_on_select_action(
                DropdownAction::SelectActionAndClose(
                    AgentManagementViewAction::SetCreatedOnFilter(CreatedOnFilter::LastWeek),
                ),
            )),
        ];

        dropdown.set_rich_items(items, ctx);
        dropdown.set_selected_by_index(0, ctx);
        dropdown
    }

    fn create_artifact_dropdown(
        ctx: &mut ViewContext<Dropdown<AgentManagementViewAction>>,
    ) -> Dropdown<AgentManagementViewAction> {
        let mut dropdown = Dropdown::new(ctx);
        Self::setup_filter_menu(&mut dropdown, "Has artifact", ctx);

        let items = vec![
            MenuItem::Item(MenuItemFields::new("All").with_on_select_action(
                DropdownAction::SelectActionAndClose(AgentManagementViewAction::SetArtifactFilter(
                    ArtifactFilter::All,
                )),
            )),
            MenuItem::Item(MenuItemFields::new("Pull Request").with_on_select_action(
                DropdownAction::SelectActionAndClose(AgentManagementViewAction::SetArtifactFilter(
                    ArtifactFilter::PullRequest,
                )),
            )),
            MenuItem::Item(MenuItemFields::new("Plan").with_on_select_action(
                DropdownAction::SelectActionAndClose(AgentManagementViewAction::SetArtifactFilter(
                    ArtifactFilter::Plan,
                )),
            )),
            MenuItem::Item(MenuItemFields::new("Screenshot").with_on_select_action(
                DropdownAction::SelectActionAndClose(AgentManagementViewAction::SetArtifactFilter(
                    ArtifactFilter::Screenshot,
                )),
            )),
            MenuItem::Item(MenuItemFields::new("File").with_on_select_action(
                DropdownAction::SelectActionAndClose(AgentManagementViewAction::SetArtifactFilter(
                    ArtifactFilter::File,
                )),
            )),
        ];

        dropdown.set_rich_items(items, ctx);
        dropdown.set_selected_by_index(0, ctx);
        dropdown
    }

    fn create_harness_dropdown(
        ctx: &mut ViewContext<Dropdown<AgentManagementViewAction>>,
    ) -> Dropdown<AgentManagementViewAction> {
        let mut dropdown = Dropdown::new(ctx);
        Self::setup_filter_menu(&mut dropdown, "Harness", ctx);

        let items = Self::build_harness_dropdown_items(ctx);
        dropdown.set_rich_items(items, ctx);
        dropdown.set_selected_by_index(0, ctx);
        dropdown
    }

    fn build_harness_dropdown_items(
        app: &AppContext,
    ) -> Vec<MenuItem<DropdownAction<AgentManagementViewAction>>> {
        let mut items = vec![MenuItem::Item(
            MenuItemFields::new("All").with_on_select_action(DropdownAction::SelectActionAndClose(
                AgentManagementViewAction::SetHarnessFilter(HarnessFilter::All),
            )),
        )];

        let availability = HarnessAvailabilityModel::as_ref(app);
        for entry in availability.available_harnesses() {
            let harness = entry.harness;
            let mut fields = MenuItemFields::new(entry.display_name.clone())
                .with_icon(harness_display::icon_for(harness))
                .with_on_select_action(DropdownAction::SelectActionAndClose(
                    AgentManagementViewAction::SetHarnessFilter(HarnessFilter::Specific(harness)),
                ));
            if let Some(color) = harness_display::brand_color(harness) {
                fields = fields.with_override_icon_color(Fill::from(color));
            }
            items.push(MenuItem::Item(fields));
        }

        items
    }

    fn create_environment_dropdown(
        ctx: &mut ViewContext<FilterableDropdown<AgentManagementViewAction>>,
    ) -> FilterableDropdown<AgentManagementViewAction> {
        let mut dropdown = FilterableDropdown::new(ctx);
        Self::setup_searchable_filter_menu(&mut dropdown, "Environment", ctx);

        // Keep the button compact when a specific environment ID is selected by abbreviating the
        // displayed ID. (The dropdown menu still shows the full ID.)
        dropdown.set_menu_header_text_override(|text| {
            if matches!(text, "All" | "None") {
                return format!("Environment: {text}");
            }

            let abbreviated = text.chars().take(6).collect::<String>();
            if abbreviated == text {
                format!("Environment: {text}")
            } else {
                format!("Environment: {abbreviated}…")
            }
        });

        dropdown.set_top_bar_max_width(ENV_DROPDOWN_WIDTH);
        dropdown.set_menu_width(ENV_DROPDOWN_WIDTH, ctx);

        dropdown
    }

    fn create_creator_dropdown(
        ctx: &mut ViewContext<FilterableDropdown<AgentManagementViewAction>>,
    ) -> FilterableDropdown<AgentManagementViewAction> {
        let mut dropdown = FilterableDropdown::new(ctx);
        Self::setup_searchable_filter_menu(&mut dropdown, "Created by", ctx);
        dropdown
    }

    // Initialize the dropdown menu for the filter dropdowns (status, source)
    fn setup_filter_menu<A: Action + Clone>(
        dropdown: &mut Dropdown<A>,
        label_prefix: &'static str,
        ctx: &mut ViewContext<Dropdown<A>>,
    ) {
        dropdown.set_menu_width(160., ctx);
        dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
        dropdown.set_menu_header_text_override(move |text| format!("{}: {}", label_prefix, text));
        dropdown.set_style(DropdownStyle::ActionButtonSecondary, ctx);
    }

    // Initialize the dropdown menu for the searchable filter dropdowns (creator)
    fn setup_searchable_filter_menu<A: Action + Clone>(
        dropdown: &mut FilterableDropdown<A>,
        label_prefix: &'static str,
        ctx: &mut ViewContext<FilterableDropdown<A>>,
    ) {
        dropdown.set_menu_width(320., ctx);
        dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
        dropdown.set_menu_header_text_override(move |text| format!("{}: {}", label_prefix, text));
        dropdown.set_button_variant(ButtonVariant::Secondary);
    }

    fn update_harness_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let items = Self::build_harness_dropdown_items(ctx);
        self.harness_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(items, ctx);
        });
    }

    /// Since the valid set of environments depends on what tasks we have loaded in,
    /// we use this function to update the available options depending on the most recent
    /// set of tasks.
    fn update_environment_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let model = AgentConversationsModel::as_ref(ctx);
        let envs = model.get_all_environment_ids_and_names(ctx);

        let selected_name = match &self.filters.environment {
            EnvironmentFilter::All => Some("All".to_string()),
            EnvironmentFilter::NoEnvironment => Some("None".to_string()),
            EnvironmentFilter::Specific(id) => envs.get(id).cloned(),
        };

        self.environment_dropdown.update(ctx, |dropdown, ctx| {
            let mut items = vec![MenuItem::Item(
                MenuItemFields::new("All").with_on_select_action(
                    DropdownAction::SelectActionAndClose(
                        AgentManagementViewAction::SetEnvironmentFilter(EnvironmentFilter::All),
                    ),
                ),
            )];

            items.push(MenuItem::Item(
                MenuItemFields::new("None").with_on_select_action(
                    DropdownAction::SelectActionAndClose(
                        AgentManagementViewAction::SetEnvironmentFilter(
                            EnvironmentFilter::NoEnvironment,
                        ),
                    ),
                ),
            ));

            let mut sorted_envs: Vec<_> = envs.into_iter().collect();
            sorted_envs.sort_by(|(_, name_a), (_, name_b)| name_a.cmp(name_b));

            for (environment_id, environment_name) in sorted_envs {
                items.push(MenuItem::Item(
                    MenuItemFields::new(environment_name).with_on_select_action(
                        DropdownAction::SelectActionAndClose(
                            AgentManagementViewAction::SetEnvironmentFilter(
                                EnvironmentFilter::Specific(environment_id),
                            ),
                        ),
                    ),
                ));
            }

            dropdown.set_rich_items(items, ctx);
            if let Some(selected_name) = selected_name {
                dropdown.set_selected_by_name(&selected_name, ctx);
            }
        });
    }

    fn update_creator_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let creators = AgentConversationsModel::as_ref(ctx).get_all_creators(ctx);
        let creator_filter_name = match &self.filters.creator {
            CreatorFilter::All => "All",
            CreatorFilter::Specific { name, .. } => name,
        };
        self.creator_dropdown.update(ctx, |dropdown, ctx| {
            let mut items = vec![MenuItem::Item(
                MenuItemFields::new("All").with_on_select_action(
                    DropdownAction::SelectActionAndClose(
                        AgentManagementViewAction::SetCreatorFilter(CreatorFilter::All),
                    ),
                ),
            )];
            for (name, uid) in creators {
                items.push(MenuItem::Item(
                    MenuItemFields::new(&name).with_on_select_action(
                        DropdownAction::SelectActionAndClose(
                            AgentManagementViewAction::SetCreatorFilter(CreatorFilter::Specific {
                                name,
                                uid,
                            }),
                        ),
                    ),
                ));
            }
            dropdown.set_rich_items(items, ctx);
            dropdown.set_selected_by_name(creator_filter_name, ctx);
        });
    }

    fn handle_search_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        if let EditorEvent::Edited(_) = event {
            let new_query = self.search_editor.as_ref(ctx).buffer_text(ctx);
            if new_query != self.search_query {
                self.search_query = new_query;
                self.get_tasks_from_model(ctx);
            }
        }
    }

    /// Common handler for filter changes: fetch, refresh tasks, and save.
    fn on_filter_changed(&mut self, ctx: &mut ViewContext<Self>) {
        self.trigger_filter_fetch(ctx);
        self.get_tasks_from_model(ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
    }

    /// Trigger a server fetch for tasks matching current filters.
    fn trigger_filter_fetch(&self, ctx: &mut ViewContext<Self>) {
        let current_user_uid = AuthStateProvider::handle(ctx)
            .as_ref(ctx)
            .get()
            .user_id()
            .map(|uid| uid.as_string());
        if let Some(uid) = current_user_uid {
            let filters = self.filters.clone();
            AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.fetch_tasks_for_filters(&filters, &uid, ctx);
            });
        }
    }

    /// Shows the setup guide from a deep-link/action without toggling it off on repeated calls.
    pub(crate) fn show_setup_guide_from_link(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_viewing_setup_guide {
            send_telemetry_from_ctx!(AgentManagementTelemetryEvent::OpenSetupGuide, ctx);
        }
        self.is_viewing_setup_guide = true;
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn is_showing_setup_guide(&self) -> bool {
        self.is_viewing_setup_guide
    }

    pub(crate) fn apply_environment_filter_from_link(
        &mut self,
        environment_id: String,
        ctx: &mut ViewContext<Self>,
    ) {
        // This navigation should show the team/global task runs list.
        self.filters.owners = OwnerFilter::All;
        self.filters.reset_all_but_owner();
        self.filters.environment = EnvironmentFilter::Specific(environment_id);
        self.update_filter_buttons(ctx);

        // Clear search query.
        self.search_query.clear();
        self.search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });

        // Reset the selected states for the dropdowns.
        self.status_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(0, ctx);
        });
        self.source_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(0, ctx);
        });
        self.created_on_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(0, ctx);
        });
        self.artifact_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(0, ctx);
        });
        self.harness_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(0, ctx);
        });

        self.update_environment_dropdown(ctx);
        self.update_creator_dropdown(ctx);
        self.on_filter_changed(ctx);
    }

    /// Sync all tasks from the management model, and update the ListState
    fn get_tasks_from_model(&mut self, ctx: &mut ViewContext<Self>) {
        // Collect all card data we need
        struct CardData {
            item_id: ManagementCardItemId,
            artifacts: Vec<Artifact>,
            action_buttons_config: ActionButtonsConfig,
        }

        // Get sorted tasks and conversations from model
        let model = AgentConversationsModel::as_ref(ctx);
        let search_query = self.search_query.trim().to_lowercase();
        let cards: Vec<CardData> = model
            .get_entries(&self.filters, ctx)
            .into_iter()
            .filter(|entry| {
                if search_query.is_empty() {
                    return true;
                }
                match_indices_case_insensitive(&entry.display.title, &search_query).is_some()
            })
            .map(|entry| {
                let item_id = entry.id;
                let copy_link_url = entry
                    .capabilities
                    .can_copy_link
                    .then(|| {
                        AgentConversationsModel::resolve_copy_link(
                            AgentConversationNavigationSubject::Entry(entry.id),
                            ctx,
                        )
                    })
                    .flatten();
                let mut config = Self::action_buttons_config_for_entry(&entry, None, copy_link_url);
                if FeatureFlag::AgentManagementDetailsView.is_enabled() {
                    config.view_details_item_id = Some(item_id);
                }

                CardData {
                    item_id,
                    artifacts: entry.display.artifacts,
                    action_buttons_config: config,
                }
            })
            .collect();

        // Drain old state cards for reuse
        let mut old_items: HashMap<String, CardState> = self
            .items
            .drain(..)
            .map(|i| (i.item_id.as_key(), i))
            .collect();

        // Reset the list state, but save scroll position
        // We rebuild from scratch because items may have moved around in the view (if updated_at changes, for example)
        let current_scroll_index = self.list_state.get_scroll_index();
        let current_scroll_offset = self.list_state.get_scroll_offset();
        self.list_state = Self::construct_fresh_list_state(ctx.handle());

        // Rebuild the list state
        let mut new_items = Vec::with_capacity(cards.len());
        for card in cards {
            self.list_state.add_item();
            let card_key = card.item_id.as_key();

            if let Some(mut existing) = old_items.remove(&card_key) {
                // Update artifacts view if it exists, or create if needed
                if should_show_artifacts(&card.artifacts) {
                    if let Some(view) = &existing.artifact_buttons_view {
                        view.update(ctx, |v, ctx| v.update_artifacts(&card.artifacts, ctx));
                    } else {
                        existing.artifact_buttons_view =
                            Some(self.create_artifact_buttons_view(&card.artifacts, ctx));
                    }
                } else {
                    existing.artifact_buttons_view = None;
                }

                existing.action_buttons_view.update(ctx, |row, ctx| {
                    row.set_config(card.action_buttons_config, ctx)
                });

                new_items.push(existing);
            } else {
                let artifact_buttons_view = if should_show_artifacts(&card.artifacts) {
                    Some(self.create_artifact_buttons_view(&card.artifacts, ctx))
                } else {
                    None
                };
                let action_buttons_view =
                    self.create_action_buttons_view(card.item_id, card.action_buttons_config, ctx);

                new_items.push(CardState {
                    hover_state: MouseStateHandle::default(),
                    avatar_hover_state: MouseStateHandle::default(),
                    session_status_hover_state: MouseStateHandle::default(),
                    action_buttons_hover_state: MouseStateHandle::default(),
                    artifact_buttons_view,
                    action_buttons_view,
                    item_id: card.item_id,
                });
            }
        }

        // Restore approximate scroll position for the new list state
        let num_items = new_items.len();
        if num_items != 0 {
            if current_scroll_index > num_items {
                // If we have fewer items than we used to, scroll to the bottom of the list
                self.list_state.scroll_to(num_items - 1);
            } else {
                self.list_state
                    .scroll_to_with_offset(current_scroll_index, current_scroll_offset);
            }
        }

        self.items = new_items;
        ctx.notify();
    }

    fn create_action_buttons_view(
        &self,
        item_id: ManagementCardItemId,
        config: ActionButtonsConfig,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<ConversationActionButtonsRow> {
        let view = ctx.add_typed_action_view(ConversationActionButtonsRow::new);
        ctx.subscribe_to_view(&view, move |me, _, event, ctx| {
            me.handle_action_buttons_event(&item_id, event, ctx);
        });
        view.update(ctx, |row, ctx| row.set_config(config, ctx));
        view
    }

    fn action_buttons_config_for_entry(
        entry: &AgentConversationEntry,
        open_action: Option<WorkspaceAction>,
        copy_link_url: Option<String>,
    ) -> ActionButtonsConfig {
        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            ActionButtonsConfig::for_task(
                task_id,
                &entry.display.status,
                open_action,
                copy_link_url,
            )
        } else if let Some(conversation_id) = entry.identity.local_conversation_id {
            ActionButtonsConfig::for_conversation(conversation_id, open_action, copy_link_url)
        } else {
            ActionButtonsConfig {
                open_action,
                copy_link_url,
                ..Default::default()
            }
        }
    }

    fn handle_action_buttons_event(
        &mut self,
        item_id: &ManagementCardItemId,
        event: &AgentDetailsButtonEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentDetailsButtonEvent::Open => {
                // Open button only shown in details panel, not in management view cards.
                // We open the cards directly via clicking on them.
            }
            AgentDetailsButtonEvent::CancelTask { task_id } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::CloudRunCancelled {
                        task_id: task_id.to_string(),
                    },
                    ctx
                );

                cancel_task_with_toast(*task_id, ctx);
            }
            AgentDetailsButtonEvent::ForkConversation { conversation_id } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ConversationForked {
                        conversation_id: conversation_id.to_string(),
                    },
                    ctx
                );

                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id: *conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: None,
                    destination: ForkedConversationDestination::NewTab,
                });
            }
            AgentDetailsButtonEvent::ViewDetails { item_id } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::DetailsViewed {
                        item_id: item_id.as_key(),
                        viewed_from: OpenedFrom::ManagementView,
                    },
                    ctx
                );

                self.update_details_panel_for_item(item_id, ctx);
                self.selected_item_id = Some(*item_id);
                ctx.notify();
            }
            AgentDetailsButtonEvent::CopyLink { link } => {
                match item_id {
                    ManagementCardItemId::Conversation(conversation_id) => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::ConversationLinkCopied {
                                conversation_id: conversation_id.to_string(),
                                copied_from: OpenedFrom::ManagementView,
                            },
                            ctx
                        );
                    }
                    ManagementCardItemId::AmbientRun(task_id) => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::SessionLinkCopied {
                                task_id: task_id.to_string(),
                                copied_from: OpenedFrom::ManagementView,
                            },
                            ctx
                        );
                    }
                }

                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.clone()));
            }
        }
    }

    fn create_artifact_buttons_view(
        &self,
        artifacts: &[Artifact],
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<ArtifactButtonsRow> {
        let view = ctx.add_typed_action_view(|ctx| ArtifactButtonsRow::new(artifacts, ctx));
        ctx.subscribe_to_view(&view, Self::handle_artifact_buttons_event);
        view
    }

    fn handle_artifact_buttons_event(
        &mut self,
        _view: ViewHandle<ArtifactButtonsRow>,
        event: &ArtifactButtonsRowEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ArtifactButtonsRowEvent::OpenPlan { notebook_uid } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ArtifactClicked {
                        artifact_type: ArtifactType::Plan
                    },
                    ctx
                );
                ctx.emit(AgentManagementViewEvent::OpenPlanNotebook {
                    notebook_uid: *notebook_uid,
                });
            }
            ArtifactButtonsRowEvent::CopyBranch { branch } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ArtifactClicked {
                        artifact_type: ArtifactType::Branch
                    },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(branch.clone()));

                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::default("Copied branch name".to_string());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            ArtifactButtonsRowEvent::OpenPullRequest { url } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ArtifactClicked {
                        artifact_type: ArtifactType::PullRequest
                    },
                    ctx
                );
                ctx.open_url(url);
            }
            ArtifactButtonsRowEvent::ViewScreenshots { artifact_uids } => {
                crate::ai::artifacts::open_screenshot_lightbox(artifact_uids, ctx);
            }
            ArtifactButtonsRowEvent::DownloadFile { artifact_uid } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ArtifactClicked {
                        artifact_type: ArtifactType::File
                    },
                    ctx
                );
                crate::ai::artifacts::download_file_artifact(artifact_uid, ctx);
            }
        }
    }

    fn handle_agent_management_model_event(
        &mut self,
        _model: ModelHandle<AgentConversationsModel>,
        event: &AgentConversationsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentConversationsModelEvent::ConversationsLoaded
            | AgentConversationsModelEvent::NewTasksReceived
            | AgentConversationsModelEvent::TasksUpdated => {
                self.update_creator_dropdown(ctx);
                self.update_environment_dropdown(ctx);
                self.update_source_dropdown(ctx);
                self.refresh_details_panel_if_needed(ctx);
                self.get_tasks_from_model(ctx);
            }
            AgentConversationsModelEvent::ConversationUpdated { kind } => {
                self.handle_conversation_updated(*kind, ctx);
            }
            AgentConversationsModelEvent::ConversationArtifactsUpdated { conversation_id } => {
                self.update_artifacts_for_conversation(*conversation_id, ctx);
                self.refresh_details_panel_if_needed(ctx);
            }
        }
    }

    /// Refresh the details panel if it's currently showing an item
    fn refresh_details_panel_if_needed(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(item_id) = self.selected_item_id {
            self.update_details_panel_for_item(&item_id, ctx);
        }
    }

    /// Decide how much work a `ConversationUpdated` event requires, based on its kind and the
    /// active status filter:
    /// * `Restored`: the underlying status didn't change, so the visible cards don't change
    ///   either. Just refresh the details panel.
    /// * `StatusSet` that crosses the active status filter: rebuild the
    ///   card list via `get_tasks_from_model`.
    /// * `StatusSet` that doesn't cross the active filter (or `All` is active):
    ///   just refresh the details panel re-render so the status icon picks up the new value.
    fn handle_conversation_updated(
        &mut self,
        kind: ConversationUpdateKind,
        ctx: &mut ViewContext<Self>,
    ) {
        match kind {
            ConversationUpdateKind::Restored => {}
            ConversationUpdateKind::MetadataChanged => self.get_tasks_from_model(ctx),
            ConversationUpdateKind::StatusSet {
                prev_filter,
                new_filter,
            } => {
                if self
                    .filters
                    .status
                    .is_membership_crossed(prev_filter, new_filter)
                {
                    self.get_tasks_from_model(ctx);
                } else {
                    ctx.notify();
                }
            }
        }
        self.refresh_details_panel_if_needed(ctx);
    }

    /// Update the details panel with fresh data for the given item.
    fn update_details_panel_for_item(
        &mut self,
        item_id: &ManagementCardItemId,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = AgentConversationsModel::as_ref(ctx);
        let Some(entry) = model.get_entry_by_id(item_id, ctx) else {
            return;
        };
        let open_action = AgentConversationsModel::resolve_open_action(
            AgentConversationNavigationSubject::Entry(*item_id),
            Some(RestoreConversationLayout::NewTab),
            ctx,
        );
        let copy_link_url = AgentConversationsModel::resolve_copy_link(
            AgentConversationNavigationSubject::Entry(*item_id),
            ctx,
        );
        let task = entry
            .identity
            .ambient_agent_task_id
            .and_then(|task_id| model.get_task_data(&task_id));
        let data = ConversationDetailsData::from_agent_conversation_entry(
            &entry,
            task.as_ref(),
            open_action,
            copy_link_url,
        );

        self.details_panel.update(ctx, |p, ctx| {
            p.set_conversation_details(data, ctx);
        });
    }

    /// Update just the artifact buttons for a specific conversation
    fn update_artifacts_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = AgentConversationsModel::as_ref(ctx);
        let Some((index, entry)) = self.items.iter().enumerate().find_map(|(index, card)| {
            let entry = model.get_entry_by_id(&card.item_id, ctx)?;
            (entry.identity.local_conversation_id == Some(conversation_id))
                .then_some((index, entry))
        }) else {
            return;
        };
        let artifacts = entry.display.artifacts;

        // Update the artifact buttons for this card
        if should_show_artifacts(&artifacts) {
            if let Some(view) = &self.items[index].artifact_buttons_view {
                view.update(ctx, |v, ctx| v.update_artifacts(&artifacts, ctx));
            } else {
                let new_view = self.create_artifact_buttons_view(&artifacts, ctx);
                self.items[index].artifact_buttons_view = Some(new_view);
            }
        } else {
            self.items[index].artifact_buttons_view = None;
        }
        ctx.notify();
    }

    fn handle_details_panel_event(
        &mut self,
        _view: ViewHandle<ConversationDetailsPanel>,
        event: &ConversationDetailsPanelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ConversationDetailsPanelEvent::Close => {
                self.selected_item_id = None;
                ctx.notify();
            }
            ConversationDetailsPanelEvent::OpenPlanNotebook { notebook_uid } => {
                ctx.emit(AgentManagementViewEvent::OpenPlanNotebook {
                    notebook_uid: *notebook_uid,
                });
            }
        }
    }

    fn handle_agent_type_selector_event(
        &mut self,
        _view: ViewHandle<AgentTypeSelector>,
        event: &AgentTypeSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentTypeSelectorEvent::Selected(agent_type) => {
                self.is_agent_type_selector_open = false;
                match agent_type {
                    AgentType::Cloud => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::SpawnNewCloudAgent,
                            ctx
                        );
                        ctx.dispatch_typed_action(&WorkspaceAction::AddAmbientAgentTab);
                    }
                    AgentType::Local => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::SpawnNewLocalAgent,
                            ctx
                        );
                        ctx.dispatch_typed_action(&WorkspaceAction::NewTabInAgentMode {
                            entrypoint: AgentModeEntrypoint::AgentManagementView,
                            zero_state_prompt_suggestion_type: None,
                        });
                    }
                }
                ctx.notify();
            }
            AgentTypeSelectorEvent::Dismissed => {
                self.is_agent_type_selector_open = false;
                ctx.notify();
            }
        }
    }

    fn build_avatar(name: &str, appearance: &Appearance) -> Container {
        let theme = appearance.theme();
        // Use a hash function for a range of colors in the IDs
        let mut hasher = *HASHER;
        name.hash(&mut hasher);
        let hash = hasher.finish();

        let color = match hash % 6 {
            0 => theme.ansi_fg_red(),
            1 => theme.ansi_fg_blue(),
            2 => theme.ansi_fg_green(),
            3 => theme.ansi_fg_yellow(),
            4 => theme.ansi_fg_magenta(),
            _ => theme.ansi_fg_cyan(),
        };

        Avatar::new(
            AvatarContent::DisplayName(name.to_string()),
            UiComponentStyles {
                width: Some(BUTTON_SIZE),
                height: Some(BUTTON_SIZE),
                font_size: Some(CREATOR_AVATAR_FONT_SIZE),
                font_family_id: Some(appearance.monospace_font_family()),
                font_weight: Some(Weight::Bold),
                font_color: Some(theme.surface_1().into()),
                background: Some(color.into()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_SIZE / 2.0))),
                ..Default::default()
            },
        )
        .build()
    }

    fn render_avatar_with_tooltip(
        creator_name: &str,
        appearance: &Appearance,
        mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let avatar = Self::build_avatar(creator_name, appearance);
        let tooltip_text = creator_name.to_string();
        let ui_builder = appearance.ui_builder().clone();

        // Add a tooltip displaying the creator's full name
        Hoverable::new(mouse_state, move |state| {
            let mut stack = Stack::new().with_child(avatar.finish());
            if state.is_hovered() {
                let tooltip = ui_builder.tool_tip(tooltip_text.clone()).build().finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    }

    // Renders a session status label based on the provided session status
    fn render_session_status_label(
        appearance: &Appearance,
        mouse_state: MouseStateHandle,
        session_status: &SessionStatus,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let ui_builder = appearance.ui_builder().clone();

        // Early return if session is available - no status label rendered
        let (label_text, tooltip_text_opt) = match session_status {
            SessionStatus::Expired => ("Session expired", Some(SESSION_EXPIRED_TEXT)),
            SessionStatus::Unavailable => ("No session available", None),
            SessionStatus::Available => return Empty::new().finish(),
        };

        Hoverable::new(mouse_state, move |state| {
            let label = Text::new_inline(label_text, font_family, font_size)
                .with_color(theme.nonactive_ui_text_color().into());

            let container = Container::new(label.finish())
                .with_background(internal_colors::fg_overlay_2(theme))
                .with_horizontal_padding(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));

            let mut stack = Stack::new().with_child(container.finish());
            if state.is_hovered() {
                if let Some(tooltip_text) = tooltip_text_opt {
                    let tooltip = ui_builder
                        .tool_tip(tooltip_text.to_string())
                        .build()
                        .finish();
                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                }
            }
            stack.finish()
        })
        .finish()
    }

    // Create a skeleton card for the loading screen
    fn render_skeleton_card(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Gradient values taken from the placeholders in the code review pane
        let base_gradient_color_start = internal_colors::neutral_3(theme);
        let mut base_gradient_color_end = base_gradient_color_start;
        base_gradient_color_end.a = 26;

        // Helper to create a gradient placeholder rect
        let placeholder = |max_width: f32, height: f32| {
            ConstrainedBox::new(
                Rect::new()
                    .with_horizontal_background_gradient(
                        base_gradient_color_start,
                        base_gradient_color_end,
                    )
                    .finish(),
            )
            .with_max_width(max_width)
            .with_height(height)
            .finish()
        };

        let header_row = placeholder(180., 14.);
        let metadata_row = placeholder(300., 12.);

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.)
            .with_child(header_row)
            .with_child(metadata_row)
            .finish();

        Container::new(content)
            .with_background(theme.surface_1())
            .with_border(Border::all(1.).with_border_fill(theme.surface_2()))
            .with_uniform_padding(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }

    fn render_loading_state(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let skeleton_cards = (0..5).map(|_| self.render_skeleton_card(appearance));

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(skeleton_cards)
            .with_spacing(8.)
            .finish()
    }

    fn render_card_at_index(&self, index: usize, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let Some(card_state) = self.items.get(index) else {
            return Empty::new().finish();
        };

        let model = AgentConversationsModel::as_ref(app);
        let Some(entry) = model.get_entry_by_id(&card_state.item_id, app) else {
            return Empty::new().finish();
        };

        self.render_card(card_state, &entry, appearance, app)
    }

    fn render_card(
        &self,
        card_state: &CardState,
        entry: &AgentConversationEntry,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let artifact_buttons_element = card_state.artifact_buttons_view.as_ref().and_then(|view| {
            if view.as_ref(app).is_empty() || !FeatureFlag::ConversationArtifacts.is_enabled() {
                None
            } else {
                Some(ChildView::new(view).finish())
            }
        });

        let action_buttons_mouse_over = card_state
            .action_buttons_hover_state
            .lock()
            .expect("action buttons hover state lock poisoned")
            .is_mouse_over_element();
        let action_buttons_is_empty = card_state.action_buttons_view.as_ref(app).is_empty();

        let card_hoverable = Hoverable::new(card_state.hover_state.clone(), move |mouse_state| {
            let mut card_content = Flex::column()
                .with_spacing(CARD_ROW_SPACING)
                .with_child(Self::render_header_row(card_state, entry, appearance))
                .with_child(Self::render_metadata_row(entry, appearance, app));

            // Add artifacts row if there is a buttons view
            if let Some(buttons_element) = artifact_buttons_element {
                card_content.add_child(buttons_element);
            }

            // Determine whether to show the buttons based on whether we are hovering on the action buttons or the card,
            // to prevent lots of flickering.
            let should_show_action_buttons = mouse_state.is_hovered() || action_buttons_mouse_over;

            let card_background = if should_show_action_buttons {
                internal_colors::fg_overlay_2(theme)
            } else {
                internal_colors::fg_overlay_1(theme)
            };

            let card = Container::new(card_content.finish())
                .with_background(card_background)
                .with_border(Border::all(1.).with_border_fill(internal_colors::fg_overlay_2(theme)))
                .with_uniform_padding(CARD_CONTENT_PADDING)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_BORDER_RADIUS)))
                .with_margin_top(CARD_MARGIN_BOTTOM)
                .finish();

            let mut stack = Stack::new().with_child(card);
            if should_show_action_buttons && !action_buttons_is_empty {
                let action_buttons =
                    Hoverable::new(card_state.action_buttons_hover_state.clone(), |_| {
                        Container::new(ChildView::new(&card_state.action_buttons_view).finish())
                            .with_background(theme.surface_3())
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                            .with_uniform_padding(4.)
                            .with_drop_shadow(DropShadow {
                                color: ColorU::new(0, 0, 0, 77),
                                offset: vec2f(0., 4.),
                                blur_radius: 7.,
                                spread_radius: 0.,
                            })
                            .finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .with_defer_events_to_children();

                // Note: we use an overlay layer so that the hover on the top of the list can extend outside
                // of the list boundaries, rendered unclipped.
                stack.add_positioned_overlay_child(
                    action_buttons.finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(-CARD_CONTENT_PADDING, 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopRight,
                        ChildAnchor::MiddleRight,
                    ),
                );
            }

            stack.finish()
        })
        .with_defer_events_to_children();

        let item_id = card_state.item_id;
        let card_hoverable = if entry.capabilities.can_open {
            card_hoverable
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AgentManagementViewAction::OpenSession { item_id });
                })
        } else {
            card_hoverable
        };

        card_hoverable.finish()
    }

    fn render_header_row(
        card_state: &CardState,
        entry: &AgentConversationEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();

        let title_text = Text::new_inline(entry.display.title.clone(), font_family, font_size)
            .with_color(theme.active_ui_text_color().into());
        let status_icon = render_icon_with_status(
            agent_conversation_entry_icon_variant(entry),
            CARD_AGENT_ICON_SIZE,
            0.,
            theme,
            internal_colors::fg_overlay_1(theme),
        );
        let time_str = format_approx_duration_from_now_utc(entry.display.last_updated);
        let time_text = Text::new_inline(time_str, font_family, font_size)
            .with_color(theme.nonactive_ui_text_color().into());
        let creator_name = entry
            .display
            .creator
            .name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let avatar = Self::render_avatar_with_tooltip(
            &creator_name,
            appearance,
            card_state.avatar_hover_state.clone(),
        );

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.)
            .with_child(Container::new(status_icon).with_margin_right(4.).finish())
            .with_child(Expanded::new(1., title_text.finish()).finish());
        let mut time_and_avatar = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);

        if let Some(session_status) = &entry.display.session_status {
            time_and_avatar.add_child(Self::render_session_status_label(
                appearance,
                card_state.session_status_hover_state.clone(),
                session_status,
            ));
        }

        time_and_avatar.add_child(time_text.finish());
        time_and_avatar.add_child(avatar);

        row.add_child(
            Container::new(time_and_avatar.finish())
                .with_margin_right(2.)
                .finish(),
        );

        // We want to make sure the text in the row is always at least the button height
        ConstrainedBox::new(row.finish())
            .with_min_height(BUTTON_SIZE)
            .finish()
    }

    fn render_metadata_row(
        entry: &AgentConversationEntry,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let mut metadata_parts = Vec::new();

        if let Some(source) = &entry.display.source {
            metadata_parts.push(format!("Source: {}", source.display_name()));
        }

        let availability = HarnessAvailabilityModel::as_ref(app);
        if availability.should_show_harness_selector() {
            if let Some(harness) = entry.display.harness {
                metadata_parts.push(format!(
                    "Harness: {}",
                    availability.display_name_for(harness)
                ));
            }
        }

        if let Some(executor) = &entry.display.executor {
            let same_as_creator =
                executor.uid.is_some() && executor.uid == entry.display.creator.uid;
            if !same_as_creator {
                if let Some(name) = executor.name.as_deref().or(executor.uid.as_deref()) {
                    let label = if executor
                        .principal_type
                        .is_some_and(|pt| pt.is_service_account())
                    {
                        "Agent"
                    } else {
                        "Executor"
                    };
                    metadata_parts.push(format!("{label}: {name}"));
                }
            }
        }

        if let Some(run_time) = &entry.display.run_time {
            metadata_parts.push(format!("Run time: {run_time}"));
        }

        if let Some(usage) = entry.display.request_usage.map(format_credits) {
            metadata_parts.push(format!("Credits used: {usage}"));
        }

        Text::new(metadata_parts.join(" • "), font_family, font_size)
            .with_color(theme.nonactive_ui_text_color().into())
            .finish()
    }

    // Render the main page header based on the current view state
    fn render_header(&self, app: &AppContext) -> Box<dyn Element> {
        match self.get_view_state(app) {
            ViewState::Loading => self.render_loading_header(app),
            ViewState::SetupGuide {
                has_items: has_tasks,
            } => self.render_setup_guide_header(has_tasks, app),
            ViewState::NoFilterMatches | ViewState::HasTasks => self.render_task_list_header(app),
        }
    }

    fn render_setup_guide_header(&self, has_tasks: bool, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let size_switch_threshold = MEDIUM_SIZE_SWITCH_THRESHOLD * appearance.monospace_ui_scalar();

        let build_header = |use_expanded: bool| {
            let mut header_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max);

            if has_tasks {
                header_row.add_child(ChildView::new(&self.view_agents_button).finish());
            }

            header_row.add_child(Expanded::new(1., Empty::new().finish()).finish());

            if !has_tasks && !cfg!(target_family = "wasm") {
                let button = if use_expanded {
                    self.new_agent_button.expanded_button()
                } else {
                    self.new_agent_button.compact_button()
                };
                header_row.add_child(ChildView::new(button).finish());
            }

            header_row.finish()
        };

        SizeConstraintSwitch::new(
            build_header(true),
            vec![(
                SizeConstraintCondition::WidthLessThan(size_switch_threshold),
                build_header(false),
            )],
        )
        .finish()
    }

    fn render_task_list_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let is_loading = AgentConversationsModel::handle(app)
            .as_ref(app)
            .is_loading();

        let is_on_team = UserWorkspaces::as_ref(app).current_team().is_some();

        let size_switch_threshold = MEDIUM_SIZE_SWITCH_THRESHOLD * appearance.monospace_ui_scalar();

        let build_header = |use_expanded: bool| {
            let title = Text::new_inline(
                "Runs",
                appearance.ui_font_family(),
                appearance.ui_font_size() + 4.,
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(theme.active_ui_text_color().into())
            .finish();

            let setup_guide_button = if use_expanded {
                self.setup_guide_button.expanded_button()
            } else {
                self.setup_guide_button.compact_button()
            };

            let new_agent_button = if use_expanded {
                self.new_agent_button.expanded_button()
            } else {
                self.new_agent_button.compact_button()
            };

            let mut header_top = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(4.)
                .with_child(Container::new(title).with_margin_right(12.).finish());

            if is_on_team {
                header_top.add_child(ChildView::new(&self.personal_filter_button).finish());
                header_top.add_child(ChildView::new(&self.all_filter_button).finish());
            }

            if is_loading {
                header_top.add_child(self.render_cloud_loading_icon(appearance));
            }

            header_top.add_child(Expanded::new(1., Empty::new().finish()).finish());
            header_top.add_child(ChildView::new(setup_guide_button).finish());

            if !cfg!(target_family = "wasm") {
                header_top.add_child(ChildView::new(new_agent_button).finish());
            }

            let mut filters_wrap = Wrap::row()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(ChildView::new(&self.status_dropdown).finish())
                .with_child(ChildView::new(&self.source_dropdown).finish())
                .with_child(ChildView::new(&self.created_on_dropdown).finish())
                .with_child(ChildView::new(&self.artifact_dropdown).finish());

            if HarnessAvailabilityModel::as_ref(app).should_show_harness_selector() {
                filters_wrap.add_child(ChildView::new(&self.harness_dropdown).finish());
            }

            filters_wrap.add_child(ChildView::new(&self.environment_dropdown).finish());

            if self.filters.owners != OwnerFilter::PersonalOnly {
                filters_wrap.add_child(ChildView::new(&self.creator_dropdown).finish());
            }

            if self.filters.is_filtering() {
                filters_wrap.add_child(ChildView::new(&self.clear_all_filters_button).finish());
            }

            let search_input = self.render_search_input(app);
            let header_bottom = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_child(Expanded::new(3., filters_wrap.finish()).finish())
                .with_child(Expanded::new(1., search_input).finish())
                .finish();

            let header_bottom = Container::new(header_bottom).with_margin_top(4.).finish();

            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(header_top.finish())
                .with_child(header_bottom)
                .finish()
        };

        SizeConstraintSwitch::new(
            build_header(true),
            vec![(
                SizeConstraintCondition::WidthLessThan(size_switch_threshold),
                build_header(false),
            )],
        )
        .finish()
    }

    fn render_cloud_loading_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder().clone();
        let icon_size = appearance.ui_font_size();

        let loading_icon = ConstrainedBox::new(
            Icon::Refresh
                .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
                .finish(),
        )
        .with_height(icon_size)
        .with_width(icon_size)
        .finish();

        Hoverable::new(self.loading_icon_mouse_state.clone(), move |mouse_state| {
            let mut stack = Stack::new().with_child(loading_icon);
            if mouse_state.is_hovered() {
                let tooltip = ui_builder
                    .tool_tip(String::from("Loading cloud agent runs"))
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    }

    fn render_loading_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let title = Text::new_inline(
            "Runs",
            appearance.ui_font_family(),
            appearance.ui_font_size() + 4.,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into())
        .finish();

        let loading_icon = ConstrainedBox::new(
            Icon::Loading
                .to_warpui_icon(Fill::Solid(internal_colors::neutral_6(theme)))
                .finish(),
        )
        .with_height(appearance.ui_font_size() + 2.)
        .with_width(appearance.ui_font_size() + 2.)
        .finish();

        let loading_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(loading_icon).with_margin_right(10.).finish())
            .with_child(
                Text::new_inline(
                    "Loading agents...",
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.active_ui_text_color().into())
                .finish(),
            )
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(title)
            .with_child(loading_row)
            .finish()
    }

    fn render_search_input(&self, app: &AppContext) -> Box<dyn Element> {
        let theme = Appearance::handle(app).as_ref(app).theme();

        let search_row = Clipped::new(ChildView::new(&self.search_editor).finish()).finish();

        Container::new(
            ConstrainedBox::new(
                Container::new(search_row)
                    .with_padding(
                        Padding::uniform(0.)
                            .with_vertical(5.)
                            .with_left(8.)
                            .with_right(8.),
                    )
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            )
            .with_height(30.)
            .finish(),
        )
        .with_margin_top(FILTER_ROW_VERTICAL_MARGIN)
        .with_margin_bottom(FILTER_ROW_VERTICAL_MARGIN)
        .finish()
    }

    fn render_no_results_view(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let icon = ConstrainedBox::new(
            Icon::FilterOff
                .to_warpui_icon(appearance.theme().nonactive_ui_text_color())
                .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish();

        let text = Text::new_inline(
            "No results matched your filters",
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(appearance.theme().nonactive_ui_text_color().into())
        .finish();

        Align::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(12.)
                .with_child(icon)
                .with_child(text)
                .with_child(ChildView::new(&self.no_filter_results_button).finish())
                .finish(),
        )
        .finish()
    }

    fn render_default_scroll_view(&self, app: &AppContext) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        let axis_config = SingleAxisConfig::Manual {
            handle: self.scroll_state.clone(),
            child: NewScrollableElement::finish_scrollable(List::new(self.list_state.clone())),
        };
        NewScrollable::vertical(
            axis_config,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::None, false))
        .with_always_handle_events_first(false)
        .finish()
    }
}

impl Entity for AgentManagementView {
    type Event = AgentManagementViewEvent;
}

impl View for AgentManagementView {
    fn ui_name() -> &'static str {
        "AgentManagementView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let main_content = match self.get_view_state(app) {
            ViewState::Loading => self.render_loading_state(app),
            ViewState::SetupGuide { .. } => ChildView::new(&self.cloud_setup_guide_view).finish(),
            ViewState::NoFilterMatches => self.render_no_results_view(app),
            ViewState::HasTasks => self.render_default_scroll_view(app),
        };

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(app))
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(main_content).with_margin_top(12.).finish(),
                )
                .finish(),
            )
            .finish();

        let centered = Align::new(content).top_center().finish();

        let main_view = Container::new(centered).with_uniform_margin(16.).finish();

        // Wrap main view with details panel if we have selected an item
        let base_view = if self.selected_item_id.is_some() {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Expanded::new(1., main_view).finish())
                .with_child(ChildView::new(&self.details_panel).finish())
                .finish()
        } else {
            main_view
        };

        if self.is_agent_type_selector_open {
            Stack::new()
                .with_child(base_view)
                .with_child(ChildView::new(&self.agent_type_selector).finish())
                .finish()
        } else {
            base_view
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentManagementViewAction {
    SetOwnerFilter(OwnerFilter),
    SetStatusFilter(StatusFilter),
    SetSourceFilter(SourceFilter),
    SetCreatedOnFilter(CreatedOnFilter),
    SetArtifactFilter(ArtifactFilter),
    SetEnvironmentFilter(EnvironmentFilter),
    SetCreatorFilter(CreatorFilter),
    SetHarnessFilter(HarnessFilter),
    ClearFilters,
    ToggleSetupGuide,
    ShowAgentTypeSelector,
    OpenSession { item_id: ManagementCardItemId },
    FocusSearch,
}

pub enum AgentManagementViewEvent {
    OpenNewTabAndRunWorkflow(Box<WorkflowType>),
    OpenPlanNotebook { notebook_uid: NotebookId },
}

impl TypedActionView for AgentManagementView {
    type Action = AgentManagementViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentManagementViewAction::SetOwnerFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::Owner
                    },
                    ctx
                );
                self.filters.owners = *filter;
                self.update_filter_buttons(ctx);
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetStatusFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::Status
                    },
                    ctx
                );
                self.filters.status = *filter;
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetSourceFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::Source
                    },
                    ctx
                );
                self.filters.source = filter.clone();
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetCreatedOnFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::CreatedOn
                    },
                    ctx
                );
                self.filters.created_on = *filter;
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetArtifactFilter(filter) => {
                self.filters.artifact = *filter;
                self.get_tasks_from_model(ctx);
                ctx.dispatch_global_action("workspace:save_app", ());
            }
            AgentManagementViewAction::SetEnvironmentFilter(filter) => {
                self.filters.environment = filter.clone();
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetCreatorFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::Creator
                    },
                    ctx
                );
                self.filters.creator = filter.clone();
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::SetHarnessFilter(filter) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::FilterChanged {
                        filter_type: FilterType::Harness
                    },
                    ctx
                );
                self.filters.harness = *filter;
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::ClearFilters => {
                self.filters.reset_all_but_owner();

                // Reset the selected states for the dropdowns
                self.status_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_index(0, ctx);
                });
                self.source_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_index(0, ctx);
                });
                self.created_on_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_index(0, ctx);
                });
                self.artifact_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_index(0, ctx);
                });
                self.harness_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_index(0, ctx);
                });
                self.update_environment_dropdown(ctx);
                self.update_creator_dropdown(ctx);
                self.on_filter_changed(ctx);
            }
            AgentManagementViewAction::ToggleSetupGuide => {
                if self.is_viewing_setup_guide {
                    // User is leaving the guide - persist dismissal
                    send_telemetry_from_ctx!(AgentManagementTelemetryEvent::DismissSetupGuide, ctx);
                    if !self.has_dismissed_setup_guide {
                        AISettings::handle(ctx).update(ctx, |settings, ctx| {
                            let _ = settings.did_dismiss_cloud_setup_guide.set_value(true, ctx);
                        });
                        self.has_dismissed_setup_guide = true;
                    }
                    self.is_viewing_setup_guide = false;
                } else {
                    self.show_setup_guide_from_link(ctx);
                }
                ctx.notify();
            }
            AgentManagementViewAction::ShowAgentTypeSelector => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::AgentTypeSelectorOpened,
                    ctx
                );
                self.is_agent_type_selector_open = true;
                ctx.focus(&self.agent_type_selector);
                ctx.notify();
            }
            AgentManagementViewAction::OpenSession { item_id } => {
                let Some(action) = AgentConversationsModel::resolve_open_action(
                    AgentConversationNavigationSubject::Entry(*item_id),
                    Some(RestoreConversationLayout::NewTab),
                    ctx,
                ) else {
                    return;
                };

                match item_id {
                    ManagementCardItemId::Conversation(conversation_id) => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::ConversationOpened {
                                conversation_id: conversation_id.to_string(),
                                opened_from: OpenedFrom::ManagementView,
                            },
                            ctx
                        );
                    }
                    ManagementCardItemId::AmbientRun(task_id) => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::CloudRunOpened {
                                task_id: task_id.to_string(),
                                opened_from: OpenedFrom::ManagementView,
                            },
                            ctx
                        );
                    }
                }
                ctx.dispatch_typed_action(&action);
            }
            AgentManagementViewAction::FocusSearch => {
                ctx.focus(&self.search_editor);
            }
        }
    }
}
