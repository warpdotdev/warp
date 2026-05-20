//! A reusable side panel component for displaying conversation metadata.

use std::{collections::HashMap, str::FromStr, sync::Arc};

use chrono::{DateTime, Duration, Local};
use instant::Instant;
use parking_lot::RwLock;
use pathfinder_color::ColorU;
use warp_cli::agent::Harness;
use warp_cli::skill::SkillSpec;
use warp_core::channel::ChannelState;
use warp_core::ui::color::coloru_with_opacity;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        new_scrollable::{NewScrollable, SingleAxisConfig},
        resizable_state_handle, Border, ChildView, ClippedScrollStateHandle, ConstrainedBox,
        Container, CornerRadius, CrossAxisAlignment, DragBarSide, Empty, Expanded, Flex,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Resizable,
        ResizableStateHandle, SelectableArea, SelectionHandle, Shrinkable, Text, Wrap,
    },
    fonts::{Properties, Weight},
    keymap::FixedBinding,
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, StatusColorStyle,
};
use crate::ai::agent_conversations_model::entry::PrincipalType;
use crate::ai::agent_conversations_model::{AgentConversationEntry, AgentRunDisplayStatus};
use crate::ai::agent_management::details_action_buttons::{
    ActionButtonsConfig, AgentDetailsButtonEvent, ConversationActionButtonsRow,
};
use crate::ai::agent_management::telemetry::{AgentManagementTelemetryEvent, OpenedFrom};
use crate::ai::ambient_agents::task::TaskPrincipalInfo;
use crate::ai::ambient_agents::{cancel_task_with_toast, AmbientAgentTaskId};
use crate::ai::artifacts::{Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::cloud_environments::{AmbientAgentEnvironment, CloudAmbientAgentEnvironment};
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::harness_display;
use crate::appearance::Appearance;
use crate::auth::UserUid;
use crate::notebooks::NotebookId;
use crate::send_telemetry_from_ctx;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::AmbientAgentTask;
#[cfg(not(target_family = "wasm"))]
use crate::settings::ai::{AISettings, AISettingsChangedEvent};
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::util::bindings::CustomAction;
use crate::util::time_format::{format_approx_duration_from_now, human_readable_precise_duration};
#[cfg(not(target_family = "wasm"))]
use crate::view_components::action_button::PrimaryTheme;
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};
use crate::view_components::copyable_text_field::{
    render_copyable_text_field, CopyableTextFieldConfig, COPY_FEEDBACK_DURATION,
};
use crate::view_components::DismissibleToast;
use crate::workspace::{ForkedConversationDestination, ToastStack, WorkspaceAction};
use crate::workspaces::user_profiles::UserProfiles;

const FIELD_SPACING: f32 = 16.0;
const HEADER_SPACING: f32 = 12.0;
const STATUS_ICON_SIZE: f32 = 12.0;
const HARNESS_CIRCLE_SIZE: f32 = 16.0;
const HARNESS_ICON_IN_CIRCLE: f32 = 9.0;
const LABEL_VALUE_GAP: f32 = 4.0;
const SECTION_HEADER_GAP: f32 = 8.0;

/// Panel rendering mode.
#[derive(Debug, Clone, PartialEq)]
enum PanelMode {
    Conversation {
        /// Working directory where the conversation took place.
        directory: Option<String>,
        /// Unique identifier for the conversation (server token).
        server_conversation_id: Option<String>,
        /// Internal conversation ID (for action buttons).
        ai_conversation_id: Option<AIConversationId>,
        /// Status of the conversation.
        status: Option<ConversationStatus>,
    },
    Task {
        /// Unique identifier for the task.
        task_id: Option<AmbientAgentTaskId>,
        /// Working directory from the linked conversation, if available.
        directory: Option<String>,
        /// User-visible status derived from task and conversation state.
        display_status: Option<AgentRunDisplayStatus>,
        /// Error message, if we have one.
        error_message: Option<String>,
        /// Environment ID.
        environment_id: Option<String>,
        /// Server conversation ID (for copy link).
        conversation_id: Option<String>,
    },
}

impl Default for PanelMode {
    fn default() -> Self {
        PanelMode::Conversation {
            directory: None,
            server_conversation_id: None,
            ai_conversation_id: None,
            status: None,
        }
    }
}

/// Groups mouse state handles for the panel.
#[derive(Default)]
struct PanelMouseStates {
    close_button: MouseStateHandle,
    copy_directory: MouseStateHandle,
    copy_conversation_id: MouseStateHandle,
    copy_run_id: MouseStateHandle,
    copy_environment_id: MouseStateHandle,
    copy_docker_image: MouseStateHandle,
    copy_fetch_error: MouseStateHandle,
    copy_error: MouseStateHandle,
    copy_setup_commands: MouseStateHandle,
    skill_link: MouseStateHandle,
    skill_source_link: MouseStateHandle,
    executor_agent_link: MouseStateHandle,
}

/// Tracks which copy button action was last triggered (for checkmark feedback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CopyButtonKind {
    Directory,
    ConversationId,
    RunId,
    EnvironmentId,
    DockerImage,
    FetchError,
    Error,
    SetupCommands,
}

/// Information about a principal involved in a conversation.
#[derive(Debug, Clone)]
struct PrincipalInfo {
    /// Display name of the principal (or fallback identifier).
    pub display_name: String,
    /// Optional photo URL for the avatar.
    pub photo_url: Option<String>,
    /// UID of the principal, when known (used for building Oz links).
    pub uid: Option<String>,
    /// Whether this principal is a service account.
    pub is_service_account: bool,
}

impl PrincipalInfo {
    /// Create a new PrincipalInfo with a display name and optional photo URL.
    fn new(display_name: String, photo_url: Option<String>) -> Self {
        Self {
            display_name,
            photo_url,
            uid: None,
            is_service_account: false,
        }
    }

    /// Create a PrincipalInfo with just the first character as a fallback.
    fn from_uid_fallback(uid: &str) -> Self {
        let first_char = uid.chars().next().unwrap_or('?').to_uppercase().to_string();
        Self::new(first_char, None)
    }
}

impl From<&TaskPrincipalInfo> for PrincipalInfo {
    fn from(p: &TaskPrincipalInfo) -> Self {
        Self {
            display_name: p.display_name.clone().unwrap_or_else(|| p.uid.clone()),
            photo_url: None,
            uid: Some(p.uid.clone()),
            is_service_account: PrincipalType::parse(&p.creator_type)
                .is_some_and(|pt| pt.is_service_account()),
        }
    }
}

/// Data model for the conversation details panel.
/// Any field that is left as None will not be rendered.
#[derive(Debug, Clone, Default)]
pub struct ConversationDetailsData {
    mode: PanelMode,
    title: String,
    /// Information about the creator.
    creator: Option<PrincipalInfo>,
    /// Principal the cloud run executed as.
    executor: Option<PrincipalInfo>,
    /// When the conversation was created.
    created_at: Option<DateTime<Local>>,
    /// Total credits spent on the conversation/task.
    credits: Option<f32>,
    /// Total duration of the conversation.
    run_time: Option<Duration>,
    /// Artifacts created during the conversation (plans, PRs, branches).
    artifacts: Vec<Artifact>,
    /// Action to dispatch when "Open" button is clicked.
    open_action: Option<WorkspaceAction>,
    /// Source prompt that initiated this conversation/task.
    source_prompt: Option<String>,
    /// Copy link URL (session link if sandbox running, otherwise conversation link).
    copy_link_url: Option<String>,
    /// Parsed skill spec referenced by the task configuration.
    skill_spec: Option<SkillSpec>,
    /// Execution harness for this conversation/task.
    harness: Option<Harness>,
    /// Error message displayed when the API call to fetch run data failed.
    fetch_error: Option<String>,
}

impl ConversationDetailsData {
    fn directory_for_task(task: &AmbientAgentTask, app: &AppContext) -> Option<String> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let conversation_id = history_model
            .conversation_id_for_agent_id(&task.run_id().to_string())
            .or_else(|| {
                task.conversation_id().and_then(|conversation_id| {
                    history_model.find_conversation_id_by_server_token(
                        &ServerConversationToken::new(conversation_id.to_string()),
                    )
                })
            })?;

        history_model
            .conversation(&conversation_id)
            .and_then(|conversation| conversation.initial_working_directory())
            .or_else(|| {
                history_model
                    .get_conversation_metadata(&conversation_id)
                    .and_then(|metadata| metadata.initial_working_directory.clone())
            })
    }

    /// Build details data from an in-memory `AIConversation`. Used both by the WASM
    /// transcript/shared-session details panel and by the native pane-level details panel
    /// when the active conversation is a local (non-cloud) Warp Agent run.
    pub fn from_conversation(conversation: &AIConversation, app: &AppContext) -> Self {
        let mut directory = None;
        let mut conversation_id = None;

        // Server metadata (creator, timestamps)
        let mut creator = None;
        if let Some(server_metadata) = conversation.server_metadata() {
            if let Some(creator_uid_str) = &server_metadata.metadata.creator_uid {
                let creator_uid = UserUid::new(creator_uid_str);
                let user_profiles = UserProfiles::handle(app).as_ref(app);

                if let Some(profile) = user_profiles.profile_for_uid(creator_uid) {
                    let display_name = profile.displayable_identifier();
                    let photo_url = Some(profile.photo_url.clone()).filter(|url| !url.is_empty());
                    creator = Some(PrincipalInfo::new(display_name, photo_url));
                } else {
                    // Fallback to first character of UID
                    creator = Some(PrincipalInfo::from_uid_fallback(creator_uid_str));
                }
            }

            // Conversation ID (from server token)
            conversation_id = Some(
                server_metadata
                    .server_conversation_token
                    .as_str()
                    .to_string(),
            );
        }

        // Calculate run time from exchanges
        let first_exchange = conversation.first_exchange();
        let last_exchange = conversation.latest_exchange();
        let mut run_time = None;
        let mut created_at = None;
        if let (Some(first), Some(last)) = (first_exchange, last_exchange) {
            if let Some(finish_time) = last.finish_time {
                let duration = finish_time.signed_duration_since(first.start_time);
                if duration.num_seconds() >= 0 {
                    run_time = Some(duration);
                }
            }
            // Created at from first exchange
            created_at = Some(first.start_time);
        }

        // Working directory from first exchange
        if let Some(first_exchange) = first_exchange {
            directory = first_exchange.working_directory.clone();
        }

        let copy_link_url = conversation_id
            .as_ref()
            .map(|id| ServerConversationToken::new(id.clone()).conversation_link());

        let harness = conversation
            .server_metadata()
            .map(|m| Harness::from(m.harness))
            .or(Some(Harness::Oz));

        ConversationDetailsData {
            mode: PanelMode::Conversation {
                directory,
                server_conversation_id: conversation_id,
                ai_conversation_id: None,
                status: Some(conversation.status().clone()),
            },
            title: conversation
                .title()
                .unwrap_or_else(|| "Conversation".to_string()),
            creator,
            executor: None,
            created_at,
            credits: Some(conversation.credits_spent()),
            run_time,
            artifacts: conversation.artifacts().to_vec(),
            open_action: None,
            source_prompt: conversation.initial_query(),
            copy_link_url,
            skill_spec: None,
            harness,
            fetch_error: None,
        }
    }

    pub fn from_task(
        task: &AmbientAgentTask,
        open_action: Option<WorkspaceAction>,
        copy_link_url: Option<String>,
        app: &AppContext,
    ) -> Self {
        let error_message = if task.state.is_failure_like() {
            task.status_message.as_ref().map(|m| m.message.clone())
        } else {
            None
        };

        let environment_id = task
            .agent_config_snapshot
            .as_ref()
            .and_then(|config| config.environment_id.clone());

        let credits = task.credits_used();

        let skill_spec = task
            .agent_config_snapshot
            .as_ref()
            .and_then(|config| config.skill_spec.as_ref())
            .and_then(|spec_str| SkillSpec::from_str(spec_str).ok());

        let harness = task.agent_config_snapshot.as_ref().and_then(|config| {
            config
                .harness
                .as_ref()
                .map(|h| h.harness_type)
                .or(Some(Harness::Oz))
        });

        ConversationDetailsData {
            mode: PanelMode::Task {
                task_id: Some(task.run_id()),
                directory: Self::directory_for_task(task, app),
                display_status: Some(AgentRunDisplayStatus::from_task(task, app)),
                error_message,
                environment_id,
                conversation_id: task.conversation_id().map(str::to_string),
            },
            title: task.title.clone(),
            created_at: Some(task.created_at.with_timezone(&Local)),
            artifacts: task.artifacts.clone(),
            credits,
            run_time: task.run_time(),
            open_action,
            creator: task
                .creator
                .as_ref()
                .filter(|c| c.display_name.is_some())
                .map(PrincipalInfo::from),
            executor: task.executor.as_ref().map(PrincipalInfo::from),
            source_prompt: Some(task.prompt.clone()),
            copy_link_url,
            skill_spec,
            harness,
            fetch_error: None,
        }
    }

    pub fn from_agent_conversation_entry(
        entry: &AgentConversationEntry,
        task: Option<&AmbientAgentTask>,
        open_action: Option<WorkspaceAction>,
        copy_link_url: Option<String>,
    ) -> Self {
        let creator = entry
            .display
            .creator
            .name
            .clone()
            .map(|name| PrincipalInfo::new(name, None));
        let executor = entry.display.executor.as_ref().and_then(|e| {
            let display_name = e.name.clone().or_else(|| e.uid.clone())?;
            Some(PrincipalInfo {
                display_name,
                photo_url: None,
                uid: e.uid.clone(),
                is_service_account: e.principal_type.is_some_and(|pt| pt.is_service_account()),
            })
        });
        let created_at = Some(entry.display.created_at.with_timezone(&Local));
        let source_prompt = entry.display.initial_query.clone();
        let harness = entry.display.harness;

        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            let error_message = task.and_then(|task| {
                task.state
                    .is_failure_like()
                    .then(|| task.status_message.as_ref().map(|m| m.message.clone()))
                    .flatten()
            });
            // Fall back to the entry's denormalized total when the task record isn't
            // currently loaded, so the panel stays consistent with the card metadata
            // (which always reads `entry.display.request_usage`).
            let credits = task
                .and_then(AmbientAgentTask::credits_used)
                .or(entry.display.request_usage);
            let skill_spec = task
                .and_then(|task| task.agent_config_snapshot.as_ref())
                .and_then(|config| config.skill_spec.as_ref())
                .and_then(|spec_str| SkillSpec::from_str(spec_str).ok());

            return ConversationDetailsData {
                mode: PanelMode::Task {
                    task_id: Some(task_id),
                    directory: entry.display.working_directory.clone(),
                    display_status: Some(entry.display.status.clone()),
                    error_message,
                    environment_id: entry.display.environment_id.clone(),
                    conversation_id: entry
                        .identity
                        .server_conversation_token
                        .as_ref()
                        .map(|token| token.as_str().to_string()),
                },
                title: entry.display.title.clone(),
                creator,
                executor,
                created_at,
                credits,
                run_time: task.and_then(AmbientAgentTask::run_time),
                artifacts: entry.display.artifacts.clone(),
                open_action,
                source_prompt,
                copy_link_url,
                skill_spec,
                harness,
                fetch_error: None,
            };
        }

        ConversationDetailsData {
            mode: PanelMode::Conversation {
                directory: entry.display.working_directory.clone(),
                server_conversation_id: entry
                    .identity
                    .server_conversation_token
                    .as_ref()
                    .map(|token| token.as_str().to_string()),
                ai_conversation_id: entry.identity.local_conversation_id,
                status: Some(entry.display.status.to_conversation_status()),
            },
            title: entry.display.title.clone(),
            creator,
            executor: None,
            created_at,
            credits: entry.display.request_usage,
            run_time: None,
            artifacts: entry.display.artifacts.clone(),
            open_action,
            source_prompt,
            copy_link_url,
            skill_spec: None,
            harness,
            fetch_error: None,
        }
    }

    /// Minimal details data for when we only know the task id (e.g. shared sessions)
    /// but have not loaded the full `AmbientAgentTask` yet.
    pub fn from_task_id(task_id: AmbientAgentTaskId, fetch_error: Option<String>) -> Self {
        ConversationDetailsData {
            mode: PanelMode::Task {
                task_id: Some(task_id),
                directory: None,
                display_status: None,
                error_message: None,
                environment_id: None,
                conversation_id: None,
            },
            title: "Cloud agent run".to_string(),
            creator: None,
            executor: None,
            created_at: None,
            credits: None,
            run_time: None,
            artifacts: vec![],
            open_action: None,
            source_prompt: None,
            copy_link_url: None,
            skill_spec: None,
            harness: None,
            fetch_error,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    /// Used to populate the details panel from the management view, where we don't always have access
    /// to the full `AIConversation`.
    pub fn from_conversation_metadata(
        ai_conversation_id: AIConversationId,
        title: String,
        creator_name: Option<String>,
        created_at: DateTime<Local>,
        directory: Option<String>,
        credits_used: Option<f32>,
        conversation_id: Option<String>,
        artifacts: Vec<Artifact>,
        open_action: Option<WorkspaceAction>,
        status: Option<ConversationStatus>,
        initial_query: Option<String>,
        copy_link_url: Option<String>,
        harness: Option<Harness>,
    ) -> Self {
        ConversationDetailsData {
            mode: PanelMode::Conversation {
                directory,
                server_conversation_id: conversation_id,
                ai_conversation_id: Some(ai_conversation_id),
                status,
            },
            title,
            creator: creator_name.map(|name| PrincipalInfo::new(name, None)),
            executor: None,
            created_at: Some(created_at),
            credits: credits_used,
            run_time: None,
            open_action,
            artifacts,
            source_prompt: initial_query,
            copy_link_url,
            skill_spec: None,
            harness,
            fetch_error: None,
        }
    }
}

/// Events emitted by the ConversationDetailsPanel.
#[derive(Debug, Clone)]
pub enum ConversationDetailsPanelEvent {
    Close,
    OpenPlanNotebook { notebook_uid: NotebookId },
}

/// Actions for the ConversationDetailsPanel.
#[derive(Debug, Clone)]
pub enum ConversationDetailsPanelAction {
    Close,
    CopyDirectory,
    CopyConversationId,
    CopyRunId,
    CopyEnvironmentId,
    CopyDockerImage,
    CopyFetchError,
    CopyError,
    CopySetupCommands(String),
    Focus,
    CopySelectedText,
    #[cfg(not(target_family = "wasm"))]
    ContinueLocally,
    OpenInOz,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::custom(
        CustomAction::Copy,
        ConversationDetailsPanelAction::CopySelectedText,
        "Copy",
        id!(ConversationDetailsPanel::ui_name()) & !id!("IMEOpen"),
    )]);
}

/// A reusable panel for displaying conversation details and metadata.
pub struct ConversationDetailsPanel {
    data: ConversationDetailsData,
    mouse_states: PanelMouseStates,
    artifact_buttons_row: ViewHandle<ArtifactButtonsRow>,
    resizable_state_handle: ResizableStateHandle,
    scroll_state: ClippedScrollStateHandle,
    action_buttons: ViewHandle<ConversationActionButtonsRow>,
    /// Whether to show the "Open conversation" button (we don't want to show a navigate to
    /// conversation button in the transcript view, but do in the management details view).
    show_open_button: bool,
    #[cfg(not(target_family = "wasm"))]
    continue_locally_button: ViewHandle<ActionButton>,
    /// Text button "View in Oz" shown next to "Continue locally".
    open_in_oz_button: ViewHandle<ActionButton>,
    /// Tracks when each copy button was last clicked (for checkmark feedback).
    copy_feedback_times: HashMap<CopyButtonKind, Instant>,
    /// Selection state for cmd+C copy.
    selection_handle: SelectionHandle,
    selected_text: Arc<RwLock<Option<String>>>,
}

impl ConversationDetailsPanel {
    /// Create a new panel.
    /// - `show_open_button`: whether to show the "Open" button (management view: true, transcript: false)
    /// - `initial_width`: starting width of the panel in pixels
    pub fn new(show_open_button: bool, initial_width: f32, ctx: &mut ViewContext<Self>) -> Self {
        let artifact_buttons_row =
            ctx.add_typed_action_view(|ctx| ArtifactButtonsRow::new(&[], ctx));
        ctx.subscribe_to_view(&artifact_buttons_row, |this, _, event, ctx| {
            this.handle_artifact_buttons_event(event, ctx)
        });

        let action_buttons = ctx.add_typed_action_view(ConversationActionButtonsRow::new);
        ctx.subscribe_to_view(&action_buttons, Self::handle_action_buttons_event);

        #[cfg(not(target_family = "wasm"))]
        let continue_locally_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Continue locally", PrimaryTheme)
                .with_tooltip("Fork this conversation locally")
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ConversationDetailsPanelAction::ContinueLocally);
                })
        });
        let open_in_oz_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("View in Oz", SecondaryTheme)
                .with_tooltip("View this run in the Oz web app")
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ConversationDetailsPanelAction::OpenInOz);
                })
        });
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::IsAnyAIEnabled { .. }) {
                ctx.notify();
            }
        });

        Self {
            data: ConversationDetailsData::default(),
            mouse_states: PanelMouseStates::default(),
            artifact_buttons_row,
            action_buttons,
            show_open_button,
            #[cfg(not(target_family = "wasm"))]
            continue_locally_button,
            open_in_oz_button,
            resizable_state_handle: resizable_state_handle(initial_width),
            scroll_state: ClippedScrollStateHandle::default(),
            copy_feedback_times: HashMap::new(),
            selection_handle: SelectionHandle::default(),
            selected_text: Default::default(),
        }
    }

    pub fn set_conversation_details(
        &mut self,
        data: ConversationDetailsData,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_artifacts(&data, ctx);
        self.set_action_buttons(&data, ctx);
        self.data = data;
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn task_display_status_for_test(&self) -> Option<AgentRunDisplayStatus> {
        match &self.data.mode {
            PanelMode::Task { display_status, .. } => display_status.clone(),
            PanelMode::Conversation { .. } => None,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn continue_locally_conversation_id(&self, app: &AppContext) -> Option<AIConversationId> {
        if !AISettings::as_ref(app).is_any_ai_enabled(app) {
            return None;
        }

        match &self.data.mode {
            PanelMode::Conversation {
                ai_conversation_id,
                status,
                ..
            } => {
                let status = status.as_ref()?;
                if status.is_in_progress() {
                    return None;
                }
                Some(*ai_conversation_id.as_ref()?)
            }
            PanelMode::Task {
                display_status,
                conversation_id,
                ..
            } => {
                let status = display_status.as_ref()?;
                if status.is_working() {
                    return None;
                }
                // Hide for non-Oz harnesses (e.g. Claude, Gemini): they can't be
                // forked into a local Warp conversation.
                if matches!(self.data.harness, Some(h) if h != Harness::Oz) {
                    return None;
                }

                let server_token = ServerConversationToken::new(conversation_id.as_ref()?.clone());
                BlocklistAIHistoryModel::as_ref(app)
                    .find_conversation_id_by_server_token(&server_token)
            }
        }
    }

    fn set_artifacts(&mut self, data: &ConversationDetailsData, ctx: &mut ViewContext<Self>) {
        self.artifact_buttons_row.update(ctx, |view, ctx| {
            view.update_artifacts(&data.artifacts, ctx);
        });
    }

    fn handle_artifact_buttons_event(
        &mut self,
        event: &ArtifactButtonsRowEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ArtifactButtonsRowEvent::OpenPlan { notebook_uid } => {
                ctx.emit(ConversationDetailsPanelEvent::OpenPlanNotebook {
                    notebook_uid: *notebook_uid,
                });
            }
            ArtifactButtonsRowEvent::CopyBranch { branch } => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(branch.clone()));

                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::default("Copied branch name".to_string());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            ArtifactButtonsRowEvent::OpenPullRequest { url } => {
                ctx.open_url(url);
            }
            ArtifactButtonsRowEvent::ViewScreenshots { artifact_uids } => {
                crate::ai::artifacts::open_screenshot_lightbox(artifact_uids, ctx);
            }
            ArtifactButtonsRowEvent::DownloadFile { artifact_uid } => {
                crate::ai::artifacts::download_file_artifact(artifact_uid, ctx);
            }
        }
    }

    /// Builds the Oz web UI URL for a task, if a task_id is available.
    fn oz_run_url(data: &ConversationDetailsData) -> Option<String> {
        if let PanelMode::Task {
            task_id: Some(task_id),
            ..
        } = &data.mode
        {
            let oz_root_url = ChannelState::oz_root_url();
            Some(format!("{oz_root_url}/runs/{task_id}"))
        } else {
            None
        }
    }

    fn action_buttons_config_from_data(
        &self,
        data: &ConversationDetailsData,
    ) -> Option<ActionButtonsConfig> {
        let open_action = self
            .show_open_button
            .then(|| data.open_action.clone())
            .flatten();
        match &data.mode {
            PanelMode::Task {
                task_id,
                display_status,
                ..
            } => {
                let task_id = *task_id.as_ref()?;
                let display_status = display_status.as_ref()?;
                Some(ActionButtonsConfig::for_task(
                    task_id,
                    display_status,
                    open_action,
                    data.copy_link_url.clone(),
                ))
            }
            PanelMode::Conversation {
                ai_conversation_id, ..
            } => {
                let conversation_id = *ai_conversation_id.as_ref()?;
                Some(ActionButtonsConfig::for_conversation(
                    conversation_id,
                    open_action,
                    data.copy_link_url.clone(),
                ))
            }
        }
    }

    fn set_action_buttons(&mut self, data: &ConversationDetailsData, ctx: &mut ViewContext<Self>) {
        let config = self
            .action_buttons_config_from_data(data)
            .unwrap_or_default();
        self.action_buttons
            .update(ctx, |row, ctx| row.set_config(config, ctx));
    }

    fn handle_action_buttons_event(
        &mut self,
        _: ViewHandle<ConversationActionButtonsRow>,
        event: &AgentDetailsButtonEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentDetailsButtonEvent::Open => {
                // Send telemetry based on panel mode
                match &self.data.mode {
                    PanelMode::Conversation {
                        ai_conversation_id: Some(conversation_id),
                        ..
                    } => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::ConversationOpened {
                                conversation_id: conversation_id.to_string(),
                                opened_from: OpenedFrom::DetailsPanel,
                            },
                            ctx
                        );
                    }
                    PanelMode::Task {
                        task_id: Some(task_id),
                        ..
                    } => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::CloudRunOpened {
                                task_id: task_id.to_string(),
                                opened_from: OpenedFrom::DetailsPanel,
                            },
                            ctx
                        );
                    }
                    _ => {}
                }

                if let Some(action) = &self.data.open_action {
                    ctx.dispatch_typed_action(action);
                }
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
            AgentDetailsButtonEvent::ViewDetails { .. } => {
                // ViewDetails not shown in the details panel because we're already viewing it,
                // only in management view cards
            }
            AgentDetailsButtonEvent::CopyLink { link } => {
                match &self.data.mode {
                    PanelMode::Conversation {
                        ai_conversation_id: Some(conversation_id),
                        ..
                    } => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::ConversationLinkCopied {
                                conversation_id: conversation_id.to_string(),
                                copied_from: OpenedFrom::DetailsPanel,
                            },
                            ctx
                        );
                    }
                    PanelMode::Task {
                        task_id: Some(task_id),
                        ..
                    } => {
                        send_telemetry_from_ctx!(
                            AgentManagementTelemetryEvent::SessionLinkCopied {
                                task_id: task_id.to_string(),
                                copied_from: OpenedFrom::DetailsPanel,
                            },
                            ctx
                        );
                    }
                    _ => {}
                }

                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.clone()));
            }
        }
    }

    fn render_creator_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let creator = self.data.creator.as_ref()?;
        let created_at = self.data.created_at?;
        let theme = appearance.theme();

        let ui_font_size = appearance.ui_font_size();
        let small_font_size = ui_font_size - 2.;

        let avatar_content = creator
            .photo_url
            .as_ref()
            .map(|url| AvatarContent::Image {
                url: url.clone(),
                display_name: creator.display_name.clone(),
            })
            .unwrap_or_else(|| AvatarContent::DisplayName(creator.display_name.clone()));
        let avatar = Avatar::new(
            avatar_content,
            warpui::ui_components::components::UiComponentStyles {
                width: Some(20.),
                height: Some(20.),
                border_radius: Some(warpui::elements::CornerRadius::with_all(
                    warpui::elements::Radius::Percentage(50.),
                )),
                background: Some(blended_colors::accent(theme).into()),
                font_color: Some(ColorU::black()),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(warpui::fonts::Weight::Bold),
                font_size: Some(small_font_size),
                ..Default::default()
            },
        )
        .build()
        .finish();

        let created_text = Text::new(
            format!(
                "Created by {} • {}",
                creator.display_name,
                format_approx_duration_from_now(created_at)
            ),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .with_selectable(true)
        .finish();

        Some(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(avatar)
                        .with_margin_right(LABEL_VALUE_GAP)
                        .finish(),
                )
                .with_child(Expanded::new(1., created_text).finish())
                .finish(),
        )
    }

    fn render_executor_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let executor = self.data.executor.as_ref()?;
        if !executor.is_service_account {
            return None;
        }
        // Hide when the executor is the same person as the creator.
        if self
            .data
            .creator
            .as_ref()
            .is_some_and(|c| match (&c.uid, &executor.uid) {
                (Some(c_uid), Some(e_uid)) => c_uid == e_uid,
                _ => c.display_name == executor.display_name,
            })
        {
            return None;
        }
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(
            "Agent".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        let agent_name_element = if let Some(uid) = &executor.uid {
            let oz_root_url = ChannelState::oz_root_url();
            let agent_url = format!("{oz_root_url}/agents/{}", urlencoding::encode(uid));
            appearance
                .ui_builder()
                .link(
                    executor.display_name.clone(),
                    Some(agent_url),
                    None,
                    self.mouse_states.executor_agent_link.clone(),
                )
                .build()
                .finish()
        } else {
            Text::new(
                executor.display_name.clone(),
                appearance.ui_font_family(),
                ui_font_size,
            )
            .with_color(theme.foreground().into())
            .with_selectable(true)
            .finish()
        };

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(label_text)
                        .with_margin_bottom(LABEL_VALUE_GAP)
                        .finish(),
                )
                .with_child(agent_name_element)
                .finish(),
        )
    }

    fn render_error_field(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let error_message = match &self.data.mode {
            PanelMode::Task { error_message, .. } => error_message.as_ref()?,
            _ => return None,
        };
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(
            "Error".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        let value_field = render_copyable_text_field(
            CopyableTextFieldConfig::new(error_message.clone())
                .with_font_size(ui_font_size)
                .with_text_color(theme.ansi_fg_red())
                .with_wrap_text(true)
                .with_icon_size(16.)
                .with_mouse_state(self.mouse_state_for_copy_button(CopyButtonKind::Error))
                .with_last_copied_at(self.copy_feedback_times.get(&CopyButtonKind::Error)),
            |ctx| {
                ctx.dispatch_typed_action(ConversationDetailsPanelAction::CopyError);
            },
            app,
        );

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(label_text)
                        .with_margin_bottom(LABEL_VALUE_GAP)
                        .finish(),
                )
                .with_child(value_field)
                .finish(),
        )
    }

    fn render_status_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        // Section header
        let header = Text::new(
            "Status".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let (icon, color, display_text): (Icon, _, String) = match &self.data.mode {
            PanelMode::Task { display_status, .. } => {
                let status = display_status.as_ref()?;
                let (icon, color) = status.status_icon_and_color(theme);
                (icon, color, status.to_string())
            }
            PanelMode::Conversation { status, .. } => {
                let status = status.as_ref()?;
                let (icon, color) = status.status_icon_and_color(theme, StatusColorStyle::Standard);
                (icon, color, status.to_string())
            }
        };

        let status_icon = ConstrainedBox::new(icon.to_warpui_icon(color.into()).finish())
            .with_width(STATUS_ICON_SIZE)
            .with_height(STATUS_ICON_SIZE)
            .finish();

        let status_text = Text::new(display_text, appearance.ui_font_family(), ui_font_size)
            .with_color(color)
            .with_selectable(true)
            .finish();

        let status_badge = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Container::new(status_icon).with_margin_right(4.).finish())
                .with_child(status_text)
                .finish(),
        )
        .with_uniform_padding(4.)
        .with_background(coloru_with_opacity(color, 10))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(header)
                        .with_margin_bottom(SECTION_HEADER_GAP)
                        .finish(),
                )
                .with_child(status_badge)
                .finish(),
        )
    }

    fn render_harness_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let availability = HarnessAvailabilityModel::as_ref(app);
        if !availability.should_show_harness_selector() {
            return None;
        }
        let harness = self.data.harness?;
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(
            "Harness".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        let circle_bg = harness_display::circle_background(harness, theme);
        let icon_fill = harness_display::icon_fill_on_circle(harness, theme);
        let icon_glyph = ConstrainedBox::new(
            harness_display::icon_for(harness)
                .to_warpui_icon(icon_fill)
                .finish(),
        )
        .with_width(HARNESS_ICON_IN_CIRCLE)
        .with_height(HARNESS_ICON_IN_CIRCLE)
        .finish();
        let icon_padding = (HARNESS_CIRCLE_SIZE - HARNESS_ICON_IN_CIRCLE) / 2.;
        let icon = Container::new(icon_glyph)
            .with_uniform_padding(icon_padding)
            .with_background(circle_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                HARNESS_CIRCLE_SIZE / 2.,
            )))
            .finish();

        let name = Text::new(
            availability.display_name_for(harness).to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(theme.foreground().into())
        .with_selectable(true)
        .finish();

        let value_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(icon).with_margin_right(4.).finish())
            .with_child(name)
            .finish();

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(label_text)
                        .with_margin_bottom(LABEL_VALUE_GAP)
                        .finish(),
                )
                .with_child(value_row)
                .finish(),
        )
    }

    /// Renders the primary skill that this conversation ran.
    fn render_skill_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let skill_spec = self.data.skill_spec.as_ref()?;
        let skill_name = skill_spec.skill_name();
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();
        let sub_color = blended_colors::text_sub(theme, theme.surface_1());

        let icon = ConstrainedBox::new(Icon::Warp.to_warpui_icon(theme.foreground()).finish())
            .with_width(20.)
            .with_height(20.)
            .finish();

        let skill_name_text = Text::new(
            format!("/{skill_name}"),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(sub_color)
        .with_selectable(true)
        .finish();

        let oz_root_url = ChannelState::oz_root_url();
        let encoded_skill_name = urlencoding::encode(&skill_name);
        let skill_url = format!("{oz_root_url}/agents/{encoded_skill_name}");

        let oz_link = appearance
            .ui_builder()
            .link(
                "Open in Oz".to_string(),
                Some(skill_url),
                None,
                self.mouse_states.skill_link.clone(),
            )
            .build()
            .finish();

        let separator = || {
            Container::new(
                Text::new("•".to_string(), appearance.ui_font_family(), ui_font_size)
                    .with_color(sub_color)
                    .finish(),
            )
            .with_margin_left(4.)
            .with_margin_right(4.)
            .finish()
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(icon).with_margin_right(4.).finish())
            .with_child(Shrinkable::new(1., skill_name_text).finish())
            .with_child(separator())
            .with_child(Shrinkable::new(1., oz_link).finish());

        // Add GitHub source link if we have enough info to construct it.
        if let (Some(org), Some(repo)) = (&skill_spec.org, &skill_spec.repo) {
            if skill_spec.is_full_path() {
                let github_url = format!(
                    "https://github.com/{}/{}/blob/-/{}",
                    org, repo, skill_spec.skill_identifier
                );
                let source_link = appearance
                    .ui_builder()
                    .link(
                        "Open in GitHub".to_string(),
                        Some(github_url),
                        None,
                        self.mouse_states.skill_source_link.clone(),
                    )
                    .build()
                    .finish();
                row.add_child(separator());
                row.add_child(Shrinkable::new(1., source_link).finish());
            }
        }

        Some(row.finish())
    }

    fn render_source_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let source_prompt = self.data.source_prompt.as_ref()?;
        let trimmed = source_prompt.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(self.render_simple_field("Initial query", trimmed, appearance))
    }

    fn render_artifacts_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if self.data.artifacts.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(
            "Artifacts".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(label_text)
                        .with_margin_bottom(SECTION_HEADER_GAP)
                        .finish(),
                )
                .with_child(ChildView::new(&self.artifact_buttons_row).finish())
                .finish(),
        )
    }

    fn format_setup_commands_for_copy(commands: &[String]) -> String {
        let wrapped: Vec<String> = commands.iter().map(|cmd| format!("({cmd})")).collect();
        wrapped.join(" && \n")
    }

    fn render_setup_commands_section(
        &self,
        setup_commands: &[String],
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if setup_commands.is_empty() {
            return None;
        }

        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let header_text = Text::new(
            "Environment setup commands".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        let commands_text = setup_commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| format!("{}. {cmd}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        let field = render_copyable_text_field(
            CopyableTextFieldConfig::new(commands_text)
                .with_font_size(ui_font_size)
                .with_text_color(theme.foreground().into())
                .with_icon_size(16.)
                .with_wrap_text(true)
                .with_mouse_state(self.mouse_state_for_copy_button(CopyButtonKind::SetupCommands))
                .with_last_copied_at(self.copy_feedback_times.get(&CopyButtonKind::SetupCommands)),
            {
                let copy_text = Self::format_setup_commands_for_copy(setup_commands);
                move |ctx| {
                    ctx.dispatch_typed_action(ConversationDetailsPanelAction::CopySetupCommands(
                        copy_text.clone(),
                    ));
                }
            },
            app,
        );

        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(header_text)
                        .with_margin_bottom(SECTION_HEADER_GAP)
                        .finish(),
                )
                .with_child(
                    Container::new(field)
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                )
                .finish(),
        )
    }

    fn render_environment_section(
        &self,
        environment_id: &str,
        env_model: &AmbientAgentEnvironment,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let environment_name = &env_model.name;
        let docker_image = env_model.base_image.to_string();

        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        // Section header
        let header = Text::new(
            "Environment details".to_string(),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        let mut section = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        section.add_child(
            Container::new(header)
                .with_margin_bottom(LABEL_VALUE_GAP)
                .finish(),
        );

        // Helper to render a copyable field with "Label: Value" format
        let render_copyable_field =
            |label: &str,
             value: &str,
             copy_button_kind: CopyButtonKind,
             action: ConversationDetailsPanelAction| {
                render_copyable_text_field(
                    CopyableTextFieldConfig::new(format!("{label}: {value}"))
                        .with_font_size(ui_font_size)
                        .with_text_color(theme.foreground().into())
                        .with_icon_size(16.)
                        .with_mouse_state(self.mouse_state_for_copy_button(copy_button_kind))
                        .with_last_copied_at(self.copy_feedback_times.get(&copy_button_kind)),
                    move |ctx| {
                        ctx.dispatch_typed_action(action.clone());
                    },
                    app,
                )
            };

        let name_text = Text::new(
            format!("Name: {environment_name}"),
            appearance.ui_font_family(),
            ui_font_size,
        )
        .with_color(theme.foreground().into())
        .with_selectable(true)
        .finish();
        section.add_child(
            Container::new(name_text)
                .with_vertical_padding(4.)
                .with_margin_bottom(LABEL_VALUE_GAP)
                .finish(),
        );

        section.add_child(
            Container::new(render_copyable_field(
                "ID",
                environment_id,
                CopyButtonKind::EnvironmentId,
                ConversationDetailsPanelAction::CopyEnvironmentId,
            ))
            .with_margin_bottom(LABEL_VALUE_GAP)
            .finish(),
        );

        section.add_child(
            Container::new(render_copyable_field(
                "Image",
                &docker_image,
                CopyButtonKind::DockerImage,
                ConversationDetailsPanelAction::CopyDockerImage,
            ))
            .with_margin_bottom(LABEL_VALUE_GAP)
            .finish(),
        );

        Container::new(section.finish())
            .with_margin_bottom(FIELD_SPACING)
            .finish()
    }

    // Render a simple field with a button to copy the field's contents.
    fn render_field_with_copy(
        &self,
        label: &str,
        value: &str,
        action: ConversationDetailsPanelAction,
        copy_button_kind: CopyButtonKind,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(label.to_string(), appearance.ui_font_family(), ui_font_size)
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .finish();

        let value_field = render_copyable_text_field(
            CopyableTextFieldConfig::new(value.to_string())
                .with_font_size(ui_font_size)
                .with_text_color(theme.foreground().into())
                .with_icon_size(16.)
                .with_mouse_state(self.mouse_state_for_copy_button(copy_button_kind))
                .with_last_copied_at(self.copy_feedback_times.get(&copy_button_kind)),
            move |ctx| {
                ctx.dispatch_typed_action(action.clone());
            },
            app,
        );

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Container::new(label_text)
                    .with_margin_bottom(LABEL_VALUE_GAP)
                    .finish(),
            )
            .with_child(value_field)
            .finish()
    }

    fn render_simple_field(
        &self,
        label: &str,
        value: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_font_size = appearance.ui_font_size();

        let label_text = Text::new(label.to_string(), appearance.ui_font_family(), ui_font_size)
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .finish();

        let value_text = Text::new(value.to_string(), appearance.ui_font_family(), ui_font_size)
            .with_color(theme.foreground().into())
            .with_selectable(true)
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Container::new(label_text)
                    .with_margin_bottom(LABEL_VALUE_GAP)
                    .finish(),
            )
            .with_child(value_text)
            .finish()
    }

    /// Returns the mouse state handle for the given copy button kind.
    fn mouse_state_for_copy_button(&self, kind: CopyButtonKind) -> MouseStateHandle {
        match kind {
            CopyButtonKind::Directory => self.mouse_states.copy_directory.clone(),
            CopyButtonKind::ConversationId => self.mouse_states.copy_conversation_id.clone(),
            CopyButtonKind::RunId => self.mouse_states.copy_run_id.clone(),
            CopyButtonKind::EnvironmentId => self.mouse_states.copy_environment_id.clone(),
            CopyButtonKind::DockerImage => self.mouse_states.copy_docker_image.clone(),
            CopyButtonKind::FetchError => self.mouse_states.copy_fetch_error.clone(),
            CopyButtonKind::Error => self.mouse_states.copy_error.clone(),
            CopyButtonKind::SetupCommands => self.mouse_states.copy_setup_commands.clone(),
        }
    }

    /// Records a copy action and schedules re-render to clear checkmark.
    fn record_copy(&mut self, kind: CopyButtonKind, ctx: &mut ViewContext<Self>) {
        self.copy_feedback_times.insert(kind, Instant::now());
        let duration = COPY_FEEDBACK_DURATION;
        ctx.spawn(
            async move {
                warpui::r#async::Timer::after(duration).await;
            },
            |me, _, ctx| {
                ctx.notify();
                me.copy_feedback_times
                    .retain(|_, time| time.elapsed() < COPY_FEEDBACK_DURATION);
            },
        );
        ctx.notify();
    }
}

impl View for ConversationDetailsPanel {
    fn ui_name() -> &'static str {
        "ConversationDetailsPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        // Header row with optional action buttons and close button
        let close_button = icon_button(
            appearance,
            Icon::X,
            false,
            self.mouse_states.close_button.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ConversationDetailsPanelAction::Close);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let mut header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        // Add continue locally button (left-aligned) and action icon buttons (right-aligned).
        let has_action_buttons = !self.action_buttons.as_ref(app).is_empty();

        #[cfg(not(target_family = "wasm"))]
        let has_continue_locally = self.continue_locally_conversation_id(app).is_some();
        #[cfg(target_family = "wasm")]
        let has_continue_locally = false;
        let has_oz_url = Self::oz_run_url(&self.data).is_some();

        if has_continue_locally || has_oz_url {
            let mut buttons_wrap = Wrap::row().with_spacing(8.).with_run_spacing(8.);

            #[cfg(not(target_family = "wasm"))]
            if has_continue_locally {
                buttons_wrap.add_child(ChildView::new(&self.continue_locally_button).finish());
            }
            if has_oz_url {
                buttons_wrap.add_child(ChildView::new(&self.open_in_oz_button).finish());
            }

            header_row.add_child(
                Expanded::new(
                    1.,
                    Container::new(buttons_wrap.finish())
                        .with_margin_right(8.)
                        .finish(),
                )
                .finish(),
            );
        }

        if has_action_buttons {
            header_row.add_child(ChildView::new(&self.action_buttons).finish());
            // Vertical divider between action buttons and close button
            header_row.add_child(
                Container::new(
                    ConstrainedBox::new(
                        Container::new(Empty::new().finish())
                            .with_border(Border::left(1.).with_border_fill(theme.outline()))
                            .finish(),
                    )
                    .with_height(16.)
                    .finish(),
                )
                .with_margin_left(8.)
                .with_margin_right(4.)
                .finish(),
            );
        }

        header_row.add_child(close_button);
        content.add_child(
            Container::new(header_row.finish())
                .with_margin_bottom(HEADER_SPACING)
                .finish(),
        );

        // Title
        let ui_font_size = appearance.ui_font_size();
        let title_font_size = ui_font_size + 2.;
        let skill_section = self.render_skill_section(appearance);
        let title_margin = if skill_section.is_some() {
            LABEL_VALUE_GAP
        } else {
            HEADER_SPACING
        };
        let title = Text::new(
            self.data.title.clone(),
            appearance.ui_font_family(),
            title_font_size,
        )
        .with_color(theme.foreground().into())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();
        content.add_child(
            Container::new(title)
                .with_margin_bottom(title_margin)
                .finish(),
        );

        // Skill section
        if let Some(skill_section) = skill_section {
            content.add_child(
                Container::new(skill_section)
                    .with_margin_bottom(HEADER_SPACING)
                    .finish(),
            );
        }

        // Creator section
        if let Some(creator_section) = self.render_creator_section(appearance) {
            content.add_child(
                Container::new(creator_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        // Divider
        content.add_child(
            Container::new(
                Container::new(Empty::new().finish())
                    .with_border(Border::top(1.).with_border_fill(blended_colors::neutral_2(theme)))
                    .finish(),
            )
            .with_margin_bottom(FIELD_SPACING)
            .finish(),
        );

        // Fetch error banner (shown when the API call to load run data failed)
        if let Some(fetch_error) = &self.data.fetch_error {
            let error_icon = ConstrainedBox::new(
                Icon::Triangle
                    .to_warpui_icon(theme.ansi_fg_red().into())
                    .finish(),
            )
            .with_width(STATUS_ICON_SIZE)
            .with_height(STATUS_ICON_SIZE)
            .finish();
            let error_text = render_copyable_text_field(
                CopyableTextFieldConfig::new(fetch_error.clone())
                    .with_font_size(ui_font_size)
                    .with_text_color(theme.ansi_fg_red())
                    .with_wrap_text(true)
                    .with_icon_size(16.)
                    .with_mouse_state(self.mouse_state_for_copy_button(CopyButtonKind::FetchError))
                    .with_last_copied_at(self.copy_feedback_times.get(&CopyButtonKind::FetchError))
                    .with_cross_axis_alignment(CrossAxisAlignment::Start),
                |ctx| {
                    ctx.dispatch_typed_action(ConversationDetailsPanelAction::CopyFetchError);
                },
                app,
            );
            let error_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(Container::new(error_icon).with_margin_right(4.).finish())
                .with_child(Expanded::new(1., error_text).finish())
                .finish();
            let error_banner = Container::new(error_row)
                .with_uniform_padding(8.)
                .with_background(coloru_with_opacity(theme.ansi_fg_red(), 10))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish();
            content.add_child(
                Container::new(error_banner)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        // Status section
        if let Some(status_section) = self.render_status_section(appearance) {
            content.add_child(
                Container::new(status_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        // Executor section
        if let Some(executor_section) = self.render_executor_section(appearance) {
            content.add_child(
                Container::new(executor_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        if let Some(harness_section) = self.render_harness_section(appearance, app) {
            content.add_child(
                Container::new(harness_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        if let Some(artifacts_section) = self.render_artifacts_section(appearance) {
            content.add_child(
                Container::new(artifacts_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        // Mode-specific fields
        match &self.data.mode {
            PanelMode::Conversation {
                directory,
                server_conversation_id: conversation_id,
                ai_conversation_id: _,
                status: _,
            } => {
                if let Some(directory) = directory {
                    content.add_child(
                        Container::new(self.render_field_with_copy(
                            "Directory",
                            directory,
                            ConversationDetailsPanelAction::CopyDirectory,
                            CopyButtonKind::Directory,
                            appearance,
                            app,
                        ))
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                    );
                }

                if let Some(id) = conversation_id {
                    content.add_child(
                        Container::new(self.render_field_with_copy(
                            "Conversation ID",
                            id,
                            ConversationDetailsPanelAction::CopyConversationId,
                            CopyButtonKind::ConversationId,
                            appearance,
                            app,
                        ))
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                    );
                }
            }
            PanelMode::Task {
                directory, task_id, ..
            } => {
                if let Some(directory) = directory {
                    content.add_child(
                        Container::new(self.render_field_with_copy(
                            "Directory",
                            directory,
                            ConversationDetailsPanelAction::CopyDirectory,
                            CopyButtonKind::Directory,
                            appearance,
                            app,
                        ))
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                    );
                }
                if let Some(task_id) = task_id {
                    content.add_child(
                        Container::new(self.render_field_with_copy(
                            "Run ID",
                            &task_id.to_string(),
                            ConversationDetailsPanelAction::CopyRunId,
                            CopyButtonKind::RunId,
                            appearance,
                            app,
                        ))
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                    );
                }
            }
        }

        if let Some(credits) = self.data.credits {
            let formatted = format!("{credits:.1}");
            content.add_child(
                Container::new(self.render_simple_field("Credits used", &formatted, appearance))
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        if let Some(duration) = self.data.run_time {
            let formatted = human_readable_precise_duration(duration);
            content.add_child(
                Container::new(self.render_simple_field("Run time", &formatted, appearance))
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        if let Some(created_at) = self.data.created_at {
            let formatted = created_at.format("%I:%M %p on %-m/%-d/%Y").to_string();
            content.add_child(
                Container::new(self.render_simple_field("Created on", &formatted, appearance))
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        // Task-only fields
        if let PanelMode::Task { environment_id, .. } = &self.data.mode {
            if let Some((eid, env)) = environment_id.as_deref().and_then(|eid| {
                let server_id = ServerId::try_from(eid).ok()?;
                let sync_id = SyncId::ServerId(server_id);
                let env = CloudAmbientAgentEnvironment::get_by_id(&sync_id, app).cloned()?;
                Some((eid, env))
            }) {
                let env_model = &env.model().string_model;
                content.add_child(self.render_environment_section(eid, env_model, appearance, app));

                if let Some(setup_commands_section) =
                    self.render_setup_commands_section(&env_model.setup_commands, appearance, app)
                {
                    content.add_child(setup_commands_section);
                }
            }

            if let Some(error_field) = self.render_error_field(appearance, app) {
                content.add_child(
                    Container::new(error_field)
                        .with_margin_bottom(FIELD_SPACING)
                        .finish(),
                );
            }
        }

        if let Some(source_section) = self.render_source_section(appearance) {
            content.add_child(
                Container::new(source_section)
                    .with_margin_bottom(FIELD_SPACING)
                    .finish(),
            );
        }

        let scrollable_content = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: Container::new(content.finish())
                    .with_uniform_padding(12.)
                    .finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();

        let selected_text = self.selected_text.clone();
        let scrollable_content = SelectableArea::new(
            self.selection_handle.clone(),
            move |selection_args, _, _| {
                *selected_text.write() = selection_args.selection.filter(|s| !s.is_empty());
            },
            scrollable_content,
        )
        .on_selection_updated(|ctx, _| {
            ctx.dispatch_typed_action(ConversationDetailsPanelAction::Focus);
        })
        .finish();

        let panel_content = Flex::column()
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(scrollable_content)
                        .with_border(
                            Border::left(1.).with_border_fill(blended_colors::neutral_2(theme)),
                        )
                        .finish(),
                )
                .finish(),
            )
            .finish();

        // On mobile, add background and skip Resizable
        #[cfg(target_family = "wasm")]
        if warpui::platform::wasm::is_mobile_device() {
            return Container::new(panel_content)
                .with_background(theme.surface_1())
                .finish();
        }

        Resizable::new(self.resizable_state_handle.clone(), panel_content)
            .with_dragbar_side(DragBarSide::Left)
            .with_bounds_callback(Box::new(|_| (200.0, 800.0)))
            .on_resize(|ctx, _| ctx.notify())
            .finish()
    }
}

impl Entity for ConversationDetailsPanel {
    type Event = ConversationDetailsPanelEvent;
}

impl TypedActionView for ConversationDetailsPanel {
    type Action = ConversationDetailsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ConversationDetailsPanelAction::Close => {
                ctx.emit(ConversationDetailsPanelEvent::Close);
            }
            ConversationDetailsPanelAction::CopyDirectory => match &self.data.mode {
                PanelMode::Conversation {
                    directory: Some(directory),
                    ..
                }
                | PanelMode::Task {
                    directory: Some(directory),
                    ..
                } => {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(directory.clone()));
                    self.record_copy(CopyButtonKind::Directory, ctx);
                }
                _ => {}
            },
            ConversationDetailsPanelAction::CopyConversationId => {
                if let PanelMode::Conversation {
                    server_conversation_id: Some(id),
                    ..
                } = &self.data.mode
                {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(id.clone()));
                    self.record_copy(CopyButtonKind::ConversationId, ctx);
                }
            }
            ConversationDetailsPanelAction::CopyRunId => {
                if let PanelMode::Task {
                    task_id: Some(task_id),
                    ..
                } = &self.data.mode
                {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(task_id.to_string()));
                    self.record_copy(CopyButtonKind::RunId, ctx);
                }
            }
            ConversationDetailsPanelAction::CopyEnvironmentId => {
                if let PanelMode::Task {
                    environment_id: Some(env_id),
                    ..
                } = &self.data.mode
                {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(env_id.clone()));
                    self.record_copy(CopyButtonKind::EnvironmentId, ctx);
                }
            }
            ConversationDetailsPanelAction::CopyDockerImage => {
                if let PanelMode::Task {
                    environment_id: Some(env_id),
                    ..
                } = &self.data.mode
                {
                    // Fetch docker image from environment
                    if let Ok(server_id) = ServerId::try_from(env_id.as_str()) {
                        let sync_id = SyncId::ServerId(server_id);
                        if let Some(env) = CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx) {
                            let docker_image = env.model().string_model.base_image.to_string();
                            ctx.clipboard()
                                .write(ClipboardContent::plain_text(docker_image));
                            self.record_copy(CopyButtonKind::DockerImage, ctx);
                        }
                    }
                }
            }
            ConversationDetailsPanelAction::CopyFetchError => {
                if let Some(error) = &self.data.fetch_error {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(error.clone()));
                    self.record_copy(CopyButtonKind::FetchError, ctx);
                }
            }
            ConversationDetailsPanelAction::CopyError => {
                if let PanelMode::Task {
                    error_message: Some(error),
                    ..
                } = &self.data.mode
                {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(error.clone()));
                    self.record_copy(CopyButtonKind::Error, ctx);
                }
            }
            ConversationDetailsPanelAction::CopySetupCommands(text) => {
                if !text.is_empty() {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(text.clone()));
                    self.record_copy(CopyButtonKind::SetupCommands, ctx);
                }
            }
            ConversationDetailsPanelAction::Focus => {
                ctx.focus_self();
            }
            ConversationDetailsPanelAction::CopySelectedText => {
                if let Some(text) = self.selected_text.read().clone().filter(|t| !t.is_empty()) {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
            #[cfg(not(target_family = "wasm"))]
            ConversationDetailsPanelAction::ContinueLocally => {
                if let Some(conversation_id) = self.continue_locally_conversation_id(ctx) {
                    send_telemetry_from_ctx!(
                        AgentManagementTelemetryEvent::DetailsPanelContinueLocally,
                        ctx
                    );
                    ctx.dispatch_typed_action(&WorkspaceAction::ContinueConversationLocally {
                        conversation_id,
                    });
                }
            }
            ConversationDetailsPanelAction::OpenInOz => {
                if let Some(url) = Self::oz_run_url(&self.data) {
                    ctx.open_url(&url);
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "conversation_details_panel_tests.rs"]
mod tests;
