mod agent;
pub mod buffer_model;
mod classic;
mod cli_agent;
mod cloud_mode_v2_history_menu;
mod common;
pub mod conversations;
pub mod decorations;
pub(crate) mod handoff_compose;
pub mod inline_history;
pub mod inline_menu;
pub mod message_bar;
pub mod models;
pub mod plans;
pub mod profiles;
pub mod prompts;
pub mod repos;
pub mod rewind;
pub mod skills;
pub mod slash_command_model;
pub mod slash_commands;
mod suggestions_mode_menu;
pub mod suggestions_mode_model;
mod terminal;
mod terminal_message_bar;
mod universal;
pub mod user_query;

pub use self::handoff_compose::{HandoffComposeState, HandoffComposeStateEvent};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentExchangeId, CancellationReason};
use crate::ai::agent_conversations_model::{
    AgentConversationNavigationSubject, AgentConversationsModel,
};
use crate::ai::ambient_agents::telemetry::HandoffEntryPoint;
use crate::ai::blocklist::agent_view::shortcuts::AgentShortcutViewModel;
use crate::ai::blocklist::agent_view::{AgentViewEntryOrigin, EphemeralMessageModel};
use crate::ai::blocklist::block::cli_controller::CLISubagentController;
use crate::ai::blocklist::block::status_bar::BlocklistAIStatusBar;
use crate::ai::blocklist::{
    ai_brand_color, ai_indicator_height, BlocklistAIActionModel, SlashCommandRequest,
};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::predict::prompt_suggestions::{
    has_pending_code_or_unit_test_prompt_suggestion,
    is_accept_prompt_suggestion_bound_to_ctrl_enter,
};
use crate::ai::skills::SkillManager;
use crate::ai::skills::{SkillOpenOrigin, SkillTelemetryEvent};
use crate::context_chips::spacing;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::prompt::editor_modal::OpenSource as PromptEditorOpenSource;
use crate::search::slash_command_menu::static_commands::commands::{self, COMMAND_REGISTRY};

use crate::server::telemetry::{PaletteSource, SlashCommandAcceptedDetails, SlashMenuSource};
use crate::settings::PrivacySettings;
use crate::suggestions::ignored_suggestions_model::{
    IgnoredSuggestionsModel, IgnoredSuggestionsModelEvent, SuggestionType,
};
use crate::terminal::buy_credits_banner::{BuyCreditsBanner, BuyCreditsBannerEvent};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::plugin_manager::PluginModalKind;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::cloud_mode_v2_history_menu::CloudModeV2HistoryMenuView;
use crate::terminal::input::conversations::{
    InlineConversationMenuEvent, InlineConversationMenuView,
};
use crate::terminal::input::inline_history::InlineHistoryMenuView;
use crate::terminal::input::inline_menu::InlineMenuPositioner;
use crate::terminal::input::models::{
    InlineModelSelectorEvent, InlineModelSelectorTab, InlineModelSelectorView,
};
use crate::terminal::input::plans::{InlinePlanMenuEvent, InlinePlanMenuView};
use crate::terminal::input::profiles::{InlineProfileSelectorEvent, InlineProfileSelectorView};
use crate::terminal::input::prompts::{InlinePromptsMenuEvent, InlinePromptsMenuView};
use crate::terminal::input::repos::{InlineReposMenuEvent, InlineReposMenuView};
use crate::terminal::input::rewind::{RewindMenuEvent, RewindMenuView};
use crate::terminal::input::skills::{InlineSkillSelectorEvent, InlineSkillSelectorView};
use crate::terminal::input::slash_command_model::{SlashCommandEntryState, SlashCommandModel};
use crate::terminal::input::slash_commands::{
    CloudModeV2SlashCommandView, InlineSlashCommandView, SlashCommandDataSource,
    SlashCommandTrigger,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::input::terminal_message_bar::TerminalInputMessageBar;
use crate::terminal::input::user_query::{UserQueryMenuEvent, UserQueryMenuView};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::package_installers::command_at_cursor_has_common_package_installer_prefix;
use crate::terminal::prompt_render_helper::should_render_ps1_prompt;
use crate::terminal::universal_developer_input::AtContextMenuDisabledReason;
use crate::terminal::view::CodeDiffAction;
use crate::terminal::CLIAgent;
use crate::util::bindings::keybinding_name_to_normalized_string;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor;
use crate::util::truncation::truncate_from_end;
#[allow(unused_imports)]
use crate::ASSETS;

#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
#[cfg(feature = "local_fs")]
use crate::persistence::{database_file_path_for_scope, establish_ro_connection, PersistenceScope};

use crate::ai::attachment_utils::MAX_ATTACHMENT_SIZE_BYTES;
use crate::ai::block_context::BlockContext;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::agent_view::agent_input_footer::sort_environments_by_recency;
use crate::ai::blocklist::agent_view::{
    is_in_cloud_context, AgentInputFooter, AgentInputFooterEvent, AgentViewController,
};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::touched_repos::{
    pick_handoff_overlap_env, resolve_repo_for_path, TouchedWorkspace,
};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::{HandoffLaunchAttachments, PendingCloudLaunch};
use crate::ai::blocklist::AttachmentType;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::PendingAttachment;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::server::server_api::ai::AttachmentFileInfo;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::server::server_api::ai::AttachmentInput;
use crate::terminal::view::ambient_agent::{
    AuthSecretFtuxView, AuthSecretFtuxViewEvent, AuthSecretSelector, AuthSecretSelectorEvent,
    HarnessSelector, HarnessSelectorEvent, HostSelector, HostSelectorEvent, NakedHeaderButtonTheme,
};
use crate::{
    ai::{
        agent::{AIAgentContext, EntrypointType},
        blocklist::{
            prompt::prompt_alert::{PromptAlertEvent, PromptAlertView},
            render_ai_agent_mode_icon, render_ai_follow_up_icon,
            telemetry_banner::should_collect_ai_ugc_telemetry,
            BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIController,
            BlocklistAIControllerEvent, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
            BlocklistAIInputEvent, BlocklistAIInputModel, InputConfig, InputType,
            BLOCK_CONTEXT_ATTACHMENT_REGEX, DIFF_HUNK_ATTACHMENT_REGEX,
            DRIVE_OBJECT_ATTACHMENT_REGEX,
        },
        llms::{LLMPreferences, LLMPreferencesEvent},
        predict::{
            next_command_model::{
                is_command_valid, is_next_command_enabled, NextCommandModel, NextCommandModelEvent,
                NextCommandSuggestionState, ZeroStateSuggestionInfo,
            },
            predict_am_queries::PredictAMQueriesRequest,
        },
        AIRequestUsageModel,
    },
    ai_assistant::execution_context::WarpAiExecutionContext,
    appearance::{Appearance, AppearanceEvent},
    channel::{Channel, ChannelState},
    cloud_object::{
        model::{actions::ObjectActionType, persistence::CloudModel, view::CloudViewModel},
        CloudObject, Space,
    },
    cmd_or_ctrl_shift,
    code_review::diff_state::DiffMode,
    completer::SessionContext,
    context_chips::{
        display::{PromptDisplay, PromptDisplayEvent},
        display_chip::DisplayChipConfig,
        prompt_type::PromptType,
    },
    debounce::debounce,
    editor::{
        default_cursor_colors, position_id_for_cached_point, position_id_for_cursor,
        position_id_for_first_cursor, AttachedImage as AttachedImageRawData,
        AutosuggestionLocation, AutosuggestionType, BaselinePositionComputationMethod,
        CommandXRayAnchor, CrdtOperation, CursorColors, DisplayPoint, EditOrigin, EditorAction,
        EditorDecoratorElements, EditorOptions, EditorSnapshot, EditorView, Event as EditorEvent,
        ImageContextOptions, InteractionState, LineStartBehavior, PathTransformerFn,
        PlainTextEditorViewAction, Point as BufferPoint, PropagateAndNoOpEscapeKey,
        PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, ReplicaId, TextColors,
        TextRun, MAX_IMAGES_PER_CONVERSATION,
    },
    features::FeatureFlag,
    input_suggestions::{
        Event as InputSuggestionsEvent, HistoryInputSuggestion, InputSuggestions,
        TabCompletionsPreselectOption,
    },
    network::NetworkStatus,
    pane_group::PaneGroupAction,
    prefix::longest_common_prefix,
    report_if_error,
    resource_center::{
        mark_feature_used_and_write_to_user_defaults, Tip, TipAction, TipHint, TipsCompleted,
    },
    search::{
        ai_context_menu::{
            mixer::AIContextMenuSearchableAction, search::is_valid_search_query,
            view::AIContextMenuAction,
        },
        QueryFilter,
    },
    send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::UpdateManager,
        ids::SyncId,
        server_api::ServerApi,
        telemetry::{
            AICommandSearchEntrypoint, AgentModeAutoDetectionFalsePositivePayload,
            AgentModeAutoDetectionSettingOrigin, AnonymousUserSignupEntrypoint, CommandXRayTrigger,
            EnvVarTelemetryMetadata, TelemetryEvent, WorkflowTelemetryMetadata,
        },
    },
    session_management::SessionNavigationPromptElements,
    settings::{
        AISettings, AISettingsChangedEvent, AliasExpansionSettings, AppEditorSettings,
        AppEditorSettingsChangedEvent, InputModeSettings, InputSettings, InputSettingsChangedEvent,
        MAX_TIMES_TO_SHOW_AUTOSUGGESTION_HINT,
    },
    settings_view::{flags, SettingsSection},
    terminal::view::inline_banner::{PromptSuggestionsEvent, PromptSuggestionsView},
    ui_components::{blended_colors, icons::Icon},
    user_config::WarpConfig,
    util::bindings::{self, CustomAction},
    util::image::MAX_IMAGE_COUNT_FOR_QUERY,
    view_components::{DismissibleToast, ToastFlavor},
    voltron::{
        Voltron, VoltronEvent, VoltronFeatureView, VoltronFeatureViewHandle,
        VoltronFeatureViewMeta, VoltronItem, VoltronMetadata,
    },
    workflows::{
        self,
        aliases::WorkflowAliases,
        command_parser::{
            compute_workflow_display_data, compute_workflow_display_data_for_history_command,
            compute_workflow_display_data_with_overrides, WorkflowArgumentIndex,
            WorkflowDisplayData,
        },
        info_box::{
            WorkflowsInfoBoxViewEvent, WorkflowsMoreInfoView, WORKFLOW_PARAMETER_HIGHLIGHT_COLOR,
        },
        local_workflows::LocalWorkflows,
        workflow_enum::EnumVariants,
        WorkflowSelectionSource, WorkflowSource, WorkflowType,
    },
    workspace::{
        sync_inputs::SyncedInputState, CommandSearchOptions, ForkFromExchange,
        ForkedConversationDestination, InitContent, RestoreConversationLayout, ToastStack,
        WorkspaceAction,
    },
    workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent},
    AgentModeEntrypoint, ServerApiProvider,
};
use warp_cli::agent::Harness;

use ai::skills::SkillReference;
use base64::Engine as _;
#[cfg(feature = "local_fs")]
use diesel::SqliteConnection;
use futures::FutureExt as _;
use itertools::Itertools;
use lazy_static::lazy_static;
use ordered_float::Float;
use regex::Regex;
use serde_json::json;
use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use settings::{Setting as _, ToggleableSetting};
use std::{
    any::Any,
    borrow::Cow,
    collections::HashMap,
    fmt::Write,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};
use string_offset::CharOffset;
use vec1::Vec1;
use vim::vim::{VimHandler, VimMode};
use warp_completer::util::parse_current_commands_and_tokens;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use warpui::r#async::FutureExt as _;

use warp_completer::{
    completer::{
        self, CompleterOptions, CompletionContext, CompletionsFallbackStrategy, Description, Match,
        MatchStrategy, MatchType, PathSeparators, SuggestionResults,
    },
    meta::{HasSpan, Spanned},
    parsers::{simple::command_at_cursor_position, LiteCommand},
    signatures::CommandRegistry,
};
use warp_core::user_preferences::GetUserPreferences as _;
use warp_core::{
    context_flag::ContextFlag,
    ui::theme::{color::internal_colors, AnsiColorIdentifier},
};
use warp_editor::editor::NavigationKey;
use warp_util::path::ShellFamily;
use warpui::{
    accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole},
    clipboard::{ClipboardContent, ImageData},
    clipboard_utils::CLIPBOARD_IMAGE_MIME_TYPES,
    color::ColorU,
    elements::{
        resizable_state_handle, Align, AnchorPair, ChildAnchor, Clipped, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, DispatchEventResult, DropTargetData, Element,
        EventHandler, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
        OffsetType, ParentAnchor, ParentElement, PositionedElementOffsetBounds, PositioningAxis,
        Radius, ResizableStateHandle, SavePosition, SelectionHandle, Text, Wrap, XAxisAnchor,
        YAxisAnchor,
    },
    end_trace,
    keymap::{BindingDescription, EditableBinding, FixedBinding, Keystroke},
    platform::OperatingSystem,
    presenter::ChildView,
    r#async::SpawnedFutureHandle,
    start_trace,
    text_layout::TextStyle,
    ui_components::{
        chip::Chip,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    units::IntoPixels,
    AppContext, Entity, EntityId, FocusContext, ModelAsRef, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle, WeakViewHandle,
};
pub use warpui::{
    elements::{ParentElement as _, Stack},
    geometry::vector::{vec2f, Vector2F},
    WindowId,
};

use self::decorations::InputBackgroundJobOptions;
use super::{
    alias::is_expandable_alias,
    block_list_viewport::InputMode,
    event::{BlockCompletedEvent, BlockType, UserBlockCompleted},
    ligature_settings::LigatureSettings,
    model::{
        block::{AgentInteractionMetadata, BlockId, BlockMetadata, BlocklistEnvVarMetadata},
        session::{Session, SessionId, SessionType, Sessions},
    },
    prompt,
    prompt_render_helper::{
        should_render_prompt_on_same_line, should_render_prompt_using_editor_decorator_elements,
        PromptRenderHelper, SameLinePromptElements,
    },
    safe_mode_settings::{
        get_secret_obfuscation_mode, SafeModeSettings, SafeModeSettingsChangedEvent,
    },
    session_settings::{SessionSettings, SessionSettingsChangedEvent},
    settings::{SpacingMode, TerminalSettings, TerminalSettingsChangedEvent},
    shared_session::{
        presence_manager::PresenceManager, viewer::history_model::SharedSessionHistoryModel,
        SharedSessionStatus,
    },
    shell::ShellType,
    universal_developer_input::{
        UniversalDeveloperInputButtonBar, UniversalDeveloperInputButtonBarEvent,
    },
    view::{
        ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent},
        inline_banner::{
            PromptSuggestionBannerState, ZeroStatePromptSuggestionTriggeredFrom,
            ZeroStatePromptSuggestionType,
        },
        ExecuteCommandEvent, SyncInputType, TerminalAction,
        PADDING_LEFT as TERMINAL_VIEW_PADDING_LEFT,
    },
    warpify::SubshellSource,
    History, HistoryEntry, SizeInfo, TerminalModel, UpArrowHistoryConfig,
};
use async_channel::Sender;
use futures::stream::AbortHandle;
use parking_lot::FairMutex;
#[cfg(feature = "local_fs")]
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use string_offset::ByteOffset;

/// Drop target data for dropping content on the [`Input`].
#[derive(Debug, Clone)]
pub struct InputDropTargetData {
    pub input_view: WeakViewHandle<Input>,
}

impl InputDropTargetData {
    fn new(input_view: WeakViewHandle<Input>) -> Self {
        Self { input_view }
    }

    pub fn weak_view_handle(&self) -> WeakViewHandle<Input> {
        self.input_view.clone()
    }
}

impl DropTargetData for InputDropTargetData {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub const DEBOUNCE_INPUT_DECORATION_PERIOD: Duration = Duration::from_millis(10);
pub const DEBOUNCE_AI_QUERY_PREDICTION_PERIOD: Duration = Duration::from_millis(250);
pub(super) const CLI_AGENT_RICH_INPUT_EDITOR_MAX_HEIGHT: f32 = 236.;
pub(super) const CLI_AGENT_RICH_INPUT_EDITOR_TOP_PADDING: f32 = 10.;
pub(super) const CLI_AGENT_RICH_INPUT_EDITOR_BOTTOM_PADDING: f32 = 8.;
pub(super) const CLI_AGENT_RICH_INPUT_HINT_TEXT: &str = "Tell the agent what to build...";

const CLOUD_MODE_V2_HINT_TEXT: &str = "Kick off a cloud agent";
const SHORT_CIRCUIT_HIGHLIGHTING_ACTIONS: [Option<PlainTextEditorViewAction>; 7] = [
    Some(PlainTextEditorViewAction::Space),
    Some(PlainTextEditorViewAction::NonExpandingSpace),
    Some(PlainTextEditorViewAction::Paste),
    Some(PlainTextEditorViewAction::Tab),
    Some(PlainTextEditorViewAction::AcceptCompletionSuggestion),
    Some(PlainTextEditorViewAction::CursorChanged),
    Some(PlainTextEditorViewAction::NewLine),
];

/// Border width for the line at the top of the input box in pixels
pub fn get_input_box_top_border_width() -> f32 {
    if FeatureFlag::MinimalistUI.is_enabled() {
        0.0
    } else {
        1.0
    }
}

pub const COMPLETIONS_MENU_WIDTH: f32 = 330.;
pub const OPEN_COMPLETIONS_KEYBINDING_NAME: &str = "input:open_completion_suggestions";
pub const INPUT_A11Y_LABEL: &str = "Command Input.";
pub const INPUT_A11Y_HELPER: &str = "Input your shell command, press enter to execute. Press cmd-up to navigate to output of previously executed commands. Press cmd-l to re-focus command input.";
pub const AI_COMMAND_SEARCH_HINT_TEXT: &str = "Type '#' for AI command suggestions";

const AGENT_MODE_AI_DISABLED_AUTODETECTION_DISABLED_HINT_TEXT: &str = "Run commands";

// Rotating hint text options for new Agent Mode conversations
const AGENT_MODE_HINT_OPTIONS: &[&str] = &[
    "Warp anything e.g. Deploy my React app to Vercel and set up environment variables",
    "Warp anything e.g. Help me debug why my Python tests are failing in CI",
    "Warp anything e.g. Set up a new microservice with Docker and create the deployment pipeline",
    "Warp anything e.g. Find and fix the memory leak in my Node.js application",
    "Warp anything e.g. Create a backup script for my PostgreSQL database and schedule it",
    "Warp anything e.g. Help me migrate my data from MySQL to PostgreSQL",
    "Warp anything e.g. Set up monitoring and alerts for my AWS infrastructure",
    "Warp anything e.g. Build a REST API for my mobile app using FastAPI",
    "Warp anything e.g. Help me optimize my SQL queries that are running slowly",
    "Warp anything e.g. Create a GitHub Actions workflow to automatically deploy on merge",
    "Warp anything e.g. Set up Redis caching for my web application",
    "Warp anything e.g. Help me troubleshoot why my Kubernetes pods keep crashing",
    "Warp anything e.g. Build a data pipeline to process CSV files and load them into BigQuery",
    "Warp anything e.g. Set up SSL certificates and configure HTTPS for my domain",
    "Warp anything e.g. Help me refactor this legacy code to use modern design patterns",
    "Warp anything e.g. Create unit tests for my authentication service",
    "Warp anything e.g. Set up log aggregation with ELK stack for my distributed system",
    "Warp anything e.g. Help me implement OAuth2 authentication in my Express.js app",
    "Warp anything e.g. Optimize my Docker images to reduce build times and size",
    "Warp anything e.g. Set up A/B testing infrastructure for my web application",
];

fn get_agent_mode_new_conversation_hint_text() -> &'static str {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static HINT_INDEX: AtomicUsize = AtomicUsize::new(0);

    let index = HINT_INDEX.fetch_add(1, Ordering::Relaxed) % AGENT_MODE_HINT_OPTIONS.len();
    AGENT_MODE_HINT_OPTIONS[index]
}

fn get_stable_agent_mode_hint_text(cached_hint: &mut Option<&'static str>) -> &'static str {
    if let Some(hint) = cached_hint {
        hint
    } else {
        let new_hint = get_agent_mode_new_conversation_hint_text();
        *cached_hint = Some(new_hint);
        new_hint
    }
}

const AGENT_MODE_AI_ENABLED_STEER_HINT_TEXT_UDI: &str = "Steer the running agent";
const AGENT_MODE_AI_ENABLED_STEER_HINT_TEXT_CLASSIC: &str =
    "Steer the running agent, or backspace to exit";
const AGENT_MODE_AI_ENABLED_FOLLOW_UP_HINT_TEXT_UDI: &str = "Ask a follow up";
const AGENT_MODE_AI_ENABLED_FOLLOW_UP_HINT_TEXT_CLASSIC: &str =
    "Ask a follow up, or backspace to exit";

/// Action name for setting input mode to agent mode
pub const SET_INPUT_MODE_AGENT_ACTION_NAME: &str = "input:set_mode_agent";

/// Action name for setting input mode to terminal mode
pub const SET_INPUT_MODE_TERMINAL_ACTION_NAME: &str = "input:set_mode_terminal";

/// Action name for setting input mode to unlocked agent mode (with natural language detection)
pub const SET_INPUT_MODE_UNLOCKED_AGENT_ACTION_NAME: &str = "input:set_mode_unlocked_agent";

/// Action name for setting input mode to unlocked terminal mode (with natural language detection)
pub const SET_INPUT_MODE_UNLOCKED_TERMINAL_ACTION_NAME: &str = "input:set_mode_unlocked_terminal";

const START_NEW_CONVERSATION_KEYBINDING_NAME: &str = "input:start_new_agent_conversation";

/// The position ID used to identify the start of the replacement span for completions.
const COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID: &str =
    "start_of_completions_replacement_span";

const HISTORY_DETAILS_VIEW_WIDTH_REQUIREMENT: f32 = 1100.;

const MIN_BUFFER_LEN_TO_SHOW_COMPLETIONS_WHILE_TYPING: usize = 2;

const AI_COMMAND_SEARCH_TRIGGER: &str = "#";

/// If the editor buffer matches this prefix, AI input is enabled.
const AI_INPUT_PREFIX: &str = "* ";

/// If the editor buffer matches this prefix, terminal input is enabled and locked.
const TERMINAL_INPUT_PREFIX: &str = "!";
/// If the editor buffer matches this prefix, local agent input enters cloud handoff compose mode.
const CLOUD_HANDOFF_INPUT_PREFIX: &str = "&";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputPrefixMode {
    None,
    Shell,
    CloudHandoff,
}

const VIM_STATUS_BAR_BOTTOM_PADDING: f32 = 20.;

const DYNAMIC_ENUM_GENERATE_MESSAGE: &str = "Run the following command to generate variants:";
const DYNAMIC_ENUM_RUN_MESSAGE: &str = "Run command";
const DYNAMIC_ENUM_PENDING_MESSAGE: &str = "Command pending...";
const DYNAMIC_ENUM_FAILURE_MESSAGE: &str = "Command failed";
const DYNAMIC_ENUM_NO_RESULTS_MESSAGE: &str = "Command returned no results";
const DYNAMIC_ENUM_MENU_PADDING: f32 = 10.;
const DYNAMIC_ENUM_MENU_HEIGHT_OFFSET: f32 = 25.;
const DYNAMIC_ENUM_HORIZONTAL_TEXT_PADDING: f32 = 5.;

cfg_if::cfg_if! {
    if #[cfg(target_os = "macos")] {
        const CMD_ENTER_KEYBINDING: &str = "cmd-enter";
    } else {
        // On linux and windows, the CmdEnter EditorAction is bound to ctrl-shift-enter.
        const CMD_ENTER_KEYBINDING: &str =  "ctrl-shift-enter";
    }
}

lazy_static! {
    static ref RUN_DYNAMIC_ENUM_COMMAND_KEYSTROKE: Keystroke = if OperatingSystem::get().is_mac() {
        Keystroke {
            cmd: true,
            key: "enter".to_owned(),
            ..Default::default()
        }
    } else {
        Keystroke {
            ctrl: true,
            shift: true,
            key: "enter".to_owned(),
            ..Default::default()
        }
    };
}

#[derive(PartialEq, Eq, Copy, Clone, Serialize)]
pub enum TelemetryInputSuggestionsMode {
    HistoryFuzzySearch,
    CompletionSuggestions,
    HistoryUp,
    NaturalLanguageCommandSearch,
    StaticWorkflowEnumSuggestions,
    DynamicWorkflowEnumSuggestions,
    AIContextMenu,
    SlashCommands,
    ConversationMenu,
    ModelSelector,
    ProfileSelector,
    PromptsMenu,
    SkillMenu,
    InlineHistoryMenu,
    IndexedReposMenu,
    PlanMenu,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HistorySearchMode {
    /// Prefix match commands.
    Prefix,
    /// Fuzzy match commands.
    Fuzzy,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum TabCompletionsMenuPosition {
    /// The menu should be positioned at the last cursor.
    AtLastCursor,
    /// The menu should be positioned at the first cursor.
    AtFirstCursor,
    /// The menu should be positioned at the given position.
    AtStartOfReplacementSpan,
}

impl TabCompletionsMenuPosition {
    fn to_position_id(self, editor_view_id: EntityId) -> String {
        match self {
            Self::AtLastCursor => position_id_for_cursor(editor_view_id),
            Self::AtFirstCursor => position_id_for_first_cursor(editor_view_id),
            Self::AtStartOfReplacementSpan => position_id_for_cached_point(
                editor_view_id,
                COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID,
            ),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct BufferState {
    buffer: String,
    cursor_point: Option<BufferPoint>,
}

impl BufferState {
    pub fn new(buffer: String, cursor_point: Option<BufferPoint>) -> Self {
        Self {
            buffer,
            cursor_point,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum InputSuggestionsMode {
    /// Mode used when arrow-up is pressed.
    HistoryUp {
        /// Text in the buffer when arrow-up is pressed (possibly empty).
        original_buffer: String,
        /// Cursor point when arrow-up is pressed.
        /// This is None when there are > 1 active selections when HistoryUp is invoked.
        /// TODO: eventually, we should support saving/resetting _many_ cursors rather than a single one.
        original_cursor_point: Option<BufferPoint>,
        search_mode: HistorySearchMode,
        /// The AI input mode when arrow-up is pressed.
        original_input_type: InputType,
        /// The AI input's lock status when the arrow-up is pressed.
        original_input_was_locked: bool,
    },
    CompletionSuggestions {
        /// Stores the byte index of the beginning of the text we are replacing
        replacement_start: usize,

        /// Stores the original buffer text before the user pressed TAB.
        /// Used to close the suggestions menu if the buffer_text_original is no longer in the input buffer.
        buffer_text_original: String,

        /// Stores the suggestions for the original buffer_text_original.
        /// Used to filter down results during prefix search.
        completion_results: SuggestionResults,

        /// Stores the original trigger of the completions, so that we can track whether the menu
        /// was opened automatically (AsYouType) or manually (with Tab)
        trigger: CompletionsTrigger,

        /// Where the menu should be positioned.
        menu_position: TabCompletionsMenuPosition,
    },

    StaticWorkflowEnumSuggestions {
        /// The suggested values for the workflow argument.
        suggestions: Vec<String>,

        /// Where the menu should be positioned.
        menu_position: TabCompletionsMenuPosition,

        /// The selected ranges for every instance of the argument.
        selected_ranges: Vec<Range<ByteOffset>>,

        /// Store the cursor point of the end of the first selected argument.
        cursor_point: BufferPoint,
    },

    DynamicWorkflowEnumSuggestions {
        /// The suggested values for the workflow argument.
        suggestions: Vec<String>,

        /// Where the menu should be positioned.
        menu_position: TabCompletionsMenuPosition,

        /// The selected ranges for every instance of the argument.
        selected_ranges: Vec<Range<ByteOffset>>,

        /// Store the cursor point of the end of the first selected argument.
        cursor_point: BufferPoint,

        /// Store the current state of the dynamic enum suggestions menu.
        dynamic_enum_status: DynamicEnumSuggestionStatus,

        /// The command associated with the dynamic enum.
        command: String,
    },

    AIContextMenu {
        /// Text typed after the "@" for filtering
        filter_text: String,
        /// Byte position of the "@" symbol that triggered this menu
        at_symbol_position: usize,
    },

    SlashCommands,

    /// Conversation menu mode for selecting AI conversations.
    ConversationMenu,

    /// Model selector mode for selecting the Agent base model.
    ModelSelector,
    /// Profile selector mode for selecting an execution profile.
    ProfileSelector,

    /// Skill menu mode for /open-skill command.
    SkillMenu,

    /// Prompts menu mode for /prompts command.
    PromptsMenu,

    /// User query menu mode for selecting a query point (e.g., fork-from, rewind).
    UserQueryMenu {
        action: UserQueryMenuAction,
        conversation_id: AIConversationId,
    },

    /// Inline history menu mode for selecting commands and conversations from history.
    InlineHistoryMenu {
        original_input_config: Option<InputConfig>,
    },

    /// Indexed repos switcher menu mode.
    IndexedReposMenu,

    /// Plan menu mode for selecting among multiple AI document plans.
    PlanMenu {
        conversation_id: AIConversationId,
    },

    /// Mode indicating that no suggestion UI is being shown.
    Closed,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum UserQueryMenuAction {
    ForkFrom,
    Rewind,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum DynamicEnumSuggestionStatus {
    /// When the command has not yet been approved to run on the users laptop
    Unapproved,
    /// The command is running asynchronously, but has not yet finished so we do not have suggestions to display
    Pending,
    /// The command succeeded; display suggested variants
    Success,
    /// The command failed
    Failure,
}

impl InputSuggestionsMode {
    pub fn is_visible(&self) -> bool {
        *self != InputSuggestionsMode::Closed
    }

    pub fn is_inline_menu(&self) -> bool {
        matches!(
            self,
            Self::SlashCommands
                | Self::ConversationMenu
                | Self::ModelSelector
                | Self::PromptsMenu
                | Self::UserQueryMenu { .. }
                | Self::InlineHistoryMenu { .. }
                | Self::PlanMenu { .. }
        ) || (FeatureFlag::InlineProfileSelector.is_enabled()
            && matches!(self, Self::ProfileSelector))
            || (FeatureFlag::ListSkills.is_enabled() && matches!(self, Self::SkillMenu))
            || (FeatureFlag::InlineRepoMenu.is_enabled() && matches!(self, Self::IndexedReposMenu))
    }

    /// Whether this mode should snapshot the input buffer on open and restore it on dismiss.
    fn should_snapshot_and_restore_buffer(&self) -> bool {
        // For now this just delegates to whether the current mode is an inline menu,
        // but in the future we might build this out/add more detail here.
        self.is_inline_menu()
    }

    fn input_config_to_restore(&self) -> Option<InputConfig> {
        match self {
            Self::InlineHistoryMenu {
                original_input_config,
            } => *original_input_config,
            _ => None,
        }
    }

    /// Returns the placeholder text for this mode, if it has a custom one.
    pub fn placeholder_text(&self) -> Option<&'static str> {
        match self {
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::ForkFrom,
                ..
            } => Some("Search queries"),
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::Rewind,
                ..
            } => Some("Search queries to rewind to"),
            InputSuggestionsMode::ConversationMenu => Some("Search conversations"),
            InputSuggestionsMode::SkillMenu => Some("Search skills"),
            InputSuggestionsMode::ModelSelector => Some("Search models"),
            InputSuggestionsMode::ProfileSelector => Some("Search profiles"),
            InputSuggestionsMode::SlashCommands if FeatureFlag::AgentView.is_enabled() => {
                Some("Search commands")
            }
            InputSuggestionsMode::PromptsMenu => Some("Search prompts"),
            InputSuggestionsMode::IndexedReposMenu => Some("Search indexed repos"),
            InputSuggestionsMode::PlanMenu { .. } => Some("Search plans"),
            _ => None,
        }
    }

    fn to_telemetry_mode(&self) -> TelemetryInputSuggestionsMode {
        match *self {
            InputSuggestionsMode::HistoryUp {
                search_mode: HistorySearchMode::Prefix,
                ..
            } => TelemetryInputSuggestionsMode::HistoryUp,
            InputSuggestionsMode::HistoryUp {
                search_mode: HistorySearchMode::Fuzzy,
                ..
            } => TelemetryInputSuggestionsMode::HistoryFuzzySearch,
            InputSuggestionsMode::CompletionSuggestions { .. } => {
                TelemetryInputSuggestionsMode::CompletionSuggestions
            }
            InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. } => {
                TelemetryInputSuggestionsMode::StaticWorkflowEnumSuggestions
            }
            InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. } => {
                TelemetryInputSuggestionsMode::DynamicWorkflowEnumSuggestions
            }
            InputSuggestionsMode::AIContextMenu { .. } => {
                TelemetryInputSuggestionsMode::AIContextMenu
            }
            InputSuggestionsMode::SlashCommands => TelemetryInputSuggestionsMode::SlashCommands,
            InputSuggestionsMode::ConversationMenu => {
                TelemetryInputSuggestionsMode::ConversationMenu
            }
            InputSuggestionsMode::ModelSelector => TelemetryInputSuggestionsMode::ModelSelector,
            InputSuggestionsMode::ProfileSelector => TelemetryInputSuggestionsMode::ProfileSelector,
            InputSuggestionsMode::PromptsMenu => TelemetryInputSuggestionsMode::PromptsMenu,
            InputSuggestionsMode::SkillMenu => TelemetryInputSuggestionsMode::SkillMenu,
            InputSuggestionsMode::UserQueryMenu { .. } => {
                TelemetryInputSuggestionsMode::ConversationMenu
            }
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                TelemetryInputSuggestionsMode::InlineHistoryMenu
            }
            InputSuggestionsMode::IndexedReposMenu => {
                TelemetryInputSuggestionsMode::IndexedReposMenu
            }
            InputSuggestionsMode::PlanMenu { .. } => TelemetryInputSuggestionsMode::PlanMenu,
            InputSuggestionsMode::Closed => unreachable!(),
        }
    }
}

struct SharedSessionInputState {
    /// History model for viewers in a shared session.
    // TODO: With this current approach, the shared session history crosses
    // subshell boundaries, we'll need to make it work with our current history model
    // to ensure we show the right shell history.
    history_model: ModelHandle<SharedSessionHistoryModel>,

    // Is [`Some`] iff a command execution was requested by a shared session executor.
    pending_command_execution_request: Option<ViewerCommandExecutionRequest>,
}

struct ViewerCommandExecutionRequest {
    /// Text in buffer when command execution was requested.
    original_buffer: String,
}

/// Where a command execution request originates from.
#[derive(Clone)]
pub enum CommandExecutionSource {
    /// A non-shared command execution request from Warp AI++.
    /// Shared commands use the SharedSession variant instead.
    AI {
        /// Metadata associated with the execution.
        metadata: AgentInteractionMetadata,
    },

    /// A command execution request in a shared session (by a viewer or sharer).
    ///
    /// For a sharer, this will be processed similar to [`CommandExecutionSource::User`]
    /// except the resulting block will be annotated with the participant ID.
    ///
    /// For a viewer, this will be handled by sending the request to the sharer.
    SharedSession {
        /// The participant ID of the
        participant_id: ParticipantId,
        /// The block ID associated to the active block when
        /// the request was fired.
        block_id: BlockId,
        /// Optional AI metadata if this command was requested by the AI agent
        /// in a shared session. This is used to associate the resulting command block
        /// with the original agent command.
        ai_metadata: Option<AgentInteractionMetadata>,
    },

    /// A normal command execution request.
    User,

    EnvVarCollection {
        metadata: BlocklistEnvVarMetadata,
    },
}

impl CommandExecutionSource {
    /// Whether this command execution originates from an AI command.
    pub fn is_ai_command(&self) -> bool {
        // TODO: at some point we will want to couple both of these cases
        // into one source variant, as they are both AI sources.
        matches!(
            self,
            CommandExecutionSource::AI { .. }
                | CommandExecutionSource::SharedSession {
                    ai_metadata: Some(_),
                    ..
                }
        )
    }
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum HistoryUpMode {
    // Show prefixed results.
    Prefixed,
    // Show all results with no query.
    RegularNoQuery,
    // Show all results with query.
    RegularWithQuery,
    // Used for ConfirmSuggestion event.
    NotApplicable,
}

impl HistoryUpMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            HistoryUpMode::Prefixed => "prefixed history up",
            HistoryUpMode::RegularNoQuery => "regular history up (no query)",
            HistoryUpMode::RegularWithQuery => "regular history up (with query)",
            HistoryUpMode::NotApplicable => "history up",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEmptyStateChangeReason {
    /// The buffer transitioned between empty and non-empty due to a regular edit.
    Edited,
    /// The buffer was cleared because a user-executed command completed and we reinitialized the
    /// buffer for the next command.
    UserCommandCompleted,
}

pub enum Event {
    AutosuggestionAccepted,
    ClearSelectedBlock,
    PageUp,
    PageDown,
    SelectRecentBlocks {
        /// Select the `count` most recent blocks.
        count: usize,
    },
    Copy,
    UnhandledModifierKeyOnEditor(Arc<String>),
    ClearSelectionsWhenShellMode,
    InputStateChanged(InputState),
    /// Emitted when the input text transitions between empty and non-empty states
    InputEmptyStateChanged {
        is_empty: bool,
        reason: InputEmptyStateChangeReason,
    },
    Escape,
    /// note: Terminal Inputs should only emit the variant
    /// SyncInputType::InputEditorContentsChanged.
    SyncInput(SyncInputType),
    ShowCommandSearch(CommandSearchOptions),
    CtrlD,
    CtrlC {
        // The number of chars cleared from the buffer, if the ctrl-c triggered a buffer clear.
        cleared_buffer_len: usize,
    },
    Enter,
    ExecuteCommand(Box<ExecuteCommandEvent>),
    ExecuteAIQuery,
    EmacsBindingUsed,
    /// The input editor was locally edited and
    /// peers should be notified, if applicable.
    EditorUpdated {
        /// The block ID associated to the buffer that
        /// these operations were made in.
        block_id: BlockId,

        /// The CRDT-compliant operations.
        operations: Rc<Vec<CrdtOperation>>,
    },
    /// A viewer in a shared session is requesting to send an agent prompt.
    SendAgentPrompt {
        server_conversation_token: Option<ServerConversationToken>,
        prompt: String,
        attachments: Vec<AgentAttachment>,
    },
    /// A disconnected Cloud Mode pane is requesting to submit a cloud follow-up.
    SubmitCloudFollowup {
        prompt: String,
    },
    /// A viewer in a shared session is requesting to cancel the active agent conversation.
    CancelSharedSessionConversation {
        server_conversation_token: ServerConversationToken,
    },
    InputFocusedFromMiddleClick,
    EditorFocused,
    UnhandledCmdEnter,
    CtrlEnter,
    SignupAnonymousUser {
        entrypoint: AnonymousUserSignupEntrypoint,
    },
    OpenSettings(SettingsSection),
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        source: CodeSource,
        layout: external_editor::settings::EditorLayout,
    },
    OpenCodeReviewPane,
    /// Request to attach a diff set as context to the AI conversation
    AttachDiffSetContext {
        diff_mode: DiffMode,
    },
    OpenConversationHistory,
    OpenViewMCPPane,
    OpenAddMCPPane,
    OpenProjectRulesPane,
    OpenEnvironmentManagementPane,
    OpenFilesPalette {
        source: PaletteSource,
    },
    TryHandlePassiveCodeDiff(CodeDiffAction),
    ToggleAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    SubmitCLIAgentInput {
        text: String,
    },
    OpenAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    OpenAutoReloadModal {
        purchased_credits: i32,
    },
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },

    EnterAgentView {
        initial_prompt: Option<String>,
        conversation_id: Option<AIConversationId>,
        origin: AgentViewEntryOrigin,
    },
    EnterCloudAgentView {
        initial_prompt: Option<String>,
    },
    CreateDockerSandbox,
    /// Exit cloud mode (ambient agent) and start a new *local* agent conversation in the root terminal.
    ///
    /// If `initial_prompt` is `Some`, it should prefill the local agent prompt but not auto-send.
    ExitCloudModeAndStartLocalAgent {
        initial_prompt: Option<String>,
    },
    ScrollToExchange {
        exchange_id: AIAgentExchangeId,
    },
    /// Trigger environment setup flow with optional repository arguments
    TriggerEnvironmentSetup {
        repos: Vec<String>,
    },
    RegisterPluginListener(CLIAgent),
    #[cfg(not(target_family = "wasm"))]
    OpenPluginInstructionsPane(CLIAgent, PluginModalKind),
    OpenShareSessionModal,
    StartRemoteControl,
    OpenHandoffEnvironmentCreationModal,
    OpenCloudModeV2EnvironmentCreationModal,
}

pub enum InputState {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug)]
pub enum InputAction {
    FocusInputBox,
    CtrlR,
    CtrlD,
    Up,
    PageUp,
    PageDown,
    ClearScreen,
    SelectAndRefreshVoltron(VoltronItem),
    ShowAiCommandSearch,
    /// Open the completions menu if the cursor is in a valid position to generate completion
    /// suggestions.
    MaybeOpenCompletionSuggestions,
    HideWorkflowInfoCard,

    /// If the command originates from a workflow but doesn't match the workflow template,
    /// this action resets the command to its original workflow state.
    ResetWorkflowState,

    ToggleClassicCompletionsMode,

    /// Toggles the inline conversation menu for selecting AI conversations.
    ToggleConversationsMenu,

    StartNewAgentConversation,

    /// This is for toggling whether autodetection is enabled/disabled at the app-level,
    /// not for whether its enabled/disabled for the current input
    ToggleInputAutoDetection,

    /// Triggers the lightbulb button click behavior to enable/toggle auto-detection
    EnableAutoDetection,

    /// Generate a new Next Command suggestion.
    CycleNextCommandSuggestion,

    /// Inserts a zero state prompt suggestion into the input buffer and executes the query for Agent Mode.
    InsertZeroStatePromptSuggestion(ZeroStatePromptSuggestionType),

    /// A passive code diff action.
    TryHandlePassiveCodeDiff(CodeDiffAction),

    /// Clears the AI context menu search query back to the @ character and resets menu state.
    ClearAndResetAIContextMenuQuery,

    /// Sets the hover state of the Universal Developer Input
    SetUDIHovered(bool),

    /// Persist the completions menu width when the user resizes it.
    UpdateCompletionsMenuWidth(f32),

    /// Persist the completions menu height when the user resizes it.
    UpdateCompletionsMenuHeight(f32),

    /// Toggles the '?' shortcuts UI in the agent view.
    ToggleAgentViewShortcuts,

    /// Toggles the '/' slash commands menu in the agent view.
    ToggleSlashCommandsMenu,

    /// Opens the inline history menu for cycling through past commands and conversations.
    OpenInlineHistoryMenu,

    DismissCloudModeV2SlashCommandsMenu,

    /// Opens the model selector menu.
    OpenModelSelector,

    /// Triggers a slash command from a custom keybinding. The string is the command name.
    TriggerSlashCommandFromKeybinding(&'static str),

    /// Clears attached blocks and text selection context.
    ClearAttachedContext,

    /// Fired when the "Get Figma MCP" contextual button is clicked.
    FigmaAddButtonClicked,

    /// Fired when the "Enable Figma MCP" contextual button is clicked.
    FigmaEnableButtonClicked,

    /// Activates `&` cloud handoff compose mode from the message bar hint.
    ActivateCloudHandoff,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum MenuPositioning {
    /// Position floating input menus above the input box -- corresponds
    /// to the regular blocklist.
    #[default]
    AboveInputBox,

    /// Position floating input menus below the input box -- corresponds
    /// to the inverted blocklist.
    BelowInputBox,
}

impl MenuPositioning {
    fn completion_suggestions_y_anchor(&self) -> AnchorPair<YAxisAnchor> {
        self.y_anchor()
    }

    fn history_y_anchor(&self) -> AnchorPair<YAxisAnchor> {
        self.y_anchor()
    }

    fn history_y_offset(&self) -> OffsetType {
        match *self {
            MenuPositioning::AboveInputBox => OffsetType::Pixel(0.),
            MenuPositioning::BelowInputBox => OffsetType::Pixel(-11.),
        }
    }

    fn command_xray_y_anchor(&self) -> AnchorPair<YAxisAnchor> {
        self.y_anchor()
    }

    fn workflows_info_y_anchor(&self) -> AnchorPair<YAxisAnchor> {
        self.y_anchor()
    }

    fn voltron_parent_anchor(&self) -> ParentAnchor {
        match *self {
            MenuPositioning::AboveInputBox => ParentAnchor::BottomLeft,
            MenuPositioning::BelowInputBox => ParentAnchor::TopLeft,
        }
    }

    fn voltron_child_anchor(&self) -> ChildAnchor {
        match *self {
            MenuPositioning::AboveInputBox => ChildAnchor::BottomLeft,
            MenuPositioning::BelowInputBox => ChildAnchor::TopLeft,
        }
    }

    fn voltron_offset(&self) -> Vector2F {
        match *self {
            MenuPositioning::AboveInputBox => vec2f(11., -11.),
            MenuPositioning::BelowInputBox => vec2f(11., -66.),
        }
    }

    fn y_anchor(&self) -> AnchorPair<YAxisAnchor> {
        match *self {
            MenuPositioning::AboveInputBox => {
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom)
            }
            MenuPositioning::BelowInputBox => {
                AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top)
            }
        }
    }
}

impl MenuPositioningProvider for MenuPositioning {
    fn menu_position(&self, _app: &AppContext) -> MenuPositioning {
        *self
    }
}

struct WorkflowsState {
    selected_workflow_state: Option<SelectedWorkflowState>,
}

struct EnvVarCollectionState {
    selected_env_vars: Option<SyncId>,
}

/// State when a workflow is selected.
#[derive(Clone)]
struct SelectedWorkflowState {
    /// A handle to the WorkflowsMoreInfoView shown for the selected workflow.
    ///
    /// Note that this is unconditionally constructed, even when `should_show_more_info_view` is
    /// `false`, because the `WorkflowsMoreInfoView` itself contains business logic for the state
    /// of the input editor when editing workflow arguments with the shift-tab UX. This isn't
    /// ideal, and more of a symptom of retrofitting a `WorkflowsMoreInfoView`-less version of the
    /// shift-tab UX specifically for up-arrow history.
    more_info_view: ViewHandle<WorkflowsMoreInfoView>,

    /// Map of arguments to the corresponding index of highlights. This is necessary so that we can
    /// select all instances of an argument when a user changes the selected argument.
    argument_index_to_highlight_index: HashMap<WorkflowArgumentIndex, Vec<usize>>,

    /// Map of arguments with enum variants to those variants, which are used as suggested inputs to the argument.
    argument_index_to_enum_variants: HashMap<WorkflowArgumentIndex, EnumVariants>,

    workflow_source: WorkflowSource,
    workflow_type: WorkflowType,
    workflow_selection_source: WorkflowSelectionSource,

    /// `true` if the WorkflowsMoreInfoView should be shown for the selected workflow. This is true
    /// in all cases except when a workflow-linked history command is selected from up-arrow
    /// history.
    should_show_more_info_view: bool,
}

/// Helper struct for differentiating the cases when the command is able to be
/// parsed into the workflow it originates from versus when it's been edited to
/// the point of us not being able to determine where the arguments are.
pub enum CommandMatchesWorkflowTemplate {
    Yes(WorkflowDisplayData),
    No,
}

/// Helper struct for performing alias expansion.
struct ExpansionInfo {
    /// The expanded text to replace the alias with.
    alias_value: String,
    /// The buffer text to replace the alias in.
    buffer_text: String,
    /// The byte indices that should be replaced with the alias_value.
    byte_range: Range<usize>,
}

/// For inserting last word of last command in history - by default, this is the last command but consecutive
/// inserts fetch further in history. Represents reverse index of history command to reference.
/// (insert_command_from_history_index=0 for most recent, 1 for command before it, etc.) See self.update_last_word_insertion_state()
struct LastWordInsertion {
    insert_command_from_history_index: usize,
    is_latest_editor_event: bool,
}

/// Data pertaining to the session state and history is bundled together, making
/// it accessible to other objects coupled with the same terminal session, such as a notebook.
#[derive(Clone)]
pub struct CompleterData {
    pub sessions: ModelHandle<Sessions>,
    pub active_block_metadata: Option<BlockMetadata>,
    command_registry: Arc<CommandRegistry>,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    last_user_block_completed: Option<UserBlockCompleted>,
}

impl CompleterData {
    pub fn new(
        sessions: ModelHandle<Sessions>,
        active_block_metadata: Option<BlockMetadata>,
        command_registry: Arc<CommandRegistry>,
        last_user_block_completed: Option<UserBlockCompleted>,
    ) -> Self {
        Self {
            sessions,
            active_block_metadata,
            command_registry,
            last_user_block_completed,
        }
    }

    pub fn active_block_session_id(&self) -> Option<SessionId> {
        self.active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::session_id)
    }

    pub fn completion_session_context(&self, app: &AppContext) -> Option<SessionContext> {
        let active_block_session_id = self.active_block_session_id()?;
        let current_session = self.sessions.as_ref(app).get(active_block_session_id);
        let pwd = self
            .active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::current_working_directory)
            .map(str::to_owned);

        current_session.zip(pwd).map(|(current_session, pwd)| {
            // TODO(abhishek): Ideally, BlockMetadata::current_working_directory should directly
            // return a TypedPathBuf. This shouldn't happen here in the view.
            let current_working_directory =
                current_session.convert_directory_to_typed_path_buf(pwd);

            SessionContext::new(
                current_session,
                self.command_registry.clone(),
                current_working_directory,
                app,
            )
        })
    }
}

/// Autosuggestion result returned by the generator.
pub struct AutoSuggestionResult {
    /// Text in the editor buffer.
    pub buffer_text: String,
    /// Generated autosuggestion result.
    pub autosuggestion_result: Option<String>,
}

/// Views that call into the autosuggestion generation logic must implement the Autosuggester
/// trait. This requires a callback on_autosuggestion_result and functions to set and abort
/// the latest future that's been spawned for autosuggestions.
pub trait Autosuggester {
    fn on_autosuggestion_result(
        &mut self,
        _result: AutoSuggestionResult,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn abort_latest_autosuggestion_future(&mut self);

    fn set_autosuggestion_future(&mut self, abort_handle: AbortHandle);
}

/// Implement this trait to provide whether menus like autocomplete, voltron, etc
/// should be positionined above or below the input.
pub trait MenuPositioningProvider {
    fn menu_position(&self, app: &AppContext) -> MenuPositioning;

    fn inline_menu_position(&self, _inline_menu_height: f32, _app: &AppContext) -> MenuPositioning {
        MenuPositioning::AboveInputBox
    }
}

/// Stores state referenced by the Input view and PromptRenderHelper.
/// Note that this is largely a workaround to avoid having to pass/upgrade
/// a weak view handle from `Input` to `PromptRenderHelper` for this state.
pub struct InputRenderStateModel {
    editor_modified_since_block_finished: bool,
    // For future: we should explore reading this directly off TerminalModel.
    size_info: SizeInfo,
}

impl InputRenderStateModel {
    pub fn new(editor_modified_since_block_finished: bool, size_info: SizeInfo) -> Self {
        Self {
            editor_modified_since_block_finished,
            size_info,
        }
    }

    pub fn editor_modified_since_block_finished(&self) -> bool {
        self.editor_modified_since_block_finished
    }

    pub fn size_info(&self) -> SizeInfo {
        self.size_info
    }

    pub fn set_editor_modified_since_block_finished(
        &mut self,
        editor_modified_since_block_finished: bool,
    ) {
        self.editor_modified_since_block_finished = editor_modified_since_block_finished;
    }

    pub fn set_size_info(&mut self, size_info: SizeInfo) {
        self.size_info = size_info;
    }
}

impl Entity for InputRenderStateModel {
    type Event = ();
}

lazy_static! {
    /// Define the regex patterns that we show completions-as-you-type in AI input on.
    /// We only show file completions - as such, we match on the following patterns:
    /// 1. "/": The last word starts with a slash
    /// 2. "./": The last word starts with "./"
    /// 3. "../": The last word starts with "../"
    /// 4. "{text}/": The last word contains a slash after some text
    /// We combine all the regex patterns for performance reasons (one string scan).
    /// NOTE: this assumes Unix-style paths. When we expand to Windows, we'll want to update this!
    static ref FILEPATH_PATTERN: Regex = Regex::new(
        r"^(?:/|\.\/|\.\./|[^/]+/)"
    ).expect("Expect regex to be valid");
}

/// Returns boolean indicating whether completions-as-you-type should pop up, while in AI input.
/// This is primarily based on the last word in the buffer text, and whether it makes sense to show
/// filepath completions.
fn should_show_completions_in_ai_input(buffer_text: &str) -> bool {
    if buffer_text.ends_with(char::is_whitespace) {
        return false;
    }

    let last_word = buffer_text.split_whitespace().last();

    if let Some(last_word) = last_word {
        FILEPATH_PATTERN.is_match(last_word)
    } else {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DenyExecutionReason {
    /// Can't execute command because shell bootstrapping is still underway; shell isn't ready to
    /// execute user-supplied commands yet.
    NotBootstrapped,

    /// Can't execute command because there's an active command in control of the pty.
    ExistingActiveCommand,

    /// With the exception of shared sessions, we should only execute commands if they can be
    /// recorded in history.
    ///
    /// Gonna be honest, I (zach b) have the least amount of context on this one, don't really know
    /// why this is the case.
    ///
    /// This is not returned as a `CancellationReason::No` for shared sessions even if it may be
    /// true; we do not record shared sessions in the History model thus they are default not-
    /// appendable.
    HistoryNotAppendable,
}

impl DenyExecutionReason {
    pub fn is_existing_active_command(&self) -> bool {
        matches!(self, DenyExecutionReason::ExistingActiveCommand)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanExecuteCommand {
    Yes,
    No(DenyExecutionReason),
}

impl CanExecuteCommand {
    pub fn is_no(&self) -> bool {
        matches!(self, CanExecuteCommand::No(_))
    }
}

pub struct Input {
    model: Arc<FairMutex<TerminalModel>>,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    tips_completed: ModelHandle<TipsCompleted>,
    editor: ViewHandle<EditorView>,
    server_api: Arc<ServerApi>,
    input_suggestions: ViewHandle<InputSuggestions>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    completions_menu_resizable_width: ResizableStateHandle,
    completions_menu_resizable_height: ResizableStateHandle,
    sessions: ModelHandle<Sessions>,
    focus_handle: Option<PaneFocusHandle>,
    active_block_metadata: Option<BlockMetadata>,
    /// The [`EntityId`] of the terminal view that this input view is inside.
    terminal_view_id: EntityId,
    view_id: EntityId,
    input_render_state_model_handle: ModelHandle<InputRenderStateModel>,
    workflows_state: WorkflowsState,
    env_var_collection_state: EnvVarCollectionState,
    voltron_view: ViewHandle<Voltron>,
    is_voltron_open: bool,
    command_x_ray_description: Option<Arc<Description>>,
    last_parsed_tokens: Option<decorations::ParsedTokensSnapshot>,
    debounce_input_background_tx: Sender<InputBackgroundJobOptions>,
    debounce_ai_query_prediction_tx: Sender<()>,
    /// If true, will submit the command in the editor to the shell upon receiving the
    /// precmd message.
    has_pending_command: bool,
    last_word_insertion: LastWordInsertion,

    ai_controller: ModelHandle<BlocklistAIController>,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    ai_action_model: ModelHandle<BlocklistAIActionModel>,
    /// The input is responsible for managing the lifetime
    /// of this mouse state handle.
    #[allow(dead_code)]
    ai_follow_up_icon_mouse_state: MouseStateHandle,

    /// To ensure we only have one run of completions-as-you-type at any given time,
    /// we keep an abort handle of the current run. If we have reason to start a new run
    /// (e.g. new input), we simply abort the existing run. The same applies to the
    /// syntax highlighting and autosuggestions features (all which use the completer).
    completions_abort_handle: Option<AbortHandle>,
    decorations_future_handle: Option<SpawnedFutureHandle>,
    autosuggestions_abort_handle: Option<AbortHandle>,

    pub prompt_render_helper: PromptRenderHelper,
    prompt_type: ModelHandle<PromptType>,
    // A cached copy of enable_autosuggestions from settings (to avoid
    // a settings read on every typed character).
    enable_autosuggestions_setting: bool,

    /// Manages the input state for a shared session.
    /// Is [`Some`] iff this is a viewer in a shared session.
    shared_session_input_state: Option<SharedSessionInputState>,

    /// Manages presence state for shared session.
    ///
    /// Only [`Some`] if this is a shared session.
    shared_session_presence_manager: Option<ModelHandle<PresenceManager>>,

    /// A cache of the local buffer operations for the latest instance
    /// of the input buffer. Specifically, these only include operations
    /// resulting from local changes to the buffer (not remote changes / operations).
    /// Note that the input buffer is reinstantiated every time a command is executed,
    /// while ultimately clears this set.
    ///
    /// Today, we only expect to use this with when starting
    /// a shared session.
    ///
    /// TODO (suraj): technically, we don't need the full
    /// history for _selections_; we just need the latest.
    latest_buffer_operations: Vec<CrdtOperation>,

    /// Incoming remote edits that are not yet applied
    /// because the block ID they were meant for was
    /// not active when these operations were received.
    ///
    /// When the buffer is reinstantiated, we check
    /// if any of these pending remote edits can be flushed.
    ///
    /// Today, we only expect to use this for shared session viewers.
    deferred_remote_operations: DeferredRemoteOperations,

    prompt_suggestions_banner_state: Option<PromptSuggestionBannerState>,
    /// Shared flag checked by the editor's keymap context modifier to determine whether
    /// to suppress the editor's ctrl-enter newline insertion when a prompt suggestion
    /// banner is pending.
    has_prompt_suggestion_banner: Arc<AtomicBool>,
    /// Whether the most recent intelligent autosuggestion was accepted or not.
    /// Cleared once a command is run.
    was_intelligent_autosuggestion_accepted: bool,
    /// We store info about the last intelligent autosuggestion because we need it for
    /// data collection when the command completes, but state is cleared when the command is executed.
    last_intelligent_autosuggestion_result: Option<IntelligentAutosuggestionResult>,
    next_command_model: ModelHandle<NextCommandModel>,

    /// The last block that the user ran. This is used for generating autosuggestions.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    last_user_block_completed: Option<UserBlockCompleted>,

    hoverable_handle: MouseStateHandle,

    #[cfg(feature = "local_fs")]
    conn: Option<Arc<Mutex<SqliteConnection>>>,

    /// Cached hint text to ensure it remains stable during shell initialization hooks
    cached_agent_mode_hint_text: Option<&'static str>,

    predict_am_queries_future_handle: Option<SpawnedFutureHandle>,

    attachment_chips: Vec<AttachmentChip>,

    is_processing_attached_images: bool,

    universal_developer_input_button_bar: ViewHandle<UniversalDeveloperInputButtonBar>,

    terminal_input_message_bar: ViewHandle<TerminalInputMessageBar>,

    agent_input_footer: ViewHandle<AgentInputFooter>,
    prompt_suggestions_view: ViewHandle<PromptSuggestionsView>,
    handoff_compose_state: ModelHandle<HandoffComposeState>,

    inline_slash_commands_view: ViewHandle<InlineSlashCommandView>,
    cloud_mode_v2_slash_commands_view: Option<ViewHandle<CloudModeV2SlashCommandView>>,
    slash_command_data_source: ModelHandle<SlashCommandDataSource>,
    cloud_mode_composer_slash_command_data_source: Option<ModelHandle<SlashCommandDataSource>>,

    /// Inline conversation menu for selecting AI conversations.
    inline_conversation_menu_view: ViewHandle<InlineConversationMenuView>,

    /// Inline plan menu for selecting among multiple plans.
    inline_plan_menu_view: ViewHandle<InlinePlanMenuView>,

    /// Inline repos switcher menu.
    inline_repos_menu_view: ViewHandle<InlineReposMenuView>,

    /// Inline model selector for choosing the Agent base model.
    inline_model_selector_view: ViewHandle<InlineModelSelectorView>,
    /// Inline profile selector for choosing the active execution profile.
    inline_profile_selector_view: ViewHandle<InlineProfileSelectorView>,

    /// Inline skill selector for /open-skill command.
    inline_skill_selector_view: ViewHandle<InlineSkillSelectorView>,

    /// Whether the skill selector should invoke (true) or open (false) the skill.
    skill_selector_should_invoke: bool,

    /// Inline prompts menu for /prompts command.
    inline_prompts_menu_view: ViewHandle<InlinePromptsMenuView>,

    /// Inline menu for selecting a query point when forking a conversation.
    user_query_menu_view: ViewHandle<UserQueryMenuView>,

    /// Inline menu for selecting a rewind point in a conversation.
    rewind_menu_view: ViewHandle<RewindMenuView>,

    /// Inline history menu for up-arrow with conversations and commands.
    inline_history_menu_view: ViewHandle<InlineHistoryMenuView>,

    pub(super) cloud_mode_v2_history_menu_view: Option<ViewHandle<CloudModeV2HistoryMenuView>>,

    inline_terminal_menu_positioner: ModelHandle<InlineMenuPositioner>,

    /// Model for managing slash command state.
    slash_command_model: ModelHandle<SlashCommandModel>,

    /// Cached flag indicating whether the editor buffer is empty, used to track changes between
    /// empty and non-empty states.
    ///
    /// If simply looking for if the editor contents empty, check the editor view directly instead
    /// of using this flag.
    is_editor_empty_on_last_edit: bool,

    /// Weak handle to this input view for drop target data
    weak_view_handle: WeakViewHandle<Input>,

    buy_credits_banner: ViewHandle<BuyCreditsBanner>,
    agent_status_view: ViewHandle<BlocklistAIStatusBar>,
    agent_view_controller: ModelHandle<AgentViewController>,
    agent_shortcut_view_model: ModelHandle<AgentShortcutViewModel>,
    ambient_agent_view_state: Option<AmbientAgentViewState>,
    ephemeral_message_model: ModelHandle<EphemeralMessageModel>,

    /// When a command is executed from a prompt chip (e.g. `cd` from the directory dropdown),
    /// we snapshot the current input contents here so we can restore them after the command
    /// completes and the buffer would normally be cleared.
    input_contents_before_prompt_chip_command: Option<String>,
}

struct AmbientAgentViewState {
    view_model: ModelHandle<AmbientAgentViewModel>,
    #[allow(dead_code)]
    harness_selector: ViewHandle<HarnessSelector>,
    host_selector: Option<ViewHandle<HostSelector>>,
    auth_secret_selector: Option<ViewHandle<AuthSecretSelector>>,
    auth_secret_ftux_view: Option<ViewHandle<AuthSecretFtuxView>>,
}

impl AmbientAgentViewState {
    fn view_model(&self) -> &ModelHandle<AmbientAgentViewModel> {
        &self.view_model
    }
}

#[derive(Clone)]
struct AttachmentChip {
    file_name: String,
    mouse_state_handle: MouseStateHandle,
    attachment_type: AttachmentType,
    /// Index into the unified pending_attachments list for deletion.
    index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligentAutosuggestionResult {
    #[serde(rename = "was_autosuggestion_accepted")]
    pub was_suggestion_accepted: bool,
    #[serde(rename = "was_autosuggestion_from_ai")]
    pub is_from_ai: bool,
    pub predicted_command: String,
}

/// A map of remote buffer operations that were deferred because
/// the corresponding block ID was not active when these operations
/// were received.
struct DeferredRemoteOperations {
    /// The latest block ID that we flushed for.
    latest_block_id: BlockId,

    /// The deferred operations.
    deferred_ops: HashMap<BlockId, Vec<CrdtOperation>>,
}

impl DeferredRemoteOperations {
    fn new(latest_block_id: BlockId) -> Self {
        Self {
            latest_block_id,
            deferred_ops: HashMap::new(),
        }
    }

    /// Defers the `operations` corresponding to the `block_id`.
    fn defer(&mut self, block_id: BlockId, operations: Vec<CrdtOperation>) {
        self.deferred_ops
            .entry(block_id)
            .or_default()
            .extend(operations);
    }

    /// Removes and returns the deferred operations for the latest block ID, if any.
    fn flush(&mut self) -> Option<Vec<CrdtOperation>> {
        self.deferred_ops.remove(&self.latest_block_id)
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    if cfg!(feature = "integration_tests") {
        app.register_fixed_bindings([
            // Hack: Add explicit ctrl-r binding for integration tests, since the tests' injected
            // keypresses won't trigger Mac menu items. Unfortunately we can't use
            // cfg[test] because we are a separate process!
            FixedBinding::new(
                "ctrl-r",
                WorkspaceAction::ShowCommandSearch(Default::default()),
                id!("Input") & !id!("VoltronActive"),
            ),
        ]);
    }

    app.register_fixed_bindings(vec![
        FixedBinding::new("ctrl-d", InputAction::CtrlD, id!("Input")),
        FixedBinding::custom(
            CustomAction::History,
            InputAction::Up,
            "Show History",
            // We need to ensure the workflow info box is not open as the "up" arrow
            // key is used to navigate the environment variables dropdown.
            // Same goes with the LLM menu.
            id!("Input")
                & !id!("IMEOpen")
                & !id!("VoltronActive")
                & !id!("WorkflowInfoBox")
                & !id!("ProfileModelSelectorOpen")
                & !id!("PromptChipMenuOpen")
                & !id!("AIContextMenuOpen")
                & !id!("BuyCreditsBannerOpen"),
        ),
    ]);

    app.register_editable_bindings([EditableBinding::new(
        "input:insert_network_logging_workflow",
        "Show Warp network log",
        WorkspaceAction::OpenNetworkLogPane,
    )
    .with_enabled(|| ContextFlag::NetworkLogConsole.is_enabled())]);

    app.register_editable_bindings([EditableBinding::new(
        "input:clear_screen",
        "Clear screen",
        InputAction::ClearScreen,
    )
    .with_context_predicate(id!("Input"))
    .with_key_binding("ctrl-l")]);

    app.register_editable_bindings([
        EditableBinding::new(
            "terminal:scroll_up_one_page",
            "Scroll terminal output up one page",
            InputAction::PageUp,
        )
        .with_context_predicate(id!("Input") & !id!("IMEOpen"))
        .with_key_binding("pageup"),
        EditableBinding::new(
            "terminal:scroll_down_one_page",
            "Scroll terminal output down one page",
            InputAction::PageDown,
        )
        .with_context_predicate(id!("Input") & !id!("IMEOpen"))
        .with_key_binding("pagedown"),
    ]);

    app.register_editable_bindings([EditableBinding::new(
        "workspace:edit_prompt",
        BindingDescription::new("Edit Prompt")
            .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Edit Prompt"),
        WorkspaceAction::OpenPromptEditor {
            open_source: PromptEditorOpenSource::CommandPalette,
        },
    )
    .with_group(bindings::BindingGroup::Settings.as_str())
    .with_context_predicate(
        id!("Input")
            & id!(SharedSessionStatus::ActiveSharer.as_keymap_context())
            & !id!("LongRunningCommand")
            & !id!(flags::ACTIVE_AGENT_VIEW)
            & !id!(flags::ACTIVE_INLINE_AGENT_VIEW),
    )]);

    if FeatureFlag::ClassicCompletions.is_enabled()
        && !FeatureFlag::ForceClassicCompletions.is_enabled()
    {
        app.register_editable_bindings([EditableBinding::new(
            "input:toggle_classic_completions_mode",
            "(Experimental) Toggle classic completions mode",
            InputAction::ToggleClassicCompletionsMode,
        )
        .with_context_predicate(id!("Input"))]);
    }

    // Register editable bindings relating to Command Search.
    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:show_command_search",
            "Command Search",
            WorkspaceAction::ShowCommandSearch(Default::default()),
        )
        // Only show command search if none of the input-related panels are open, and if we aren't
        // in Vim normal mode. Command Search is ctrl-r by default, and so is Redo in Vim (in
        // normal mode). So, the child should be allowed to handle this action first. Child views
        // normally do get first precedence to handle keybindings, but this is _not_ the case when
        // a parent view binds a CustomAction, which is what is happening here in the Input view.
        // Therefore, this binding is guarded with !id!("VimNormalMode"). Note that although there
        // is usually a conflict between these, that isn't always the case if the user has
        // re-mapped CommandSearch to something else. However, we don't account for that here.
        .with_context_predicate(id!("Input") & !id!("VoltronActive") & !id!("VimNormalMode"))
        .with_custom_action(CustomAction::CommandSearch),
        EditableBinding::new(
            "input:search_command_history",
            "History Search",
            WorkspaceAction::ShowCommandSearch(CommandSearchOptions {
                filter: Some(QueryFilter::History),
                init_content: Default::default(),
            }),
        )
        .with_context_predicate(id!("Input") & !id!("VoltronActive"))
        .with_custom_action(CustomAction::HistorySearch),
        EditableBinding::new(
            OPEN_COMPLETIONS_KEYBINDING_NAME,
            "Open completions menu",
            InputAction::MaybeOpenCompletionSuggestions,
        )
        .with_context_predicate(id!("Input"))
        .with_key_binding("tab"),
    ]);

    if let Some(custom_action) = workflows::CategoriesView::custom_action() {
        app.register_editable_bindings([EditableBinding::new(
            "input:toggle_workflows",
            "Workflows",
            InputAction::SelectAndRefreshVoltron(VoltronItem::Workflows),
        )
        .with_context_predicate(id!("Input"))
        .with_custom_action(custom_action)]);
    }

    if ChannelState::channel() == Channel::Integration {
        app.register_fixed_bindings([
            // Hack: Add explicit bindings for the tests, since the tests' injected
            // keypresses won't trigger Mac menu items. Unfortunately we can't use
            // cfg[test] because we are a separate process!
            FixedBinding::new(
                "ctrl-shift-R",
                InputAction::SelectAndRefreshVoltron(VoltronItem::Workflows),
                id!("Input"),
            ),
        ]);
    }

    app.register_editable_bindings([
        EditableBinding::new(
            "input:toggle_natural_language_command_search",
            "Open AI Command Suggestions",
            InputAction::ShowAiCommandSearch,
        )
        .with_context_predicate(
            id!("Input")
                & !id!(SharedSessionStatus::reader().as_keymap_context())
                & id!(flags::IS_ANY_AI_ENABLED)
                & !id!("AIInput"),
        )
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_custom_action(CustomAction::AISearch),
        EditableBinding::new(
            START_NEW_CONVERSATION_KEYBINDING_NAME,
            "New agent conversation",
            InputAction::StartNewAgentConversation,
        )
        .with_enabled(|| !FeatureFlag::AgentView.is_enabled())
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_context_predicate(
            id!("Input") & id!(flags::IS_ANY_AI_ENABLED) & id!("TerminalView_NonEmptyBlockList"),
        )
        .with_mac_key_binding("cmd-shift-N")
        .with_linux_or_windows_key_binding("ctrl-alt-shift-N"),
        EditableBinding::new(
            "input:enable_auto_detection",
            "Trigger Auto Detection",
            InputAction::EnableAutoDetection,
        )
        .with_enabled(|| FeatureFlag::AgentMode.is_enabled())
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_context_predicate(
            id!("Input")
                & id!("UniversalDeveloperInput")
                & id!(flags::IS_ANY_AI_ENABLED)
                & !id!("IMEOpen"),
        )
        .with_key_binding("alt-shift-I"),
        EditableBinding::new(
            "input:clear_and_reset_ai_context_menu_query",
            "Clear and reset AI context menu query",
            InputAction::ClearAndResetAIContextMenuQuery,
        )
        .with_context_predicate(id!("Input") & id!("AIContextMenuOpen") & !id!("IMEOpen"))
        .with_mac_key_binding("cmd-shift-backspace")
        .with_linux_or_windows_key_binding("ctrl-shift-backspace"),
    ]);

    let slash_command_bindings = COMMAND_REGISTRY
        .all_commands()
        .map(|command| {
            use crate::search::slash_command_menu::static_commands::{
                bindings as slash_command_bindings, bindings::DefaultSlashCommandBinding,
            };

            let context_predicate = id!("Input")
                & !id!("IMEOpen")
                & id!(command.name)
                & !id!(flags::ACTIVE_INLINE_AGENT_VIEW)
                & (id!(flags::ACTIVE_AGENT_VIEW) | id!(flags::SLASH_COMMANDS_IN_TERMINAL_FLAG));

            let mut binding = EditableBinding::new(
                command.name,
                slash_command_bindings::binding_description(command),
                InputAction::TriggerSlashCommandFromKeybinding(command.name),
            )
            .with_context_predicate(context_predicate);

            binding = match slash_command_bindings::default_binding_for_command(command.name) {
                DefaultSlashCommandBinding::None => binding,
                DefaultSlashCommandBinding::Single(keys) => binding.with_key_binding(keys),
                DefaultSlashCommandBinding::PerPlatform(keys) => binding
                    .with_mac_key_binding(keys.mac)
                    .with_linux_or_windows_key_binding(keys.linux_and_windows),
            };

            binding
        })
        .collect::<Vec<_>>();

    app.register_editable_bindings(slash_command_bindings);

    // Fixed bindings for passive code diffs
    app.register_fixed_bindings([FixedBinding::new(
        cmd_or_ctrl_shift("e"),
        InputAction::TryHandlePassiveCodeDiff(CodeDiffAction::Edit),
        id!("Input")
            & id!(flags::CODE_SUGGESTIONS_FLAG)
            & id!(flags::PASSIVE_CODE_DIFF_KEYBINDINGS_ENABLED),
    )]);

    if FeatureFlag::AgentView.is_enabled() {
        app.register_fixed_bindings([FixedBinding::new(
            "shift-?",
            InputAction::ToggleAgentViewShortcuts,
            id!("Input")
                & !id!("IMEOpen")
                & id!(flags::EMPTY_INPUT_BUFFER)
                & id!(flags::ACTIVE_AGENT_VIEW)
                & !id!("LongRunningCommand")
                & !(id!(flags::TERMINAL_MODE_INPUT) & id!(flags::LOCKED_INPUT)),
        )]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompletionsTrigger {
    Keybinding,
    AsYouType,
}

/// Represents whether the input editor should render the subshell flag.
#[derive(Clone, Debug)]
enum SubshellRenderState {
    /// Contains the subshell-spawning command for the flag. Render the flag
    /// and extend the flag into the input editor.
    Flag(SubshellSource),
    /// The input is inside a subshell, extend the flag into the input editor,
    /// but do not render the actual flag.
    Flagpole,
}

/// Represents whether a command is currently being executed.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Executing {
    Yes,
    No,
}

impl Input {
    pub fn send_input_buffer_to_terminal_editor(
        &mut self,
        buffer_contents: Arc<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_for_syncing_inputs(buffer_contents, ctx);
        });
    }

    pub fn run_command_in_synced_terminal_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.has_pending_command = true;
        self.execute_pending_command(ctx);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        tips_completed: ModelHandle<TipsCompleted>,
        server_api: Arc<ServerApi>,
        sessions: ModelHandle<Sessions>,
        size_info: SizeInfo,
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        current_prompt: ModelHandle<PromptType>,
        ai_controller: ModelHandle<BlocklistAIController>,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        ai_input_model: ModelHandle<BlocklistAIInputModel>,
        ai_action_model: ModelHandle<BlocklistAIActionModel>,
        cli_subagent_controller: ModelHandle<CLISubagentController>,
        terminal_view_id: EntityId,
        current_repo_path: Option<PathBuf>,
        model_events: ModelHandle<crate::terminal::model_events::ModelEventDispatcher>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        active_session: ModelHandle<ActiveSession>,
        ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let initial_session_context = {
            let completer_data = CompleterData::new(
                sessions.clone(),
                None, // active_block_metadata will be set later when blocks are available
                CommandRegistry::global_instance(),
                None, // last_user_block_completed will be set later
            );
            completer_data.completion_session_context(ctx)
        };

        let is_shared_session_viewer = model.lock().shared_session_status().is_viewer();
        let handoff_compose_state = ctx.add_model(|_ctx| HandoffComposeState::default());
        ctx.subscribe_to_model(&handoff_compose_state, |me, _, _, ctx| {
            me.set_zero_state_hint_text(ctx);
            ctx.notify();
        });

        let footer_display_chip_config = DisplayChipConfig {
            ai_input_model: ai_input_model.clone(),
            ai_context_model: ai_context_model.clone(),
            terminal_view_id,
            menu_positioning_provider: menu_positioning_provider.clone(),
            session_context: initial_session_context.clone(),
            current_repo_path: current_repo_path.clone(),
            model_events: model_events.clone(),
            is_shared_session_viewer,
            agent_view_controller: agent_view_controller.clone(),
            ambient_agent_view_model: ambient_agent_view_model.clone(),
        };

        let prompt_view = ctx.add_typed_action_view(|ctx| {
            PromptDisplay::new(
                current_prompt.clone(),
                ai_input_model.clone(),
                ai_context_model.clone(),
                terminal_view_id,
                menu_positioning_provider.clone(),
                initial_session_context.clone(),
                current_repo_path.clone(),
                model_events.clone(),
                agent_view_controller.clone(),
                is_shared_session_viewer,
                ctx,
            )
        });
        ctx.subscribe_to_view(&prompt_view, |me, _, event, ctx| {
            me.handle_prompt_event(event, ctx);
        });
        ctx.subscribe_to_model(&Appearance::handle(ctx), move |me, _, event, ctx| {
            if let AppearanceEvent::ThemeChanged = event {
                me.handle_theme_change(ctx);
            }
        });
        // Keep the rich input editor's text colors legible against alt-screen
        // CLI agent backgrounds (e.g. OpenCode) when the terminal enters/exits
        // the alt screen.
        ctx.subscribe_to_model(&model_events, |me, _, event, ctx| {
            if let crate::terminal::model_events::ModelEvent::TerminalModeSwapped(_) = event {
                me.update_cli_agent_editor_text_colors(ctx);
            }
        });
        ctx.subscribe_to_model(&TerminalSettings::handle(ctx), move |_, _, event, ctx| {
            if let TerminalSettingsChangedEvent::Spacing { .. } = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&agent_view_controller, |me, _, event, ctx| {
            use crate::ai::blocklist::agent_view::AgentViewControllerEvent;
            if let AgentViewControllerEvent::EnteredAgentView { origin, .. } = event {
                me.close_suggestion_modes_for_new_conversation(ctx);
                // Entering Agent View can remove multiline same-line prompt decorator content in a
                // single render pass. Reset shrink-delay so we don't hold onto stale input height
                // for one frame (which shows up as extra bottom padding/jitter).
                me.editor.update(ctx, |editor, ctx| {
                    editor.reset_height_shrink_delay(ctx);
                });

                if *origin == AgentViewEntryOrigin::CloudAgent {
                    // By default, shared session viewers cannot edit the input - override that for composing ambient agent queries.
                    me.editor.update(ctx, |editor, ctx| {
                        editor.set_interaction_state(InteractionState::Editable, ctx);
                    });
                    me.set_zero_state_hint_text(ctx);
                }
            }
            ctx.notify();
        });

        if let Some(ambient_agent_view_model) = ambient_agent_view_model.as_ref() {
            ctx.subscribe_to_model(ambient_agent_view_model, |me, handle, event, ctx| {
                let is_ambient = handle.as_ref(ctx).is_ambient_agent();
                me.editor.update(ctx, |editor, ctx| {
                    if let Some(ai_context_menu) = editor.ai_context_menu() {
                        ai_context_menu.update(ctx, |menu, ctx| {
                            menu.set_is_in_ambient_agent(is_ambient, ctx);
                        });
                    }
                });
                // Surface async snapshot upload failures as a toast.
                if let AmbientAgentViewModelEvent::HandoffSnapshotUploadFailed { error_message } =
                    event
                {
                    let window_id = ctx.window_id();
                    let toast_message = format!("Failed to prepare cloud handoff: {error_message}");
                    ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                        ts.add_ephemeral_toast(
                            DismissibleToast::error(toast_message),
                            window_id,
                            ctx,
                        );
                    });
                }

                // Re-render on status-footer transitions and on status-affecting events that
                // decide whether the input is in its composing shape.
                let should_notify = handle.as_ref(ctx).should_show_status_footer()
                    || matches!(
                        event,
                        AmbientAgentViewModelEvent::EnteredSetupState
                            | AmbientAgentViewModelEvent::EnteredComposingState
                            | AmbientAgentViewModelEvent::DispatchedAgent
                            | AmbientAgentViewModelEvent::SessionReady { .. }
                            | AmbientAgentViewModelEvent::Failed { .. }
                            | AmbientAgentViewModelEvent::Cancelled
                            | AmbientAgentViewModelEvent::NeedsGithubAuth
                            | AmbientAgentViewModelEvent::HarnessSelected
                            | AmbientAgentViewModelEvent::PendingHandoffChanged
                            | AmbientAgentViewModelEvent::HandoffSnapshotUploadFailed { .. }
                    );

                if should_notify {
                    me.set_zero_state_hint_text(ctx);
                    ctx.notify();
                }
            });
        }

        let prompt_selection_state_handle = SelectionHandle::default();

        let view_id = ctx.view_id();

        let input_render_state_model_handle: ModelHandle<InputRenderStateModel> =
            ctx.add_model(|_| InputRenderStateModel::new(false, size_info));

        let universal_developer_input_button_bar = ctx.add_typed_action_view(|ctx| {
            UniversalDeveloperInputButtonBar::new(
                menu_positioning_provider.clone(),
                terminal_view_id,
                ai_input_model.clone(),
                cli_subagent_controller.clone(),
                ambient_agent_view_model.clone(),
                model.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_view(
            &universal_developer_input_button_bar,
            |me, _, event, ctx| {
                me.handle_universal_developer_input_button_bar_event(event, ctx);
            },
        );
        let agent_input_footer = ctx.add_typed_action_view(|ctx| {
            AgentInputFooter::new(
                menu_positioning_provider.clone(),
                terminal_view_id,
                ai_input_model.clone(),
                model.clone(),
                ambient_agent_view_model.clone(),
                handoff_compose_state.clone(),
                current_prompt.clone(),
                footer_display_chip_config.clone(),
                ctx,
            )
        });

        let mut ambient_agent_view_state =
            ambient_agent_view_model
                .as_ref()
                .map(|view_model| AmbientAgentViewState {
                    view_model: view_model.clone(),
                    harness_selector: {
                        let harness_selector = ctx.add_typed_action_view(|ctx| {
                            HarnessSelector::new(
                                menu_positioning_provider.clone(),
                                view_model.clone(),
                                ctx,
                            )
                        });
                        if FeatureFlag::CloudModeInputV2.is_enabled() {
                            harness_selector.update(ctx, |selector, ctx| {
                                selector.set_button_theme(NakedHeaderButtonTheme, ctx);
                            });
                        }
                        // Mirror the V2 model selector / host selector refocus path: when the
                        // harness selector menu closes (item picked or dismissed via Esc /
                        // click-outside), restore focus to the input editor so typing resumes
                        // immediately. This powers the "input is focused after the harness
                        // selector closes" UX for the `/harness` slash command.
                        ctx.subscribe_to_view(&harness_selector, |me, _, event, ctx| {
                            let HarnessSelectorEvent::MenuVisibilityChanged { open } = event;
                            if !*open {
                                me.focus_input_box(ctx);
                            }
                        });
                        harness_selector
                    },
                    host_selector: if FeatureFlag::CloudModeInputV2.is_enabled() {
                        let view = ctx.add_typed_action_view(|ctx| {
                            HostSelector::new(menu_positioning_provider.clone(), ctx)
                        });
                        // Env var takes priority over workspace setting for developer testing.
                        let effective_host = std::env::var("WARP_CLOUD_MODE_DEFAULT_HOST")
                            .ok()
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                UserWorkspaces::as_ref(ctx)
                                    .default_host_slug()
                                    .map(String::from)
                            });
                        if let Some(slug) = &effective_host {
                            view.update(ctx, |selector, ctx| {
                                selector.set_default_host(slug.clone(), ctx);
                            });
                        }
                        if let Some(slug) = effective_host {
                            view_model.update(ctx, |model, _ctx| {
                                model.set_worker_host(Some(slug));
                            });
                        }
                        // When the host selector menu closes (item picked or dismissed via
                        // Esc / click-outside), restore focus to the input editor so typing
                        // resumes immediately.
                        ctx.subscribe_to_view(&view, |me, _, event, ctx| {
                            if matches!(
                                event,
                                HostSelectorEvent::MenuVisibilityChanged { open: false }
                            ) {
                                me.focus_input_box(ctx);
                            }
                        });
                        // Propagate host selection changes to the view model when a host is
                        // explicitly selected, rather than on menu close, to avoid a race
                        // where the menu closes before the selection updates.
                        let vm_for_host = view_model.clone();
                        ctx.subscribe_to_view(&view, move |_me, handle, event, ctx| {
                            if matches!(event, HostSelectorEvent::HostSelected) {
                                let selected = handle.as_ref(ctx).selected().clone();
                                vm_for_host.update(ctx, |model, _ctx| {
                                    model.set_worker_host(selected.worker_host_value());
                                });
                            }
                        });
                        // Keep the host selector and view model in sync when workspace
                        // metadata refreshes (e.g. admin changes default_host_slug).
                        let view_for_ws = view.clone();
                        let vm_for_ws = view_model.clone();
                        ctx.subscribe_to_model(
                            &UserWorkspaces::handle(ctx),
                            move |_me, _, event, ctx| {
                                if !matches!(event, UserWorkspacesEvent::TeamsChanged) {
                                    return;
                                }
                                let effective_host = std::env::var("WARP_CLOUD_MODE_DEFAULT_HOST")
                                    .ok()
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| {
                                        UserWorkspaces::as_ref(ctx)
                                            .default_host_slug()
                                            .map(String::from)
                                    });
                                if let Some(slug) = &effective_host {
                                    view_for_ws.update(ctx, |selector, ctx| {
                                        selector.set_default_host(slug.clone(), ctx);
                                    });
                                }
                                if let Some(slug) = effective_host {
                                    vm_for_ws.update(ctx, |model, _ctx| {
                                        model.set_worker_host(Some(slug));
                                    });
                                }
                            },
                        );
                        Some(view)
                    } else {
                        None
                    },
                    auth_secret_selector: None,
                    auth_secret_ftux_view: None,
                });
        if let Some(state) = ambient_agent_view_state.as_mut() {
            if FeatureFlag::CloudModeInputV2.is_enabled() {
                let selector = ctx.add_typed_action_view(|ctx| {
                    AuthSecretSelector::new(
                        menu_positioning_provider.clone(),
                        state.view_model.clone(),
                        ctx,
                    )
                });
                ctx.subscribe_to_view(&selector, |me, _, event, ctx| match event {
                    AuthSecretSelectorEvent::MenuVisibilityChanged { open: false } => {
                        me.focus_input_box(ctx);
                    }
                    AuthSecretSelectorEvent::NewTypeSelected {
                        harness,
                        type_index,
                    } => {
                        if let Some(ftux_view) = me.auth_secret_ftux_view().cloned() {
                            let harness = *harness;
                            let type_index = *type_index;
                            ftux_view.update(ctx, |view, ctx| {
                                view.enter_creation_state_public(harness, type_index, ctx);
                            });
                        }
                        ctx.notify();
                    }
                    AuthSecretSelectorEvent::MenuVisibilityChanged { open: true } => {}
                });
                let initial_harness = state.view_model.as_ref(ctx).selected_harness();
                let ftux_view =
                    ctx.add_typed_action_view(|ctx| AuthSecretFtuxView::new(initial_harness, ctx));

                // Forward the pane's harness changes into the FTUX view.
                let ftux_for_harness_sub = ftux_view.clone();
                ctx.subscribe_to_model(&state.view_model, move |_me, vm, event, ctx| {
                    if matches!(event, AmbientAgentViewModelEvent::HarnessSelected) {
                        let harness = vm.as_ref(ctx).selected_harness();
                        ftux_for_harness_sub.update(ctx, |view, ctx| {
                            view.set_harness(harness, ctx);
                        });
                    }
                });

                // Cloud-mode side effects on FTUX events: update the pane's
                // harness auth secret, persist `last_selected_auth_secret`,
                // mark FTUX completed.
                let vm_for_events = state.view_model.clone();
                ctx.subscribe_to_view(&ftux_view, move |_me, _, event, ctx| match event {
                    AuthSecretFtuxViewEvent::SecretSelected { harness, name }
                    | AuthSecretFtuxViewEvent::Created { harness, name } => {
                        let harness = *harness;
                        let name = name.clone();
                        vm_for_events.update(ctx, |model, ctx| {
                            model.set_harness_auth_secret_name(Some(name.clone()), ctx);
                        });
                        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                            settings.mark_harness_auth_ftux_completed(harness, ctx);
                            let mut map = settings.last_selected_auth_secret.value().clone();
                            map.insert(harness.config_name().to_string(), name);
                            let _ = settings.last_selected_auth_secret.set_value(map, ctx);
                        });
                    }
                    AuthSecretFtuxViewEvent::Cancelled => {
                        vm_for_events.update(ctx, |model, ctx| {
                            model.set_harness(Harness::Oz, ctx);
                        });
                    }
                    AuthSecretFtuxViewEvent::Skipped { harness } => {
                        let harness = *harness;
                        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                            settings.mark_harness_auth_ftux_completed(harness, ctx);
                        });
                    }
                    AuthSecretFtuxViewEvent::Failed { .. } => {}
                });

                state.auth_secret_selector = Some(selector);
                state.auth_secret_ftux_view = Some(ftux_view);
            }
        }
        ctx.subscribe_to_view(&agent_input_footer, |me, _, event, ctx| {
            match event {
                #[cfg(feature = "voice_input")]
                AgentInputFooterEvent::ToggleVoiceInput(from) => {
                    me.toggle_voice_input(from, ctx);
                }
                AgentInputFooterEvent::SelectFile => {
                    me.select_image(ctx);
                }
                AgentInputFooterEvent::StartRemoteControl
                | AgentInputFooterEvent::StopRemoteControl => {
                    // Handled by UseAgentToolbar's subscription, not here.
                }
                // These events are handled by UseAgentToolbar's subscription.
                // The UseAgentToolbar shares this same AgentInputFooter instance,
                // so its subscriber always fires alongside ours for every chip click.
                AgentInputFooterEvent::WriteToPty(_)
                | AgentInputFooterEvent::InsertIntoCLIRichInput(_)
                | AgentInputFooterEvent::ToggleCodeReviewPane(_)
                | AgentInputFooterEvent::ToggleFileExplorer(_)
                | AgentInputFooterEvent::OpenRichInput
                | AgentInputFooterEvent::HideRichInput => {}
                AgentInputFooterEvent::ToggledChipMenu { open } => {
                    me.handle_prompt_event(&PromptDisplayEvent::ToggleMenu { open: *open }, ctx);
                }
                AgentInputFooterEvent::TryExecuteChipCommand(cmd) => {
                    me.handle_prompt_event(
                        &PromptDisplayEvent::TryExecuteCommand(cmd.clone()),
                        ctx,
                    );
                }
                AgentInputFooterEvent::PromptAlert(prompt_alert_event) => {
                    me.handle_prompt_alert(prompt_alert_event, ctx);
                }
                AgentInputFooterEvent::ModelSelectorOpened => {
                    me.close_overlays(false, ctx);
                }
                AgentInputFooterEvent::ModelSelectorClosed
                | AgentInputFooterEvent::EnvironmentSelectorClosed => {
                    me.focus_input_box(ctx);
                }
                AgentInputFooterEvent::ToggleInlineModelSelector { initial_tab } => {
                    if me
                        .suggestions_mode_model
                        .as_ref(ctx)
                        .is_inline_model_selector()
                    {
                        me.suggestions_mode_model.update(ctx, |model, ctx| {
                            model.set_mode(InputSuggestionsMode::Closed, ctx);
                        });
                        ctx.notify();
                    } else {
                        me.close_overlays(false, ctx);
                        let has_input = !me.editor.as_ref(ctx).buffer_text(ctx).is_empty();
                        me.inline_model_selector_view.update(ctx, |v, ctx| {
                            if has_input {
                                v.set_filter_results_by_input(false);
                            }
                            v.set_active_tab(*initial_tab, ctx);
                        });
                        me.open_model_selector(ctx);
                    }
                }
                AgentInputFooterEvent::OpenSettings(section) => {
                    ctx.emit(Event::OpenSettings(*section));
                }
                AgentInputFooterEvent::OpenCodeReview => {
                    ctx.emit(Event::OpenCodeReviewPane);
                }
                AgentInputFooterEvent::OpenAIDocument {
                    document_id,
                    document_version,
                } => {
                    ctx.emit(Event::ToggleAIDocumentPane {
                        document_id: *document_id,
                        document_version: *document_version,
                    });
                }
                AgentInputFooterEvent::ShowContextMenu { position } => {
                    let position_id = format!("prompt_area_{}", me.view_id);
                    let offset = if let Some(prompt_rect) = ctx.element_position_by_id(&position_id)
                    {
                        *position - prompt_rect.origin()
                    } else {
                        *position
                    };
                    ctx.dispatch_typed_action(&TerminalAction::PromptContextMenu {
                        position_offset_from_prompt: offset,
                    });
                }
                AgentInputFooterEvent::OpenEnvironmentManagementPane => {
                    ctx.emit(Event::OpenEnvironmentManagementPane);
                }
                AgentInputFooterEvent::PluginInstalled(agent) => {
                    ctx.emit(Event::RegisterPluginListener(*agent));
                }
                #[cfg(not(target_family = "wasm"))]
                AgentInputFooterEvent::OpenPluginInstructionsPane(agent, kind) => {
                    ctx.emit(Event::OpenPluginInstructionsPane(*agent, *kind));
                }
                AgentInputFooterEvent::OpenHandoffPane => {
                    me.activate_cloud_handoff_compose(HandoffEntryPoint::FooterChip, ctx);
                }
            }
        });
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            let CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id,
                new_input_state,
                ..
            } = event
            else {
                return;
            };
            if *terminal_view_id != me.terminal_view_id {
                return;
            }

            match new_input_state {
                CLIAgentInputState::Open { .. } => {
                    // Input just opened — switch to agent mode.
                    me.set_input_mode_agent(true, ctx);
                    me.clear_buffer_and_reset_undo_stack(ctx);

                    // Restore any draft text saved when the composer was last
                    // closed, so the user doesn't lose work-in-progress.
                    let terminal_view_id = me.terminal_view_id;
                    let draft = CLIAgentSessionsModel::handle(ctx)
                        .update(ctx, |sessions_model, _| {
                            sessions_model.take_draft(terminal_view_id)
                        });
                    if let Some(draft) = draft {
                        me.replace_buffer_content(&draft, ctx);
                    }
                }
                CLIAgentInputState::Closed => {
                    // Input just closed — clear the buffer.
                    me.clear_buffer_and_reset_undo_stack(ctx);
                }
            }

            // Set the CLI agent flag after the mode switch so that
            // refresh_categories_state sees the correct is_ai_or_autodetect_mode.
            let is_cli_agent_input = matches!(new_input_state, CLIAgentInputState::Open { .. });
            me.editor.update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |menu, ctx| {
                        menu.set_is_cli_agent_input(is_cli_agent_input, ctx);
                    });
                }
            });
            // Sync the editor text colors with the (now active or inactive)
            // alt-screen CLI agent background so input text stays legible.
            me.update_cli_agent_editor_text_colors(ctx);
            me.set_zero_state_hint_text(ctx);
            ctx.notify();
        });

        let prompt_render_helper = PromptRenderHelper::new(
            sessions.clone(),
            prompt_view,
            prompt_selection_state_handle,
            view_id,
            input_render_state_model_handle.clone(),
            ai_input_model.clone(),
        );

        let next_command_model = ctx.add_model(|_| {
            NextCommandModel::new(sessions.clone(), model.clone(), server_api.clone())
        });
        ctx.subscribe_to_model(&next_command_model, |me, _, event, ctx| {
            me.handle_next_command_model_event(event, ctx);
        });

        let ai_follow_up_icon_mouse_state = MouseStateHandle::default();
        let has_prompt_suggestion_banner = Arc::new(AtomicBool::new(false));
        let editor = {
            // Clones used in render_decorator_elements closure below.
            let prompt_render_helper_clone = prompt_render_helper.clone();
            let model_clone = model.clone();
            // Clone used in keymap_context_modifier closure below.
            let terminal_model_for_keymap_context = model.clone();
            let has_prompt_suggestion_banner_for_keymap = has_prompt_suggestion_banner.clone();
            let input_render_state_model_handle_clone = input_render_state_model_handle.clone();

            let ai_context_model_clone = ai_context_model.clone();
            let ai_input_model = ai_input_model.clone();

            ctx.subscribe_to_model(&ai_input_model, |me, _, _, ctx| {
                #[cfg(feature = "voice_input")]
                me.update_voice_transcription_options(ctx);
                me.update_image_context_options(ctx);
                me.update_ai_context_menu(ctx);
                me.check_slash_menu_disabled_state(ctx);
            });

            let ai_input_model_clone = ai_input_model.clone();
            let ai_follow_up_icon_mouse_state_clone = ai_follow_up_icon_mouse_state.clone();
            let agent_view_controller_clone = agent_view_controller.clone();
            let other_agent_view_controller_clone = agent_view_controller.clone();
            let handoff_compose_state_for_decorator = handoff_compose_state.clone();

            ctx.add_typed_action_view(|ctx| {
                let options = EditorOptions {
                    autogrow: true,
                    autocomplete_symbols: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_horizontal_navigation_keys:
                        PropagateHorizontalNavigationKeys::AtBoundary,
                    propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
                    soft_wrap: true,
                    // TODO: Revisit terminal-style Ctrl-E behavior for multiline prompts.
                    line_start_behavior: LineStartBehavior::PreviousLineWhenAtStart,
                    supports_vim_mode: true,
                    use_settings_line_height_ratio: true,
                    render_decorator_elements: Some(Box::new(
                        move |app| -> EditorDecoratorElements {
                            let terminal_model = model_clone.lock();
                            let active_block = terminal_model.block_list().active_block();

                            let mut editor_decorator_elements = EditorDecoratorElements::default();

                            let is_universal_developer_input_enabled = InputSettings::as_ref(app)
                                .is_universal_developer_input_enabled(app);

                            if (!FeatureFlag::AgentView.is_enabled()
                                || !agent_view_controller_clone.as_ref(app).is_active())
                                && should_render_prompt_using_editor_decorator_elements(
                                    is_universal_developer_input_enabled,
                                    &ai_input_model,
                                    &terminal_model,
                                    app,
                                )
                            {
                                let SameLinePromptElements {
                                    lprompt_top,
                                    lprompt_bottom,
                                    rprompt,
                                } = prompt_render_helper_clone.render_same_line_prompt_areas(
                                    &terminal_model,
                                    Appearance::as_ref(app),
                                    app,
                                );

                                editor_decorator_elements.top_section = lprompt_top;
                                editor_decorator_elements.left_notch = lprompt_bottom;
                                editor_decorator_elements.right_notch = rprompt;
                                editor_decorator_elements.right_notch_offset_px = Some(
                                    active_block.rprompt_render_offset(
                                        &input_render_state_model_handle_clone
                                            .as_ref(app)
                                            .size_info,
                                    ),
                                )
                            }

                            // Render the AI mode indicator to the left of the editor if we're in AI mode or the AI suggested a command.
                            // Also renders the reply icon when following up in an existing conversation.
                            if let Some(ai_input_indicator) = maybe_render_ai_input_indicators(
                                &ai_input_model,
                                &ai_context_model_clone,
                                &agent_view_controller_clone,
                                &handoff_compose_state_for_decorator,
                                ai_follow_up_icon_mouse_state_clone.clone(),
                                terminal_view_id,
                                app,
                            ) {
                                editor_decorator_elements.left_notch =
                                    match editor_decorator_elements.left_notch {
                                        Some(left_notch) => {
                                            // If there is already a left notch, place the  to
                                            // the right of the notch to keep the pill immediately
                                            // to the left of the editor.
                                            Some(
                                                Flex::row()
                                                    .with_child(left_notch)
                                                    .with_child(ai_input_indicator)
                                                    .finish(),
                                            )
                                        }
                                        None => Some(ai_input_indicator),
                                    }
                            }

                            editor_decorator_elements
                        },
                    )),
                    cursor_colors_fn: Box::new(move |app| {
                        let is_ai_input_enabled =
                            ai_input_model_clone.as_ref(app).is_ai_input_enabled();
                        let appearance = Appearance::as_ref(app);
                        if is_ai_input_enabled {
                            let color_identifier = if FeatureFlag::AgentView.is_enabled() {
                                AnsiColorIdentifier::Magenta
                            } else {
                                AnsiColorIdentifier::Yellow
                            };
                            let cursor_color = color_identifier
                                .to_ansi_color(&appearance.theme().terminal_colors().normal);
                            let selection_color = ColorU::new(
                                cursor_color.r,
                                cursor_color.g,
                                cursor_color.b,
                                // Text selection color tones down the alpha to 40%.
                                (0.4 * 255.) as u8,
                            );

                            CursorColors {
                                cursor: cursor_color.into(),
                                selection: selection_color.into(),
                            }
                        } else {
                            default_cursor_colors(app)
                        }
                    }),
                    baseline_position_computation_method: BaselinePositionComputationMethod::Grid,
                    // We implement middle-click paste at the [`TerminalView`] level,
                    // and we don't want to double-paste.
                    middle_click_paste: false,
                    allow_user_cursor_preference: true,
                    #[cfg(not(target_family = "wasm"))]
                    include_ai_context_menu: true,
                    #[cfg(target_family = "wasm")]
                    include_ai_context_menu: false,
                    delegate_paste_handling: true,
                    keymap_context_modifier: Some(Box::new(move |context, app| {
                        context
                            .set
                            .insert(flags::TERMINAL_INPUT_PAGE_KEYS_HANDLED_BY_INPUT);

                        // When ctrl-enter is bound to accepting prompt suggestions and there's
                        // a pending passive code diff, suggested prompt, or prompt suggestion
                        // banner, set a flag so the editor's ctrl-enter binding doesn't match
                        // (allowing the terminal-level binding to handle it).
                        if is_accept_prompt_suggestion_bound_to_ctrl_enter(app)
                            && (has_pending_code_or_unit_test_prompt_suggestion(
                                &terminal_model_for_keymap_context.lock(),
                                app,
                            ) || has_prompt_suggestion_banner_for_keymap
                                .load(Ordering::Relaxed))
                        {
                            context
                                .set
                                .insert(flags::CTRL_ENTER_ACCEPTS_PROMPT_SUGGESTION);
                        }

                        if FeatureFlag::AgentView.is_enabled() {
                            context.set.insert(flags::AGENT_VIEW_ENABLED);
                        }

                        if !other_agent_view_controller_clone.as_ref(app).is_active()
                            && !cfg!(target_os = "macos")
                        {
                            context.set.insert(flags::CTRL_ENTER_ENTERS_AGENT_VIEW);
                        }

                        if CLIAgentSessionsModel::as_ref(app).is_input_open(terminal_view_id) {
                            context.set.insert(flags::CLI_AGENT_RICH_INPUT_OPEN);
                        }
                    })),
                    ..Default::default()
                };
                EditorView::new(options, ctx)
                    .with_next_command_model(next_command_model.clone())
                    .with_context_model(ai_context_model.clone())
            })
        };

        let buffer_model = ctx.add_model(|ctx| InputBufferModel::new(&editor, ctx));
        let suggestions_mode_model =
            ctx.add_model(|_| InputSuggestionsModeModel::new(buffer_model.clone()));

        let terminal_content_element_position_id =
            format!("terminal_content_element_{terminal_view_id}");
        let input_save_position_id = format!("status_free_input_{}", ctx.view_id());
        let window_id = ctx.window_id();
        let inline_terminal_menu_positioner = ctx.add_model(|ctx| {
            InlineMenuPositioner::new(
                &suggestions_mode_model,
                &agent_view_controller,
                terminal_content_element_position_id,
                input_save_position_id,
                size_info,
                window_id,
                ctx,
            )
        });

        let inline_history_menu_view = ctx.add_view({
            let active_session = active_session.clone();
            let buffer_model = buffer_model.clone();
            |ctx| {
                inline_history::InlineHistoryMenuView::new(
                    terminal_view_id,
                    active_session,
                    &suggestions_mode_model,
                    agent_view_controller.clone(),
                    &inline_terminal_menu_positioner,
                    buffer_model,
                    ctx,
                )
            }
        });
        if FeatureFlag::InlineHistoryMenu.is_enabled() {
            ctx.subscribe_to_view(&inline_history_menu_view, |me, _, event, ctx| {
                if me.is_cloud_mode_input_v2_composing(ctx) {
                    return;
                }
                me.handle_inline_history_menu_event(event, ctx);
            });
        }
        let inline_history_model = inline_history_menu_view.as_ref(ctx).model().clone();

        let cloud_mode_v2_history_menu_view = if FeatureFlag::CloudModeInputV2.is_enabled() {
            let view = ctx.add_view({
                let active_session = active_session.clone();
                let buffer_model = buffer_model.clone();
                let agent_view_controller = agent_view_controller.clone();
                |ctx| {
                    CloudModeV2HistoryMenuView::new(
                        terminal_view_id,
                        active_session,
                        &suggestions_mode_model,
                        agent_view_controller,
                        &inline_terminal_menu_positioner,
                        buffer_model,
                        ctx,
                    )
                }
            });
            if FeatureFlag::InlineHistoryMenu.is_enabled() {
                ctx.subscribe_to_view(&view, |me, _, event, ctx| {
                    if !me.is_cloud_mode_input_v2_composing(ctx) {
                        return;
                    }
                    me.handle_inline_history_menu_event(event, ctx);
                });
            }
            Some(view)
        } else {
            None
        };

        let terminal_input_message_bar = ctx.add_view(|ctx| {
            TerminalInputMessageBar::new(
                model.clone(),
                ai_input_model.clone(),
                buffer_model.clone(),
                ai_context_model.clone(),
                suggestions_mode_model.clone(),
                inline_history_model,
                ctx,
            )
        });

        let agent_shortcut_view_model = ctx.add_model(|ctx| {
            AgentShortcutViewModel::new(buffer_model.clone(), agent_view_controller.clone(), ctx)
        });
        ctx.subscribe_to_model(&agent_shortcut_view_model, |_, _, _, ctx| {
            ctx.notify();
        });

        current_prompt.update(ctx, |prompt_type, ctx| {
            if let PromptType::Dynamic { prompt } = prompt_type {
                prompt.update(ctx, |current_prompt, ctx| {
                    current_prompt.subscribe_to_input_editor(editor.clone(), ctx);
                });
            }
        });

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let input_suggestions = ctx.add_typed_action_view(InputSuggestions::new);
        ctx.subscribe_to_view(&input_suggestions, move |me, _, event, ctx| {
            me.handle_suggestions_event(event, ctx);
        });

        let app_workflows = LocalWorkflows::as_ref(ctx)
            .app_workflows()
            .cloned()
            .collect_vec();
        let local_user_workflows = WarpConfig::as_ref(ctx).local_user_workflows().clone();

        let workflows_search_view = ctx.add_typed_action_view(|ctx| {
            workflows::CategoriesView::new(local_user_workflows, app_workflows, ctx)
        });
        ctx.subscribe_to_view(&workflows_search_view, move |me, _, event, ctx| {
            me.handle_workflows_event(event, ctx);
        });

        let safe_mode_settings = SafeModeSettings::handle(ctx);
        ctx.subscribe_to_model(&safe_mode_settings, |me, _, event, ctx| {
            me.handle_safe_mode_settings_changed_event(event, ctx)
        });

        ctx.subscribe_to_model(&InputModeSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        let (debounce_input_background_tx, debounce_input_background_rx) =
            async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            debounce(
                DEBOUNCE_INPUT_DECORATION_PERIOD,
                debounce_input_background_rx,
            ),
            |me, mode, ctx| me.run_input_background_jobs(mode, ctx),
            |_me, _ctx| {},
        );

        let (debounce_ai_query_prediction_tx, debounce_ai_query_prediction_rx) =
            async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            debounce(
                DEBOUNCE_AI_QUERY_PREDICTION_PERIOD,
                debounce_ai_query_prediction_rx,
            ),
            |me, _, ctx| me.predict_am_query(ctx),
            |_me, _ctx| {},
        );

        let voltron_features = Vec1::new(VoltronFeatureView::new(
            VoltronItem::Workflows,
            VoltronFeatureViewHandle::Workflows(workflows_search_view.clone()),
        ));
        let voltron_view = { ctx.add_typed_action_view(|ctx| Voltron::new(voltron_features, ctx)) };
        ctx.subscribe_to_view(&voltron_view, move |me, _, event, ctx| {
            me.handle_voltron_event(event, ctx);
        });

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), move |me, _, evt, ctx| {
            me.handle_session_settings_event(evt, ctx);
        });

        let editor_settings_handle = &AppEditorSettings::handle(ctx);
        ctx.subscribe_to_model(
            editor_settings_handle,
            Self::handle_app_editor_settings_event,
        );

        ctx.subscribe_to_model(&LigatureSettings::handle(ctx), |_, _, _, ctx| ctx.notify());

        let workflows_state = WorkflowsState {
            selected_workflow_state: None,
        };

        let env_var_collection_state = EnvVarCollectionState {
            selected_env_vars: None,
        };

        let last_word_insertion = LastWordInsertion {
            insert_command_from_history_index: 0,
            is_latest_editor_event: false,
        };

        ctx.subscribe_to_model(
            &InputSettings::handle(ctx),
            Self::handle_input_settings_event,
        );

        ctx.subscribe_to_model(&ai_controller, |me, _, event, ctx| match event {
            BlocklistAIControllerEvent::SentRequest {
                contains_user_query: is_user_initiated,
                is_queued_prompt,
                ..
            } => {
                // Skip the buffer clear for queued prompts. The user may have typed new
                // input while the agent was busy and we don't want to wipe it on auto-send.
                if *is_user_initiated && !*is_queued_prompt {
                    me.editor.update(ctx, |editor, ctx| {
                        editor.system_clear_buffer(true, ctx);
                    });
                    ctx.notify();
                }
            }
            BlocklistAIControllerEvent::ExportConversationToFile {
                #[cfg_attr(target_family = "wasm", allow(unused))]
                filename,
            } => {
                #[cfg(not(target_family = "wasm"))]
                {
                    me.export_conversation_to_file(filename.clone(), ctx);
                }
                #[cfg(target_family = "wasm")]
                {
                    log::warn!("Export to file is not supported on WASM");
                }
            }
            _ => {}
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, _, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged {
                buffer_to_restore,
                input_config_to_restore,
            } = event;
            if let Some(buffer_state) = buffer_to_restore {
                me.restore_buffer_state(buffer_state, ctx);
            }
            if let Some(input_config) = input_config_to_restore {
                let is_buffer_empty = me.editor.as_ref(ctx).buffer_text(ctx).is_empty();
                me.ai_input_model.update(ctx, |ai_input_model, ctx| {
                    ai_input_model.set_input_config(*input_config, is_buffer_empty, ctx);
                });
            }

            me.set_zero_state_hint_text(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&ai_input_model, |me, _, event, ctx| {
            let _ = me
                .debounce_input_background_tx
                .try_send(InputBackgroundJobOptions::default().with_command_decoration());

            let config = event.updated_config();
            if config.is_locked && me.suggestions_mode_model.as_ref(ctx).is_visible() {
                // Preserve certain menus when input type changes - they handle their own
                // input type transitions during navigation.
                let should_preserve_menu = me
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()
                    || (me.suggestions_mode_model.as_ref(ctx).is_slash_commands()
                        && config.input_type.is_ai());

                if !should_preserve_menu {
                    // If switching to any locked mode (not autodetect), close suggestions
                    me.close_input_suggestions(/*should_focus_input=*/ false, ctx);
                }
            }

            if config.input_type.is_ai() && !config.is_locked {
                // If command x-ray is open, hide it -- it shouldn't show in AI input mode.
                me.hide_x_ray(ctx);
            }

            me.editor.update(ctx, |editor, ctx| {
                editor.maybe_populate_intelligent_autosuggestion(config.input_type, ctx);
            });
            me.set_zero_state_hint_text(ctx);
            ctx.notify();
        });
        // Re-render the input's placeholder/hint text when any history event that could
        // change `selected_conversation_status_for_hint` fires — the selected conversation
        // swapping, its status changing, its title being set/unset, it becoming non-empty,
        // or its being cleared. Mirrors the set used by
        // `agent_conversation_event_affects_vertical_tabs` in `workspace/view.rs` that keeps
        // vertical tab progress indicators in sync.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, _, event, ctx| {
                let affects_hint = matches!(
                    event,
                    BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                        | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                        | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                        | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
                        | BlocklistAIHistoryEvent::StartedNewConversation { .. }
                        | BlocklistAIHistoryEvent::SplitConversation { .. }
                        | BlocklistAIHistoryEvent::AppendedExchange { .. }
                        | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                        | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                        | BlocklistAIHistoryEvent::RestoredConversations { .. }
                );
                if !affects_hint {
                    return;
                }
                if event.terminal_view_id() != Some(terminal_view_id) {
                    return;
                }
                me.set_zero_state_hint_text(ctx);
                ctx.notify();
            },
        );

        ctx.subscribe_to_model(&ai_context_model, |me, context_model, event, ctx| {
            match event {
                BlocklistAIContextEvent::PendingQueryStateUpdated => {
                    me.remove_excess_images(ctx);
                    me.update_image_context_options(ctx);
                    me.set_zero_state_hint_text(ctx);
                    // If buffer empty and autodetect enabled, set the underlying input type to AI.
                    // Visually to the user, empty buffer is really a separate unclassified state. But since we don't support a third state
                    // in the model right now, we set the type to AI to make sure conversation block context is rendered when a conversation is selected
                    // on empty buffer. The actual underlying type doesn't otherwise matter on an empty buffer.
                    let is_empty_buffer = me.editor().as_ref(ctx).buffer_text(ctx).is_empty();
                    if is_empty_buffer {
                        me.ai_input_model.update(ctx, |ai_input_model, ctx| {
                            let is_auto_detection_enabled = !ai_input_model.is_input_type_locked();
                            if is_auto_detection_enabled {
                                ai_input_model.set_input_type(InputType::AI, ctx);
                            }
                        });
                    }
                    // The editor view renders the follow up icon, so we need to re-render the editor view.
                    me.editor().update(ctx, |_, ctx| {
                        ctx.notify();
                    })
                }
                BlocklistAIContextEvent::UpdatedPendingContext { .. } => {
                    me.update_image_context_options(ctx);
                    me.attachment_chips = context_model
                        .as_ref(ctx)
                        .pending_attachments()
                        .iter()
                        .enumerate()
                        .map(|(i, attachment)| AttachmentChip {
                            file_name: attachment.file_name().to_string(),
                            mouse_state_handle: Default::default(),
                            attachment_type: attachment.attachment_type(),
                            index: i,
                        })
                        .collect_vec();
                }
                BlocklistAIContextEvent::QueueNextPromptToggled => {}
            }
            ctx.notify();
        });

        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedActiveAgentModeLLM = event {
                // If the new model doesn't support vision and we had image chips,
                // the context model already cleared them — show a toast.
                let has_image_chips = me
                    .attachment_chips
                    .iter()
                    .any(|c| matches!(c.attachment_type, AttachmentType::Image));
                let vision_supported =
                    LLMPreferences::as_ref(ctx).vision_supported(ctx, Some(me.terminal_view_id));
                if has_image_chips && !vision_supported {
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                        ts.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Attached images were removed — the selected model does not support images.".to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                }
                me.update_image_context_options(ctx);
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            me.handle_ai_settings_changed_event(event, ctx)
        });

        ctx.subscribe_to_model(
            &IgnoredSuggestionsModel::handle(ctx),
            |me, _, event, ctx| {
                me.handle_ignored_suggestions_event(event, ctx);
            },
        );

        let prompt_suggestions_view = ctx
            .add_typed_action_view(|ctx| PromptSuggestionsView::new(ai_input_model.clone(), ctx));
        ctx.subscribe_to_view(&prompt_suggestions_view, move |me, _, event, ctx| {
            me.handle_prompt_suggestions_event(event, ctx);
        });

        let slash_command_data_source = ctx.add_model(|ctx| {
            let args = slash_commands::DataSourceArgs {
                active_session: active_session.clone(),
                agent_view_controller: agent_view_controller.clone(),
                cli_subagent_controller: cli_subagent_controller.clone(),
                terminal_view_id,
                ambient_agent_view_model: ambient_agent_view_model.clone(),
            };
            SlashCommandDataSource::new(args, ctx)
        });

        let cloud_mode_composer_slash_command_data_source =
            if FeatureFlag::CloudModeInputV2.is_enabled() {
                let args = slash_commands::DataSourceArgs {
                    active_session: active_session.clone(),
                    agent_view_controller: agent_view_controller.clone(),
                    cli_subagent_controller: cli_subagent_controller.clone(),
                    terminal_view_id,
                    ambient_agent_view_model: ambient_agent_view_model.clone(),
                };
                Some(ctx.add_model(|ctx| SlashCommandDataSource::for_cloud_mode_v2(args, ctx)))
            } else {
                None
            };
        let slash_command_model = ctx.add_model(|ctx| {
            SlashCommandModel::new(
                &buffer_model,
                &ai_input_model,
                active_session.clone(),
                slash_command_data_source.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_model(&slash_command_model, move |me, _, event, ctx| {
            me.handle_slash_command_model_event(event, ctx);
        });

        let inline_conversation_menu_view = ctx.add_view(|ctx| {
            InlineConversationMenuView::new(
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                &inline_terminal_menu_positioner,
                active_session.clone(),
                ctx,
            )
        });
        if FeatureFlag::AgentView.is_enabled() {
            ctx.subscribe_to_view(&inline_conversation_menu_view, |me, _, event, ctx| {
                me.handle_conversation_menu_event(event, ctx);
            });
            ctx.subscribe_to_model(&inline_terminal_menu_positioner, |_, _, _, ctx| {
                ctx.notify();
            });
        }

        let inline_repos_menu_view = ctx.add_view(|ctx| {
            InlineReposMenuView::new(
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                &inline_terminal_menu_positioner,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_repos_menu_view, |me, _, event, ctx| {
            me.handle_repos_menu_event(event, ctx);
        });

        let inline_model_selector_view = ctx.add_view(|ctx| {
            InlineModelSelectorView::new(
                terminal_view_id,
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                cli_subagent_controller.clone(),
                &inline_terminal_menu_positioner,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_model_selector_view, |me, _, event, ctx| {
            me.handle_inline_model_selector_event(event, ctx);
        });

        let inline_profile_selector_view = ctx.add_view(|ctx| {
            InlineProfileSelectorView::new(
                terminal_view_id,
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                &inline_terminal_menu_positioner,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_profile_selector_view, |me, _, event, ctx| {
            me.handle_inline_profile_selector_event(event, ctx);
        });

        let inline_prompts_menu_view = ctx.add_view(|ctx| {
            InlinePromptsMenuView::new(
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                &inline_terminal_menu_positioner,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_prompts_menu_view, |me, _, event, ctx| {
            me.handle_inline_prompts_menu_event(event, ctx);
        });

        let inline_skill_selector_view = ctx.add_view(|ctx| {
            InlineSkillSelectorView::new(
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &buffer_model,
                &inline_terminal_menu_positioner,
                active_session,
                terminal_view_id,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_skill_selector_view, |me, _, event, ctx| {
            me.handle_inline_skill_selector_event(event, ctx);
        });

        let user_query_menu_view = ctx.add_view(|ctx| {
            UserQueryMenuView::new(
                AIConversationId::default(),
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &inline_terminal_menu_positioner,
                &buffer_model,
                ctx,
            )
        });
        if FeatureFlag::AgentView.is_enabled() {
            ctx.subscribe_to_view(&user_query_menu_view, |me, _, event, ctx| {
                me.handle_user_query_menu_event(event, ctx);
            });
        }

        let inline_plan_menu_view = ctx.add_view(|ctx| {
            InlinePlanMenuView::new(
                AIConversationId::default(),
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &inline_terminal_menu_positioner,
                &buffer_model,
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_plan_menu_view, |me, _, event, ctx| {
            me.handle_plan_menu_event(event, ctx);
        });

        let rewind_menu_view = ctx.add_view(|ctx| {
            RewindMenuView::new(
                AIConversationId::default(),
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                &inline_terminal_menu_positioner,
                &buffer_model,
                ctx,
            )
        });
        ctx.subscribe_to_view(&rewind_menu_view, |me, _, event, ctx| {
            me.handle_rewind_menu_event(event, ctx);
        });

        let inline_slash_commands_view = ctx.add_view(|ctx| {
            InlineSlashCommandView::new(
                &slash_command_model,
                &inline_terminal_menu_positioner,
                slash_command_data_source.clone(),
                suggestions_mode_model.clone(),
                agent_view_controller.clone(),
                buffer_model.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_view(&inline_slash_commands_view, |me, _, event, ctx| {
            me.handle_slash_commands_menu_event(event, ctx);
        });

        let cloud_mode_v2_slash_commands_view =
            if let Some(v2_data_source) = cloud_mode_composer_slash_command_data_source.clone() {
                let view = ctx.add_typed_action_view(|ctx| {
                    CloudModeV2SlashCommandView::new(
                        &slash_command_model,
                        v2_data_source,
                        suggestions_mode_model.clone(),
                        buffer_model.clone(),
                        ctx,
                    )
                });
                ctx.subscribe_to_view(&view, |me, _, event, ctx| {
                    me.handle_slash_commands_menu_event(event, ctx);
                });
                Some(view)
            } else {
                None
            };

        ctx.subscribe_to_model(&ai_input_model, move |me, _, event, ctx| {
            match event {
                BlocklistAIInputEvent::InputTypeChanged { .. }
                | BlocklistAIInputEvent::LockChanged { .. } => {
                    // Close slash command menu if we're now in locked shell mode
                    if me.is_locked_in_shell_mode(ctx)
                        && me.suggestions_mode_model.as_ref(ctx).is_slash_commands()
                    {
                        me.suggestions_mode_model.update(ctx, |m, ctx| {
                            m.set_mode(InputSuggestionsMode::Closed, ctx);
                        });
                        ctx.notify();
                    }
                }
            }
        });

        let ai_req_usage_model = AIRequestUsageModel::handle(ctx);
        ctx.subscribe_to_model(&ai_req_usage_model, |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.observe(&ai_req_usage_model, |_, _, ctx| {
            ctx.notify();
        });

        let buy_credits_banner = ctx.add_typed_action_view(BuyCreditsBanner::new);
        ctx.subscribe_to_view(&buy_credits_banner, |me, _, event, ctx| match event {
            BuyCreditsBannerEvent::OpenBillingAndUsage => {
                ctx.emit(Event::OpenSettings(SettingsSection::BillingAndUsage));
            }
            BuyCreditsBannerEvent::RefocusInput => {
                ctx.focus(&me.editor);
            }
            BuyCreditsBannerEvent::OpenAutoReloadModal { purchased_credits } => {
                ctx.emit(Event::OpenAutoReloadModal {
                    purchased_credits: *purchased_credits,
                });
            }
            BuyCreditsBannerEvent::ShowAutoReloadError { error_message } => {
                ctx.emit(Event::ShowToast {
                    message: error_message.to_string(),
                    flavor: ToastFlavor::Error,
                });
            }
        });

        let agent_status_view = ctx.add_typed_action_view(|ctx| {
            BlocklistAIStatusBar::new(
                ai_controller.clone(),
                agent_view_controller.clone(),
                cli_subagent_controller,
                ai_action_model.clone(),
                ai_context_model.clone(),
                ai_input_model.clone(),
                buffer_model,
                &model_events,
                model.clone(),
                agent_shortcut_view_model.clone(),
                ambient_agent_view_model.clone(),
                suggestions_mode_model.clone(),
                slash_command_model.clone(),
                ephemeral_message_model.clone(),
                handoff_compose_state.clone(),
                terminal_view_id,
                ctx,
            )
        });

        let deferred_remote_operations =
            DeferredRemoteOperations::new(model.lock().block_list().active_block_id().clone());

        // Use persisted menu sizes from settings, or fall back to defaults
        let input_settings = InputSettings::as_ref(ctx);
        let completions_menu_width = *input_settings.completions_menu_width.value();
        let completions_menu_height = *input_settings.completions_menu_height.value();

        let is_editor_empty = editor.as_ref(ctx).is_empty(ctx);
        let mut input = Self {
            input_suggestions,
            suggestions_mode_model,
            completions_menu_resizable_width: resizable_state_handle(completions_menu_width),
            completions_menu_resizable_height: resizable_state_handle(completions_menu_height),
            tips_completed,
            editor,
            model,
            server_api,
            sessions,
            focus_handle: None,
            active_block_metadata: None,
            view_id,
            input_render_state_model_handle,
            workflows_state,
            env_var_collection_state,
            voltron_view,
            is_voltron_open: false,
            command_x_ray_description: None,
            last_parsed_tokens: None,
            debounce_input_background_tx,
            debounce_ai_query_prediction_tx,
            has_pending_command: false,
            last_word_insertion,
            decorations_future_handle: None,
            autosuggestions_abort_handle: None,
            completions_abort_handle: None,
            menu_positioning_provider,
            universal_developer_input_button_bar,
            terminal_input_message_bar,
            prompt_render_helper,
            prompt_type: current_prompt,
            ai_controller,
            ai_context_model,
            ai_input_model,
            ai_action_model,
            ai_follow_up_icon_mouse_state: MouseStateHandle::default(),
            enable_autosuggestions_setting: *editor_settings_handle
                .as_ref(ctx)
                .enable_autosuggestions,
            latest_buffer_operations: Vec::new(),
            deferred_remote_operations,
            shared_session_input_state: None,
            shared_session_presence_manager: None,
            prompt_suggestions_banner_state: None,
            has_prompt_suggestion_banner,
            was_intelligent_autosuggestion_accepted: false,
            last_intelligent_autosuggestion_result: None,
            next_command_model,
            last_user_block_completed: None,
            hoverable_handle: Default::default(),
            terminal_view_id,
            #[cfg(feature = "local_fs")]
            conn: None,
            predict_am_queries_future_handle: None,
            attachment_chips: Default::default(),
            is_processing_attached_images: false,
            prompt_suggestions_view,
            handoff_compose_state,
            slash_command_model,
            inline_slash_commands_view,
            cloud_mode_v2_slash_commands_view,
            inline_conversation_menu_view,
            inline_plan_menu_view,
            inline_repos_menu_view,
            inline_model_selector_view,
            inline_profile_selector_view,
            inline_prompts_menu_view,
            inline_skill_selector_view,
            skill_selector_should_invoke: false,
            user_query_menu_view,
            rewind_menu_view,
            inline_history_menu_view,
            cloud_mode_v2_history_menu_view,
            inline_terminal_menu_positioner,
            cached_agent_mode_hint_text: None,
            is_editor_empty_on_last_edit: is_editor_empty,
            weak_view_handle: ctx.handle(),
            buy_credits_banner,
            agent_status_view,
            agent_view_controller,
            agent_input_footer,
            agent_shortcut_view_model,
            ambient_agent_view_state,
            slash_command_data_source,
            cloud_mode_composer_slash_command_data_source,
            ephemeral_message_model,
            input_contents_before_prompt_chip_command: None,
        };

        #[cfg(feature = "local_fs")]
        if let Some(db_url) = database_file_path_for_scope(&PersistenceScope::App).to_str() {
            if let Ok(conn) = establish_ro_connection(db_url) {
                input.conn = Some(Arc::new(Mutex::new(conn)));
            }
        }

        if input.model.lock().shared_session_status().is_viewer() {
            input.editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Selectable, ctx);
            });
        } else {
            input.set_zero_state_hint_text(ctx);
        }

        #[cfg(feature = "voice_input")]
        input.update_voice_transcription_options(ctx);
        input.update_image_context_options(ctx);
        input.update_ai_context_menu(ctx);
        input
    }

    #[cfg(feature = "voice_input")]
    fn update_voice_transcription_options(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        let ai_settings = AISettings::as_ref(ctx);

        let voice_transcription_options = match (
            ai_input_model.input_type(),
            ai_settings.is_voice_input_enabled(ctx),
        ) {
            (InputType::AI, true) => crate::editor::VoiceTranscriptionOptions::Enabled {
                // If UDI is enabled, we show the button below the text input
                show_button: !self.should_show_universal_developer_input(ctx)
                    && !FeatureFlag::AgentView.is_enabled(),
            },
            (InputType::Shell, true) => {
                crate::editor::VoiceTranscriptionOptions::Enabled { show_button: false }
            }
            (_, false) => crate::editor::VoiceTranscriptionOptions::Disabled,
        };

        self.editor.update(ctx, move |editor, ctx| {
            editor.update_voice_transcription_options(voice_transcription_options, ctx);
            ctx.notify();
        });
    }

    fn update_ai_context_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        let is_ai_input = ai_input_model.input_type().is_ai();
        self.check_and_update_ai_context_menu_disabled_state(ctx);
        self.editor.update(ctx, move |editor, ctx| {
            editor.set_is_ai_input(is_ai_input, ctx);
            ctx.notify();
        });
    }

    pub fn agent_status_bar(&self) -> &ViewHandle<BlocklistAIStatusBar> {
        &self.agent_status_view
    }

    pub fn agent_input_footer(&self) -> &ViewHandle<AgentInputFooter> {
        &self.agent_input_footer
    }

    fn ambient_agent_view_model(&self) -> Option<&ModelHandle<AmbientAgentViewModel>> {
        self.ambient_agent_view_state
            .as_ref()
            .map(AmbientAgentViewState::view_model)
    }

    fn harness_selector(&self) -> Option<&ViewHandle<HarnessSelector>> {
        self.ambient_agent_view_state
            .as_ref()
            .map(|state| &state.harness_selector)
    }

    fn host_selector(&self) -> Option<&ViewHandle<HostSelector>> {
        self.ambient_agent_view_state
            .as_ref()
            .and_then(|state| state.host_selector.as_ref())
    }

    fn auth_secret_selector(&self) -> Option<&ViewHandle<AuthSecretSelector>> {
        self.ambient_agent_view_state
            .as_ref()
            .and_then(|state| state.auth_secret_selector.as_ref())
    }

    pub(super) fn auth_secret_ftux_view(&self) -> Option<&ViewHandle<AuthSecretFtuxView>> {
        self.ambient_agent_view_state
            .as_ref()
            .and_then(|state| state.auth_secret_ftux_view.as_ref())
    }

    /// Opens the V2 cloud-mode host selector popover, if the feature is enabled and the
    /// selector is constructed. No-op otherwise. Used by the `/host` slash command to
    /// programmatically open the same popover that the V2 footer's host button toggles.
    pub(super) fn open_v2_host_selector(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(host_selector) = self.host_selector().cloned() else {
            return;
        };
        host_selector.update(ctx, |selector, ctx| selector.open_menu(ctx));
    }

    /// Opens the V2 cloud-mode harness selector popover, if the feature is enabled and the
    /// selector is constructed. No-op otherwise. Used by the `/harness` slash command to
    /// programmatically open the same popover that the V2 footer's harness button toggles.
    pub(super) fn open_v2_harness_selector(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(harness_selector) = self.harness_selector().cloned() else {
            return;
        };
        harness_selector.update(ctx, |selector, ctx| selector.open_menu(ctx));
    }

    pub(super) fn open_v2_environment_selector(&mut self, ctx: &mut ViewContext<Self>) {
        self.agent_input_footer
            .clone()
            .update(ctx, |footer, ctx| footer.open_v2_environment_selector(ctx));
    }

    /// Restores the `&` handoff compose draft after a workspace failure.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn restore_cloud_handoff_draft(
        &mut self,
        launch: PendingCloudLaunch,
        environment_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.activate_cloud_handoff_compose(HandoffEntryPoint::Ampersand, ctx);
        self.editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&launch.prompt, ctx);
        });
        self.ai_context_model.update(ctx, |model, ctx| {
            for attachment in launch.attachments.display_attachments {
                model.append_pending_attachments(vec![attachment], ctx);
            }
        });
        if let Some(env_id) = environment_id {
            self.handoff_compose_state.update(ctx, |state, ctx| {
                state.set_environment_id(Some(env_id), true, ctx);
            });
        }
    }

    fn prefix_mode(&self, ctx: &AppContext) -> InputPrefixMode {
        let is_handoff_active = self.handoff_compose_state.as_ref(ctx).is_active();
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        let is_shell_active =
            ai_input_model.is_input_type_locked() && !ai_input_model.input_type().is_ai();

        if is_handoff_active {
            InputPrefixMode::CloudHandoff
        } else if is_shell_active {
            InputPrefixMode::Shell
        } else {
            InputPrefixMode::None
        }
    }

    /// Switches the input into cloud handoff compose mode, locking it to AI input
    /// and activating the handoff compose state.
    fn activate_cloud_handoff_compose(
        &mut self,
        entry_point: HandoffEntryPoint,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.prefix_mode(ctx) == InputPrefixMode::CloudHandoff {
            return;
        }

        let is_input_buffer_empty = self.editor.as_ref(ctx).is_empty(ctx);
        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            ai_input_model.set_input_config(
                InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                },
                is_input_buffer_empty,
                ctx,
            );
        });

        self.handoff_compose_state
            .update(ctx, |state, ctx| state.activate(entry_point, ctx));
        self.is_editor_empty_on_last_edit = is_input_buffer_empty;

        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        self.auto_select_environment_from_pwd(ctx);

        ctx.notify();
    }

    /// Spawns an async task to resolve the pwd's git repo and pick the best
    /// environment overlap, updating the handoff compose state when done.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    fn auto_select_environment_from_pwd(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(pwd) = self
            .active_session_path_if_local(ctx)
            .map(Path::to_path_buf)
        else {
            return;
        };

        let handoff_compose_state = self.handoff_compose_state.clone();
        ctx.spawn(
            async move {
                resolve_repo_for_path(&pwd)
                    .with_timeout(Duration::from_secs(5))
                    .await
                    .ok()
                    .flatten()
            },
            move |_input, touched_repo, ctx| {
                let Some(touched_repo) = touched_repo else {
                    return;
                };
                let workspace = TouchedWorkspace {
                    repos: vec![touched_repo],
                    orphan_files: vec![],
                };
                let mut envs = CloudAmbientAgentEnvironment::get_all(ctx);
                sort_environments_by_recency(&mut envs);
                if let Some(overlap_env) = pick_handoff_overlap_env(&workspace, envs) {
                    handoff_compose_state.update(ctx, |state, ctx| {
                        state.set_environment_id(Some(overlap_env), false, ctx);
                    });
                }
            },
        );
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn handoff_entry_point(&self, ctx: &AppContext) -> HandoffEntryPoint {
        self.handoff_compose_state.as_ref(ctx).entry_point()
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn exit_cloud_handoff_compose_and_clear(&mut self, ctx: &mut ViewContext<Self>) {
        self.exit_cloud_handoff_compose(ctx);
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
        self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.clear_pending_attachments(ctx);
        });
    }

    fn exit_cloud_handoff_compose(&mut self, ctx: &mut ViewContext<Self>) {
        if self.prefix_mode(ctx) != InputPrefixMode::CloudHandoff {
            return;
        }

        let is_input_buffer_empty = self.editor.as_ref(ctx).is_empty(ctx);
        self.handoff_compose_state
            .update(ctx, |state, ctx| state.exit(ctx));
        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            ai_input_model.set_input_config(
                InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                }
                .unlocked_if_autodetection_enabled(true, ctx),
                is_input_buffer_empty,
                ctx,
            );
        });
    }

    // Cloud handoff methods — candidates for extraction to a separate file
    // following the pattern used by `agent.rs`, `classic.rs`, etc.
    fn can_activate_cloud_handoff_prefix(
        &self,
        edit_origin: &EditOrigin,
        ctx: &AppContext,
    ) -> bool {
        let is_powershell_with_nld_enabled = self.editor.as_ref(ctx).shell_family()
            == Some(ShellFamily::PowerShell)
            && AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
        let is_cloud = {
            let terminal_model = self.model.lock();
            is_in_cloud_context(
                terminal_model.block_list().agent_view_state(),
                &terminal_model,
            )
        };
        *edit_origin == EditOrigin::UserTyped
            && AISettings::as_ref(ctx)
                .is_ampersand_handoff_enabled_for_terminal_view(self.terminal_view_id, ctx)
            && !is_powershell_with_nld_enabled
            && FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_fullscreen()
            && !is_cloud
            && !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id)
            && self.prefix_mode(ctx) == InputPrefixMode::None
    }

    fn maybe_activate_cloud_handoff_prefix(
        &mut self,
        edit_origin: &EditOrigin,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let is_new_handoff_prefix = {
            let editor = self.editor.as_ref(ctx);
            editor
                .buffer_text(ctx)
                .starts_with(CLOUD_HANDOFF_INPUT_PREFIX)
                && !editor
                    .last_buffer_text(ctx)
                    .starts_with(CLOUD_HANDOFF_INPUT_PREFIX)
        };
        if !self.can_activate_cloud_handoff_prefix(edit_origin, ctx) || !is_new_handoff_prefix {
            return false;
        }

        let is_input_buffer_empty = self.editor.update(ctx, |editor, ctx| {
            if let Some(rest) = editor
                .buffer_text(ctx)
                .strip_prefix(CLOUD_HANDOFF_INPUT_PREFIX)
            {
                editor.set_buffer_text(rest, ctx);
            }
            editor.buffer_text(ctx).is_empty()
        });
        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            ai_input_model.set_input_config(
                InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                },
                is_input_buffer_empty,
                ctx,
            );
        });

        self.handoff_compose_state.update(ctx, |state, ctx| {
            state.activate(HandoffEntryPoint::Ampersand, ctx)
        });
        self.is_editor_empty_on_last_edit = is_input_buffer_empty;

        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        self.auto_select_environment_from_pwd(ctx);

        ctx.notify();
        true
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn collect_cloud_launch_attachments(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> HandoffLaunchAttachments {
        if !FeatureFlag::CloudModeImageContext.is_enabled() {
            return HandoffLaunchAttachments::default();
        }

        let mut request_attachments: Vec<AttachmentInput> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .iter()
            .map(|image| AttachmentInput {
                file_name: image.file_name.clone(),
                mime_type: image.mime_type.clone(),
                data: image.data.clone(),
            })
            .collect();

        let mut skipped_files: Vec<String> = Vec::new();
        for file in self.ai_context_model.as_ref(ctx).pending_files() {
            match std::fs::read(&file.file_path) {
                Ok(bytes) => {
                    if bytes.len() > MAX_ATTACHMENT_SIZE_BYTES {
                        skipped_files.push(file.file_name.clone());
                        continue;
                    }
                    request_attachments.push(AttachmentInput {
                        file_name: file.file_name.clone(),
                        mime_type: file.mime_type.clone(),
                        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                    });
                }
                Err(e) => {
                    log::error!("Failed to read file {}: {e}", file.file_path.display());
                }
            }
        }

        if !skipped_files.is_empty() {
            let window_id = ctx.window_id();
            let message = if skipped_files.len() == 1 {
                format!(
                    "{} was not attached — exceeds 10MB limit.",
                    skipped_files[0]
                )
            } else {
                format!(
                    "{} files were not attached — exceed 10MB limit.",
                    skipped_files.len()
                )
            };
            ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                ts.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
            });
        }

        let display_attachments: Vec<PendingAttachment> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_attachments()
            .to_vec();

        HandoffLaunchAttachments {
            request_attachments,
            display_attachments,
        }
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    fn maybe_launch_cloud_handoff_request(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !FeatureFlag::OzHandoff.is_enabled()
            || !FeatureFlag::HandoffLocalCloud.is_enabled()
            || !cfg!(all(feature = "local_fs", not(target_family = "wasm")))
            || self.prefix_mode(ctx) != InputPrefixMode::CloudHandoff
        {
            return false;
        }

        let prompt = self.editor.as_ref(ctx).buffer_text(ctx).trim().to_owned();
        if prompt.is_empty() {
            return true;
        }

        if CloudAmbientAgentEnvironment::get_all(ctx).is_empty() {
            ctx.emit(Event::OpenHandoffEnvironmentCreationModal);
            return true;
        }

        let attachments = self.collect_cloud_launch_attachments(ctx);
        let environment_id = self
            .handoff_compose_state
            .as_ref(ctx)
            .selected_environment_id()
            .cloned();
        let entry_point = self.handoff_compose_state.as_ref(ctx).entry_point();
        let launch = PendingCloudLaunch {
            prompt,
            attachments,
        };

        self.exit_cloud_handoff_compose_and_clear(ctx);

        ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenLocalToCloudHandoffPane {
            launch: Some(launch),
            environment_id,
            entry_point,
        });
        true
    }

    #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
    fn maybe_launch_cloud_handoff_request(&mut self, _ctx: &mut ViewContext<Self>) -> bool {
        false
    }

    /// Update the at button's disabled state based on whether AI context menu should render
    pub fn check_and_update_ai_context_menu_disabled_state(&mut self, ctx: &mut ViewContext<Self>) {
        let disable_reason = AtContextMenuDisabledReason::get_disable_reason(
            self.active_block_metadata.as_ref(),
            self.sessions.as_ref(ctx),
            &self.ai_input_model.as_ref(ctx).input_config(),
            ctx,
        );

        self.universal_developer_input_button_bar
            .update(ctx, |button_bar, ctx| {
                button_bar.set_at_button_disabled(disable_reason, ctx);
            });
    }

    fn check_slash_menu_disabled_state(&mut self, ctx: &mut ViewContext<Self>) {
        let should_disable =
            !self.editor().as_ref(ctx).is_empty(ctx) || self.is_locked_in_shell_mode(ctx);
        self.universal_developer_input_button_bar
            .update(ctx, |button_bar, ctx| {
                button_bar.set_slash_button_disabled(should_disable, ctx);
            });
    }

    fn handle_ai_context_menu_search(&mut self, is_navigation: bool, ctx: &mut ViewContext<Self>) {
        let InputSuggestionsMode::AIContextMenu {
            at_symbol_position,
            filter_text: prev_query,
        } = self.suggestions_mode_model.as_ref(ctx).mode()
        else {
            return;
        };
        let at_symbol_position = *at_symbol_position;
        let prev_query = prev_query.clone();
        let cursor_position = self
            .editor
            .read(ctx, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
            })
            .as_usize();

        let buffer_text = self
            .editor
            .read(ctx, |editor, _ctx| editor.buffer_text(ctx));

        let first_char_pos = at_symbol_position + 1;
        let num_chars = cursor_position.saturating_sub(first_char_pos);

        // Extract text between @ and cursor
        let filter_text = buffer_text
            .chars()
            .skip(first_char_pos)
            .take(num_chars)
            .collect::<String>();

        if !is_valid_search_query(is_navigation, &prev_query, &filter_text) {
            self.close_ai_context_menu(ctx);
        } else {
            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_mode(
                    InputSuggestionsMode::AIContextMenu {
                        filter_text: filter_text.clone(),
                        at_symbol_position,
                    },
                    ctx,
                );
            });
            // Update the search bar in the AI context menu with the new filter text
            self.editor.update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |menu, ctx| {
                        menu.update_search_query(filter_text, ctx);
                    });
                }
            });
        }
    }

    fn render_ai_context_menu(
        &self,
        stack: &mut Stack,
        menu_positioning: &MenuPositioning,
        app: &AppContext,
    ) {
        if let Some(ai_context_menu) = self.editor.as_ref(app).render_ai_context_menu() {
            let position = position_id_for_cursor(self.editor.id());

            let y_anchor = if self.is_cloud_mode_input_v2_composing(app) {
                AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top)
            } else {
                menu_positioning.completion_suggestions_y_anchor()
            };

            stack.add_positioned_overlay_child(
                ai_context_menu,
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &position,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &position,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(0.),
                        y_anchor,
                    ),
                ),
            );
        }
    }

    fn close_ai_context_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.suggestions_mode_model.as_ref(ctx).is_ai_context_menu() {
            return;
        }

        // Reset the AI context menu to the main menu position when closing
        self.editor.update(ctx, |editor, ctx| {
            if let Some(ai_context_menu) = editor.ai_context_menu() {
                ai_context_menu.update(ctx, |menu, ctx| {
                    menu.close(ctx);
                });
            }
        });

        // Directly close the menu without trying to update search state
        self.suggestions_mode_model.update(ctx, |m, ctx| {
            m.set_mode(InputSuggestionsMode::Closed, ctx);
        });
        self.focus_input_box(ctx);
        ctx.notify();
    }

    fn clear_and_reset_ai_context_menu_query(&mut self, ctx: &mut ViewContext<Self>) {
        if let InputSuggestionsMode::AIContextMenu {
            at_symbol_position, ..
        } = self.suggestions_mode_model.as_ref(ctx).mode()
        {
            let at_pos = *at_symbol_position;

            // Clear text from cursor back to the @ character (keeping the @)
            self.editor.update(ctx, |editor, ctx| {
                let cursor_pos = editor.start_byte_index_of_last_selection(ctx).as_usize();

                // Only clear if cursor is after the @ symbol
                if cursor_pos > at_pos {
                    // Calculate the range to delete (from @ + 1 to cursor position)
                    let start_pos = at_pos + 1; // Keep the @ character
                    let end_pos = cursor_pos;

                    if start_pos < end_pos {
                        editor.select_and_replace(
                            "",
                            [ByteOffset::from(start_pos)..ByteOffset::from(end_pos)],
                            PlainTextEditorViewAction::Delete,
                            ctx,
                        );
                    }
                }

                // Reset the AI context menu state
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |menu, ctx| {
                        menu.reset_menu_state(ctx);
                    });
                }
            });
        }
    }

    fn set_ai_context_menu_open(&mut self, open: bool, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::AIContextMenuEnabled.is_enabled() && open {
            let cursor_position = self.editor.read(ctx, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
            });

            let buffer_text = self
                .editor
                .read(ctx, |editor, _ctx| editor.buffer_text(ctx));

            if buffer_text
                .chars()
                .nth(cursor_position.as_usize().saturating_sub(1))
                != Some('@')
            {
                self.editor.update(ctx, |editor, ctx| {
                    editor.insert_char('@', ctx);
                });
            }

            // Update AI context menu input mode based on current state
            // Show AI categories if we're in AI mode OR if autodetection is enabled (not locked)
            let ai_input_model = self.ai_input_model.as_ref(ctx);
            let is_ai_or_autodetect_mode =
                ai_input_model.input_type().is_ai() || !ai_input_model.is_input_type_locked();

            self.editor.update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |menu, ctx| {
                        menu.set_input_mode(is_ai_or_autodetect_mode, ctx);
                    });
                }
            });

            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_mode(
                    InputSuggestionsMode::AIContextMenu {
                        filter_text: "".to_owned(),
                        at_symbol_position: cursor_position.as_usize(),
                    },
                    ctx,
                );
            });

            // Emit telemetry for @ menu opened
            let is_udi_enabled =
                InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
            let current_input_mode = self.ai_input_model.as_ref(ctx).input_type();

            send_telemetry_from_ctx!(
                TelemetryEvent::AtMenuInteracted {
                    action: "opened".to_string(),
                    item_count: None,
                    query_length: None,
                    is_udi_enabled,
                    current_input_mode,
                },
                ctx
            );
        } else if self.suggestions_mode_model.as_ref(ctx).is_ai_context_menu() {
            self.close_ai_context_menu(ctx);
        }
        ctx.notify();
    }

    fn open_slash_commands_menu(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't open menu if there's a long-running command — unless the CLI agent
        // rich input is open (the CLI agent itself is the long-running command).
        let is_cli_agent_input =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
        if !is_cli_agent_input
            && self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_active_and_long_running()
        {
            return;
        }
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::SlashCommands, ctx);
        });
        ctx.notify();
    }

    fn toggle_legacy_slash_commands_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let is_slash_menu_open = self.suggestions_mode_model.as_ref(ctx).is_slash_commands();

        if is_slash_menu_open {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            });
            self.slash_command_model.update(ctx, |model, ctx| {
                model.disable(ctx);
            });
            self.close_slash_commands_menu(ctx);
        } else {
            self.system_insert("/", ctx);
            let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
                && self.agent_view_controller.as_ref(ctx).is_fullscreen();
            send_telemetry_from_ctx!(
                TelemetryEvent::OpenSlashMenu {
                    source: SlashMenuSource::SlashButton,
                    is_inline_ui_enabled: true,
                    is_in_agent_view,
                },
                ctx
            );
        }
    }

    fn handle_conversation_menu_event(
        &mut self,
        event: &InlineConversationMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InlineConversationMenuEvent::NavigateToConversation { item_id } => {
                let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_fullscreen();
                send_telemetry_from_ctx!(
                    TelemetryEvent::InlineConversationMenuItemSelected { is_in_agent_view },
                    ctx
                );

                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_conversation_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
                self.clear_buffer_and_reset_undo_stack(ctx);
                if let Some(action) = AgentConversationsModel::resolve_open_action(
                    AgentConversationNavigationSubject::Entry(*item_id),
                    Some(RestoreConversationLayout::ActivePane),
                    ctx,
                ) {
                    ctx.dispatch_typed_action_deferred(action);
                } else {
                    ctx.emit(Event::ShowToast {
                        message: "Couldn't navigate to conversation.".to_string(),
                        flavor: ToastFlavor::Error,
                    });
                }
            }
            InlineConversationMenuEvent::Dismissed => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_conversation_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
            }
        }
    }

    fn handle_repos_menu_event(
        &mut self,
        event: &InlineReposMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InlineReposMenuEvent::NavigateToRepo { path } => {
                if self.suggestions_mode_model.as_ref(ctx).is_repos_menu() {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
                self.clear_buffer_and_reset_undo_stack(ctx);
                let path_str = path.to_string_lossy().replace("'", "'\\''");
                let cd_command = format!("cd '{path_str}'");
                self.try_execute_command(&cd_command, ctx);
            }
            InlineReposMenuEvent::Dismissed => {
                if self.suggestions_mode_model.as_ref(ctx).is_repos_menu() {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
            }
        }
    }

    fn handle_inline_model_selector_event(
        &mut self,
        event: &InlineModelSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InlineModelSelectorEvent::SelectedModel {
                id,
                selected_tab,
                set_as_default,
            } => {
                let profile_id = *AIExecutionProfilesModel::as_ref(ctx)
                    .active_profile(Some(self.terminal_view_id), ctx)
                    .id();

                match selected_tab {
                    InlineModelSelectorTab::BaseAgent => {
                        LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                            preferences.update_preferred_agent_mode_llm(
                                id,
                                self.terminal_view_id,
                                ctx,
                            );
                        });
                        if *set_as_default {
                            AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
                                profiles.set_base_model(profile_id, Some(id.clone()), ctx);
                            });
                        }
                    }
                    InlineModelSelectorTab::FullTerminalUse => {
                        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
                            profiles.set_cli_agent_model(profile_id, Some(id.clone()), ctx);
                        });
                    }
                }
                // Accept path: close the model selector.
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_model_selector()
                    && !self
                        .inline_model_selector_view
                        .as_ref(ctx)
                        .filter_results_by_input()
                {
                    // The user had a pre-existing prompt; restore it (do NOT clear buffer).
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                } else {
                    // Clear the buffer for:
                    //  1) Selector open AND input was used as filter query — close menu, then clear.
                    //  2) Selector not open — just clear.
                    if self
                        .suggestions_mode_model
                        .as_ref(ctx)
                        .is_inline_model_selector()
                    {
                        self.suggestions_mode_model.update(ctx, |model, ctx| {
                            model.set_mode(InputSuggestionsMode::Closed, ctx);
                        });
                        ctx.notify();
                    }
                    self.clear_buffer_and_reset_undo_stack(ctx);
                }
            }
            InlineModelSelectorEvent::Dismissed => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_model_selector()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
            }
        }
        self.focus_input_box(ctx);
    }

    fn handle_inline_profile_selector_event(
        &mut self,
        event: &InlineProfileSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InlineProfileSelectorEvent::SelectedProfile { profile_id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_active_profile(self.terminal_view_id, *profile_id, ctx);
                });

                // Remove any LLM override when switching profiles
                // (mirroring the profile-selecting behavior from the profile chip).
                LLMPreferences::handle(ctx).update(ctx, |llm_prefs, ctx| {
                    llm_prefs.remove_llm_override(self.terminal_view_id, ctx);
                });
            }
            InlineProfileSelectorEvent::ManageProfiles => {
                ctx.emit(Event::OpenSettings(SettingsSection::AgentProfiles));
            }
            InlineProfileSelectorEvent::Dismissed => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_profile_selector()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
                return;
            }
        }

        if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_profile_selector()
        {
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
            ctx.notify();
        }
        self.clear_buffer_and_reset_undo_stack(ctx);
        self.focus_input_box(ctx);
    }

    fn handle_inline_prompts_menu_event(
        &mut self,
        event: &InlinePromptsMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let InlinePromptsMenuEvent::SelectedPrompt { id } = event;

        let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(id).cloned() else {
            log::warn!("Tried to open saved prompt for id {id:?} but it does not exist");
            return;
        };

        if self.suggestions_mode_model.as_ref(ctx).is_prompts_menu() {
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
            ctx.notify();
        }
        self.clear_buffer_and_reset_undo_stack(ctx);
        self.focus_input_box(ctx);

        self.show_workflows_info_box_on_workflow_selection(
            WorkflowType::Cloud(Box::new(workflow)),
            WorkflowSource::WarpAI,
            WorkflowSelectionSource::SlashMenu,
            None,
            ctx,
        );
    }

    fn handle_inline_skill_selector_event(
        &mut self,
        event: &InlineSkillSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let InlineSkillSelectorEvent::SelectedSkill {
            skill_name,
            skill_reference,
        } = event;

        if self.skill_selector_should_invoke {
            // Insert the skill invocation into the buffer using the CLI agent's
            // native prefix (e.g. "/" for most agents, "$" for Codex).
            let prefix = CLIAgentSessionsModel::as_ref(ctx)
                .session(self.terminal_view_id)
                .map(|s| s.agent.skill_command_prefix())
                .unwrap_or("/");
            self.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(format!("{prefix}{skill_name} ").as_str(), ctx);
            });

            // Close the menu but keep input focused so user can press Enter
            if self.suggestions_mode_model.as_ref(ctx).is_skill_menu() {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                ctx.notify();
            }
            self.focus_input_box(ctx);
        } else {
            // Open the skill file in editor (from /open-skill command)
            send_telemetry_from_ctx!(
                SkillTelemetryEvent::Opened {
                    reference: skill_reference.clone(),
                    name: Some(skill_name.clone()),
                    origin: SkillOpenOrigin::OpenSkillCommand,
                },
                ctx
            );

            ctx.dispatch_typed_action(&TerminalAction::OpenEditSkillPane {
                skill_reference: skill_reference.clone(),
            });

            // Close the skill selector menu and clear the buffer
            if self.suggestions_mode_model.as_ref(ctx).is_skill_menu() {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                ctx.notify();
            }
            self.clear_buffer_and_reset_undo_stack(ctx);
            self.focus_input_box(ctx);
        }
    }

    fn open_model_selector(&mut self, ctx: &mut ViewContext<Self>) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::ModelSelector, ctx);
        });

        ctx.notify();
    }

    fn open_profile_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::InlineProfileSelector.is_enabled() {
            return;
        }

        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::ProfileSelector, ctx);
        });

        ctx.notify();
    }

    fn open_prompts_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::PromptsMenu, ctx);
        });

        ctx.notify();
    }

    fn open_skill_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::ListSkills.is_enabled() {
            return;
        }

        self.skill_selector_should_invoke = false;
        self.inline_skill_selector_view.update(ctx, |view, ctx| {
            view.set_include_bundled(false, ctx);
        });
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::SkillMenu, ctx);
        });

        ctx.notify();
    }

    fn open_invoke_skill_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::ListSkills.is_enabled() {
            return;
        }

        self.skill_selector_should_invoke = true;
        self.inline_skill_selector_view.update(ctx, |view, ctx| {
            view.set_include_bundled(true, ctx);
        });
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::SkillMenu, ctx);
        });

        ctx.notify();
    }

    pub fn open_plan_menu(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::PlanMenu { conversation_id }, ctx);
        });
        ctx.notify();
    }

    fn handle_plan_menu_event(&mut self, event: &InlinePlanMenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            InlinePlanMenuEvent::OpenPlan {
                document_id,
                document_version,
            } => {
                ctx.emit(Event::OpenAIDocumentPane {
                    document_id: *document_id,
                    document_version: *document_version,
                });
                if self.suggestions_mode_model.as_ref(ctx).is_plan_menu() {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
            }
            InlinePlanMenuEvent::Dismissed => {
                if self.suggestions_mode_model.as_ref(ctx).is_plan_menu() {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
            }
        }
    }

    fn open_conversation_menu(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't open menu if there's a long-running command
        if self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            return;
        }

        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::ConversationMenu, ctx);
        });
        let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_fullscreen();
        send_telemetry_from_ctx!(
            TelemetryEvent::InlineConversationMenuOpened { is_in_agent_view },
            ctx
        );
        ctx.notify();
    }

    fn open_repos_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::IndexedReposMenu, ctx);
        });
        ctx.notify();
    }

    fn handle_user_query_menu_event(
        &mut self,
        event: &UserQueryMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.suggestions_mode_model.as_ref(ctx).is_user_query_menu() {
            log::error!("handle_user_query_menu_event called when mode is not UserQueryMenu");
            return;
        }

        match event {
            UserQueryMenuEvent::SelectedQuery { exchange_id } => {
                ctx.emit(Event::ScrollToExchange {
                    exchange_id: *exchange_id,
                });
            }
            UserQueryMenuEvent::AcceptedQuery {
                exchange_id,
                cmd_enter,
            } => {
                let Some(conversation_id) = self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .user_query_conversation_id()
                else {
                    log::error!("No conversation_id in UserQueryMenu mode when accepting");
                    return;
                };

                let destination = if *cmd_enter {
                    ForkedConversationDestination::SplitPane
                } else {
                    ForkedConversationDestination::CurrentPane
                };
                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id,
                    fork_from_exchange: Some(ForkFromExchange {
                        exchange_id: *exchange_id,
                        fork_from_exact_exchange: false,
                    }),
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: None,
                    destination,
                });

                let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_active();
                send_telemetry_from_ctx!(
                    TelemetryEvent::SlashCommandAccepted {
                        command_details: SlashCommandAcceptedDetails::StaticCommand {
                            command_name: commands::FORK_FROM.name.to_owned(),
                        },
                        is_in_agent_view,
                    },
                    ctx
                );

                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                ctx.notify();
                self.clear_buffer_and_reset_undo_stack(ctx);
            }
            UserQueryMenuEvent::Dismissed => {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.close_and_restore_buffer(ctx);
                });
                ctx.notify();
            }
        }
    }

    fn handle_inline_history_menu_event(
        &mut self,
        event: &inline_history::InlineHistoryMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            inline_history::InlineHistoryMenuEvent::NavigateToConversation { conversation_id } => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
                self.clear_buffer_and_reset_undo_stack(ctx);
                self.agent_view_controller.update(ctx, |controller, ctx| {
                    let _ = controller.try_enter_agent_view(
                        Some(*conversation_id),
                        AgentViewEntryOrigin::InlineHistoryMenu,
                        ctx,
                    );
                });
            }
            inline_history::InlineHistoryMenuEvent::AcceptCommand { command, .. } => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(command, ctx);
                });
                self.input_enter(ctx);
            }
            inline_history::InlineHistoryMenuEvent::AcceptAIPrompt { query_text } => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                }
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(query_text, ctx);
                });
                self.input_enter(ctx);
            }
            inline_history::InlineHistoryMenuEvent::SelectCommand {
                command,
                linked_workflow_data,
            } => {
                if let Some((workflow_type, workflow_source)) = linked_workflow_data
                    .as_ref()
                    .and_then(|linked_workflow_data| linked_workflow_data.linked_workflow(ctx))
                {
                    // TODO(ben): We should include the chosen env vars in the history
                    // entry.
                    let env_vars = workflow_type.as_workflow().default_env_vars();
                    self.insert_workflow_into_input(
                        workflow_type,
                        workflow_source,
                        WorkflowSelectionSource::UpArrowHistory,
                        None,
                        Some(command),
                        env_vars,
                        /*should_show_more_info_view=*/ false,
                        ctx,
                    );
                } else {
                    self.editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text_ignoring_undo(command, ctx);
                    });
                }

                // In fullscreen agent view, lock to Shell mode so the '!' indicator is
                // rendered while cycling through shell command history.
                let is_agent_view_fullscreen =
                    self.agent_view_controller.as_ref(ctx).is_fullscreen();
                self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                    if is_agent_view_fullscreen {
                        ai_input_model.set_input_config(
                            InputConfig {
                                input_type: InputType::Shell,
                                is_locked: true,
                            },
                            false,
                            ctx,
                        );
                    } else {
                        ai_input_model.set_input_type(InputType::Shell, ctx);
                    }
                });
            }
            inline_history::InlineHistoryMenuEvent::SelectAIPrompt { query_text } => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text_ignoring_undo(query_text, ctx);
                });

                self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                    ai_input_model.set_input_type(InputType::AI, ctx);
                });
            }
            inline_history::InlineHistoryMenuEvent::SelectConversation => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text_ignoring_undo("", ctx);
                });
            }
            inline_history::InlineHistoryMenuEvent::Close => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                }
            }
            inline_history::InlineHistoryMenuEvent::NoResults => {
                // Both the regular inline view and the cloud-mode V2 wrapper
                // render their own "No results" placeholder UI when the
                // mixer query produces zero rows. This handler is therefore
                // a no-op; the user dismisses via Escape.
            }
        }
    }

    fn restore_buffer_state(&mut self, buffer_state: &BufferState, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_ignoring_undo(&buffer_state.buffer, ctx);
            if let Some(original_cursor_point) = &buffer_state.cursor_point {
                editor.reset_selections_to_point(original_cursor_point, ctx);
            }
        });
        ctx.notify();
    }

    fn open_user_query_menu(&mut self, action: UserQueryMenuAction, ctx: &mut ViewContext<Self>) {
        // Don't reopen if already open.
        if self.suggestions_mode_model.as_ref(ctx).is_user_query_menu() {
            return;
        }

        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };

        // Close any other menus first
        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
        }

        // Clear the input buffer
        self.clear_buffer_and_reset_undo_stack(ctx);

        // Open the menu - conversation_id is stored in the mode and the view reads it from there
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(
                InputSuggestionsMode::UserQueryMenu {
                    action,
                    conversation_id,
                },
                ctx,
            );
        });

        ctx.notify();
    }

    fn open_rewind_menu(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't reopen if already open.
        if self.suggestions_mode_model.as_ref(ctx).is_rewind_menu() {
            return;
        }

        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };

        // Close any other menus first
        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
        }

        // Clear the input buffer
        self.clear_buffer_and_reset_undo_stack(ctx);

        // Open the rewind menu
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(
                InputSuggestionsMode::UserQueryMenu {
                    action: UserQueryMenuAction::Rewind,
                    conversation_id,
                },
                ctx,
            );
        });

        ctx.notify();
    }

    fn handle_rewind_menu_event(&mut self, event: &RewindMenuEvent, ctx: &mut ViewContext<Self>) {
        if !self.suggestions_mode_model.as_ref(ctx).is_rewind_menu() {
            log::error!("handle_rewind_menu_event called when mode is not RewindMenu");
            return;
        }

        match event {
            RewindMenuEvent::Dismissed => {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.close_and_restore_buffer(ctx);
                });
                ctx.notify();
            }
            RewindMenuEvent::AcceptedRewindPoint { exchange_id } => {
                // If exchange_id is None, user selected "Current" - just close menu
                let Some(exchange_id) = exchange_id else {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    ctx.notify();
                    self.clear_buffer_and_reset_undo_stack(ctx);
                    return;
                };

                let Some(conversation_id) = self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .rewind_conversation_id()
                else {
                    log::error!("No conversation_id in RewindMenu mode when accepting");
                    return;
                };

                ctx.dispatch_typed_action(&TerminalAction::ExecuteRewindFromInlineMenu {
                    conversation_id,
                    exchange_id: *exchange_id,
                });

                let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_active();
                send_telemetry_from_ctx!(
                    TelemetryEvent::SlashCommandAccepted {
                        command_details: SlashCommandAcceptedDetails::StaticCommand {
                            command_name: commands::REWIND.name.to_owned(),
                        },
                        is_in_agent_view,
                    },
                    ctx
                );

                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                ctx.notify();
                self.clear_buffer_and_reset_undo_stack(ctx);
            }
        }
    }

    fn open_inline_history_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::InlineHistoryMenu.is_enabled() {
            return;
        }

        // Don't open inline history menu if a chip menu or model selector is already open
        let agent_footer = self.agent_input_footer.as_ref(ctx);
        if self.prompt_render_helper.has_open_chip_menu(ctx)
            || agent_footer.has_open_chip_menu(ctx)
            || agent_footer.is_model_selector_open(ctx)
        {
            return;
        }

        let original_input_config = self.ai_input_model.as_ref(ctx).input_config();
        self.suggestions_mode_model.update(ctx, |m, ctx| {
            m.set_mode(
                InputSuggestionsMode::InlineHistoryMenu {
                    original_input_config: Some(original_input_config),
                },
                ctx,
            );
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::OpenSuggestionsMenu(
                self.suggestions_mode_model
                    .as_ref(ctx)
                    .mode()
                    .to_telemetry_mode(),
            ),
            ctx
        );
        ctx.notify();
    }

    /// Executes a skill command.
    ///
    /// This enters AI mode, resolves the skill from SkillManager, and submits it.
    ///
    /// When `is_queued_prompt` is true, this is the first send of a previously queued prompt:
    /// the input buffer is left alone (the user may have typed new input while the agent was
    /// busy) and the emitted `SentRequest` event is tagged as a queued-prompt submission so
    /// other UI subscribers also skip their user-submission side effects.
    ///
    /// Returns `true` if execution was handled.
    fn execute_skill_command(
        &mut self,
        reference: SkillReference,
        user_query: Option<String>,
        is_queued_prompt: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Resolve the skill from SkillManager
        let skill = match SkillManager::handle(ctx)
            .as_ref(ctx)
            .skill_by_reference(&reference)
        {
            Some(skill) => skill.clone(),
            None => {
                // Show error toast if skill not found
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(format!("Skill not found: {}", reference)),
                        window_id,
                        ctx,
                    );
                });
                return true;
            }
        };

        // Clear the buffer (unless this is a queued-prompt auto-send, in which case
        // the buffer may contain new input the user has started typing).
        if !is_queued_prompt {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            });
        }

        // Enter agent view if not already active
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.agent_view_controller.update(ctx, |controller, ctx| {
                let _ = controller.try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::SlashCommand {
                        trigger: SlashCommandTrigger::input(),
                    },
                    ctx,
                );
            });
        }

        // Send the skill invocation request
        let request = SlashCommandRequest::InvokeSkill { skill, user_query };
        self.ai_controller.update(ctx, move |controller, ctx| {
            if is_queued_prompt {
                controller.send_queued_slash_command_request(request, ctx);
            } else {
                controller.send_slash_command_request(request, ctx);
            }
        });

        true
    }

    #[cfg(not(target_family = "wasm"))]
    fn export_conversation_to_file(
        &mut self,
        filename_arg: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        use chrono::Local;
        use std::fs;
        use std::path::PathBuf;

        let history = BlocklistAIHistoryModel::handle(ctx);
        let Some(conversation) = history
            .as_ref(ctx)
            .active_conversation(self.terminal_view_id)
        else {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast =
                    DismissibleToast::default(String::from("No active conversation to export"));
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
            return;
        };

        // Determine the filename
        let filename = if let Some(name) = filename_arg.as_ref().filter(|s| !s.trim().is_empty()) {
            name.trim().to_string()
        } else {
            // Generate default filename: timestamp-conversation_title.md
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            let title = conversation
                .title()
                .unwrap_or_else(|| "conversation".to_string())
                .chars()
                .map(|c| {
                    // Replace spaces with underscores, keep alphanumeric, underscores, and hyphens
                    if c.is_whitespace() {
                        '_'
                    } else if c.is_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            format!("{timestamp}-{title}.md")
        };

        // Ensure the filename has .md extension
        let filename = if !filename.ends_with(".md") {
            format!("{filename}.md")
        } else {
            filename
        };

        let current_dir = self
            .active_block_metadata
            .as_ref()
            .and_then(|metadata| metadata.current_working_directory())
            .map(PathBuf::from)
            .or_else(|| {
                log::debug!(
                    "No CWD from active_block_metadata, falling back to std::env::current_dir()"
                );
                std::env::current_dir().ok()
            })
            .unwrap_or_else(|| {
                log::warn!("Failed to determine current directory, using '.'");
                PathBuf::from(".")
            });

        let file_path = current_dir.join(&filename);

        let action_model = self.ai_action_model.as_ref(ctx);
        let conversation_text = conversation.export_to_markdown(Some(action_model));

        // Check if file already exists and warn user
        let file_exists = file_path.exists();
        if file_exists {
            let window_id = ctx.window_id();
            let display_path = file_path.display().to_string();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast = DismissibleToast::default(format!(
                    "File {display_path} already exists and will be overwritten"
                ));
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
        }

        // Write to file
        match fs::write(&file_path, conversation_text) {
            Ok(_) => {
                // Show success toast
                let window_id = ctx.window_id();
                let display_path = file_path.display().to_string();
                ToastStack::handle(ctx).update(ctx, move |toast_stack, ctx| {
                    let toast = DismissibleToast::default(format!(
                        "Conversation exported to {display_path}"
                    ));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            Err(e) => {
                // Show error toast with user-friendly message
                let user_message = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        format!(
                            "Permission denied writing to {}. Check file permissions.",
                            file_path.display()
                        )
                    }
                    std::io::ErrorKind::NotFound => {
                        format!(
                            "Directory not found: {}",
                            file_path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default()
                        )
                    }
                    std::io::ErrorKind::AlreadyExists => {
                        format!("File {} already exists", file_path.display())
                    }
                    _ => {
                        format!("Failed to export to {}: {}", file_path.display(), e)
                    }
                };

                log::error!(
                    "Failed to write conversation to file {}: {}",
                    file_path.display(),
                    e
                );
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, move |toast_stack, ctx| {
                    let toast = DismissibleToast::default(user_message);
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
        }

        // Clear the buffer after execution
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
    }
    /// When the active conversation is changed, the number of attached images may exceed the
    /// limit of images for a conversation
    pub fn remove_excess_images(&mut self, ctx: &mut ViewContext<Self>) {
        let num_images_attached = self.ai_context_model.as_ref(ctx).pending_images().len();

        let Some(conversation) = self.ai_context_model.as_ref(ctx).selected_conversation(ctx)
        else {
            return;
        };

        let num_images_in_conversation = conversation
            .get_root_task()
            .into_iter()
            .flat_map(|task| {
                task.all_contexts()
                    .filter(|context| matches!(context, AIAgentContext::Image(_)))
            })
            .count();

        let excess_images = (num_images_in_conversation + num_images_attached)
            .saturating_sub(MAX_IMAGES_PER_CONVERSATION);

        let images_removed = self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.remove_last_pending_images(excess_images, ctx)
        });

        if images_removed > 0 {
            let window_id = ctx.window_id();

            let message = if images_removed == 1 {
                "1 image was removed - limit is 20 per conversation.".into()
            } else {
                format!("{images_removed} images were removed - limit is 20 per conversation.")
            };

            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_persistent_toast(DismissibleToast::error(message), window_id, ctx);
            });
        }
    }

    pub fn update_image_context_options(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_input_model = self.ai_input_model.as_ref(ctx);

        let llm_prefs = LLMPreferences::as_ref(ctx);

        let vision_supported = llm_prefs.vision_supported(ctx, Some(self.terminal_view_id));

        let num_images_attached = self.ai_context_model.as_ref(ctx).pending_images().len();

        let conversation = self.ai_context_model.as_ref(ctx).selected_conversation(ctx);

        let num_images_in_conversation = conversation
            .and_then(|conversation| conversation.get_root_task())
            .into_iter()
            .flat_map(|task| {
                task.all_contexts()
                    .filter(|context| matches!(context, AIAgentContext::Image(_)))
            })
            .count();

        // Image context is available whenever the feature flag is enabled and we're in AI input
        // mode, including cloud mode
        let image_context_options = if FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI)
        {
            ImageContextOptions::Enabled {
                unsupported_model: !vision_supported,
                is_processing_attached_images: self.is_processing_attached_images,
                num_images_attached,
                num_images_in_conversation,
            }
        } else {
            ImageContextOptions::Disabled
        };

        self.editor.update(ctx, move |editor, ctx| {
            editor.update_image_context_options(image_context_options, ctx);
            ctx.notify();
        });
    }

    pub fn set_shared_session_presence_manager(
        &mut self,
        presence_manager: ModelHandle<PresenceManager>,
    ) {
        self.shared_session_presence_manager = Some(presence_manager);
    }

    pub fn set_prompt_suggestions_banner_state(
        &mut self,
        banner_state: Option<PromptSuggestionBannerState>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.has_prompt_suggestion_banner
            .store(banner_state.is_some(), Ordering::Relaxed);
        self.prompt_suggestions_banner_state = banner_state.clone();

        if let Some(banner_state) = banner_state {
            self.prompt_suggestions_view.update(ctx, |view, ctx| {
                view.set_banner_state(banner_state);
                ctx.notify();
            });
        }

        ctx.notify();
    }

    pub fn maybe_set_prompt_suggestions_banner_state_should_hide(&mut self, should_hide: bool) {
        if let Some(banner_state) = &mut self.prompt_suggestions_banner_state {
            banner_state.should_hide = should_hide;
        }
    }

    // Auto-attach the last block for this query.
    fn auto_attach_last_block_for_query(&mut self, ctx: &mut ViewContext<Self>) {
        let last_block_id = {
            let model = self.model.lock();
            model
                .block_list()
                .last_non_hidden_block()
                .map(|block| block.id().clone())
        };

        if let Some(block_id) = last_block_id {
            self.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_context_block_ids(vec![block_id], true, ctx);
            });
        }
    }

    pub fn clear_attached_context(&mut self, ctx: &mut ViewContext<Self>) {
        self.ai_context_model.update(ctx, |model, ctx| {
            model.reset_context_to_default(ctx);
        });
        ctx.emit(Event::ClearSelectionsWhenShellMode);
    }

    pub fn ai_input_model(&self) -> &ModelHandle<BlocklistAIInputModel> {
        &self.ai_input_model
    }

    /// Inserts a zero state prompt suggestion into the input buffer and executes the query for Agent Mode.
    pub fn insert_zero_state_prompt_suggestion(
        &mut self,
        suggestion_type: ZeroStatePromptSuggestionType,
        triggered_from: ZeroStatePromptSuggestionTriggeredFrom,
        ctx: &mut ViewContext<Self>,
    ) {
        if !AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx) {
            return;
        }

        match suggestion_type {
            ZeroStatePromptSuggestionType::Explain | ZeroStatePromptSuggestionType::Fix => {
                self.auto_attach_last_block_for_query(ctx);
            }
            _ => {}
        }

        self.focus_input_box(ctx);
        // TODO(advait): Avoid using user-simulated codepaths here. Revisit function to use here.
        self.submit_ai_query(Some(suggestion_type), ctx);

        send_telemetry_from_ctx!(
            TelemetryEvent::ZeroStatePromptSuggestionUsed {
                suggestion_type,
                triggered_from
            },
            ctx
        );

        ctx.notify()
    }

    fn cancel_active_conversation(
        &mut self,
        ctx: &mut ViewContext<Self>,
        cancellation_reason: CancellationReason,
    ) {
        self.ai_controller.update(ctx, |controller, ctx| {
            let active_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(self.terminal_view_id)
                .filter(|conversation| conversation.status().is_in_progress())
                .map(|conversation| conversation.id());
            if let Some(active_conversation_id) = active_conversation_id {
                controller.cancel_conversation_progress(
                    active_conversation_id,
                    cancellation_reason,
                    ctx,
                );
            }
        });
    }

    fn handle_prompt_event(&mut self, event: &PromptDisplayEvent, ctx: &mut ViewContext<Self>) {
        match event {
            PromptDisplayEvent::OpenFile(file_name) => {
                // Insert the filename into the terminal input
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(file_name, ctx);
                });
                ctx.notify();
            }
            PromptDisplayEvent::OpenTextFileInCodeEditor(file_name) => {
                // Open text file in a new code editor pane
                let result = self.open_file_in_code_editor(file_name, ctx);
                if let Err(e) = result {
                    log::warn!("Failed to open file in code editor: {e}");
                }
            }
            PromptDisplayEvent::ToggleMenu { open } => {
                if *open {
                    // Close any open input suggestion menus (history, Ctrl+R, etc.) when chip menus
                    // are opened to prevent overlapping menus in UDI
                    self.close_overlays(false, ctx);
                    ctx.notify();
                } else {
                    self.focus_input_box(ctx);
                }
            }
            PromptDisplayEvent::OpenCodeReview => {
                ctx.emit(Event::OpenCodeReviewPane);
            }
            PromptDisplayEvent::OpenConversationHistory => {
                // Emit event to open command palette with conversation filter
                ctx.emit(Event::OpenConversationHistory);
            }
            PromptDisplayEvent::OpenCommandPaletteFiles => {
                ctx.emit(Event::OpenFilesPalette {
                    source: PaletteSource::ContextChip,
                });
            }
            PromptDisplayEvent::RunAgentQuery(query) => {
                self.cancel_active_conversation(ctx, CancellationReason::UserCommandExecuted);
                let query = query.clone();
                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.send_user_query_in_new_conversation(
                        query,
                        None,
                        EntrypointType::UserInitiated,
                        None,
                        ctx,
                    );
                });
            }
            PromptDisplayEvent::TryExecuteCommand(command) => {
                // Snapshot the current input so we can restore it after the command completes.
                let current_input = self.buffer_text(ctx);
                if self.try_execute_command_from_source(command, CommandExecutionSource::User, ctx)
                {
                    self.cancel_active_conversation(ctx, CancellationReason::UserCommandExecuted);
                    if !current_input.is_empty() {
                        self.input_contents_before_prompt_chip_command = Some(current_input);
                    }
                }
            }
            PromptDisplayEvent::OpenAIDocument {
                document_id,
                document_version,
            } => {
                ctx.emit(Event::ToggleAIDocumentPane {
                    document_id: *document_id,
                    document_version: *document_version,
                });
            }
        }
    }

    fn open_file_in_code_editor(
        &mut self,
        _file_name: &str,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let Some(session_id) = self.active_block_session_id() else {
            return Err("Tried to open file in code editor without a session id".to_string());
        };

        let Some(session) = self.sessions.as_ref(ctx).get(session_id) else {
            return Err("Tried to open file in code editor without a session".to_string());
        };

        if !session.is_local() {
            return Err("Tried to open file in code editor for a remote session".to_string());
        }

        #[cfg(feature = "local_fs")]
        {
            // Get the current working directory from the active terminal session
            let current_dir = self
                .active_block_metadata
                .as_ref()
                .and_then(|metadata| metadata.current_working_directory())
                .map(std::path::PathBuf::from)
                .ok_or("Failed to get current working directory".to_string())?;
            let file_path = current_dir.join(_file_name);
            // Create a CodeSource for the file
            let code_source = CodeSource::Link {
                path: file_path,
                range_start: None,
                range_end: None,
            };
            // Emit an event to create a new code pane
            ctx.emit(Event::OpenCodeInWarp {
                source: code_source,
                layout: *external_editor::EditorSettings::as_ref(ctx)
                    .open_file_layout
                    .value(),
            });
        }

        Ok(())
    }

    fn handle_theme_change(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_apply_decorations(ctx) {
            self.run_input_background_jobs(
                InputBackgroundJobOptions::default().with_command_decoration(),
                ctx,
            );
        }
        // Recompute the contrast-adjusted editor text colors for the CLI agent
        // rich input, in case the new theme's defaults contrast differently
        // against an alt-screen CLI agent background.
        self.update_cli_agent_editor_text_colors(ctx);
    }

    pub fn sessions<'a, A: ModelAsRef>(&self, ctx: &'a A) -> &'a Sessions {
        self.sessions.as_ref(ctx)
    }

    pub fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle.clone());
        let focus_model = focus_handle.focus_state_handle().clone();
        ctx.subscribe_to_model(&focus_model, move |me, _, event, ctx| {
            if !focus_handle.is_affected(event) {
                return;
            }

            let is_focused = focus_handle.is_focused(ctx);

            me.prompt_render_helper
                .prompt_view()
                .update(ctx, |prompt_view, ctx| {
                    prompt_view.on_pane_focus_changed(is_focused, ctx);
                });

            me.set_zero_state_hint_text(ctx);

            // Update the universal developer input button bar blurred state when focus changes
            if me.should_show_universal_developer_input(ctx) {
                me.universal_developer_input_button_bar
                    .update(ctx, |button_bar, ctx| {
                        button_bar.set_is_in_active_terminal(is_focused, ctx);
                    });
            }
        });
    }

    fn is_pane_focused(&self, app: &AppContext) -> bool {
        // If the focus handle hasn't been set yet, assume we're not in a split pane and therefore focused.
        self.focus_handle.as_ref().is_none_or(|h| h.is_focused(app))
    }

    fn is_active_session(&self, app: &AppContext) -> bool {
        self.focus_handle
            .as_ref()
            .is_some_and(|h| h.is_active_session(app))
    }

    pub fn menu_positioning(&self, app: &AppContext) -> MenuPositioning {
        self.menu_positioning_provider.menu_position(app)
    }

    fn size_info(&self, ctx: &AppContext) -> SizeInfo {
        ctx.model(&self.input_render_state_model_handle).size_info()
    }

    pub fn set_size_info(&mut self, size_info: SizeInfo, ctx: &mut AppContext) {
        self.input_render_state_model_handle
            .update(ctx, |input_render_state_model, _| {
                input_render_state_model.set_size_info(size_info);
            });
    }

    pub fn editor(&self) -> &ViewHandle<EditorView> {
        &self.editor
    }

    pub(crate) fn ai_context_model(&self) -> &ModelHandle<BlocklistAIContextModel> {
        &self.ai_context_model
    }

    pub fn buffer_text(&self, ctx: &AppContext) -> String {
        self.editor.as_ref(ctx).buffer_text(ctx)
    }

    pub fn buffer_text_number_of_lines(&self, ctx: &AppContext) -> usize {
        self.buffer_text(ctx).lines().count()
    }

    #[cfg(feature = "integration_tests")]
    pub fn input_suggestions(&self) -> &ViewHandle<InputSuggestions> {
        &self.input_suggestions
    }

    pub fn suggestions_mode_model(&self) -> &ModelHandle<InputSuggestionsModeModel> {
        &self.suggestions_mode_model
    }

    pub fn inline_terminal_menu_positioner(&self) -> &ModelHandle<InlineMenuPositioner> {
        &self.inline_terminal_menu_positioner
    }

    pub fn completer_data(&self) -> CompleterData {
        CompleterData::new(
            self.sessions.clone(),
            self.active_block_metadata.clone(),
            CommandRegistry::global_instance(),
            self.last_user_block_completed.clone(),
        )
    }

    fn start_byte_index_of_first_selection(&self, ctx: &ViewContext<Self>) -> ByteOffset {
        self.editor
            .as_ref(ctx)
            .start_byte_index_of_first_selection(ctx)
    }

    // Returns the appropriate hint/placeholder text to render in an empty input when Agent Mode is
    // enabled (the feature flag, not the specific AI input mode). This method ensures that hint text
    // is cached when needed for new conversations.
    fn agent_mode_hint_text(&mut self, app: &AppContext) -> &str {
        let input_model = self.ai_input_model.as_ref(app);
        let is_udi_enabled = InputSettings::as_ref(app).is_universal_developer_input_enabled(app);

        match (
            input_model.input_type(),
            input_model.should_run_input_autodetection(app),
        ) {
            (InputType::Shell, false) => AGENT_MODE_AI_DISABLED_AUTODETECTION_DISABLED_HINT_TEXT,
            (InputType::Shell, true) => {
                // Ensure hint text is cached for new conversations
                get_stable_agent_mode_hint_text(&mut self.cached_agent_mode_hint_text)
            }
            (InputType::AI, _) => {
                // Follow the `agent_indicator` pattern (see `app/src/tab.rs`):
                //  * `None` (no conversation, empty, passive, or untitled) => new conversation => "Warp anything"
                //  * `InProgress`                                           => agent running    => "Steer"
                //  * Any other status                                       => finished         => "Ask a follow up"
                match self
                    .ai_context_model
                    .as_ref(app)
                    .selected_conversation_status_for_hint(app)
                {
                    Some(status) if status.is_in_progress() => {
                        if is_udi_enabled {
                            AGENT_MODE_AI_ENABLED_STEER_HINT_TEXT_UDI
                        } else {
                            AGENT_MODE_AI_ENABLED_STEER_HINT_TEXT_CLASSIC
                        }
                    }
                    Some(_) => {
                        if is_udi_enabled {
                            AGENT_MODE_AI_ENABLED_FOLLOW_UP_HINT_TEXT_UDI
                        } else {
                            AGENT_MODE_AI_ENABLED_FOLLOW_UP_HINT_TEXT_CLASSIC
                        }
                    }
                    None => {
                        // Ensure hint text is cached for new conversations
                        get_stable_agent_mode_hint_text(&mut self.cached_agent_mode_hint_text)
                    }
                }
            }
        }
    }

    fn handle_input_settings_event(
        &mut self,
        input_settings: ModelHandle<InputSettings>,
        event: &InputSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InputSettingsChangedEvent::ShowHintText { .. } => {
                self.set_zero_state_hint_text(ctx);
                ctx.notify();
            }
            InputSettingsChangedEvent::SyntaxHighlighting { .. } => {
                if !*input_settings.as_ref(ctx).syntax_highlighting.value() {
                    self.clear_decorations(ctx);
                }
                self.run_input_background_jobs(
                    InputBackgroundJobOptions::default().with_command_decoration(),
                    ctx,
                );
            }
            InputSettingsChangedEvent::ErrorUnderliningEnabled { .. } => {
                if !*input_settings.as_ref(ctx).error_underlining.value() {
                    self.clear_decorations(ctx);
                }
                self.run_input_background_jobs(
                    InputBackgroundJobOptions::default().with_command_decoration(),
                    ctx,
                );
            }
            InputSettingsChangedEvent::InputBoxTypeSetting { .. } => {
                // Force a re-render when switching between Universal and Classic input modes
                // to ensure all UI elements update in real-time
                self.set_zero_state_hint_text(ctx);
                ctx.notify();
            }
            InputSettingsChangedEvent::AtContextMenuInTerminalMode { .. } => {
                self.check_and_update_ai_context_menu_disabled_state(ctx);
                ctx.notify();
            }
            InputSettingsChangedEvent::CompletionsMenuWidth { .. } => {
                let new_value = *input_settings.as_ref(ctx).completions_menu_width.value();
                if let Ok(mut guard) = self.completions_menu_resizable_width.lock() {
                    guard.set_size(new_value);
                }
                ctx.notify();
            }
            InputSettingsChangedEvent::CompletionsMenuHeight { .. } => {
                let new_value = *input_settings.as_ref(ctx).completions_menu_height.value();
                if let Ok(mut guard) = self.completions_menu_resizable_height.lock() {
                    guard.set_size(new_value);
                }
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_next_command_model_event(
        &mut self,
        event: &NextCommandModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            NextCommandModelEvent::NextCommandSuggestionReady => {
                let NextCommandSuggestionState::Ready { is_from_cycle, .. } =
                    self.next_command_model.as_ref(ctx).get_state()
                else {
                    return;
                };

                // If there is already an autosuggestion for some reason, don't replace it to avoid flickering.
                // But if the suggestion came from cycling, we want to replace it.
                let editor = self.editor.as_ref(ctx);
                if !is_from_cycle && editor.active_autosuggestion() {
                    return;
                }

                let input_type = self.ai_input_model.as_ref(ctx).input_type();
                self.editor.update(ctx, |editor, ctx| {
                    editor.maybe_populate_intelligent_autosuggestion(input_type, ctx);
                });
            }
        }
    }

    #[cfg(feature = "voice_input")]
    pub(super) fn toggle_voice_input(
        &mut self,
        from: &voice_input::VoiceInputToggledFrom,
        ctx: &mut ViewContext<Self>,
    ) {
        self.enter_ai_mode(ctx);
        let did_start_listening = self
            .editor
            .update(ctx, |editor, ctx| editor.toggle_voice_input(from, ctx));
        if did_start_listening {
            self.focus_input_box(ctx);
        }
    }

    fn select_image(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_input_box(ctx);

        self.ensure_agent_mode_for_ai_features(true, ctx);

        // Update image context options immediately after switching to AI mode
        // to ensure attach_images has the correct state
        self.update_image_context_options(ctx);

        self.editor.update(ctx, |editor, ctx| {
            editor.attach_files(ctx);
        });
    }
    pub(super) fn insert_into_cli_agent_rich_input(
        &mut self,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.focus_input_box(ctx);
        self.editor.update(ctx, |editor, ctx| {
            editor.user_initiated_insert(text, PlainTextEditorViewAction::Paste, ctx);
        });
    }

    fn handle_prompt_alert(
        &mut self,
        prompt_alert: &PromptAlertEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match prompt_alert {
            PromptAlertEvent::SignupAnonymousUser => {
                ctx.emit(Event::SignupAnonymousUser {
                    entrypoint: AnonymousUserSignupEntrypoint::SignUpAIPrompt,
                });
            }
            PromptAlertEvent::OpenBillingAndUsagePage => {
                ctx.emit(Event::OpenSettings(SettingsSection::BillingAndUsage));
            }
            PromptAlertEvent::OpenPrivacyPage => {
                ctx.emit(Event::OpenSettings(SettingsSection::Privacy));
            }
            PromptAlertEvent::OpenBillingPortal { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
        }
    }

    fn enable_auto_detection(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't allow input mode changes for read-only viewers in shared sessions
        if self.model.lock().shared_session_status().is_reader() {
            return;
        }

        // Don't allow enabling autodetection when agent is monitoring a command
        if self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_in_control_or_tagged_in()
        {
            return;
        }

        let ai_settings = AISettings::as_ref(ctx);
        if FeatureFlag::AgentView.is_enabled() {
            if self.agent_view_controller.as_ref(ctx).is_fullscreen() {
                if !ai_settings.is_ai_autodetection_enabled(ctx) {
                    return;
                }
            } else if !ai_settings.is_nld_in_terminal_enabled(ctx) {
                return;
            }
        } else if !ai_settings.is_ai_autodetection_enabled(ctx) {
            return;
        }

        self.focus_input_box(ctx);

        if !self.ai_input_model.as_ref(ctx).is_input_type_locked() {
            return;
        }

        let buffer_text = self.buffer_text(ctx);
        if buffer_text.is_empty() {
            // For empty buffer, immediately set to Shell mode with auto-detection enabled
            self.ai_input_model.update(ctx, |model, ctx| {
                let new_config = InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false, // Set to auto-detection mode
                };
                model.set_input_config(new_config, buffer_text.is_empty(), ctx);
            });
        } else {
            // For non-empty buffer, run the actual auto-detection algorithm
            // First unlock the input mode to enable auto-detection
            self.ai_input_model.update(ctx, |model, ctx| {
                let current_config = model.input_config();
                let new_config = InputConfig {
                    input_type: current_config.input_type, // Keep current type temporarily
                    is_locked: false,                      // Enable auto-detection
                };
                model.set_input_config(new_config, buffer_text.is_empty(), ctx);
            });

            // Then run auto-detection on the current buffer content
            self.run_input_background_jobs(
                InputBackgroundJobOptions::default().with_ai_input_detection(),
                ctx,
            );
        }
    }

    fn handle_universal_developer_input_button_bar_event(
        &mut self,
        event: &UniversalDeveloperInputButtonBarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            #[cfg(feature = "voice_input")]
            UniversalDeveloperInputButtonBarEvent::ToggleVoiceInput(from) => {
                self.toggle_voice_input(from, ctx);
            }
            UniversalDeveloperInputButtonBarEvent::InputTypeSelected(input_type) => {
                if self.is_input_mode_toggle_disabled(ctx) {
                    return;
                }

                self.focus_input_box(ctx);

                let is_input_buffer_empty = self.editor.as_ref(ctx).buffer_text(ctx).is_empty();

                let switch_to_auto = self.ai_input_model.update(ctx, |model, ctx| {
                    let is_autodetection_enabled =
                        AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
                    let input_type = *input_type;

                    // If the user clicked on the button to "unlock" the current mode,
                    // we want to enable autodetection.
                    if input_type == model.input_type()
                        && is_autodetection_enabled
                        && model.is_input_type_locked()
                    {
                        true
                    } else {
                        let new_config = InputConfig {
                            input_type,
                            is_locked: true,
                        };
                        model.set_input_config(new_config, is_input_buffer_empty, ctx);
                        false
                    }
                });

                if switch_to_auto {
                    self.set_input_mode_natural_language_detection(ctx);
                } else if *input_type == InputType::AI {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeClickedEntrypoint {
                            entrypoint: AgentModeEntrypoint::UDITerminalInputSwitcher,
                        },
                        ctx
                    );
                }
            }
            UniversalDeveloperInputButtonBarEvent::EnableAutoDetection => {
                self.enable_auto_detection(ctx);
            }
            UniversalDeveloperInputButtonBarEvent::SelectFile => {
                self.select_image(ctx);
            }
            UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(open) => {
                self.focus_input_box(ctx);
                self.set_ai_context_menu_open(*open, ctx);
            }
            UniversalDeveloperInputButtonBarEvent::PromptAlert(prompt_alert_event) => {
                self.handle_prompt_alert(prompt_alert_event, ctx);
            }
            UniversalDeveloperInputButtonBarEvent::ModelSelectorOpened => {
                self.close_overlays(false, ctx);
            }
            UniversalDeveloperInputButtonBarEvent::ModelSelectorClosed => {
                // When the model selector menu closes (model was selected), focus the input field
                self.focus_input_box(ctx);
            }
            UniversalDeveloperInputButtonBarEvent::OpenSettings(section) => {
                ctx.emit(Event::OpenSettings(*section));
            }
            UniversalDeveloperInputButtonBarEvent::OpenSlashCommandMenu => {
                self.focus_input_box(ctx);
                if !FeatureFlag::AgentView.is_enabled() {
                    self.ensure_agent_mode_for_ai_features(false, ctx);
                }
                self.toggle_legacy_slash_commands_menu(ctx);
            }
        }
    }

    /// Switches to AI mode but preserves current lock state.
    fn enter_ai_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.ai_input_model.update(ctx, |input_model, ctx| {
            input_model.set_input_type(InputType::AI, ctx);
        });
    }

    /// Helper function to ensure agent mode when needed, using the same logic as SelectFile.
    /// This handles the transition from shell mode to agent mode while preserving lock semantics.
    /// Only forces agent mode if the user hasn't explicitly locked the mode to Shell.
    pub fn ensure_agent_mode_for_ai_features(
        &mut self,
        should_override_shell_lock: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        let config = ai_input_model.input_config();

        // Don't force agent mode if user has explicitly locked to Shell mode
        if (!should_override_shell_lock || FeatureFlag::AgentView.is_enabled())
            && config.is_locked
            && !config.input_type.is_ai()
        {
            return;
        }

        self.enter_ai_mode(ctx);
    }

    fn cycle_next_command_suggestion(&mut self, ctx: &mut ViewContext<Self>) {
        self.next_command_model.update(ctx, |model, ctx| {
            model.cycle_next_command_suggestion(ctx);
        });
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_autosuggestion(ctx);
        });
    }

    /// Predicts the next action using an AI model and past context on blocks within Warp.
    /// Populates the autosuggestion with the predicted action, if any. Otherwise, falls back to
    /// existing autosuggestion logic.
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    fn maybe_predict_next_action_ai(
        &mut self,
        block_completed: UserBlockCompleted,
        ctx: &mut ViewContext<Self>,
    ) {
        if !is_next_command_enabled(ctx) {
            return;
        }

        // If the last block was empty, don't create any suggestions.
        // Also don't create suggestions for requested commands part of an agent mode conversation.
        if block_completed.command.is_empty() || block_completed.was_part_of_agent_interaction {
            return;
        }

        // If we already have an active autosuggestion (e.g. from command corrections), don't regenerate.
        let editor = self.editor.as_ref(ctx);
        if editor.active_autosuggestion() {
            return;
        }

        // We only have intelligent autosuggestions on empty buffer for now.
        if !self.buffer_text(ctx).is_empty() {
            return;
        }

        // Don't generate any next command suggestions if there is no internet.
        // This is needed to prevent generating history-based suggestions.
        if !NetworkStatus::as_ref(ctx).is_online() {
            return;
        }

        let Some(session) = self.active_session(ctx) else {
            return;
        };
        let context = WarpAiExecutionContext::new(&session);
        let completer_data = self.completer_data();
        let block_context = Some(BlockContext::from_completed_block(&block_completed));
        let previous_result = self.last_intelligent_autosuggestion_result.take();
        self.next_command_model.update(ctx, |model, ctx| {
            model.generate_next_command_suggestion(
                block_completed,
                context,
                completer_data,
                block_context,
                previous_result,
                ctx,
            );
        });
    }

    /// Clear the cached hint text to generate a new one on next render
    pub fn clear_cached_hint_text(&mut self) {
        self.cached_agent_mode_hint_text = None;
    }
    fn cli_agent_rich_input_hint_text(&self, ctx: &ViewContext<Self>) -> Cow<'static, str> {
        if self.is_locked_in_shell_mode(ctx) {
            return Cow::Borrowed(AGENT_MODE_AI_DISABLED_AUTODETECTION_DISABLED_HINT_TEXT);
        }

        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.terminal_view_id)
            .map(|session| match session.agent {
                CLIAgent::Unknown => Cow::Borrowed(CLI_AGENT_RICH_INPUT_HINT_TEXT),
                _ => Cow::Owned(format!(
                    "Enter prompt for {}...",
                    session.agent.display_name()
                )),
            })
            .unwrap_or(Cow::Borrowed(CLI_AGENT_RICH_INPUT_HINT_TEXT))
    }

    pub fn set_zero_state_hint_text(&mut self, ctx: &mut ViewContext<Self>) {
        if CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id) {
            let hint = self.cli_agent_rich_input_hint_text(ctx);
            self.editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(hint, ctx);
            });
            return;
        }
        if self.prefix_mode(ctx) == InputPrefixMode::CloudHandoff {
            let conversation_is_empty = BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(self.terminal_view_id)
                .is_none_or(|c| c.is_empty());
            let hint = if conversation_is_empty {
                CLOUD_MODE_V2_HINT_TEXT.to_owned()
            } else {
                self.handoff_compose_state
                    .as_ref(ctx)
                    .selected_environment_id()
                    .and_then(|id| CloudAmbientAgentEnvironment::get_by_id(id, ctx))
                    .map(|env| format!("Hand off to {}", env.model().string_model.display_name()))
                    .unwrap_or_else(|| "Handoff to cloud".to_owned())
            };
            self.editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(&hint, ctx);
            });
            return;
        }

        if self.is_cloud_mode_input_v2_composing(ctx) {
            let show_hint = *InputSettings::as_ref(ctx).show_hint_text;
            self.editor.update(ctx, |editor, ctx| {
                if show_hint {
                    editor.set_placeholder_text(CLOUD_MODE_V2_HINT_TEXT, ctx);
                } else {
                    editor.clear_placeholder_text(ctx);
                }
            });
            return;
        }
        // If the current input suggestions mode has a custom placeholder,
        // that takes precedence over other placeholders.
        if let Some(placeholder) = self
            .suggestions_mode_model
            .as_ref(ctx)
            .mode()
            .placeholder_text()
        {
            self.editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(placeholder, ctx);
            });
            return;
        }

        let toggled_on = *InputSettings::as_ref(ctx).show_hint_text;

        // Loop through all static commands and set placeholders for those with hint text
        self.editor.update(ctx, |editor, ctx| {
            for command in COMMAND_REGISTRY.all_commands() {
                if let Some(hint_text) = command
                    .argument
                    .as_ref()
                    .and_then(|argument| argument.hint_text)
                {
                    editor.set_placeholder_text_with_prefix(
                        format!("{} ", command.name),
                        hint_text,
                        ctx,
                    );
                }
            }
        });

        // Now handle the default (empty prefix) placeholder
        if toggled_on && AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            if FeatureFlag::AgentMode.is_enabled() {
                // agent_mode_hint_text now handles caching internally
                let hint_text = self.agent_mode_hint_text(ctx).to_string();
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_placeholder_text(&hint_text, ctx);
                });
            } else {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_placeholder_text(AI_COMMAND_SEARCH_HINT_TEXT, ctx);
                });
            }
        } else {
            self.editor.update(ctx, |editor, ctx| {
                // Clear only the default placeholder, keep slash command placeholders
                editor.clear_placeholder_text(ctx);
                ctx.notify();
            });
        }
    }

    /// Finds the start byte of the token under the given hovered point
    fn start_byte_index_at_point(
        &self,
        point: &DisplayPoint,
        ctx: &AppContext,
    ) -> Option<ByteOffset> {
        self.editor.read(ctx, |editor, ctx| {
            editor.start_byte_offset_at_point(point, ctx)
        })
    }

    fn handle_safe_mode_settings_changed_event(
        &mut self,
        event: &SafeModeSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SafeModeSettingsChangedEvent::SafeModeEnabled { .. }
            | SafeModeSettingsChangedEvent::HideSecretsInBlockList { .. }
            | SafeModeSettingsChangedEvent::SecretDisplayModeSetting { .. } => {
                self.model
                    .lock()
                    .set_obfuscate_secrets(get_secret_obfuscation_mode(ctx));
            }
        }
    }

    fn handle_ai_settings_changed_event(
        &mut self,
        event: &AISettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AISettingsChangedEvent::AgentModeQuerySuggestionsEnabled { .. }
            | AISettingsChangedEvent::IsAnyAIEnabled { .. }
            | AISettingsChangedEvent::IsActiveAIEnabled { .. } => {
                let ai_settings = AISettings::handle(ctx);
                if !ai_settings
                    .as_ref(ctx)
                    .is_intelligent_autosuggestions_enabled(ctx)
                    && matches!(
                        self.editor.as_ref(ctx).active_autosuggestion_type(),
                        Some(AutosuggestionType::Command {
                            was_intelligent_autosuggestion: true
                        })
                    )
                {
                    self.editor.update(ctx, |editor, ctx| {
                        editor.clear_autosuggestion(ctx);
                    });
                    self.next_command_model.update(ctx, |model, _| {
                        model.clear_state();
                    });
                }
                self.set_zero_state_hint_text(ctx);

                if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = event {
                    let is_input_buffer_empty = self.editor.as_ref(ctx).buffer_text(ctx).is_empty();
                    // If there is no AI enabled, ensure input is locked in command mode.
                    if !ai_settings.as_ref(ctx).is_any_ai_enabled(ctx) {
                        self.ai_input_model.update(ctx, |input_model, ctx| {
                            input_model.set_input_config(
                                InputConfig {
                                    input_type: InputType::Shell,
                                    is_locked: true,
                                },
                                is_input_buffer_empty,
                                ctx,
                            );
                        });
                    }
                }

                ctx.notify();
            }
            AISettingsChangedEvent::AIAutoDetectionEnabled { .. }
            | AISettingsChangedEvent::NLDInTerminalEnabled { .. } => {
                // NLD is irrelevant in cloud mode v2 — the input is always AI.
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    return;
                }
                // The input model handles updating the lock state via its own subscription.
                // If NLD is now enabled for the current context and the buffer is non-empty,
                // trigger autodetection on the current buffer contents.
                if self
                    .ai_input_model
                    .as_ref(ctx)
                    .should_run_input_autodetection(ctx)
                    && !self.editor.as_ref(ctx).buffer_text(ctx).is_empty()
                {
                    self.run_input_background_jobs(
                        InputBackgroundJobOptions::default().with_ai_input_detection(),
                        ctx,
                    );
                }
            }
            #[cfg(feature = "voice_input")]
            AISettingsChangedEvent::VoiceInputEnabled { .. } => {
                self.update_voice_transcription_options(ctx);
            }
            _ => {}
        }
    }

    fn handle_ignored_suggestions_event(
        &mut self,
        event: &IgnoredSuggestionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            IgnoredSuggestionsModelEvent::SuggestionIgnored => {
                // We may need to regenerate the autosuggestion if the suggestion just ignored
                // was the one suggested in the input.
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_autosuggestion(ctx);
                });
                self.maybe_generate_autosuggestion(ctx);
            }
        }
    }

    /// Returns `true` if we can query the [`History`] model for the active session.
    fn can_query_history(&self, ctx: &AppContext) -> bool {
        let model = self.model.lock();
        let Some(session_id) = model.block_list().active_block().session_id() else {
            return false;
        };

        let is_bootstrapped = model.block_list().is_bootstrapped();
        let is_history_queryable = History::as_ref(ctx).is_queryable(&session_id);

        // TODO: we should investigate why we need to check for bootstrapped here.
        // It's confusing and might actually be implied
        // (session history is only queryable if the session is bootstrapped).

        // We also return true for shared session executors since they're able to view the history
        // of a shared session without yet being hooked up to the history model.
        is_bootstrapped && (is_history_queryable || model.shared_session_status().is_executor())
    }

    /// Returns enum indicating if we can execute a command in the active session.
    ///
    /// We can only execute a command if:
    /// 1. the session is bootstrapped, because we don't want to interfere
    ///    with the PTY while bootstrapping is in progress
    /// 2. there isn't an active, long-running command (in-band commands are okay)
    /// 3. if the history for the session is appendable, because we want to
    ///    acknowledge the command in the session's history. Except when viewing
    ///    a shared session, since those sessions aren't registered in the [`History`]
    ///    model.
    fn can_execute_command(&self, ctx: &AppContext) -> CanExecuteCommand {
        let model = self.model.lock();
        let active_block = model.block_list().active_block();

        if !model.block_list().is_bootstrapped() {
            CanExecuteCommand::No(DenyExecutionReason::NotBootstrapped)
        } else if active_block.is_active_and_long_running()
            && !active_block.is_in_band_command_block()
        {
            CanExecuteCommand::No(DenyExecutionReason::ExistingActiveCommand)
        } else if !model.shared_session_status().is_executor()
            && active_block
                .session_id()
                .is_none_or(|session_id| !History::as_ref(ctx).is_appendable(&session_id))
        {
            CanExecuteCommand::No(DenyExecutionReason::HistoryNotAppendable)
        } else {
            CanExecuteCommand::Yes
        }
    }

    pub fn execute_pending_command(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.has_pending_command {
            return;
        }

        let command = self.get_command(ctx);
        if self.can_execute_command(ctx).is_no() {
            return;
        }

        self.try_execute_command(&command, ctx);
        self.has_pending_command = false;

        self.editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
        });
    }

    fn should_block_cloud_mode_setup_submission(&self, app: &AppContext) -> bool {
        if !FeatureFlag::CloudModeSetupV2.is_enabled() {
            return false;
        }

        self.ambient_agent_view_model()
            .is_some_and(|ambient_agent_model| {
                let ambient_agent_model = ambient_agent_model.as_ref(app);
                ambient_agent_model.is_ambient_agent()
                    && !ambient_agent_model.is_configuring_ambient_agent()
                    && !ambient_agent_model.is_agent_running()
            })
    }

    /// Try to execute a command in the local session that was
    /// requested by a shared session participant (sharer or viewer).
    ///
    /// Returns `true` if the command was executed, `false` otherwise.
    pub fn try_execute_command_on_behalf_of_shared_session_participant(
        &mut self,
        command: &str,
        participant_id: ParticipantId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Cancel any active agent conversation when the sharer executes a command on behalf of the viewer
        // (this is handled automatically when the sharer executes a command that they requested).
        // This will also notify viewers to cancel their representation of the conversation.
        let is_participant_viewer = self
            .shared_session_presence_manager
            .as_ref()
            .and_then(|pm| pm.as_ref(ctx).get_participant(&participant_id))
            .and_then(|participant| participant.role)
            .is_some();
        if FeatureFlag::AgentMode.is_enabled()
            && self.model.lock().shared_session_status().is_sharer()
            && is_participant_viewer
        {
            self.cancel_active_agent_conversation_for_shared_session(
                CancellationReason::UserCommandExecuted,
                ctx,
            );
        }

        let block_id = self.model.lock().block_list().active_block_id().clone();
        self.try_execute_command_from_source(
            command,
            CommandExecutionSource::SharedSession {
                participant_id,
                block_id,
                ai_metadata: None,
            },
            ctx,
        )
    }

    /// Freeze the editor and put it in a loading state.
    pub fn freeze_input_in_loading_state(&mut self, ctx: &mut ViewContext<Self>) -> String {
        self.editor.update(ctx, |editor, ctx| {
            // Use an ephemeral edit to show the loading state
            // and disallow edits.
            // TODO: the ◌ treatment is a stop-gap to rendering an svg
            // to the right of the buffer text.
            let buffer_text = editor.buffer_text(ctx);
            editor.set_buffer_text_ignoring_undo(&format!("{buffer_text} ◌"), ctx);
            editor.set_interaction_state(InteractionState::Selectable, ctx);

            // We manually set the text color to appear disabled.
            // We could use the [`InteractionState::Disabled`] interaction state
            // but that disallows text selection.
            let appearance = Appearance::as_ref(ctx);
            editor.set_text_colors(TextColors::all_hint_color(appearance), ctx);

            buffer_text
        })
    }

    pub fn try_execute_command(&mut self, command: &str, ctx: &mut ViewContext<Self>) -> bool {
        let shared_session_status = self.model.lock().shared_session_status().clone();
        if shared_session_status.is_sharer_or_viewer() {
            // If this is a viewer who isn't also an executor, they should not
            // be allowed to execute commands.
            if shared_session_status.is_reader() {
                // TODO: consider showing a toast in this scenario. It should be unlikely
                // that a viewer can get here without being an executor because the main
                // caller of this API is the `enter` handler.
                log::warn!("Viewer tried to execute a command as a reader");
                return false;
            } else if shared_session_status.is_executor() {
                let original_buffer = self.freeze_input_in_loading_state(ctx);

                if let Some(shared_session_input_state) = self.shared_session_input_state.as_mut() {
                    shared_session_input_state.pending_command_execution_request =
                        Some(ViewerCommandExecutionRequest { original_buffer });
                }
            }

            // Get our own shared session participant ID.
            let Some(participant_id) = self
                .shared_session_presence_manager
                .as_ref()
                .map(|m| m.as_ref(ctx).id())
            else {
                return false;
            };
            self.try_execute_command_on_behalf_of_shared_session_participant(
                command,
                participant_id,
                ctx,
            )
        } else {
            self.try_execute_command_from_source(command, CommandExecutionSource::User, ctx)
        }
    }

    /// Executes the given command if the terminal session is in a valid state to accept and
    /// execute a command. Afterwards, ensures the workflows info menu and input suggestions menu
    /// are both closed.
    ///
    /// This will _not_ execute a command if any of the following are true:
    ///     1. The history list and/or blocklist are not yet bootstrapped.
    ///     2. The active blocklist has not yet received the precmd payload.
    ///     3. There is an active, long-running command.
    ///
    /// Returns `true` if the command was executed, `false` otherwise.
    fn try_execute_command_from_source(
        &mut self,
        command: &str,
        source: CommandExecutionSource,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let CanExecuteCommand::No(reason) = self.can_execute_command(ctx) {
            if reason.is_existing_active_command() {
                const MAX_COMMAND_LENGTH: usize = 43;
                let truncated_command = truncate_from_end(command, MAX_COMMAND_LENGTH);

                // Block user submissions while a requested command is actively running
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(format!(
                            "Cannot run `{truncated_command}` (command already running)."
                        )),
                        window_id,
                        ctx,
                    );
                });
            }

            log::warn!("Tried to execute command but can_execute_command was false: {reason:?}");
            return false;
        }

        // Save the zero state next command state before clearing it.
        let zerostate_next_command_suggestion_info = self
            .next_command_model
            .as_ref(ctx)
            .get_zero_state_suggestion_info()
            .cloned();
        // Clear the auto-suggestion in the editor, so the height of
        // the input box is not inaccurate for its contents. Since we
        // we adjust the height of the long running block to be the same
        // as the height of the input box, we don't want the long
        // running block to have a lot of extra space for the frames
        // before it has any output or if it's a command that doesn't
        // have any output.
        //
        // Note that we do not clear the input box here (we do it in
        // `TerminalView` when we handle the `BlockCompleted` message
        // instead) for a similar reason. Specifically, we don't want
        // multi-line commands to have the height of the empty input
        // box because we don't want its contents to be cut off.
        //
        // If we had a zero-state autosuggestion and the user created an empty block,
        // keep the zero-state autosuggestion.
        if !command.is_empty() {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_autosuggestion(ctx);
                editor.clear_all_placeholder_text();
                ctx.notify();
            });
            self.next_command_model.update(ctx, |model, _| {
                model.clear_state();
            });
        }

        let home_dir = prompt::home_dir_for_block(
            self.model.lock().block_list().active_block(),
            self.sessions.as_ref(ctx),
        );
        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_home_dir(home_dir);

        let env_var_collection_id = self.env_var_collection_state.selected_env_vars;
        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_cloud_env_var_state(env_var_collection_id);

        // Record whether NLD was overridden (input type manually locked) at submission time.
        let nld_overridden = self.ai_input_model.as_ref(ctx).is_input_type_locked();
        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_nld_overridden(nld_overridden);

        let did_execute: bool;
        if self
            .model
            .lock()
            .block_list()
            .active_block()
            .has_received_precmd()
        {
            // Skip any empty blocks created by the user. Keep the last zero-state autosuggestion
            // until the user executes a command.
            if !command.is_empty() {
                if let Some(ZeroStateSuggestionInfo {
                    request,
                    response,
                    is_from_ai,
                    history_based_autosuggestion_state,
                    request_duration_ms,
                }) = zerostate_next_command_suggestion_info
                {
                    self.last_intelligent_autosuggestion_result =
                        Some(IntelligentAutosuggestionResult {
                            was_suggestion_accepted: self.was_intelligent_autosuggestion_accepted,
                            is_from_ai,
                            predicted_command: response.most_likely_action.clone(),
                        });

                    let should_collect_ugc = should_collect_ai_ugc_telemetry(
                        ctx,
                        PrivacySettings::as_ref(ctx).is_telemetry_enabled,
                    );
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModePrediction {
                            was_suggestion_accepted: self.was_intelligent_autosuggestion_accepted,
                            request_duration_ms,
                            is_from_ai,
                            does_actual_command_match_prediction: response.most_likely_action
                                == command,
                            does_actual_command_match_history_prediction:
                                history_based_autosuggestion_state.history_command_prediction
                                    == command,
                            history_prediction_likelihood: history_based_autosuggestion_state
                                .history_command_prediction_likelihood,
                            total_history_count: history_based_autosuggestion_state
                                .total_history_count,
                            actual_next_command_run: should_collect_ugc
                                .then_some(command.to_string()),
                            history_based_autosuggestion_state: should_collect_ugc
                                .then_some(history_based_autosuggestion_state.clone()),
                            generate_ai_input_suggestions_request: should_collect_ugc
                                .then_some(*request),
                            generate_ai_input_suggestions_response: should_collect_ugc
                                .then(|| response.clone())
                        },
                        ctx
                    );
                }
            }
            // Reset state for whether the user accepted the intelligent autosuggestion.
            self.was_intelligent_autosuggestion_accepted = false;

            self.tips_completed.update(ctx, |tips, ctx| {
                mark_feature_used_and_write_to_user_defaults(
                    Tip::Hint(TipHint::CreateBlock),
                    tips,
                    ctx,
                );
                ctx.notify();
            });

            if !command.is_empty() {
                IgnoredSuggestionsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.remove_ignored_suggestion(
                        command.to_string(),
                        SuggestionType::ShellCommand,
                        ctx,
                    );
                });
            }

            self.start_block_and_write_command_to_pty(command, source, ctx);
            did_execute = true;
        } else {
            // We don't want to submit the command if precmd has not
            // been received. Instead, we want the user to be aware
            // that the prompt might not be up to date.
            send_telemetry_from_ctx!(TelemetryEvent::TriedToExecuteBeforePrecmd, ctx);
            did_execute = false;
        }

        // Close the workflows info box if it was open.
        self.clear_selected_workflow(ctx);

        // Close the input suggestions menu if it was open.
        self.close_input_suggestions(/*should_focus_input=*/ false, ctx);
        did_execute
    }

    /// We locked the viewer's input when they attempted to execute a command.
    /// On failure, we must restore the editor to its original state before the attempt.
    pub fn on_execute_command_for_shared_session_participant_failure(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(shared_session_input_state) = self.shared_session_input_state.as_mut() else {
            return;
        };
        let Some(ViewerCommandExecutionRequest { original_buffer }) = shared_session_input_state
            .pending_command_execution_request
            .as_ref()
        else {
            return;
        };

        // Unfreeze the editor
        if let SharedSessionStatus::ActiveViewer { role } =
            self.model.lock().shared_session_status()
        {
            self.editor.update(ctx, |editor, ctx| {
                // Restore the original buffer and interaction state based on the viewer's role.
                editor.set_buffer_text(original_buffer, ctx);
                editor.set_interaction_state(role.into(), ctx);

                // Shared-session pending-command and cloud-followup flows can swap the editor into
                // a frozen/pending color treatment, so restore the normal palette alongside the
                // buffer + interaction state reset.
                let appearance: &Appearance = Appearance::as_ref(ctx);
                editor.set_text_colors(TextColors::from_appearance(appearance), ctx);
            });
        }
        shared_session_input_state.pending_command_execution_request = None;
    }

    /// This clears the loading state and input buffer for both the sharer and viewer
    /// once an agent request is in flight or cancelled.
    pub fn unfreeze_and_clear_agent_input(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(
            self.model.lock().shared_session_status(),
            SharedSessionStatus::ActiveViewer { .. } | SharedSessionStatus::ActiveSharer
        ) {
            self.editor.update(ctx, |editor, ctx| {
                // Reinitialize the buffer to properly clear it
                editor.reinitialize_buffer(None, ctx);

                if let SharedSessionStatus::ActiveViewer { role } =
                    self.model.lock().shared_session_status()
                {
                    // reinstate role for viewers
                    editor.set_interaction_state(role.into(), ctx);
                }

                let appearance: &Appearance = Appearance::as_ref(ctx);
                editor.set_text_colors(TextColors::from_appearance(appearance), ctx);
            });
        }
    }

    pub fn reset_after_cloud_followup_submission(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
            editor.clear_buffer_and_reset_undo_stack(ctx);

            let appearance: &Appearance = Appearance::as_ref(ctx);
            editor.set_text_colors(TextColors::from_appearance(appearance), ctx);
        });
    }

    /// Cancel any active agent conversation in a shared session
    /// and fan out a cancellation control action.
    pub(crate) fn cancel_active_agent_conversation_for_shared_session(
        &mut self,
        cancellation_reason: CancellationReason,
        ctx: &mut ViewContext<Self>,
    ) {
        let active_conversation =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation(self.terminal_view_id);

        if self.model.lock().shared_session_status().is_viewer() {
            let server_conversation_token = active_conversation
                .and_then(|conversation| conversation.server_conversation_token().cloned())
                .and_then(|server_token| {
                    server_token
                        .as_str()
                        .parse()
                        .ok()
                        .map(ServerConversationToken::from_uuid)
                });

            if let Some(server_conversation_token) = server_conversation_token {
                ctx.emit(Event::CancelSharedSessionConversation {
                    server_conversation_token,
                });
            }
        } else if self.model.lock().shared_session_status().is_sharer() {
            let active_conversation_id = active_conversation
                .filter(|conversation| conversation.status().is_in_progress())
                .map(|conversation| conversation.id());

            if let Some(active_conversation_id) = active_conversation_id {
                // First, cancel locally via the existing pipeline.
                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.cancel_conversation_progress(
                        active_conversation_id,
                        cancellation_reason,
                        ctx,
                    );
                });
            }
        }
    }

    fn clear_selected_env_var_collection(&mut self) {
        self.env_var_collection_state.selected_env_vars = None;
    }

    /// Closes the workflows panel.
    fn clear_selected_workflow(&mut self, ctx: &mut ViewContext<Self>) {
        // Clear the env var state if we had one.
        self.clear_selected_env_var_collection();

        // `take()` closes the Workflows panel because the panel is only
        // rendered if `selected_workflow_state` is Some(..).
        if let Some(state) = self.workflows_state.selected_workflow_state.take() {
            self.update_workflows_info_box_expanded_setting(ctx, &state);
        }
        ctx.notify();
    }

    /// Hides the workflows panel, persisting the shift-tab UX.
    fn hide_workflows_info_box(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(state) = &mut self.workflows_state.selected_workflow_state {
            state.should_show_more_info_view = false;
        }
        if let Some(state) = self.workflows_state.selected_workflow_state.clone() {
            self.update_workflows_info_box_expanded_setting(ctx, &state);
        }
        ctx.notify();
    }

    /// Returns the starting byte index position of the last selection.
    fn start_byte_index_of_last_selection(&self, ctx: &ViewContext<Self>) -> ByteOffset {
        self.editor
            .as_ref(ctx)
            .start_byte_index_of_last_selection(ctx)
    }

    fn handle_session_settings_event(
        &mut self,
        evt: &SessionSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match evt {
            SessionSettingsChangedEvent::HonorPS1 { .. } => {
                let mut model = self.model.lock();
                model.set_honor_ps1(*SessionSettings::as_ref(ctx).honor_ps1);
                ctx.notify();
            }
            SessionSettingsChangedEvent::SavedPrompt { .. } => {
                self.notify_and_notify_children(ctx);
            }
            _ => {}
        }
    }

    fn handle_app_editor_settings_event(
        &mut self,
        settings: ModelHandle<AppEditorSettings>,
        evt: &AppEditorSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let AppEditorSettingsChangedEvent::EnableAutosuggestions { .. } = evt {
            let next_enable_autosuggestions_setting =
                *AppEditorSettings::as_ref(ctx).enable_autosuggestions;
            if self.enable_autosuggestions_setting && !next_enable_autosuggestions_setting {
                // Clear the active autosuggestion if autosuggestions was turned off.
                self.editor.update(ctx, |view, ctx| {
                    view.clear_autosuggestion(ctx);
                });
                ctx.notify();
            }
            // Ensure our cached copy of the enabled_autosuggestions setting
            // is up-to-date.
            self.enable_autosuggestions_setting = next_enable_autosuggestions_setting;
        }

        // The cursor and status bar may change appearance when vim mode is enabled or disabled.
        if let AppEditorSettingsChangedEvent::VimModeEnabled { .. } = evt {
            ctx.notify();
        }

        if let AppEditorSettingsChangedEvent::CursorDisplayState { .. } = evt {
            ctx.notify();
        }

        // The vim status bar should be shown and hidden immediately upon toggling.
        if settings.as_ref(ctx).vim_mode_enabled() {
            if let AppEditorSettingsChangedEvent::VimStatusBar { .. } = evt {
                ctx.notify();
            }
        }
    }

    pub fn set_autosuggestion(
        &mut self,
        autosuggestion: impl Into<String>,
        autosuggestion_type: AutosuggestionType,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_autosuggestion(
                autosuggestion,
                AutosuggestionLocation::EndOfBuffer,
                autosuggestion_type,
                ctx,
            );
        })
    }

    fn handle_workflows_event(
        &mut self,
        event: &workflows::CategoriesViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            workflows::CategoriesViewEvent::Close => {
                self.focus_input_box(ctx);
                self.close_voltron(ctx);
            }
            workflows::CategoriesViewEvent::WorkflowSelected {
                workflow,
                workflow_source,
            } => {
                let workflow_id = workflow.server_id();
                let workflow_source = *workflow_source;
                let space = workflow_id.and_then(|id| {
                    CloudViewModel::as_ref(ctx)
                        .object_space(&id.to_string(), ctx)
                        .map(Into::into)
                });

                send_telemetry_from_ctx!(
                    TelemetryEvent::WorkflowSelected(WorkflowTelemetryMetadata {
                        workflow_source,
                        workflow_categories: workflow.as_workflow().tags().cloned(),
                        workflow_selection_source: WorkflowSelectionSource::Voltron,
                        workflow_id,
                        workflow_space: space,
                        enum_ids: workflow.as_workflow().get_server_enum_ids()
                    }),
                    ctx
                );

                self.show_workflows_info_box_on_workflow_selection(
                    *workflow.clone(),
                    workflow_source,
                    WorkflowSelectionSource::Voltron,
                    None,
                    ctx,
                );
                self.close_voltron(ctx);
            }
        }
    }

    fn handle_voltron_event(&mut self, event: &VoltronEvent, ctx: &mut ViewContext<Self>) {
        match event {
            VoltronEvent::Close => {
                self.close_voltron(ctx);
            }
        }
    }

    // Whether a workflow info box is open or not
    pub fn is_workflows_info_box_open(&self) -> bool {
        self.workflows_state.selected_workflow_state.is_some()
    }

    pub fn workflows_info_box_open_workflow_cloud_id(&self) -> Option<SyncId> {
        if let Some(state) = &self.workflows_state.selected_workflow_state {
            match &state.workflow_type {
                WorkflowType::Cloud(workflow) => Some(workflow.id),
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn show_workflows_info_box_on_workflow_selection(
        &mut self,
        workflow_type: WorkflowType,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_override: Option<HashMap<String, String>>,
        ctx: &mut ViewContext<Input>,
    ) {
        // Should not show workflows info box for read-only viewers
        let should_show_more_info_view = !self.model.lock().shared_session_status().is_reader();
        let env_vars = workflow_type.as_workflow().default_env_vars();
        self.insert_workflow_into_input(
            workflow_type,
            workflow_source,
            workflow_selection_source,
            argument_override,
            None,
            env_vars,
            should_show_more_info_view,
            ctx,
        );
    }

    pub fn show_workflow_info_box_for_history_command(
        &mut self,
        history_command: &str,
        workflow_type: WorkflowType,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        ctx: &mut ViewContext<Input>,
    ) {
        // Should not show workflows info box for read-only viewers
        let should_show_more_info_view = !self.model.lock().shared_session_status().is_reader();
        let env_vars = workflow_type.as_workflow().default_env_vars();
        self.insert_workflow_into_input(
            workflow_type,
            workflow_source,
            workflow_selection_source,
            None,
            Some(history_command),
            env_vars,
            should_show_more_info_view,
            ctx,
        );
    }

    /// Helper function to see if the selected history command matches the template of the workflow.
    fn command_matches_workflow_template(
        &self,
        history_command: &str,
        workflow_type: WorkflowType,
    ) -> CommandMatchesWorkflowTemplate {
        // if let Some(history_command) = history_command {
        if let Some(display_data) = compute_workflow_display_data_for_history_command(
            history_command,
            workflow_type.as_workflow(),
        ) {
            CommandMatchesWorkflowTemplate::Yes(display_data)
        } else {
            // In this case, the workflow comes from a history command but the command has been edited so
            // it no longer matches the original workflow template (e.g., a flag was added). We want
            // to treat this command as a workflow but without the argument parsing and shift-tab UX.
            CommandMatchesWorkflowTemplate::No
        }
    }

    /// Inserts the given workflow into the input editor and initiates the shift-tab workflow
    /// parameter editing "mode".
    ///
    /// If `should_show_more_info_view`, the `WorkflowsMoreInfoView` for the selected workflow is
    /// displayed above the input.
    ///
    /// If `history_command` is `Some()` _and_ matches the contained workflow in `workflow_type`,
    /// `history_command` is inserted into the input instead, with its parameters highlighted and
    /// made editable via the shift-tab UX.
    #[allow(clippy::too_many_arguments)]
    fn insert_workflow_into_input(
        &mut self,
        workflow_type: WorkflowType,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_overrides: Option<HashMap<String, String>>,
        history_command: Option<&str>,
        selected_env_vars: Option<SyncId>,
        should_show_more_info_view: bool,
        ctx: &mut ViewContext<Input>,
    ) {
        let input_type = if workflow_type.as_workflow().is_agent_mode_workflow() {
            InputType::AI
        } else {
            InputType::Shell
        };

        // Set input type based on whether or not this is a shell or AI workflow.
        self.ai_input_model.update(ctx, |input_model, ctx| {
            input_model.set_input_type(input_type, ctx);
        });

        // As the first step, clear the existing buffer so that selecting a workflow
        // is effectively a buffer replacement (not append).
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });

        if let Some(env_vars_command) = selected_env_vars
            .as_ref()
            .and_then(|id| self.env_vars_command_prefix(id, ctx))
        {
            self.editor.update(ctx, |editor, ctx| {
                editor.system_insert(
                    &env_vars_command,
                    PlainTextEditorViewAction::SystemInsert,
                    ctx,
                )
            });
        }

        // The workflow may or may not come from a history command. If it does, the history command may or may not match
        // the template of the original workflow. If it does match, we have extra display data to show (such as the indices in
        // the command to highlight as arguments). If it doesn't match, there's no additional display data to show. Then, in the
        // default case where there is no history command, there is additional display data.
        let (command_to_insert, display_data) = match history_command {
            Some(history_command) => {
                match self.command_matches_workflow_template(history_command, workflow_type.clone())
                {
                    CommandMatchesWorkflowTemplate::Yes(workflow_display_data) => (
                        workflow_display_data
                            .command_with_replaced_arguments
                            .clone(),
                        Some(workflow_display_data),
                    ),
                    CommandMatchesWorkflowTemplate::No => (history_command.to_string(), None),
                }
            }
            None => {
                let data = if let Some(arguments_to_override) = argument_overrides {
                    compute_workflow_display_data_with_overrides(
                        workflow_type.as_workflow(),
                        arguments_to_override,
                    )
                } else {
                    compute_workflow_display_data(workflow_type.as_workflow())
                };
                (data.command_with_replaced_arguments.clone(), Some(data))
            }
        };

        match display_data {
            Some(WorkflowDisplayData {
                command_with_replaced_arguments,
                replaced_ranges,
                argument_index_to_highlight_index_map,
                argument_index_to_object_id_map,
                ..
            }) => {
                let text_style_ranges = replaced_ranges
                    .into_iter()
                    .map(|range| {
                        (
                            range,
                            TextStyle::new().with_background_color(ColorU::from_u32(
                                WORKFLOW_PARAMETER_HIGHLIGHT_COLOR,
                            )),
                        )
                    })
                    .collect_vec();

                self.editor.update(ctx, |editor, ctx| {
                    editor.insert_with_styles(
                        &command_with_replaced_arguments,
                        &text_style_ranges,
                        PlainTextEditorViewAction::SystemInsert,
                        ctx,
                    );
                });

                // Get enum variants
                let cloud_model = CloudModel::as_ref(ctx);
                let enum_variants_map = argument_index_to_object_id_map
                    .iter()
                    .filter_map(|(index, object_id)| {
                        cloud_model
                            .get_workflow_enum(object_id)
                            .map(|workflow_enum| {
                                workflow_enum.model().string_model.variants.clone()
                            })
                            .map(|variants| (*index, variants))
                    })
                    .collect();

                self.workflows_state.selected_workflow_state = Some(SelectedWorkflowState {
                    more_info_view: self.create_workflows_info_view(
                        workflow_type.clone(),
                        true,
                        ctx,
                    ),
                    argument_index_to_highlight_index: argument_index_to_highlight_index_map,
                    argument_index_to_enum_variants: enum_variants_map,
                    workflow_source,
                    workflow_type,
                    workflow_selection_source,
                    should_show_more_info_view,
                });
            }
            None => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.user_initiated_insert(
                        &command_to_insert,
                        PlainTextEditorViewAction::SystemInsert,
                        ctx,
                    )
                });

                self.workflows_state.selected_workflow_state = Some(SelectedWorkflowState {
                    more_info_view: self.create_workflows_info_view(
                        workflow_type.clone(),
                        false,
                        ctx,
                    ),
                    argument_index_to_highlight_index: HashMap::new(),
                    argument_index_to_enum_variants: HashMap::new(),
                    workflow_source,
                    workflow_type,
                    workflow_selection_source,
                    should_show_more_info_view,
                });
            }
        };

        self.env_var_collection_state.selected_env_vars = selected_env_vars;

        // Ensure the env var selector dropdown is consistent with the selected env vars.
        if let Some(more_info_view) = self
            .workflows_state
            .selected_workflow_state
            .as_ref()
            .map(|state| &state.more_info_view)
        {
            more_info_view.update(ctx, |info_view, ctx| {
                info_view.set_environment_variables_selection(selected_env_vars, ctx);
            })
        }

        // Emit the a11y content as the last step so that it overwrites any of the a11y content
        // emitted by the editor (if multiple `AccessibilityContent`s are emitted within the same
        // event loop, the last one wins).
        let mut accessibility_text = format!("Workflow command {} inserted.", &command_to_insert);
        if let Some(a11y_content) = self.selected_workflow_a11y_text(ctx) {
            let _ = write!(accessibility_text, " {a11y_content}");
        }
        ctx.emit_a11y_content(AccessibilityContent::new(
            accessibility_text,
            "Press shift-tab to select the next workflow argument",
            WarpA11yRole::UserAction,
        ));

        // Only highlight an argument and show enum suggestions if history suggestions are not active
        if !matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::HistoryUp { .. } | InputSuggestionsMode::InlineHistoryMenu { .. },
        ) {
            self.highlight_selected_workflow_argument(
                self.get_text_style_ranges_for_workflow(ctx),
                ctx,
            );
        }
        self.focus_input_box(ctx);
    }

    /// Builds a prefix for applying env vars to a command in the current session.
    fn env_vars_command_prefix(&self, env_vars_id: &SyncId, ctx: &AppContext) -> Option<String> {
        let shell_type = self.active_session(ctx)?.shell().shell_type();
        let env_vars = &CloudModel::as_ref(ctx)
            .get_env_var_collection(env_vars_id)?
            .model()
            .string_model;

        if shell_type == ShellType::Fish {
            // Warp currently doesn't support newlines in Fish, just prepend the vars
            let mut command = env_vars.export_variables_for_shell(ShellType::Fish);
            command.push(' ');
            Some(command)
        } else {
            // Add newlines at the end to separate the vars from the comment/command
            Some(format!(
                "# Environment variables\n{}\n\n",
                env_vars.export_variables(" ", shell_type.into())
            ))
        }
    }

    fn create_workflows_info_view(
        &mut self,
        workflow: WorkflowType,
        show_shift_tab_treatment: bool,
        ctx: &mut ViewContext<Input>,
    ) -> ViewHandle<WorkflowsMoreInfoView> {
        let workflow_more_info_view = ctx.add_typed_action_view(|ctx| {
            WorkflowsMoreInfoView::new(
                *InputSettings::as_ref(ctx).workflows_box_expanded.value(),
                workflow,
                show_shift_tab_treatment,
                ctx,
            )
        });

        ctx.subscribe_to_view(&workflow_more_info_view, move |me, _, event, ctx| {
            me.handle_workflow_more_info_event(event, ctx);
        });

        workflow_more_info_view
    }

    fn handle_workflow_more_info_event(
        &mut self,
        event: &WorkflowsInfoBoxViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WorkflowsInfoBoxViewEvent::PrefixCommandWithEnvironmentVariables(env_vars) => {
                self.reset_workflow_state(*env_vars, ctx);

                // The ID may be `None` if the user is *clearing* environment variables.
                if let Some(env_vars_id) = env_vars {
                    let env_vars_object =
                        CloudModel::as_ref(ctx).get_env_var_collection(env_vars_id);
                    let telemetry_metadata = EnvVarTelemetryMetadata {
                        object_id: env_vars_id.into_server().map(Into::into),
                        team_uid: env_vars_object
                            .and_then(|object| object.permissions.owner.into()),
                        space: env_vars_object
                            .map_or(Space::Personal, |object| object.space(ctx))
                            .into(),
                    };
                    send_telemetry_from_ctx!(
                        TelemetryEvent::EnvVarWorkflowParameterization(telemetry_metadata),
                        ctx
                    );
                }
            }
        }
    }

    /// Returns the a11y text for a workflow that is selected. `None`, if there is no workflow
    /// selected.
    fn selected_workflow_a11y_text(&self, ctx: &mut ViewContext<Self>) -> Option<String> {
        self.workflows_state
            .selected_workflow_state
            .as_ref()
            .and_then(|selected_workflow_state| {
                selected_workflow_state.more_info_view.read(ctx, |view, _| {
                    view.selected_argument()
                        .map(|argument| format!("Selected Workflow argument {}", argument.name()))
                })
            })
    }

    fn workflow_arg_was_deleted(
        &self,
        text_style_run_count: usize,
        argument_index_to_highlight_index: &HashMap<WorkflowArgumentIndex, Vec<usize>>,
    ) -> bool {
        let expected_run_count: usize = argument_index_to_highlight_index
            .values()
            .map(|indices| indices.len())
            .sum();
        text_style_run_count != expected_run_count
    }

    fn get_text_style_ranges_for_workflow(
        &self,
        ctx: &ViewContext<Self>,
    ) -> Vec<Range<ByteOffset>> {
        let text_style_runs: Vec<_> = self
            .editor
            .as_ref(ctx)
            .text_style_runs(ctx)
            .filter(|style_run| style_run.text_style().background_color.is_some())
            .collect();
        self.build_text_run_ranges_for_workflows(&text_style_runs)
    }

    /// We are currently using the styling of text runs in the input as a way of tracking
    /// where our workflow arguments are.
    /// This doesn't work in 2 cases:
    ///
    /// 1. When part of a workflow argument is subject to syntax highlighting, it breaks
    ///    a run into one or more runs. Example: "--env JOB_EXECUTION_MODE=REAL" will wind
    ///    up with syntax highlighting on `--env`, resulting in 2 runs.
    /// 2. When workflow arguments directly follow each other with no spacing, they will
    ///    both be covered by a single run.. Example: {a}{b}{c} will only get a single
    ///    run covering "abc"
    ///
    /// This helper acts as a quick hack to address the first issue:
    /// if two background-highlighted runs are contiguous, they are merged into a single run.
    /// This is a short-term fix and should be addressed in a more comprehensive way that does
    /// not rely on the styling of the input.
    ///
    /// See [CLD-997](https://linear.app/warpdotdev/issue/CLD-997)
    fn build_text_run_ranges_for_workflows(
        &self,
        text_style_runs: &[TextRun],
    ) -> Vec<Range<ByteOffset>> {
        let mut ranges = text_style_runs
            .iter()
            .map(|style_run| style_run.byte_range().clone())
            .collect::<Vec<_>>();
        ranges.sort_by(|a, b| a.start.cmp(&b.start));

        let capacity = ranges.len();

        ranges.into_iter().fold(
            Vec::<Range<ByteOffset>>::with_capacity(capacity),
            |mut acc: Vec<Range<ByteOffset>>, next| -> Vec<Range<ByteOffset>> {
                match acc.last() {
                    Some(current) if current.end >= next.start => {
                        let new_range = std::cmp::min(current.start, next.start)
                            ..std::cmp::max(current.end, next.end);
                        acc.pop();
                        acc.push(new_range);
                    }
                    _ => {
                        acc.push(next);
                    }
                };
                acc
            },
        )
    }

    /// Highlight the currently selected workflow argument and open the enum suggestions menu if applicable.
    /// Takes in `text_style_ranges`, which contains ByteOffset Ranges of arguments in the input editor.
    fn highlight_selected_workflow_argument(
        &mut self,
        text_style_ranges: Vec<Range<ByteOffset>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut variants = None;
        let mut selected_ranges = Vec::new();

        if let Some(active_workflow_state) = self.workflows_state.selected_workflow_state.as_ref() {
            active_workflow_state
                .more_info_view
                .update(ctx, |workflows_info_view, ctx| {
                    let selected_workflow_state = &mut workflows_info_view.selected_workflow_state;
                    // Update the editor given what the currently selected argument index is
                    self.editor.update(ctx, |editor, ctx| {
                        // If an argument has been completely deleted - pause the shift-tab cycling
                        if self.workflow_arg_was_deleted(
                            text_style_ranges.len(),
                            &active_workflow_state.argument_index_to_highlight_index,
                        ) {
                            selected_workflow_state.set_argument_cycling_enabled(false);
                        } else {
                            variants = active_workflow_state
                                .argument_index_to_enum_variants
                                .get(&selected_workflow_state.currently_selected_argument());

                            selected_workflow_state.set_argument_cycling_enabled(true);
                            // Get all of the highlighted ranges for the currently selected argument.
                            let byte_ranges = active_workflow_state
                                .argument_index_to_highlight_index
                                .get(&selected_workflow_state.currently_selected_argument())
                                .map(|indices| {
                                    indices
                                        .iter()
                                        .filter_map(|index| text_style_ranges.get(*index).cloned())
                                });

                            if let Some(byte_ranges) = byte_ranges {
                                selected_ranges = byte_ranges.clone().collect();
                                editor.select_ranges_by_byte_offset(byte_ranges, ctx);
                            }
                        }
                    });
                });
        }

        if let Some(enum_variants) = variants {
            self.populate_enum_suggestions_menu(enum_variants.clone(), selected_ranges, ctx);
        } else {
            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_mode(InputSuggestionsMode::Closed, ctx);
            });
        }
        ctx.notify();
    }

    fn populate_enum_suggestions_menu(
        &mut self,
        enum_variants: EnumVariants,
        selected_ranges: Vec<Range<ByteOffset>>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the newly highlighted argument has enum variants, populate the suggestions menu
        let position = self.editor.as_ref(ctx).first_selection_end_to_point(ctx);

        self.editor.update(ctx, |editor, ctx| {
            editor.cache_buffer_point(
                position,
                COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID,
                ctx,
            );
        });

        let variants = match enum_variants {
            EnumVariants::Static(variants) => {
                self.suggestions_mode_model.update(ctx, |m, ctx| {
                    m.set_mode(
                        InputSuggestionsMode::StaticWorkflowEnumSuggestions {
                            suggestions: variants.clone(),
                            menu_position: TabCompletionsMenuPosition::AtFirstCursor,
                            selected_ranges,
                            cursor_point: position,
                        },
                        ctx,
                    );
                });
                variants
            }
            EnumVariants::Dynamic(command) => {
                if FeatureFlag::DynamicWorkflowEnums.is_enabled() {
                    self.suggestions_mode_model.update(ctx, |m, ctx| {
                        m.set_mode(
                            InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                                suggestions: vec![],
                                menu_position: TabCompletionsMenuPosition::AtFirstCursor,
                                selected_ranges,
                                cursor_point: position,
                                dynamic_enum_status: DynamicEnumSuggestionStatus::Unapproved,
                                command,
                            },
                            ctx,
                        );
                    });
                }
                vec![]
            }
        };

        self.input_suggestions.update(ctx, |input, ctx| {
            input.set_enum_variants(variants, ctx);
        });

        ctx.notify();
    }

    fn handle_suggestions_event(
        &mut self,
        event: &InputSuggestionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.suggestions_mode_model.as_ref(ctx).is_visible() {
            return;
        }

        match event {
            InputSuggestionsEvent::ConfirmSuggestion {
                suggestion,
                match_type,
            } => {
                if !self.confirm_suggestion(suggestion, ctx) {
                    return;
                }

                send_telemetry_from_ctx!(
                    TelemetryEvent::ConfirmSuggestion {
                        mode: self
                            .suggestions_mode_model
                            .as_ref(ctx)
                            .mode()
                            .to_telemetry_mode(),
                        match_type: *match_type,
                    },
                    ctx
                );
                self.close_input_suggestions(/*should_focus_input=*/ true, ctx);
            }
            InputSuggestionsEvent::ConfirmAndExecuteSuggestion {
                suggestion,
                match_type,
            } => {
                if !self.confirm_and_execute_suggestion(suggestion, ctx) {
                    return;
                }

                send_telemetry_from_ctx!(
                    TelemetryEvent::ConfirmSuggestion {
                        mode: self
                            .suggestions_mode_model
                            .as_ref(ctx)
                            .mode()
                            .to_telemetry_mode(),
                        match_type: *match_type,
                    },
                    ctx
                );

                self.close_input_suggestions(/*should_focus_input=*/ true, ctx);

                let command = self.get_command(ctx);
                self.try_execute_command(&command, ctx);

                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    format!("Executed: {command}"),
                    WarpA11yRole::UserAction,
                ));
            }
            InputSuggestionsEvent::CloseSuggestion {
                should_restore_buffer_before_history_up,
            } => {
                self.close_input_suggestions_and_restore_buffer(
                    true,
                    *should_restore_buffer_before_history_up,
                    ctx,
                );
            }
            InputSuggestionsEvent::Select(selected_item) => {
                let mode = self.suggestions_mode_model.as_ref(ctx).mode().clone();
                match &mode {
                    InputSuggestionsMode::HistoryUp { .. } => {
                        if let Some((workflow_type, workflow_source)) = selected_item
                            .linked_workflow_data()
                            .and_then(|linked_workflow_data| {
                                linked_workflow_data.linked_workflow(ctx)
                            })
                        {
                            // TODO(ben): We should include the chosen env vars in the history
                            // entry.
                            let env_vars = workflow_type.as_workflow().default_env_vars();
                            self.insert_workflow_into_input(
                                workflow_type,
                                workflow_source,
                                WorkflowSelectionSource::UpArrowHistory,
                                None,
                                Some(selected_item.text()),
                                env_vars,
                                /*should_show_more_info_view=*/ false,
                                ctx,
                            );
                        } else {
                            self.editor.update(ctx, |editor, ctx| {
                                editor.set_buffer_text_ignoring_undo(selected_item.text(), ctx);
                            });
                        }

                        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                            let input_type = if selected_item.is_ai_query() {
                                InputType::AI
                            } else {
                                InputType::Shell
                            };
                            ai_input_model.set_input_type(input_type, ctx);
                        });
                    }
                    InputSuggestionsMode::CompletionSuggestions {
                        replacement_start, ..
                    } => {
                        let replacement_start = *replacement_start;
                        if self.is_classic_completions_enabled(ctx) {
                            self.editor.update(ctx, |editor, ctx| {
                                let cursor_end_offset =
                                    editor.end_byte_index_of_last_selection(ctx);
                                editor.select_and_replace(
                                    selected_item.text(),
                                    [ByteOffset::from(replacement_start)..cursor_end_offset],
                                    PlainTextEditorViewAction::CycleCompletionSuggestion,
                                    ctx,
                                );
                                ctx.notify();
                            });
                            ctx.notify();
                        }
                    }
                    InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
                    | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. } => {
                        // If in the future we want to replace the selected arguments with suggestion options as we cycle, this is where we do it
                    }
                    InputSuggestionsMode::AIContextMenu { .. } => {
                        // AI context menu selection is handled separately
                        // This shouldn't be reached since AI context menu doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::SlashCommands => {
                        // Slash commands selection is handled separately
                        // This shouldn't be reached since slash commands doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::ConversationMenu => {
                        // Conversation menu selection is handled separately
                        // This shouldn't be reached since conversation menu doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::ModelSelector => {
                        // Model selector selection is handled separately
                        // This shouldn't be reached since model selector doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::ProfileSelector => {
                        // Profile selector selection is handled separately.
                        // This shouldn't be reached since profile selector doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::PromptsMenu => {
                        // Prompts menu selection is handled via InlinePromptsMenuView
                    }
                    InputSuggestionsMode::SkillMenu => {
                        // Skill menu selection is handled via InlineSkillSelectorView
                    }
                    InputSuggestionsMode::UserQueryMenu { .. } => {
                        // User query menu selection is handled separately
                    }
                    InputSuggestionsMode::InlineHistoryMenu { .. } => {
                        // Inline history menu selection is handled separately
                        // This shouldn't be reached since inline history menu doesn't use InputSuggestions
                    }
                    InputSuggestionsMode::IndexedReposMenu => {
                        // Repos menu selection is handled separately
                    }
                    InputSuggestionsMode::PlanMenu { .. } => {
                        // Plan menu selection is handled via InlinePlanMenuView
                    }
                    InputSuggestionsMode::Closed => {
                        log::warn!("Got a InputSuggestionsEvent::Select when the mode was Closed!");
                    }
                }
            }
            InputSuggestionsEvent::IgnoreItem { item } => {
                let command_text = item.text();
                let suggestion_type = if item.is_ai_query() {
                    SuggestionType::AIQuery
                } else {
                    SuggestionType::ShellCommand
                };

                IgnoredSuggestionsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.add_ignored_suggestion(command_text.to_string(), suggestion_type, ctx);
                });

                // Refresh the history suggestions menu and keep it open
                if matches!(
                    self.suggestions_mode_model.as_ref(ctx).mode(),
                    InputSuggestionsMode::HistoryUp { .. }
                ) {
                    let history = if self.model.lock().shared_session_status().is_executor() {
                        self.shared_session_history(ctx)
                    } else {
                        self.collate_ai_and_command_history(ctx)
                    };
                    let original_buffer = if let InputSuggestionsMode::HistoryUp {
                        original_buffer,
                        ..
                    } = self.suggestions_mode_model.as_ref(ctx).mode()
                    {
                        original_buffer.clone()
                    } else {
                        String::new()
                    };

                    let matches =
                        InputSuggestions::history_prefix_search(&original_buffer, history);
                    self.input_suggestions
                        .update(ctx, move |input_suggestions, ctx| {
                            input_suggestions.set_history_matches(matches, ctx);
                        });
                }
            }
        }
    }

    /// Resets the SelectedWorkflowState back to the original workflow, with its original arguments. This
    /// is useful when the command does not match the original workflow.
    fn reset_workflow_state(&mut self, env_vars: Option<SyncId>, ctx: &mut ViewContext<Input>) {
        // We want to also initially clear the stored selected env var.
        self.clear_selected_env_var_collection();

        if let Some(state) = self.workflows_state.selected_workflow_state.take() {
            self.insert_workflow_into_input(
                state.workflow_type,
                state.workflow_source,
                state.workflow_selection_source,
                None,
                None,
                env_vars,
                true,
                ctx,
            )
        }

        ctx.notify();
    }

    fn confirm_suggestion(&mut self, suggestion: &str, ctx: &mut ViewContext<Input>) -> bool {
        self.confirm_suggestion_internal(suggestion, Executing::No, ctx)
    }

    fn confirm_and_execute_suggestion(
        &mut self,
        suggestion: &str,
        ctx: &mut ViewContext<Input>,
    ) -> bool {
        self.confirm_suggestion_internal(suggestion, Executing::Yes, ctx)
    }

    /// Handles suggestion confirmation behaviour in editor and returns true if suggestions menu should be closed
    /// For CompletionSuggestions, inserts suggestion into editor. For HistoryUp, no action since "select" populates buffer.
    /// Closed branch should never be executed (does not use the input suggestions panel).
    fn confirm_suggestion_internal(
        &mut self,
        suggestion: &str,
        executing: Executing,
        ctx: &mut ViewContext<Input>,
    ) -> bool {
        match self.suggestions_mode_model.as_ref(ctx).mode() {
            InputSuggestionsMode::Closed => false,
            InputSuggestionsMode::HistoryUp { .. } => true,
            InputSuggestionsMode::CompletionSuggestions {
                replacement_start, ..
            } => {
                self.insert_completion_result_into_editor(
                    suggestion,
                    *replacement_start,
                    executing,
                    ctx,
                );
                true
            }
            InputSuggestionsMode::StaticWorkflowEnumSuggestions {
                selected_ranges, ..
            }
            | InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                selected_ranges, ..
            } => {
                let selected_ranges = selected_ranges.clone();
                self.editor.update(ctx, |editor, ctx| {
                    editor.select_and_replace(
                        suggestion,
                        selected_ranges.iter().cloned(),
                        PlainTextEditorViewAction::AcceptCompletionSuggestion,
                        ctx,
                    );
                });
                true
            }
            InputSuggestionsMode::AIContextMenu { .. } => {
                // AI context menu selection is handled separately
                // For now, just close the menu
                false
            }
            InputSuggestionsMode::SlashCommands => {
                // Slash commands selection is handled separately
                // For now, just close the menu
                false
            }
            InputSuggestionsMode::ConversationMenu => {
                // Conversation menu selection is handled separately
                false
            }
            InputSuggestionsMode::ModelSelector => {
                // Model selector selection is handled separately
                false
            }
            InputSuggestionsMode::ProfileSelector => {
                // Profile selector selection is handled separately
                false
            }
            InputSuggestionsMode::PromptsMenu => {
                // Prompts menu selection is handled separately
                false
            }
            InputSuggestionsMode::SkillMenu => {
                // Skill menu selection is handled via InlineSkillSelectorView
                false
            }
            InputSuggestionsMode::UserQueryMenu { .. } => {
                // User query menu selection is handled separately
                false
            }
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                // Inline history menu selection is handled separately
                false
            }
            InputSuggestionsMode::IndexedReposMenu => {
                // Repos menu selection is handled separately
                false
            }
            InputSuggestionsMode::PlanMenu { .. } => {
                // Plan menu selection is handled via InlinePlanMenuView
                false
            }
        }
    }

    pub fn close_input_suggestions_and_restore_buffer(
        &mut self,
        should_focus_input: bool,
        should_restore_buffer_before_history_up: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if should_restore_buffer_before_history_up {
            if let InputSuggestionsMode::HistoryUp {
                original_buffer,
                original_cursor_point,
                original_input_was_locked,
                original_input_type,
                ..
            } = self.suggestions_mode_model.as_ref(ctx).mode()
            {
                let original_buffer = original_buffer.clone();
                let original_cursor_point = *original_cursor_point;
                let original_input_was_locked = *original_input_was_locked;
                let original_input_type = *original_input_type;
                // If the user closes the input suggestions menu, we want to reset the AI input mode
                // to the exact same state it was originally, which includes the mode itself and
                // whether it was locked to that mode.
                self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                    ai_input_model.set_input_config(
                        InputConfig {
                            input_type: original_input_type,
                            is_locked: original_input_was_locked,
                        },
                        original_buffer.is_empty(),
                        ctx,
                    );
                });

                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text_ignoring_undo(&original_buffer, ctx);
                    if let Some(original_cursor_point) = original_cursor_point {
                        editor.reset_selections_to_point(&original_cursor_point, ctx);
                    }
                });
            }
        }
        self.close_input_suggestions(/*should_focus_input=*/ should_focus_input, ctx);
    }

    pub fn close_input_suggestions(
        &mut self,
        should_focus_input: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the input suggestions view is already closed, don't refocus the input box.
        if !self.suggestions_mode_model.as_ref(ctx).is_closed() {
            let was_inline_menu_open = self
                .suggestions_mode_model
                .as_ref(ctx)
                .is_inline_menu_open();

            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_mode(InputSuggestionsMode::Closed, ctx);
            });

            // If we're closing an inline menu, trigger autodetection on the buffer contents
            if was_inline_menu_open {
                self.run_input_background_jobs(
                    InputBackgroundJobOptions::default().with_ai_input_detection(),
                    ctx,
                );
            }

            if should_focus_input {
                self.focus_input_box(ctx);
                self.maybe_generate_autosuggestion(ctx);
            } else {
                ctx.notify();
            }
        }
    }

    pub fn clear_buffer_and_reset_undo_stack(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_cached_hint_text();
        self.exit_cloud_handoff_compose(ctx);
        self.editor.update(ctx, |view, ctx| {
            view.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    pub fn replace_buffer_content(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |view, ctx| {
            view.set_buffer_text(content, ctx);
        });
    }

    // Fill the input buffer with the provided text and auto-select all of the text
    // (so that it's easy to delete).
    pub fn prefill_buffer_and_select_all(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }

        self.editor.update(ctx, |editor, ctx| {
            editor.clear_autosuggestion(ctx);
            editor.set_buffer_text_ignoring_undo(content, ctx);
            editor.handle_action(&EditorAction::SelectAll, ctx);
        });
    }

    /// Appends text to the current buffer at the cursor position, preserving existing buffer content.
    pub fn append_to_buffer(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        self.system_insert(content, ctx);
    }

    pub fn insert_typeahead_text(
        &mut self,
        num_typeahead_chars_inserted: CharOffset,
        typeahead: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |view, ctx| {
            view.replace_first_n_characters(num_typeahead_chars_inserted, typeahead, ctx);
            view.move_to_buffer_end(ctx);
        });
    }

    pub fn focus_input_box(&self, ctx: &mut ViewContext<Self>) {
        if self.should_show_auth_secret_ftux(ctx) {
            if let Some(ftux_view) = self.auth_secret_ftux_view().cloned() {
                ftux_view.update(ctx, |view, ctx| {
                    view.focus_dropdown_editor(ctx);
                });
                return;
            }
        }
        ctx.focus_self();
    }

    pub fn input_type(&self, app: &AppContext) -> InputType {
        self.ai_input_model.as_ref(app).input_type()
    }

    pub fn handle_command_search_closed(
        &mut self,
        query_when_closed: &str,
        filter_when_closed: &Option<QueryFilter>,
        ctx: &mut ViewContext<Self>,
    ) {
        // We want to restore / preserve the buffer as follows when the buffer text is "#":
        // - if command search was "#" when closed, keep the "#" in the buffer
        //   because the user probably wanted "#" without command search.
        // - if command search was "#: some_query" when closed, clear the buffer
        //   because the user probably got their answer from ai command search.
        // - if command search was empty when closed, clear the buffer
        //   because the user probably backspace'd out of "#" and then hit escape.
        let is_command_search_empty =
            filter_when_closed.is_none() && query_when_closed.trim().is_empty();
        let was_non_empty_ai_command_search =
            matches!(filter_when_closed, Some(QueryFilter::NaturalLanguage))
                && !query_when_closed.trim().is_empty();
        let was_triggered_by_hashtag = self.buffer_text(ctx).trim() == AI_COMMAND_SEARCH_TRIGGER;

        if (is_command_search_empty || was_non_empty_ai_command_search) && was_triggered_by_hashtag
        {
            self.editor().update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            });
        }
        self.focus_input_box(ctx);
    }

    /// Close all overlays managed by the input view. Does not change what is focused.
    /// If should_restore_buffer_before_history_up is true, the buffer will be restored to the state it was in before the history up menu was opened.
    pub fn close_overlays(
        &mut self,
        should_restore_buffer_before_history_up: bool,
        ctx: &mut ViewContext<Input>,
    ) {
        self.close_voltron(ctx);
        self.close_input_suggestions_and_restore_buffer(
            false,
            should_restore_buffer_before_history_up,
            ctx,
        );
        self.clear_selected_workflow(ctx);
    }

    /// Closes any active suggestion mode UI when starting a new conversation.
    ///
    /// This is intentionally narrower than `close_overlays`: it does not close Voltron, workflow
    /// info overlays, etc.
    fn close_suggestion_modes_for_new_conversation(&mut self, ctx: &mut ViewContext<Self>) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::Closed, ctx);
        });
    }

    fn close_voltron(&mut self, ctx: &mut ViewContext<Input>) {
        self.is_voltron_open = false;
        ctx.notify();
    }

    fn editor_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_show_auth_secret_ftux(ctx) {
            if let Some(ftux_view) = self.auth_secret_ftux_view().cloned() {
                ftux_view.update(ctx, |view, ctx| {
                    view.select_previous_in_dropdown(ctx);
                });
            }
            return;
        }

        if let Some(selector) = self.auth_secret_selector() {
            if selector.as_ref(ctx).is_menu_open() {
                let selector = selector.clone();
                selector.update(ctx, |selector, ctx| {
                    selector.select_previous(ctx);
                });
                return;
            }
        }

        // History and input suggestions are not available for
        // read-only viewers in a shared session
        if self.model.lock().shared_session_status().is_reader() {
            return;
        }

        // For some input suggestion modes, the menu handles its own actions.
        let handled = match self.suggestions_mode_model.as_ref(ctx).mode() {
            InputSuggestionsMode::AIContextMenu { .. } => {
                self.editor.update(ctx, |editor, ctx| {
                    if let Some(ai_context_menu) = editor.ai_context_menu() {
                        ai_context_menu.update(ctx, |menu, ctx| {
                            menu.handle_action(&AIContextMenuAction::Prev, ctx);
                        });
                    }
                });
                true
            }
            InputSuggestionsMode::SlashCommands => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
                        view.update(ctx, |view, ctx| {
                            view.select_up(ctx);
                        });
                    }
                } else {
                    self.inline_slash_commands_view.update(ctx, |view, ctx| {
                        view.select_up(ctx);
                    });
                }
                true
            }
            InputSuggestionsMode::ConversationMenu => {
                self.inline_conversation_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::ForkFrom,
                ..
            } => {
                self.user_query_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::Rewind,
                ..
            } => {
                self.rewind_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::ModelSelector => {
                self.inline_model_selector_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::ProfileSelector => {
                self.inline_profile_selector_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::PromptsMenu => {
                self.inline_prompts_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::SkillMenu => {
                self.inline_skill_selector_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    if let Some(view) = self.cloud_mode_v2_history_menu_view.clone() {
                        view.update(ctx, |view, ctx| {
                            view.select_up(ctx);
                        });
                    }
                } else {
                    self.inline_history_menu_view.update(ctx, |view, ctx| {
                        view.select_up(ctx);
                    });
                }
                true
            }
            InputSuggestionsMode::IndexedReposMenu => {
                self.inline_repos_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::PlanMenu { .. } => {
                self.inline_plan_menu_view.update(ctx, |view, ctx| {
                    view.select_up(ctx);
                });
                true
            }
            InputSuggestionsMode::HistoryUp { .. }
            | InputSuggestionsMode::CompletionSuggestions { .. }
            | InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::Closed => false,
        };

        if handled {
            return;
        }

        // If the input suggestions menu is open, always cycle to the next option.
        if self.suggestions_mode_model.as_ref(ctx).is_visible() && self.can_query_history(ctx) {
            self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.select_prev(ctx);
            });
            return;
        }

        // Otherwise, check if the cursor is on the first row and open the
        // history up menu.
        let editor = self.editor.as_ref(ctx);
        if editor.single_cursor_on_first_row(ctx) {
            if FeatureFlag::InlineHistoryMenu.is_enabled()
                && self.suggestions_mode_model.as_ref(ctx).is_closed()
            {
                self.open_inline_history_menu(ctx);
                return;
            }

            let history = if self.model.lock().shared_session_status().is_executor() {
                self.shared_session_history(ctx)
            } else {
                self.collate_ai_and_command_history(ctx)
            };
            let original_buffer = self.editor.as_ref(ctx).buffer_text(ctx);

            let matches = InputSuggestions::history_prefix_search(&original_buffer, history);
            self.input_suggestions
                .update(ctx, move |input_suggestions, ctx| {
                    input_suggestions.set_history_matches(matches, ctx);
                });

            let original_cursor_point = self.editor.as_ref(ctx).single_cursor_to_point(ctx);
            let original_input_type = self.ai_input_model.as_ref(ctx).input_type();
            let original_input_was_locked = self.ai_input_model.as_ref(ctx).is_input_type_locked();
            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_mode(
                    InputSuggestionsMode::HistoryUp {
                        original_buffer,
                        original_cursor_point,
                        search_mode: HistorySearchMode::Prefix,
                        original_input_type,
                        original_input_was_locked,
                    },
                    ctx,
                );
            });

            send_telemetry_from_ctx!(
                TelemetryEvent::OpenSuggestionsMenu(
                    self.suggestions_mode_model
                        .as_ref(ctx)
                        .mode()
                        .to_telemetry_mode(),
                ),
                ctx
            );
            ctx.notify();
            return;
        }
        // Finally, if we're neither scrolling through an existing suggestion
        // list nor entering the history mode, we move the cursor up.
        self.editor.update(ctx, |input, ctx| input.move_up(ctx));
    }

    // TODO - Implement PageUp functionality for input suggestions menu
    fn editor_page_up(&mut self, ctx: &mut ViewContext<Self>) {
        let event = self.editor.read(ctx, |editor, ctx| {
            TelemetryEvent::PageUpDownInEditorPressed {
                is_empty_editor: editor.is_empty(ctx),
                is_down: false,
            }
        });
        send_telemetry_from_ctx!(event, ctx);
        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.editor
                .update(ctx, |input, ctx| input.move_page_up(ctx));
        } else {
            ctx.emit(Event::PageUp);
        }
    }

    /// Asks the currently active inline menu whether the buffer should be restored on dismiss
    /// (defaulting to true for any inline menus that don't have specific behavior requirements for this decision).
    fn should_restore_buffer_on_inline_menu_dismiss(&self, ctx: &ViewContext<Self>) -> bool {
        match self.suggestions_mode_model.as_ref(ctx).mode() {
            // If the input is not being used as a search on the model menu
            // we should not restore/revert the changes to the input on-dismiss.
            InputSuggestionsMode::ModelSelector => self
                .inline_model_selector_view
                .as_ref(ctx)
                .filter_results_by_input(),
            _ => true,
        }
    }

    fn editor_escape(&mut self, ctx: &mut ViewContext<Self>) {
        let vim_mode = self.editor.as_ref(ctx).vim_mode(ctx);
        let has_attached_context = {
            let context_model = self.ai_context_model.as_ref(ctx);
            !context_model.pending_context_block_ids().is_empty()
                || context_model.pending_context_selected_text().is_some()
        };
        let should_escape_vim_before_dismissing = (vim_mode == Some(VimMode::Insert)
            && (self.suggestions_mode_model.as_ref(ctx).is_history_up()
                || self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_history_menu()))
            || (vim_mode == Some(VimMode::Insert)
                && self.prefix_mode(ctx) == InputPrefixMode::CloudHandoff);

        if should_escape_vim_before_dismissing {
            self.editor.update(ctx, |editor, editor_ctx| {
                editor.handle_action(&EditorAction::VimEscape, editor_ctx);
            });
        } else if self.suggestions_mode_model.as_ref(ctx).is_ai_context_menu() {
            // Handle AI context menu escape specifically to ensure proper state reset
            self.close_ai_context_menu(ctx);
        } else if self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
            if self.maybe_clear_v2_slash_section_filter(ctx) {
                return;
            }
            self.slash_command_model
                .update(ctx, |model, ctx| model.disable(ctx));
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
            ctx.notify();
        } else if self.prefix_mode(ctx) == InputPrefixMode::CloudHandoff {
            self.exit_cloud_handoff_compose(ctx);
            ctx.notify();
        } else if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_menu_open()
        {
            if self.should_restore_buffer_on_inline_menu_dismiss(ctx) {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.close_and_restore_buffer(ctx);
                });
            } else {
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
            }
            ctx.notify();
        } else if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.input_suggestions
                .update(ctx, |input_suggestions, ctx| {
                    input_suggestions.exit(true, ctx);
                });
        } else if self.workflows_state.selected_workflow_state.is_some() {
            self.clear_current_workflow(ctx);
        } else if !matches!(vim_mode, None | Some(VimMode::Normal)) {
            self.editor.update(ctx, |editor, editor_ctx| {
                editor.handle_action(&EditorAction::VimEscape, editor_ctx);
            });
        } else if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_active()
            && has_attached_context
        {
            self.clear_attached_context(ctx);
        } else {
            if FeatureFlag::AgentView.is_enabled()
                && !self.agent_view_controller.as_ref(ctx).is_fullscreen()
            {
                if self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                    // This implies the contents of the terminal input are autodetected as an agent
                    // prompt; overrides the autodetection by explicitly setting input mode back to
                    // terminal.
                    self.set_input_mode_terminal(false, ctx);
                }
            } else {
                self.set_input_mode_natural_language_detection(ctx);
            }
            ctx.emit(Event::Escape);
        }
    }

    /// Emits an `AgentModeAutodetectionFalsePositive` telemetry event if the current input text has
    /// been autodetected as AI input and the user manually toggled to shell.
    /// Also emits `AgentModeChangedInputType` if the user is part of the analytics experiment.
    ///
    /// This is intended to be called whenever the user manually toggles the input to new_input_type. Because the user is manually toggling
    /// back to shell mode after input has been autodetected as natural language, we infer that the
    /// current input text may not have been correctly classified as natural language.
    /// For users opted in to the analytics experiment, we collect the input buffer text whenever the input type is toggled
    /// in either direction.
    fn maybe_send_autodetection_telemetry_on_manual_toggle(
        &self,
        new_input_type: InputType,
        ctx: &mut ViewContext<Self>,
    ) {
        let input_buffer_text = self.buffer_text(ctx);
        let buffer_length = input_buffer_text.len();
        let input =
            should_collect_ai_ugc_telemetry(ctx, PrivacySettings::as_ref(ctx).is_telemetry_enabled)
                .then_some(input_buffer_text);
        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::AgentModeChangedInputType {
                input,
                buffer_length,
                is_manually_changed: true,
                new_input_type,
                active_block_id: self.model.lock().block_list().active_block_id().clone(),
                is_udi_enabled,
            },
            ctx
        );

        let ai_input_model = self.ai_input_model.as_ref(ctx);
        if matches!(new_input_type, InputType::Shell) && !ai_input_model.is_input_type_locked() {
            let current_input_text = self.buffer_text(ctx);
            if !current_input_text.is_empty() {
                let event_payload = if ChannelState::channel().is_dogfood() {
                    AgentModeAutoDetectionFalsePositivePayload::InternalDogfoodUsers {
                        input_text: current_input_text,
                    }
                } else {
                    AgentModeAutoDetectionFalsePositivePayload::ExternalUsers
                };
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModePotentialAutoDetectionFalsePositive(event_payload),
                    ctx
                );
            }
        }
    }

    /// Takes the current collpased/expanded state of the info box and saves it to the user's settings so that last value can be
    /// reused the next time the user opens a workflow.
    fn update_workflows_info_box_expanded_setting(
        &mut self,
        ctx: &mut ViewContext<Self>,
        selected_workflow_state: &SelectedWorkflowState,
    ) {
        let info_box_expanded = selected_workflow_state
            .more_info_view
            .as_ref(ctx)
            .info_box_expanded;

        InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
            report_if_error!(input_settings
                .workflows_box_expanded
                .set_value(info_box_expanded, ctx));
        });
    }

    fn clear_current_workflow(&mut self, ctx: &mut ViewContext<Input>) {
        // Whenever we clear the workflow we also want to clear the env vars
        self.clear_selected_env_var_collection();

        if let Some(state) = self.workflows_state.selected_workflow_state.take() {
            self.update_workflows_info_box_expanded_setting(ctx, &state);
        }
        self.editor
            .update(ctx, |editor, ctx| editor.clear_text_style_runs(ctx));
        ctx.notify();
    }

    fn editor_down(&mut self, ctx: &mut ViewContext<Self>) {
        // For some input suggestion modes, the menu handles its own actions.
        let handled = match self.suggestions_mode_model.as_ref(ctx).mode() {
            InputSuggestionsMode::AIContextMenu { .. } => {
                self.editor.update(ctx, |editor, ctx| {
                    if let Some(ai_context_menu) = editor.ai_context_menu() {
                        ai_context_menu.update(ctx, |menu, ctx| {
                            menu.handle_action(&AIContextMenuAction::Next, ctx);
                        });
                    }
                });
                true
            }
            InputSuggestionsMode::SlashCommands => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
                        view.update(ctx, |view, ctx| {
                            view.select_down(ctx);
                        });
                    }
                } else {
                    self.inline_slash_commands_view.update(ctx, |view, ctx| {
                        view.select_down(ctx);
                    });
                }
                true
            }
            InputSuggestionsMode::ConversationMenu => {
                self.inline_conversation_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::ForkFrom,
                ..
            } => {
                self.user_query_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::Rewind,
                ..
            } => {
                self.rewind_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::ModelSelector => {
                self.inline_model_selector_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::ProfileSelector => {
                self.inline_profile_selector_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::PromptsMenu => {
                self.inline_prompts_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::SkillMenu => {
                self.inline_skill_selector_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::IndexedReposMenu => {
                self.inline_repos_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::PlanMenu { .. } => {
                self.inline_plan_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
                true
            }
            InputSuggestionsMode::HistoryUp { .. }
            | InputSuggestionsMode::CompletionSuggestions { .. }
            | InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::InlineHistoryMenu { .. }
            | InputSuggestionsMode::Closed => false,
        };

        if handled {
            return;
        } else if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_history_menu()
        {
            if self.is_cloud_mode_input_v2_composing(ctx) {
                if let Some(view) = self.cloud_mode_v2_history_menu_view.clone() {
                    view.update(ctx, |view, ctx| {
                        view.select_down(ctx);
                    });
                }
            } else {
                self.inline_history_menu_view.update(ctx, |view, ctx| {
                    view.select_down(ctx);
                });
            }
            return;
        }

        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            if self.input_suggestions.as_ref(ctx).is_empty() {
                // arrow down on an empty suggestions means we should close it.
                self.close_input_suggestions_and_restore_buffer(true, true, ctx);
            } else {
                self.input_suggestions.update(ctx, |suggestions, ctx| {
                    suggestions.select_next(ctx);
                });
            }
        } else if FeatureFlag::CycleNextCommandSuggestion.is_enabled()
            && self.editor.as_ref(ctx).is_empty(ctx)
        {
            self.cycle_next_command_suggestion(ctx);
        } else {
            self.editor.update(ctx, |editor, ctx| editor.move_down(ctx));

            // Try to expand the most recent passive code diff if it exists.
            ctx.emit(Event::TryHandlePassiveCodeDiff(
                CodeDiffAction::ScrollToExpand,
            ));
        }
    }

    // TODO - Implement PageDown functionality for input suggestions menu
    fn editor_page_down(&mut self, ctx: &mut ViewContext<Self>) {
        let event = self.editor.read(ctx, |editor, ctx| {
            TelemetryEvent::PageUpDownInEditorPressed {
                is_empty_editor: editor.is_empty(ctx),
                is_down: true,
            }
        });
        send_telemetry_from_ctx!(event, ctx);
        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.editor
                .update(ctx, |input, ctx| input.move_page_down(ctx));
        } else {
            ctx.emit(Event::PageDown);
        }
    }

    fn maybe_generate_autosuggestion(&mut self, ctx: &mut ViewContext<Self>) {
        let editor = self.editor.as_ref(ctx);

        let should_generate_autosuggestion = !editor.active_autosuggestion()
            && self.enable_autosuggestions_setting
            && !self.ai_input_model.as_ref(ctx).is_ai_input_enabled();

        if should_generate_autosuggestion {
            let buffer_text = editor.buffer_text(ctx);
            self.generate_autosuggestion_async(buffer_text, self.completer_data(), ctx)
        }
    }

    /// Asynchronously generate an autosuggestion to be inserted into the editor. First, reverse
    /// search the user's history to find a possible command that starts with the buffer text. If
    /// no commands are found, run the completer in a background thread to generate a result.
    pub fn generate_autosuggestion_async(
        &mut self,
        buffer_text: String,
        completer_data: CompleterData,
        ctx: &mut ViewContext<Self>,
    ) {
        if buffer_text.is_empty() {
            return;
        }

        let Some(session_id) = completer_data.active_block_session_id() else {
            return;
        };
        self.abort_latest_autosuggestion_future();

        if FeatureFlag::PartialNextCommandSuggestions.is_enabled() && is_next_command_enabled(ctx) {
            let Some(session) = self.active_session(ctx) else {
                return;
            };
            let context = WarpAiExecutionContext::new(&session);
            if let Some(last_user_block_completed) =
                completer_data.last_user_block_completed.clone()
            {
                self.next_command_model.update(ctx, |model, ctx| {
                    model.generate_next_command_suggestion_with_prefix(
                        Some(buffer_text),
                        last_user_block_completed,
                        context,
                        completer_data,
                        None,
                        None,
                        ctx,
                    );
                });
                return;
            }
        }

        let completion_context = completer_data.completion_session_context(ctx);
        let completion_session = completion_context
            .as_ref()
            .map(|completion_context| completion_context.session.clone());

        let reverse_chronological_potential_autosuggestions =
            NextCommandModel::get_reverse_chronological_potential_autosuggestions(
                &buffer_text,
                &completer_data,
                ctx,
            );

        let session_env_vars = self.sessions.read(ctx, |sessions, _| {
            sessions.get_env_vars_for_session(session_id)
        });
        // Get current ignored shell commands to filter during generation
        let ignored_suggestions = IgnoredSuggestionsModel::as_ref(ctx)
            .get_ignored_suggestions_for_type(SuggestionType::ShellCommand);
        #[cfg(feature = "local_fs")]
        let conn = self.conn.clone();
        let abort_handle = ctx
            .spawn_abortable(
                async move {
                    #[cfg(feature = "local_fs")]
                    // First, use rich history to find commands with a matching prefix that were run
                    // in a similar context, taking into account the most recent block run.
                    if let Some(conn) = conn {
                        if let Some(last_user_block_completed) =
                            &completer_data.last_user_block_completed
                        {
                            let similar_history_contexts = {
                                let mut conn = conn.lock();
                                NextCommandModel::get_similar_history_context(
                                    &mut conn,
                                    last_user_block_completed,
                                    0,
                                )
                            };
                            if !similar_history_contexts.is_empty() {
                                let mut history_next_command_counts =
                                    counter::Counter::<String>::new();
                                // Find the most likely next command after a similar context, out of those that have a matching prefix and aren't ignored.
                                for history_context in &similar_history_contexts {
                                    if history_context
                                        .next_command
                                        .command
                                        .starts_with(&buffer_text)
                                        && !ignored_suggestions
                                            .contains(&history_context.next_command.command)
                                    {
                                        history_next_command_counts
                                            [&history_context.next_command.command] += 1;
                                    }
                                }

                                for (most_likely_next_command, _) in
                                    history_next_command_counts.k_most_common_ordered(5)
                                {
                                    if is_command_valid(
                                        &most_likely_next_command,
                                        completion_context.as_ref(),
                                        session_env_vars.as_ref(),
                                    )
                                    .await
                                    {
                                        return AutoSuggestionResult {
                                            buffer_text,
                                            autosuggestion_result: Some(
                                                most_likely_next_command.clone(),
                                            ),
                                        };
                                    }
                                }
                            }
                        }
                    }

                    // If we have no suggestion from similar historical context, fallback to the most recent
                    // command with a matching prefix run in the same pwd (if exists, otherwise just most recent command anywhere with matching prefix).
                    for reverse_chronological_command in
                        reverse_chronological_potential_autosuggestions.unwrap_or_default()
                    {
                        if !ignored_suggestions.contains(&reverse_chronological_command.command)
                            && is_command_valid(
                                &reverse_chronological_command.command,
                                completion_context.as_ref(),
                                session_env_vars.as_ref(),
                            )
                            .await
                        {
                            return AutoSuggestionResult {
                                buffer_text,
                                autosuggestion_result: Some(reverse_chronological_command.command),
                            };
                        }
                    }

                    // If we have no command anywhere in history with a matching prefix, fallback to the first completer result.
                    let Some(completion_context) = completion_context else {
                        return AutoSuggestionResult {
                            buffer_text,
                            autosuggestion_result: None,
                        };
                    };
                    let completion_result = completer::suggestions(
                        buffer_text.as_str(),
                        buffer_text.len(),
                        session_env_vars.as_ref(),
                        CompleterOptions {
                            match_strategy: MatchStrategy::CaseSensitive,
                            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
                            suggest_file_path_completions_only: false,
                            parse_quotes_as_literals: false,
                        },
                        &completion_context,
                    )
                    .await;

                    let autosuggestion = completion_result.and_then(|result| {
                        let replacement_span = result.replacement_span;
                        result
                            .suggestions
                            .into_iter()
                            .map(|s| {
                                // Reproduce the final buffer text with the autosuggestion since the
                                // completer only gives the replacement span of the suggestion.
                                format!(
                                    "{}{}",
                                    &buffer_text[..replacement_span.start()],
                                    s.replacement()
                                )
                            })
                            .find(|suggestion| !ignored_suggestions.contains(suggestion))
                    });

                    AutoSuggestionResult {
                        buffer_text,
                        autosuggestion_result: autosuggestion,
                    }
                },
                Self::on_autosuggestion_result,
                move |_, _| {
                    if let Some(session) = completion_session {
                        session.cancel_active_commands();
                    }
                },
            )
            .abort_handle();

        self.set_autosuggestion_future(abort_handle);
    }

    fn is_potential_expansion(
        token: &Spanned<String>,
        cursor_pos: usize,
        executing: Executing,
    ) -> bool {
        match executing {
            // Expansion was triggered by user entering the command to be executed.
            // To expand, cursor must be exactly at the end of the token.
            Executing::Yes => token.span().end() == cursor_pos,
            // Expansion was triggered by user pressing Space at the end of a token.
            // To expand, cursor must be one index after the end of the token.
            Executing::No => token.span().end() + 1 == cursor_pos,
        }
    }

    /// Gets the abbreviation and abbreviation value, or alias and alias value, given
    /// a command, if they exist. Will return None if the conditions for alias
    /// expansion are not met.
    fn get_valid_abbreviation_or_alias_for_expansion<'a>(
        &self,
        command: Option<&'a LiteCommand>,
        cursor_pos: usize,
        executing: Executing,
        session_context: &'a SessionContext,
        ctx: &mut ViewContext<Self>,
    ) -> Option<(&'a Spanned<String>, &'a str)> {
        // An alias must be the first token of a command
        let first_token = command?.parts.first()?;

        if !Self::is_potential_expansion(first_token, cursor_pos, executing) {
            return None;
        }

        // If there is an abbreviation, we expand it as long as we aren't executing.
        // In fish, an alias formatted like `ls=echo Hello && ls` would get expanded
        // twice if we also performed expansion on enter.
        if matches!(executing, Executing::No) {
            if let Some(abbr_value) = session_context
                .session
                .abbreviation_value(&first_token.item)
            {
                return Some((first_token, abbr_value));
            }
        }

        // We only expand aliases if the user has turned the setting on.
        if self.should_expand_aliases(ctx) {
            let alias_value = session_context.session.alias_value(&first_token.item)?;
            if !is_expandable_alias(&first_token.item, alias_value) {
                return None;
            }

            return Some((first_token, alias_value));
        }
        None
    }

    /// Function to check whether the previous token was a valid command abbreviation
    /// or alias and handle expansion. This should only be called after the user has
    /// entered a space into the input editor.
    fn run_expansion_on_space(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(expansion_info) = self.run_expansion_internal(Executing::No, ctx) {
            self.expand_alias(expansion_info.byte_range, &expansion_info.alias_value, ctx);
        }
    }

    /// Function that checks whether the current token was a valid command abbreviation
    /// or alias, and returns a String representing the input buffer with the expanded
    /// text. This should be called after the user has pressed Enter to execute the
    /// command.
    fn get_expanded_command_on_execute(&mut self, ctx: &mut ViewContext<Self>) -> Option<String> {
        self.run_expansion_internal(Executing::Yes, ctx)
            .and_then(|expansion_info| {
                let mut text = expansion_info.buffer_text;
                let is_valid_byte_range = text.is_char_boundary(expansion_info.byte_range.start)
                    && text.is_char_boundary(expansion_info.byte_range.end);
                is_valid_byte_range.then(|| {
                    text.replace_range(expansion_info.byte_range, &expansion_info.alias_value);
                    text
                })
            })
    }

    /// Helper function that handles whether there is a valid expansion based on
    /// the current input buffer and cursor position. Returns info needed to
    /// perform the expansion.
    fn run_expansion_internal(
        &mut self,
        executing: Executing,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ExpansionInfo> {
        let session_context = self.completion_session_context(ctx)?;
        let editor = self.editor.as_ref(ctx);
        editor.single_cursor_to_point(ctx)?;
        let buffer_text = editor.buffer_text(ctx);
        let cursor_pos = editor.end_byte_index_of_last_selection(ctx);
        let command = command_at_cursor_position(
            buffer_text.as_str(),
            session_context.escape_char(),
            cursor_pos,
        );

        self.get_valid_abbreviation_or_alias_for_expansion(
            command.as_ref(),
            cursor_pos.as_usize(),
            executing,
            &session_context,
            ctx,
        )
        .map(|(alias, alias_value)| ExpansionInfo {
            alias_value: alias_value.into(),
            buffer_text,
            byte_range: alias.span().start()..cursor_pos.as_usize(),
        })
    }

    fn expand_alias(
        &mut self,
        replacement_range: Range<usize>,
        alias_value: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let alias_value_with_space = format!("{alias_value} ");
        self.editor.update(ctx, |input, ctx| {
            input.select_and_replace(
                &alias_value_with_space,
                [ByteOffset::from(replacement_range.start)
                    ..ByteOffset::from(replacement_range.end)],
                PlainTextEditorViewAction::ExpandAlias,
                ctx,
            );
        });
    }

    /// If at least one input is being synced, emit an event that other
    /// terminal views can decide to process based on their sync state.
    fn send_input_sync_event(&self, edit_origin: &EditOrigin, ctx: &mut ViewContext<Self>) {
        let is_syncing_inputs =
            SyncedInputState::as_ref(ctx).is_syncing_any_inputs(ctx.window_id());

        if is_syncing_inputs
                    // If the edit we're applying in `handle_editor_event`
                    //came from another synced terminal,
                    // don't emit a new event which would create a cycle
                    && *edit_origin != EditOrigin::SyncedTerminalInput
                    // Similarly, only emit an event from the session the user is typing in
                    && self.focus_handle.as_ref().is_none_or(|h| h.is_focused(ctx))
        {
            let buffer = self.editor.as_ref(ctx).buffer_text(ctx);
            ctx.emit(Event::SyncInput(
                SyncInputType::InputEditorContentsChanged {
                    contents: Arc::new(buffer),
                },
            ));
        }
    }

    /// Whether the given event should trigger a request to generate an AI-based natural language
    /// autosuggestion, due to the buffer content meaningfully changing.
    fn is_nl_ai_autosuggestion_triggering_event(event: &EditorEvent) -> bool {
        matches!(
            event,
            EditorEvent::Edited(_)
                | EditorEvent::BufferReplaced
                | EditorEvent::InsertLastWordPrevCommand
                | EditorEvent::AutosuggestionAccepted { .. }
                | EditorEvent::DeleteAllLeft
                | EditorEvent::BackspaceOnEmptyBuffer
                | EditorEvent::BackspaceAtBeginningOfBuffer
                | EditorEvent::MiddleClickPaste
        )
    }

    fn should_close_ai_context_menu(
        &self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let InputSuggestionsMode::AIContextMenu {
            at_symbol_position, ..
        } = *self.suggestions_mode_model.as_ref(ctx).mode()
        else {
            return false;
        };

        if matches!(
            event,
            EditorEvent::DeleteAllLeft
                | EditorEvent::CtrlC { .. }
                | EditorEvent::BackspaceOnEmptyBuffer
                | EditorEvent::BackspaceAtBeginningOfBuffer
                | EditorEvent::SetAIContextMenuOpen(false)
        ) {
            return true;
        }
        if !matches!(
            event,
            EditorEvent::Edited(_)
                | EditorEvent::BufferReplaced
                | EditorEvent::InsertLastWordPrevCommand
                | EditorEvent::AutosuggestionAccepted { .. }
                | EditorEvent::MiddleClickPaste
        ) {
            return false;
        }
        let buffer = self.editor.as_ref(ctx).buffer_text(ctx);
        let cursor_pos = self
            .editor
            .as_ref(ctx)
            .start_byte_index_of_last_selection(ctx)
            .as_usize();
        // If the cursor is to the left of the "@", we should close the AI context menu.
        if cursor_pos < at_symbol_position {
            return true;
        }
        let chars_before_cursor: Vec<char> = buffer.as_str().chars().take(cursor_pos).collect();
        let iter = chars_before_cursor.into_iter().rev();
        let mut prev_char_was_space = false;
        for c in iter {
            if c.is_whitespace() && c != ' ' {
                return true;
            }
            if c == '@' {
                return prev_char_was_space;
            }
            if c == ' ' {
                if prev_char_was_space {
                    return true;
                }
                prev_char_was_space = true;
            } else {
                prev_char_was_space = false;
            }
        }
        true
    }

    /// Helper function to replace "@" symbol and filter text with new text
    pub(super) fn replace_at_symbol_with_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let is_ai_mode = self.ai_input_model.as_ref(ctx).is_ai_input_enabled();

        // Capture the at_symbol_position before it might be cleared
        let at_symbol_position = if let InputSuggestionsMode::AIContextMenu {
            at_symbol_position,
            ..
        } = self.suggestions_mode_model.as_ref(ctx).mode()
        {
            Some(*at_symbol_position)
        } else {
            None
        };

        if let Some(at_pos) = at_symbol_position {
            let cursor_position = self.editor.read(ctx, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
            });

            let replacement_range =
                ByteOffset::from(at_pos)..ByteOffset::from(cursor_position.as_usize());
            self.editor.update(ctx, |editor, ctx| {
                // Delete the range (@ symbol and any filter text) using system delete
                editor.system_delete(replacement_range, ctx);

                // Insert the text, optionally with a space in AI mode
                let text_to_insert = if is_ai_mode {
                    format!("{text} ")
                } else {
                    text.to_string()
                };
                editor.user_insert(&text_to_insert, ctx);
            });
        } else {
            // Fallback: search for the most recent "@" symbol in the buffer
            let buffer_text = self.editor.read(ctx, |editor, ctx| editor.buffer_text(ctx));
            let cursor_position = self.editor.read(ctx, |editor, ctx| {
                editor.start_byte_index_of_last_selection(ctx)
            });

            if let Some(at_position) = buffer_text[..cursor_position.as_usize()].rfind('@') {
                let replacement_range =
                    ByteOffset::from(at_position)..ByteOffset::from(cursor_position.as_usize());
                self.editor.update(ctx, |editor, ctx| {
                    // Delete the range (@ symbol and any filter text) using system delete
                    editor.system_delete(replacement_range, ctx);

                    let text_to_insert = if is_ai_mode {
                        format!("{text} ")
                    } else {
                        text.to_string()
                    };
                    editor.user_insert(&text_to_insert, ctx);
                });
            }
        }
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        // We want to clear the token description hover on any editor action
        self.hide_x_ray(ctx);

        if !matches!(event, EditorEvent::InsertLastWordPrevCommand) {
            self.update_last_word_insertion_state();
        }

        if self.should_close_ai_context_menu(event, ctx) {
            self.close_ai_context_menu(ctx);
        }

        self.check_slash_menu_disabled_state(ctx);

        let is_ai_input_enabled = self.ai_input_model.as_ref(ctx).is_ai_input_enabled();

        if Self::is_nl_ai_autosuggestion_triggering_event(event)
            && FeatureFlag::PredictAMQueries.is_enabled()
            && AISettings::as_ref(ctx).is_natural_language_autosuggestions_enabled(ctx)
            && is_ai_input_enabled
            && !self.buffer_text(ctx).is_empty()
        {
            // Cancel any pending requests for AM ghosted text predictions.
            if let Some(future_handle) = self.predict_am_queries_future_handle.take() {
                future_handle.abort();
            }
            let _ = self.debounce_ai_query_prediction_tx.try_send(());
        }

        match event {
            EditorEvent::Edited(edit_origin) => {
                // We should ideally be handling all `Edited` events, not just those that are
                // marked EditOrigin. However, we receive the notification that the block has
                // completed, in the same event we clear the input box per-command. Due to how
                // events are dispatched in the UI framework, we would receive an Edited event
                // immediately from clearing the input box. But we don't want that.
                // Only processing the user typed events should be good enough here.

                if matches!(
                    edit_origin,
                    EditOrigin::UserTyped | EditOrigin::UserInitiated
                ) {
                    self.model.lock().set_is_input_dirty(true);
                }

                if self.maybe_activate_cloud_handoff_prefix(edit_origin, ctx) {
                    return;
                }

                if *edit_origin == EditOrigin::UserTyped
                    && !ctx
                        .model(&self.input_render_state_model_handle)
                        .editor_modified_since_block_finished()
                {
                    self.input_render_state_model_handle.update(
                        ctx,
                        |input_render_state_model, _| {
                            input_render_state_model.set_editor_modified_since_block_finished(true);
                        },
                    );

                    if !self
                        .model
                        .lock()
                        .block_list()
                        .active_block()
                        .has_received_precmd()
                    {
                        send_telemetry_from_ctx!(TelemetryEvent::EditedInputBeforePrecmd, ctx);
                        ctx.notify();
                    }
                }

                let is_editor_empty = self.editor.as_ref(ctx).is_empty(ctx);
                if is_editor_empty != self.is_editor_empty_on_last_edit {
                    self.is_editor_empty_on_last_edit = is_editor_empty;
                    ctx.emit(Event::InputEmptyStateChanged {
                        is_empty: is_editor_empty,
                        reason: InputEmptyStateChangeReason::Edited,
                    });
                }

                let is_ai_input_enabled = self.ai_input_model.as_ref(ctx).is_ai_input_enabled();

                let mut short_circuit_highlighting = false;
                let mut check_alias_expansion = false;
                let mut should_open_ai_context_menu = false;

                let cursor_position = self.editor.read(ctx, |editor, editor_ctx| {
                    editor.start_byte_index_of_last_selection(editor_ctx)
                });

                let is_alias_expansion_enabled = self.should_expand_aliases(ctx);
                let session_context = self.completion_session_context(ctx);

                self.editor.read(ctx, |editor, editor_ctx| {
                    let last_action = editor.get_last_action(editor_ctx);
                    if Some(PlainTextEditorViewAction::Space) == last_action
                        && *edit_origin == EditOrigin::UserTyped
                    {
                        check_alias_expansion = true;
                    }

                    // Check if "@" was just typed in a valid context
                    if FeatureFlag::AIContextMenuEnabled.is_enabled()
                        && (is_ai_input_enabled || FeatureFlag::AtMenuOutsideOfAIMode.is_enabled())
                        && Some(PlainTextEditorViewAction::InsertChar) == last_action
                        && *edit_origin == EditOrigin::UserTyped
                    {
                        let buffer_text = editor.buffer_text(ctx);
                        let should_enable = self.should_enable_ai_context(
                            &buffer_text,
                            cursor_position.as_usize(),
                            is_alias_expansion_enabled,
                            session_context.as_ref(),
                            editor.shell_family().unwrap_or(ShellFamily::Posix),
                            ctx,
                        );
                        if should_enable {
                            should_open_ai_context_menu = true;
                        }
                    }

                    if SHORT_CIRCUIT_HIGHLIGHTING_ACTIONS.contains(&last_action) {
                        short_circuit_highlighting = true;
                    }
                });

                // Force AI mode if buffer contains any attachment patterns (blocks, drive objects, diffs)
                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) && edit_origin.is_user() {
                    let buffer_text = self.buffer_text(ctx);
                    if Self::buffer_contains_attachment_patterns(&buffer_text) {
                        self.ensure_agent_mode_for_ai_features(false, ctx);
                    }
                }

                if should_open_ai_context_menu {
                    let cursor_pos = self.editor.read(ctx, |editor, ctx| {
                        editor.start_byte_index_of_last_selection(ctx)
                    });
                    self.suggestions_mode_model.update(ctx, |m, ctx| {
                        m.set_mode(
                            InputSuggestionsMode::AIContextMenu {
                                filter_text: "".to_string(),
                                // -1 since cursor is after the @ symbol
                                at_symbol_position: cursor_pos.as_usize().saturating_sub(1),
                            },
                            ctx,
                        );
                    });

                    // Update AI context menu input mode based on current state
                    // Show AI categories if we're in AI mode OR if autodetection is enabled (not locked)
                    let ai_input_model = self.ai_input_model.as_ref(ctx);
                    let is_ai_or_autodetect_mode = ai_input_model.input_type().is_ai()
                        || !ai_input_model.is_input_type_locked();

                    self.editor.update(ctx, |editor, ctx| {
                        if let Some(ai_context_menu) = editor.ai_context_menu() {
                            ai_context_menu.update(ctx, |menu, ctx| {
                                menu.set_input_mode(is_ai_or_autodetect_mode, ctx);
                            });
                        }
                    });

                    ctx.notify();
                }

                // Update filter text for AI context menu when text changes
                self.handle_ai_context_menu_search(false, ctx);

                // Check if cursor is exactly at '@' position after deletion and reset menu state if appropriate
                if let InputSuggestionsMode::AIContextMenu {
                    at_symbol_position, ..
                } = self.suggestions_mode_model.as_ref(ctx).mode()
                {
                    let cursor_pos = self
                        .editor
                        .as_ref(ctx)
                        .start_byte_index_of_last_selection(ctx)
                        .as_usize();

                    // If cursor is exactly at the @ position, reset the menu state
                    if cursor_pos == *at_symbol_position + 1
                        && *edit_origin == EditOrigin::UserInitiated
                    {
                        self.editor.update(ctx, |editor, ctx| {
                            if let Some(ai_context_menu) = editor.ai_context_menu() {
                                ai_context_menu.update(ctx, |menu, ctx| {
                                    menu.reset_menu_state(ctx);
                                });
                            }
                        });
                    }
                }

                if check_alias_expansion {
                    self.run_expansion_on_space(ctx);
                }

                // Don't run NLD autodetection when an inline menu is open (slash commands,
                // conversation menu, model selector), as the buffer contents are being used as
                // a search query for the menu rather than as a command/prompt.
                let is_inline_menu_open = self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_inline_menu_open();

                // NLD autodetection is irrelevant in cloud mode v2 — the input is always AI.
                let should_run_ai_input_detection = if self.is_cloud_mode_input_v2_composing(ctx) {
                    false
                } else {
                    match edit_origin {
                        // Edits made by the local user should trigger autodetection, if
                        // it is enabled.
                        EditOrigin::UserInitiated
                        | EditOrigin::UserTyped
                        | EditOrigin::SyncedTerminalInput => {
                            !is_inline_menu_open
                                && self
                                    .ai_input_model
                                    .as_ref(ctx)
                                    .should_run_input_autodetection(ctx)
                        }
                        // Remote edits from shared session viewers should trigger autodetection
                        // on the sharer's side, so that the sharer's input mode adjusts as viewers type.
                        EditOrigin::RemoteEdit => {
                            let is_sharer = self.model.lock().shared_session_status().is_sharer();
                            !is_inline_menu_open
                                && is_sharer
                                && self
                                    .ai_input_model
                                    .as_ref(ctx)
                                    .should_run_input_autodetection(ctx)
                        }
                        // System edits should never trigger autodetection.
                        EditOrigin::SystemEdit => false,
                    }
                };

                // Abort any autodetection work on the old buffer state.
                self.ai_input_model.update(ctx, |controller, _| {
                    controller.abort_in_progress_detection();
                });
                // Abort any inflight request to generate a Next Command suggestion.
                self.next_command_model.update(ctx, |model, _| {
                    model.abort_inflight_request();
                });

                if self.should_apply_decorations(ctx)
                    || should_run_ai_input_detection
                    || is_ai_input_enabled
                {
                    let mut mode = InputBackgroundJobOptions::default();

                    if self.should_apply_decorations(ctx) {
                        mode = mode.with_command_decoration();
                    }

                    if should_run_ai_input_detection {
                        mode = mode.with_ai_input_detection();
                    }

                    if short_circuit_highlighting {
                        self.run_input_background_jobs(mode, ctx);
                    } else {
                        let _ = self.debounce_input_background_tx.try_send(mode);
                    }
                }

                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
                    && self.editor_starts_with_command_search_trigger(ctx)
                    && *edit_origin == EditOrigin::UserTyped
                    && !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
                {
                    // If last buffer didn't start with '#' and current buffer does,
                    // then show command search.
                    let last_buffer_text = self.editor.as_ref(ctx).last_buffer_text(ctx);
                    if !last_buffer_text.starts_with(AI_COMMAND_SEARCH_TRIGGER) {
                        self.show_ai_command_search(ctx);
                    }
                    ctx.notify();
                }

                let is_input_mode_locked = self.ai_input_model.as_ref(ctx).is_input_type_locked();
                let buffer_text = self.buffer_text(ctx);

                // If the last buffer didn't start with the AI input prefix and the current buffer does, then enable AI input.
                if FeatureFlag::AgentMode.is_enabled()
                    && !FeatureFlag::AgentView.is_enabled()
                    && AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
                    && (!is_ai_input_enabled || !is_input_mode_locked)
                {
                    if buffer_text.starts_with(AI_INPUT_PREFIX)
                        && *edit_origin == EditOrigin::UserTyped
                    {
                        let last_buffer_text = self.editor.as_ref(ctx).last_buffer_text(ctx);

                        if !last_buffer_text.starts_with(AI_INPUT_PREFIX) {
                            // Remove the prefix from the editor contents.
                            let is_input_buffer_empty =
                                self.editor.update(ctx, |editor_view, ctx| {
                                    if let Some(query) =
                                        editor_view.buffer_text(ctx).strip_prefix(AI_INPUT_PREFIX)
                                    {
                                        editor_view.set_buffer_text(query, ctx);
                                    }
                                    editor_view.buffer_text(ctx).is_empty()
                                });

                            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                                ai_input_model.set_input_config(
                                    InputConfig {
                                        input_type: InputType::AI,
                                        is_locked: true,
                                    },
                                    is_input_buffer_empty,
                                    ctx,
                                );
                            });
                        }
                    } else if buffer_text.is_empty() && is_input_mode_locked {
                        self.ai_input_model.update(ctx, |input_model, ctx| {
                            input_model.set_input_config_for_classic_mode(
                                input_model
                                    .input_config()
                                    .unlocked_if_autodetection_enabled(false, ctx),
                                ctx,
                            );
                        });
                    }

                    ctx.notify();
                }

                let ai_settings = AISettings::as_ref(ctx);
                if FeatureFlag::AgentView.is_enabled()
                    && buffer_text.is_empty()
                    && self.prefix_mode(ctx) != InputPrefixMode::CloudHandoff
                {
                    let last_buffer_text = self.editor.as_ref(ctx).last_buffer_text(ctx);
                    let was_shell_mode_prefix_stripped =
                        last_buffer_text == TERMINAL_INPUT_PREFIX && buffer_text.is_empty();

                    let is_fullscreen_agent_view_active =
                        self.agent_view_controller.as_ref(ctx).is_fullscreen();
                    let current_input_config = self.ai_input_model.as_ref(ctx).input_config();

                    // We should re-enable autodetection if the user overrode an autodetection
                    // result:
                    // * In agent view, this means the user overrode a mis-classified shell command
                    //   to be an agent prompt.
                    // * In terminal view, this eans the user overrode a mis-classified agent prompt
                    //   to a terminal command.
                    let is_cli_agent_input_open =
                        CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
                    let should_reenable_autodetection = (ai_settings
                        .is_ai_autodetection_enabled(ctx)
                        && is_fullscreen_agent_view_active
                        && current_input_config.is_ai()
                        && current_input_config.is_locked
                        && !was_shell_mode_prefix_stripped)
                        || (ai_settings.is_nld_in_terminal_enabled(ctx)
                            && !self.agent_view_controller.as_ref(ctx).is_active()
                            && !is_cli_agent_input_open
                            && current_input_config.is_shell()
                            && current_input_config.is_locked);
                    if should_reenable_autodetection {
                        self.ai_input_model.update(ctx, |input_model, ctx| {
                            input_model.enable_autodetection(input_model.input_type(), ctx);
                        });
                        ctx.notify();
                    }
                }

                // If the last buffer didn't start with the terminal input prefix and the current buffer does, then enable terminal input and lock it.
                let is_locked_shell_mode = !is_ai_input_enabled && is_input_mode_locked;
                let is_agent_view_active = self.agent_view_controller.as_ref(ctx).is_active();
                let is_agent_in_control_or_tagged_in = self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_in_control_or_tagged_in();
                let is_cli_agent_bash_mode_input_open = CLIAgentSessionsModel::as_ref(ctx)
                    .session(self.terminal_view_id)
                    .is_some_and(|s| {
                        s.agent.supports_bash_mode()
                            && matches!(s.input_state, CLIAgentInputState::Open { .. })
                    });
                if FeatureFlag::AgentMode.is_enabled()
                    && !is_locked_shell_mode
                    && (!FeatureFlag::AgentView.is_enabled()
                        || is_agent_view_active
                        || is_cli_agent_bash_mode_input_open)
                    && !is_agent_in_control_or_tagged_in
                    && self.prefix_mode(ctx) != InputPrefixMode::CloudHandoff
                {
                    let buffer_text = self.buffer_text(ctx);
                    if buffer_text.starts_with(TERMINAL_INPUT_PREFIX)
                        && *edit_origin == EditOrigin::UserTyped
                    {
                        let last_buffer_text = self.editor.as_ref(ctx).last_buffer_text(ctx);

                        if !last_buffer_text.starts_with(TERMINAL_INPUT_PREFIX) {
                            // Remove the prefix from the editor contents.
                            let is_input_buffer_empty =
                                self.editor.update(ctx, |editor_view, ctx| {
                                    if let Some(command) = editor_view
                                        .buffer_text(ctx)
                                        .strip_prefix(TERMINAL_INPUT_PREFIX)
                                    {
                                        editor_view.set_buffer_text(command, ctx);
                                    }
                                    editor_view.buffer_text(ctx).is_empty()
                                });

                            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                                ai_input_model.set_input_config(
                                    InputConfig {
                                        input_type: InputType::Shell,
                                        is_locked: true,
                                    },
                                    is_input_buffer_empty,
                                    ctx,
                                );
                            });
                        }
                    }
                    ctx.notify();
                }

                // We only sync on EditorEvent::Edited events because we're only
                // syncing terminal input editor contents, not the full
                // functionality of the terminal input in each blocklist
                // e.g., we don't want to sync EditorEvent::CmdUpOnFirstRow.
                self.send_input_sync_event(edit_origin, ctx);

                let mode = self.suggestions_mode_model.as_ref(ctx).mode().clone();
                match &mode {
                    InputSuggestionsMode::CompletionSuggestions {
                        replacement_start,
                        buffer_text_original,
                        completion_results,
                        trigger,
                        ..
                    } => {
                        let replacement_start = *replacement_start;
                        let editor_text = self.buffer_text(ctx);
                        let cursor_position = self.start_byte_index_of_last_selection(ctx);
                        let current_word =
                            editor_text.get(replacement_start..cursor_position.as_usize());
                        let current_selected_item =
                            self.input_suggestions.as_ref(ctx).get_selected_item_text();
                        let selected_item_differs_from_current_word = current_selected_item
                            .zip(current_word)
                            .map(|(selected_item, current_word)| selected_item != current_word)
                            .unwrap_or(true);

                        // To support completions-as-you-type x classic completions,
                        // we need to make sure we don't recompute the completion results
                        // as the user cycles (which inserts into buffer and thus is treated
                        // as an edit). Thus, when using the two features together, we only
                        // recompute the result set if the selected item doesn't match the
                        // current word span.
                        let old_buffer_text_original = buffer_text_original.clone();
                        if *trigger == CompletionsTrigger::AsYouType
                            && (!self.is_classic_completions_enabled(ctx)
                                || (self.is_classic_completions_enabled(ctx)
                                    && selected_item_differs_from_current_word))
                        {
                            // For as-you-type completions, we recalculate suggestions rather than
                            // filtering, since typing could involve moving to a new parameter
                            // within a given command, rather than being a strict subset as is the
                            // case with manual tab completions.
                            self.open_completion_suggestions(CompletionsTrigger::AsYouType, ctx);
                            self.maybe_generate_autosuggestion(ctx);

                            // Since tab completions are async, we should close the
                            // menu if it's been some time and the menu still hasn't updated,
                            // otherwise the user will see an old completions menu even while
                            // the buffer text has changed. We wait with a delay so that way
                            // the menu doesn't close right away and open away right after if
                            // the completions finish quickly, since that causes a jittery UX.
                            let _ = ctx.spawn(
                                async move {
                                    warpui::r#async::Timer::after(Duration::from_millis(750)).await;
                                    old_buffer_text_original
                                },
                                move |input, old_buffer_text_original, ctx| {
                                    if let InputSuggestionsMode::CompletionSuggestions {
                                        buffer_text_original,
                                        ..
                                    } = input.suggestions_mode_model.as_ref(ctx).mode()
                                    {
                                        // The menu hasn't changed since last time so
                                        // close it for now. If the menu is truly delayed,
                                        // the completions callback will eventually open it.
                                        if old_buffer_text_original == *buffer_text_original {
                                            input.close_input_suggestions(true, ctx);
                                        }
                                    }
                                },
                            );
                        } else {
                            let buffer_text_original = buffer_text_original.clone();
                            let completion_results = completion_results.clone();
                            let should_close = self.update_tab_completion_menu(
                                replacement_start,
                                buffer_text_original.as_str(),
                                &completion_results,
                                ctx,
                            );
                            if should_close {
                                self.close_input_suggestions(
                                    /*should_focus_input=*/ true, ctx,
                                );
                            }
                        }
                    }
                    InputSuggestionsMode::StaticWorkflowEnumSuggestions {
                        cursor_point, ..
                    }
                    | InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                        cursor_point, ..
                    } => {
                        let cursor_point = *cursor_point;
                        let point = self.editor.as_ref(ctx).first_selection_end_to_point(ctx);
                        let should_close = point != cursor_point;

                        if should_close {
                            self.close_input_suggestions(/*should_focus_input=*/ true, ctx);
                        }
                    }
                    InputSuggestionsMode::HistoryUp { .. } => {
                        // In HistoryUp mode, we replace the buffer as options
                        // are selected.
                        // We also dismiss the suggestion menu if the buffer
                        // is edited such that it doesn't exactly match
                        // the selected suggestion.

                        if let Some(selected_text) =
                            self.input_suggestions.as_ref(ctx).get_selected_item_text()
                        {
                            if *selected_text.to_string()
                                == self.editor.as_ref(ctx).buffer_text(ctx)
                            {
                                return;
                            }

                            let has_active_ai_block =
                                self.model.lock().block_list().has_active_ai_block(ctx);
                            // We only focus the input if there is no active AI
                            // block. Otherwise, the input is incorrectly focused
                            // when executing an AI query from the history menu.
                            self.close_input_suggestions(
                                !has_active_ai_block, /*should_focus_input=*/
                                ctx,
                            );
                        }
                    }
                    InputSuggestionsMode::Closed => {
                        if !self.can_query_history(ctx) {
                            return;
                        }

                        let editor = self.editor.as_ref(ctx);
                        let buffer_text = editor.buffer_text(ctx);

                        self.maybe_generate_autosuggestion(ctx);

                        if buffer_text.is_empty()
                            && self.workflows_state.selected_workflow_state.is_some()
                        {
                            self.clear_current_workflow(ctx);
                        }

                        if self.should_show_completions_while_typing(ctx)
                            && matches!(edit_origin, EditOrigin::UserTyped)
                        {
                            self.open_completion_suggestions(CompletionsTrigger::AsYouType, ctx);
                        }
                    }
                    InputSuggestionsMode::AIContextMenu { .. } => {
                        self.handle_ai_context_menu_search(false, ctx);
                    }
                    InputSuggestionsMode::SlashCommands => {
                        // empty for now
                    }
                    InputSuggestionsMode::ConversationMenu => {
                        // Conversation menu handles its own state
                    }
                    InputSuggestionsMode::ModelSelector => {
                        // Model selector handles its own state
                    }
                    InputSuggestionsMode::ProfileSelector => {
                        // Profile selector handles its own state
                    }
                    InputSuggestionsMode::PromptsMenu => {
                        // Prompts menu handles its own state
                    }
                    InputSuggestionsMode::SkillMenu => {
                        // Skill menu handles its own state
                    }
                    InputSuggestionsMode::UserQueryMenu { .. } => {
                        // User query menu handles its own state
                    }
                    InputSuggestionsMode::InlineHistoryMenu { .. } => {
                        let mismatched = if self.is_cloud_mode_input_v2_composing(ctx) {
                            self.cloud_mode_v2_history_menu_view
                                .as_ref()
                                .and_then(|view| view.as_ref(ctx).selected_query_text(ctx))
                                .is_some_and(|selected_text| {
                                    selected_text != self.editor.as_ref(ctx).buffer_text(ctx)
                                })
                        } else {
                            self.inline_history_menu_view
                                .as_ref(ctx)
                                .model()
                                .as_ref(ctx)
                                .selected_item()
                                .and_then(|item| item.buffer_replacement_text())
                                .is_some_and(|selected_item_text| {
                                    *selected_item_text != self.editor.as_ref(ctx).buffer_text(ctx)
                                })
                        };
                        if mismatched {
                            self.suggestions_mode_model.update(ctx, |model, ctx| {
                                model.set_mode(InputSuggestionsMode::Closed, ctx);
                            });
                            ctx.notify();
                        }
                    }
                    InputSuggestionsMode::IndexedReposMenu => {
                        // Repos menu handles its own state
                    }
                    InputSuggestionsMode::PlanMenu { .. } => {
                        // Plan menu handles its own state
                    }
                }
            }
            EditorEvent::BufferReplaced => {
                let ai_input_model = self.ai_input_model.as_ref(ctx);
                if FeatureFlag::AgentMode.is_enabled()
                    && AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
                    && !ai_input_model.is_ai_input_enabled()
                    && ai_input_model.is_input_type_locked()
                {
                    // If this edit effectively emptied the buffer and we're in shell mode,
                    // unlock the input so autodetection can kick in.
                    self.ai_input_model.update(ctx, |input_model, ctx| {
                        input_model.set_input_config_for_classic_mode(
                            input_model
                                .input_config()
                                .unlocked_if_autodetection_enabled(false, ctx),
                            ctx,
                        );
                    });
                    ctx.notify();
                }
            }
            EditorEvent::SelectionChanged => {
                let mode = self.suggestions_mode_model.as_ref(ctx).mode().clone();
                let is_completion_suggestions =
                    matches!(mode, InputSuggestionsMode::CompletionSuggestions { .. });
                if is_completion_suggestions && !self.cursor_positioned_for_completion(ctx) {
                    self.close_input_suggestions(/*should_focus_input=*/ true, ctx);
                } else {
                    match &mode {
                        InputSuggestionsMode::HistoryUp { .. } | InputSuggestionsMode::Closed => {}
                        InputSuggestionsMode::CompletionSuggestions {
                            replacement_start,
                            buffer_text_original,
                            completion_results,
                            ..
                        } => {
                            let replacement_start = *replacement_start;
                            let buffer_text_original = buffer_text_original.clone();
                            let completion_results = completion_results.clone();
                            let should_close = self.update_tab_completion_menu(
                                replacement_start,
                                buffer_text_original.as_str(),
                                &completion_results,
                                ctx,
                            );

                            if should_close {
                                self.close_input_suggestions(
                                    /*should_focus_input=*/ true, ctx,
                                );
                            }
                        }
                        InputSuggestionsMode::StaticWorkflowEnumSuggestions {
                            cursor_point,
                            ..
                        }
                        | InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                            cursor_point,
                            ..
                        } => {
                            let cursor_point = *cursor_point;
                            let point = self.editor.as_ref(ctx).first_selection_end_to_point(ctx);
                            let should_close = point != cursor_point;

                            if should_close {
                                self.close_input_suggestions(
                                    /*should_focus_input=*/ true, ctx,
                                );
                            }
                        }
                        InputSuggestionsMode::AIContextMenu {
                            at_symbol_position, ..
                        } => {
                            let at_symbol_position = *at_symbol_position;
                            // Close the AI context menu if cursor moves to the left of the @ position
                            let cursor_pos = self
                                .editor
                                .as_ref(ctx)
                                .start_byte_index_of_last_selection(ctx)
                                .as_usize();

                            if cursor_pos <= at_symbol_position {
                                self.close_ai_context_menu(ctx);
                                return;
                            }

                            self.handle_ai_context_menu_search(true, ctx);
                        }
                        InputSuggestionsMode::SlashCommands => {
                            let cursor_pos = self
                                .editor
                                .as_ref(ctx)
                                .start_byte_index_of_last_selection(ctx)
                                .as_usize();

                            if cursor_pos == 0 {
                                self.close_input_suggestions(true, ctx);
                            }
                        }
                        InputSuggestionsMode::ConversationMenu => {
                            // Conversation menu handles its own selection state
                        }
                        InputSuggestionsMode::ModelSelector => {
                            // Model selector handles its own selection state
                        }
                        InputSuggestionsMode::ProfileSelector => {
                            // Profile selector handles its own selection state
                        }
                        InputSuggestionsMode::PromptsMenu => {
                            // Prompts menu handles its own selection state
                        }
                        InputSuggestionsMode::SkillMenu => {
                            // Skill menu handles its own selection state
                        }
                        InputSuggestionsMode::UserQueryMenu { .. } => {
                            // User query menu handles its own selection state
                        }
                        InputSuggestionsMode::InlineHistoryMenu { .. } => {
                            // Inline history menu handles its own selection state
                        }
                        InputSuggestionsMode::IndexedReposMenu => {
                            // Repos menu handles its own selection state
                        }
                        InputSuggestionsMode::PlanMenu { .. } => {
                            // Plan menu handles its own selection state
                        }
                    }
                }
            }
            EditorEvent::AutosuggestionAccepted {
                insertion_length,
                buffer_char_length,
                autosuggestion_type,
            } => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AutosuggestionInserted {
                        insertion_length: *insertion_length,
                        buffer_length: *buffer_char_length
                    },
                    ctx
                );
                ctx.emit(Event::AutosuggestionAccepted);

                self.input_suggestions
                    .update(ctx, |input_suggestions, ctx| {
                        // We should not restore the buffer to the old state since we're accepting an autosuggestion from the new state.
                        input_suggestions.exit(false, ctx);
                    });
                match autosuggestion_type {
                    AutosuggestionType::Command {
                        was_intelligent_autosuggestion,
                    } => {
                        // Switch to shell input mode but preserve current lock state when accepting a command autosuggestion.
                        self.ai_input_model.update(ctx, |input_model, ctx| {
                            input_model.set_input_type(InputType::Shell, ctx);
                        });
                        if *was_intelligent_autosuggestion {
                            self.was_intelligent_autosuggestion_accepted = true;
                        } else {
                            // This accepted autosuggestion count is used to determine whether to show the right arrow to accept icon
                            // when there's an autosuggestion while the input buffer is not empty.
                            // So it should only be incremented when an autosuggestion is accepted while the buffer is not empty (is NOT intelligent/zero-state).
                            InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                                let current_count =
                                    *input_settings.autosuggestion_accepted_count.value();
                                if current_count < MAX_TIMES_TO_SHOW_AUTOSUGGESTION_HINT {
                                    let new_count = if current_count < 0 {
                                        // Note: there was a bug in the previous implementation of this method which would
                                        // cause it to overflow the i8 value to a negative value. In that case, we know
                                        // that the user has definitely accepted at _least_ 128 autosuggestions, so we can
                                        // set it to the maximum relevant value: MAX_TIMES_TO_SHOW_AUTOSUGGESTION_HINT
                                        MAX_TIMES_TO_SHOW_AUTOSUGGESTION_HINT
                                    } else {
                                        current_count + 1
                                    };

                                    report_if_error!(input_settings
                                        .autosuggestion_accepted_count
                                        .set_value(new_count, ctx))
                                }
                            })
                        }
                    }
                    AutosuggestionType::AgentModeQuery {
                        context_block_ids,
                        was_intelligent_autosuggestion,
                    } => {
                        if *was_intelligent_autosuggestion {
                            self.was_intelligent_autosuggestion_accepted = true;
                        }
                        // Switch to AI input mode but preserve current lock state when accepting an Agent Mode query autosuggestion.
                        self.enter_ai_mode(ctx);
                        self.ai_context_model.update(ctx, |context_model, ctx| {
                            context_model.set_pending_context_block_ids(
                                context_block_ids.clone(),
                                true,
                                ctx,
                            )
                        });
                    }
                };
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.editor_up(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.editor_down(ctx);
            }
            EditorEvent::Navigate(NavigationKey::PageUp) => {
                self.editor_page_up(ctx);
            }
            EditorEvent::Navigate(NavigationKey::PageDown) => {
                self.editor_page_down(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.input_tab(ctx);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.input_shift_tab(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Right) => {
                // If the AI context menu is open and we're at the end of the buffer,
                // make right arrow act like enter and select the current item
                if self.suggestions_mode_model.as_ref(ctx).is_ai_context_menu() {
                    self.editor.update(ctx, |editor, ctx| {
                        if let Some(ai_context_menu) = editor.ai_context_menu() {
                            ai_context_menu.update(ctx, |menu, ctx| {
                                menu.select_current_item(ctx);
                            });
                        }
                    });
                }
            }
            EditorEvent::Enter => self.input_enter(ctx),
            EditorEvent::CmdEnter => self.input_cmd_enter(ctx),
            EditorEvent::CtrlEnter => {
                ctx.emit(Event::CtrlEnter);
            }
            EditorEvent::Escape => self.editor_escape(ctx),
            EditorEvent::CtrlC { cleared_buffer_len } => {
                self.close_input_suggestions(/*should_focus_input=*/ true, ctx);

                self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                    ai_input_model.set_input_config_for_classic_mode(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: true,
                        }
                        .unlocked_if_autodetection_enabled(false, ctx),
                        ctx,
                    );
                });
                ctx.emit(Event::CtrlC {
                    cleared_buffer_len: *cleared_buffer_len,
                });
            }
            EditorEvent::DeleteAllLeft => {
                if self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                    let new_input_type = InputType::Shell;
                    self.maybe_send_autodetection_telemetry_on_manual_toggle(new_input_type, ctx);
                    self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                        ai_input_model.set_input_config_for_classic_mode(
                            InputConfig {
                                input_type: new_input_type,
                                is_locked: true,
                            }
                            .unlocked_if_autodetection_enabled(false, ctx),
                            ctx,
                        );
                    });
                } else if self.ai_input_model.as_ref(ctx).is_input_type_locked() {
                    let is_cli_agent_input_open =
                        CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
                    let is_agent_view_fullscreen =
                        self.agent_view_controller.as_ref(ctx).is_fullscreen();
                    if is_agent_view_fullscreen || is_cli_agent_input_open {
                        self.exit_shell_mode_to_ai(ctx);
                    }
                }
            }
            EditorEvent::CmdUpOnFirstRow => ctx.emit(Event::SelectRecentBlocks { count: 1 }),
            EditorEvent::Copy => ctx.emit(Event::Copy),
            EditorEvent::UnhandledModifierKeyOnEditor(keystroke) => {
                ctx.emit(Event::UnhandledModifierKeyOnEditor(keystroke.clone()))
            }
            EditorEvent::ClearParentSelections => {
                ctx.emit(Event::ClearSelectionsWhenShellMode);
            }
            EditorEvent::HideXRay => {
                self.hide_x_ray(ctx);
            }
            EditorEvent::TryToShowXRay(token_at) => {
                if self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                    // Don't show command x-ray for AI queries.
                    return;
                }

                match token_at {
                    CommandXRayAnchor::Cursor => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CommandXRayTriggered {
                                trigger: CommandXRayTrigger::Keystroke
                            },
                            ctx
                        );
                        let pos = self.start_byte_index_of_first_selection(ctx);
                        self.start_xray_at_offset(pos, CommandXRayTrigger::Keystroke, ctx);
                    }
                    CommandXRayAnchor::Hover(mouse_position) => {
                        if let Some(offset) = self.start_byte_index_at_point(mouse_position, ctx) {
                            if !self.suggestions_mode_model.as_ref(ctx).is_visible() {
                                send_telemetry_from_ctx!(
                                    TelemetryEvent::CommandXRayTriggered {
                                        trigger: CommandXRayTrigger::Hover
                                    },
                                    ctx
                                );
                                self.start_xray_at_offset(offset, CommandXRayTrigger::Hover, ctx);
                            }
                        }
                    }
                }
            }
            EditorEvent::InsertLastWordPrevCommand => self.insert_last_word_previous_command(ctx),
            // For this particular view, the terminal Input, we ignore search direction because in
            // this context, search means search through History which isn't actually sensitive to
            // left/right direction.
            EditorEvent::Search { term, .. } => {
                ctx.emit(Event::ShowCommandSearch(CommandSearchOptions {
                    filter: Some(QueryFilter::History),
                    init_content: InitContent::Custom(term.clone().unwrap_or("".to_owned())),
                }));
            }
            // For this view, the terminal Input, we do not support ex-commands. The closest
            // analogy we have in this view would be workflows. So, open command search with the
            // workflows filter to handle this event.
            EditorEvent::ExCommand => ctx.emit(Event::ShowCommandSearch(CommandSearchOptions {
                filter: Some(QueryFilter::Workflows),
                init_content: InitContent::Custom("".to_owned()),
            })),
            EditorEvent::VimStatusUpdate => ctx.notify(),
            EditorEvent::BackspaceOnEmptyBuffer | EditorEvent::BackspaceAtBeginningOfBuffer => {
                self.handle_backspace_at_buffer_boundary(ctx);
            }
            EditorEvent::EmacsBindingUsed => {
                ctx.emit(Event::EmacsBindingUsed);
            }
            EditorEvent::UpdatePeers { operations } => {
                self.latest_buffer_operations.extend(operations.to_vec());

                // TODO (suraj): we might want to push down the buffer ID to the buffer
                // and have it returned as part of the event. That way, we aren't subject
                // to any skew of the block ID from the time the event is emitted (when the edit
                // is processed) to the time when we query the block ID (now).
                ctx.emit(Event::EditorUpdated {
                    block_id: self.model.lock().block_list().active_block_id().clone(),
                    operations: operations.clone(),
                })
            }
            EditorEvent::MiddleClickPaste => {
                ctx.emit(Event::InputFocusedFromMiddleClick);
            }
            EditorEvent::Focused => ctx.emit(Event::EditorFocused),
            EditorEvent::ProcessingAttachedImages(is_processing) => {
                self.set_is_processing_attached_images(*is_processing, ctx);
            }
            EditorEvent::VoiceStateUpdated {
                is_listening,
                is_transcribing,
            } => {
                self.universal_developer_input_button_bar
                    .update(ctx, |button_bar, ctx| {
                        button_bar.set_voice_is_listening(*is_listening, ctx);
                    });
                self.agent_input_footer.update(ctx, |footer, ctx| {
                    footer.set_voice_is_active(*is_listening || *is_transcribing, ctx);
                });

                if *is_listening || *is_transcribing {
                    // Show voice status as placeholder when the buffer is empty.
                    if self.editor.as_ref(ctx).is_empty(ctx) {
                        let placeholder = if *is_listening {
                            "Listening..."
                        } else {
                            "Transcribing..."
                        };
                        self.editor.update(ctx, |editor, ctx| {
                            editor.set_placeholder_text(placeholder, ctx);
                        });
                    }
                } else {
                    self.set_zero_state_hint_text(ctx);
                }
            }
            EditorEvent::SetAIContextMenuOpen(open) => {
                self.set_ai_context_menu_open(*open, ctx);
            }
            EditorEvent::SelectAIContextMenuCategory { .. } => {
                // Get the at_symbol_position and clear the text
                if let Some(at_pos) = if let InputSuggestionsMode::AIContextMenu {
                    at_symbol_position,
                    ..
                } = self.suggestions_mode_model.as_ref(ctx).mode()
                {
                    Some(*at_symbol_position)
                } else {
                    None
                } {
                    let cursor_position = self.editor.read(ctx, |editor, ctx| {
                        editor.start_byte_index_of_last_selection(ctx)
                    });

                    // Delete text from @ to cursor using system delete
                    let replacement_range =
                        ByteOffset::from(at_pos + 1)..ByteOffset::from(cursor_position.as_usize());
                    self.editor.update(ctx, |editor, ctx| {
                        editor.system_delete(replacement_range, ctx);
                    });
                }
            }
            EditorEvent::AcceptAIContextMenuItem(action) => {
                // Handle different action types
                match action {
                    AIContextMenuSearchableAction::InsertText { text } => {
                        // Only enter AI mode if we're in autodetect mode (not locked in terminal mode)
                        if self
                            .ai_input_model
                            .as_ref(ctx)
                            .should_run_input_autodetection(ctx)
                        {
                            self.enter_ai_mode(ctx);
                        }

                        // For InsertText, we replace the "@" and any filter text with the provided text
                        self.replace_at_symbol_with_text(text, ctx);
                    }
                    AIContextMenuSearchableAction::InsertFilePath { file_path } => {
                        // Handle file/directory path insertion
                        let is_ai_mode = self.ai_input_model.as_ref(ctx).is_ai_input_enabled();
                        let file_path = if is_ai_mode {
                            file_path.to_string()
                        } else {
                            #[cfg(feature = "local_fs")]
                            {
                                // Try to get current working directory and process the file path
                                let processed_path = self
                                    .active_block_metadata
                                    .as_ref()
                                    .and_then(BlockMetadata::current_working_directory)
                                    .and_then(|pwd| {
                                        // Find git repo and construct absolute path
                                        use repo_metadata::repositories::DetectedRepositories;
                                        use warp_util::local_or_remote_path::LocalOrRemotePath;
                                        let git_repo_path = DetectedRepositories::as_ref(ctx)
                                            .get_root_for_path(&LocalOrRemotePath::Local(
                                                Path::new(pwd).to_path_buf(),
                                            ))
                                            .and_then(|r| PathBuf::try_from(r).ok())?;
                                        let absolute_path = git_repo_path.join(file_path);

                                        // Try to get relative path if it's shorter
                                        let is_wsl = self
                                            .active_session(ctx)
                                            .map(|session| session.is_wsl())
                                            .unwrap_or(false);

                                        let relative_path = warp_util::path::to_relative_path(
                                            is_wsl,
                                            &absolute_path,
                                            Path::new(pwd),
                                        );

                                        match relative_path {
                                            Some(rel)
                                                if rel.len()
                                                    < absolute_path.to_string_lossy().len() =>
                                            {
                                                Some(rel)
                                            }
                                            _ => Some(absolute_path.to_string_lossy().to_string()),
                                        }
                                    });

                                processed_path.unwrap_or_else(|| file_path.to_string())
                            }

                            #[cfg(not(feature = "local_fs"))]
                            file_path.to_string()
                        };
                        self.replace_at_symbol_with_text(&file_path, ctx);
                    }
                    AIContextMenuSearchableAction::InsertDriveObject {
                        object_type,
                        object_uid,
                    } => {
                        // For InsertDriveObject, format as <object_type:uid> and replace the "@" and any filter text
                        let drive_object_text = format!("<{object_type}:{object_uid}>");
                        self.replace_at_symbol_with_text(&drive_object_text, ctx);
                    }
                    AIContextMenuSearchableAction::InsertPlan { ai_document_uid } => {
                        // For InsertPlan, format as <plan:uid> and replace the "@" and any filter text
                        let ai_document_text = format!("<plan:{ai_document_uid}>");
                        self.replace_at_symbol_with_text(&ai_document_text, ctx);
                    }
                    AIContextMenuSearchableAction::InsertConversation { conversation_id } => {
                        let conversation_text = format!("<convo:{conversation_id}>");
                        self.replace_at_symbol_with_text(&conversation_text, ctx);
                    }
                    AIContextMenuSearchableAction::InsertDiffSet { diff_mode } => {
                        // Emit event to the TerminalView to attach the diff set
                        ctx.emit(Event::AttachDiffSetContext {
                            diff_mode: diff_mode.clone(),
                        });
                    }
                    AIContextMenuSearchableAction::InsertSkill { name } => {
                        self.replace_at_symbol_with_text(&format!("/{name}"), ctx);
                    }
                }
                self.close_ai_context_menu(ctx);
            }
            EditorEvent::Paste => {
                self.process_paste_event(ctx);
            }
            EditorEvent::DroppedImageFiles(image_filepaths) => {
                // Handle image processing from EditorView drag-and-drop
                let num_attached =
                    self.handle_pasted_or_dragdropped_image_filepaths(image_filepaths.clone(), ctx);

                // If any attachment failed, insert all dropped image paths as text. Apply the
                // same session-aware path transformation that the editor uses for dropped
                // non-image paths so the fallback matches the primary drop flow (e.g.
                // `/mnt/c/...` in a WSL session).
                if num_attached < image_filepaths.len() {
                    let shell_family = self.editor.read(ctx, |editor, _| editor.shell_family());
                    let converter = self
                        .active_session(ctx)
                        .as_deref()
                        .and_then(Session::windows_path_converter);
                    let transformed: Vec<String> = match converter {
                        Some(convert) => image_filepaths.iter().map(|p| convert(p)).collect(),
                        None => image_filepaths.clone(),
                    };
                    let paths_str =
                        warpui::clipboard_utils::escaped_paths_str(&transformed, shell_family);

                    self.editor.update(ctx, |editor, ctx| {
                        editor.user_insert(&paths_str, ctx);
                    });
                }
            }
            EditorEvent::IgnoreAutosuggestion { suggestion } => {
                IgnoredSuggestionsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.add_ignored_suggestion(
                        suggestion.clone(),
                        SuggestionType::ShellCommand,
                        ctx,
                    );
                });

                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_autosuggestion(ctx);
                });
            }
            _ => {}
        }
    }

    /// Process paste event by checking clipboard for images and handling appropriately.
    fn process_paste_event(&mut self, ctx: &mut ViewContext<Self>) {
        // Read from app clipboard
        let content = ctx.clipboard().read();

        // If AI is disabled, attachment isn't possible
        if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            self.insert_clipboard_text_content(ctx, content);
            return;
        }

        // Shared session viewers cannot attach images unless in cloud mode
        let is_viewer = self.model.lock().shared_session_status().is_viewer();
        let is_cloud_mode_with_images = FeatureFlag::CloudModeImageContext.is_enabled()
            && self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model.as_ref(ctx).is_ambient_agent()
                });
        if is_viewer && !is_cloud_mode_with_images {
            self.insert_clipboard_text_content(ctx, content);
            return;
        }

        // Check if we should insert clipboard text in advance
        let mut already_inserted_text = false;
        if warpui::clipboard::should_insert_text_on_paste(&content) {
            self.insert_clipboard_text_content(ctx, content.clone());
            already_inserted_text = true;
        }

        // Try to attach images
        // If any attachment fails, should_insert_text = true.
        let should_insert_text = if content.has_image_data() {
            // If we have image data, process the image data.
            self.handle_pasted_image_data(content.clone(), ctx) == 0
        } else if content.num_paths() > 0 {
            // Else, we check the pasted file paths for any images.
            let image_filepaths = warpui::clipboard_utils::get_image_filepaths_from_paths(
                content.paths.as_deref().unwrap_or(&[]),
            );
            let num_images_expected = image_filepaths.len();
            self.handle_pasted_or_dragdropped_image_filepaths(image_filepaths, ctx)
                < num_images_expected
        } else {
            true
        };

        // Fallback to inserting text
        if should_insert_text && !already_inserted_text {
            self.insert_clipboard_text_content(ctx, content);
        }
    }

    /// Insert clipboard text content (paths / plaintext)
    fn insert_clipboard_text_content(
        &self,
        ctx: &mut ViewContext<Self>,
        content: ClipboardContent,
    ) {
        let clipboard_content_str = self
            .editor
            .read(ctx, |editor, _| editor.clipboard_text_content(content));
        self.editor.update(ctx, |editor, ctx| {
            editor.user_initiated_insert(
                &clipboard_content_str,
                PlainTextEditorViewAction::Paste,
                ctx,
            );
        });
    }

    /// Check if we can attach on filepaths paste or drag-drop
    fn can_attach_on_filepaths_paste_or_dragdrop(&self, ctx: &mut ViewContext<Self>) -> bool {
        // Shared session viewers cannot attach images unless in cloud mode
        // with the CloudModeImageContext feature enabled.
        let is_viewer = self.model.lock().shared_session_status().is_viewer();
        let is_cloud_mode_with_images = FeatureFlag::CloudModeImageContext.is_enabled()
            && self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model.as_ref(ctx).is_ambient_agent()
                });
        if is_viewer && !is_cloud_mode_with_images {
            return false;
        }

        // CLI agent rich input always supports image attachment, independent of
        // the UDI setting or the `AgentView` feature flag. Its own composer
        // gates image chips on `ImageAsContext` + an active CLI agent session.
        let is_cli_agent_input_open =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
        if is_cli_agent_input_open {
            return true;
        }

        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        if !is_udi_enabled && !FeatureFlag::AgentView.is_enabled() {
            return false;
        }

        // Check if Agent Mode enabled, in active agent view, or if the buffer is empty
        // (if the buffer is empty, we assume that the user wants the images to be attached).
        let ai_input = self.ai_input_model.as_ref(ctx);
        let in_agent_mode = matches!(ai_input.input_type(), InputType::AI);
        let is_buffer_empty = self.buffer_text(ctx).is_empty();
        let in_active_agent_view = self.agent_view_controller.as_ref(ctx).is_active();
        in_agent_mode || is_buffer_empty || in_active_agent_view
    }

    /// Handle direct image data from clipboard (e.g., copied images). Returns number of images attached.
    fn handle_pasted_image_data(
        &mut self,
        clipboard_content: ClipboardContent,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        if self.check_image_limits_for_paste(1, ctx) == 0 {
            return 0;
        }

        if let Some(images) = clipboard_content.images {
            let best_image = CLIPBOARD_IMAGE_MIME_TYPES
                .iter()
                .find_map(|format| images.iter().find(|img| img.mime_type == *format));

            if let Some(image) = best_image {
                self.process_and_attach_clipboard_image(image.clone(), ctx);
                return 1;
            }
        }

        0
    }

    /// Handle pasted file paths that point to images for auto-attachment. Returns number of images attached.
    pub fn handle_pasted_or_dragdropped_image_filepaths(
        &mut self,
        image_filepaths: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        // Return early if no image paths
        if image_filepaths.is_empty() {
            return 0;
        }

        if !self.can_attach_on_filepaths_paste_or_dragdrop(ctx) {
            return 0;
        }

        self.maybe_enter_agent_view_for_image_add(ctx);

        let num_images_to_attach = self.check_image_limits_for_paste(image_filepaths.len(), ctx);
        if num_images_to_attach == 0 {
            return 0;
        }

        let is_buffer_empty = self.buffer_text(ctx).is_empty();
        let in_active_agent_view = self.agent_view_controller.as_ref(ctx).is_active();
        if is_buffer_empty || in_active_agent_view {
            self.set_input_mode_agent(true, ctx);
            self.update_image_context_options(ctx);
        }

        let paths_to_process: Vec<String> = image_filepaths
            .into_iter()
            .take(num_images_to_attach)
            .collect();

        let num_paths = paths_to_process.len();
        self.editor.update(ctx, |editor, ctx| {
            editor.read_and_process_images_async(num_paths, paths_to_process, ctx);
        });
        num_paths
    }

    /// Convert clipboard image data to AttachedImage and attach to editor in Agent Mode.
    fn process_and_attach_clipboard_image(
        &mut self,
        image: ImageData,
        ctx: &mut ViewContext<Self>,
    ) {
        self.maybe_enter_agent_view_for_image_add(ctx);

        // Switch to AI mode with block-level lock, unless already AI-mode-locked
        if !self.is_locked_in_ai_mode(ctx) {
            self.set_input_mode_agent(true, ctx);
            self.update_image_context_options(ctx);
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let ext = match image.mime_type.as_str() {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "img",
        };

        // Use preserved filename if available, otherwise generate fallback name
        let file_name = if let Some(original_filename) = &image.filename {
            original_filename.clone()
        } else {
            format!("pasted-image-{timestamp}.{ext}")
        };

        let attached_image = AttachedImageRawData {
            data: image.data,
            mime_type: image.mime_type,
            file_name,
        };

        self.editor.update(ctx, |editor, ctx| {
            editor.process_and_attach_images_as_ai_context(1, vec![attached_image], ctx);
        });
    }

    /// Enters agent view when adding images, unless the CLI agent rich input is
    /// open (which is already a composer context and doesn't use the agent view),
    /// Agent View is disabled, we're already in the agent view, or a long running
    /// command is in progress.
    fn maybe_enter_agent_view_for_image_add(&mut self, ctx: &mut ViewContext<Self>) {
        let is_cli_agent_input_open =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
        if is_cli_agent_input_open {
            return;
        }

        let is_in_long_running_command = self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_active_and_long_running();
        if !FeatureFlag::AgentView.is_enabled()
            || self.agent_view_controller.as_ref(ctx).is_active()
            || is_in_long_running_command
        {
            return;
        }

        if let Err(e) = self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(None, AgentViewEntryOrigin::ImageAdded, ctx)
        }) {
            log::error!("Failed to enter agent view when adding images: {e:?}");
        }
    }

    /// Display an error toast for image paste operation failures.
    fn show_image_paste_error(&self, ctx: &mut ViewContext<Self>, message: String) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_persistent_toast(DismissibleToast::error(message), window_id, ctx);
        });
    }

    /// Check attachment limits, return attachable count (shows toast for excess).
    fn check_image_limits_for_paste(
        &self,
        num_images_to_add: usize,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let (num_images_attached, num_images_in_conversation) =
            self.editor.read(ctx, |editor, _| {
                (
                    editor.image_context_options.num_images_attached(),
                    editor.image_context_options.num_images_in_conversation(),
                )
            });

        // Calculate how many images we can add based on per-query limit
        let available_per_query = MAX_IMAGE_COUNT_FOR_QUERY.saturating_sub(num_images_attached);

        // Calculate how many images we can add based on per-conversation limit
        let total_images_current = num_images_attached + num_images_in_conversation;
        let available_per_conversation =
            MAX_IMAGES_PER_CONVERSATION.saturating_sub(total_images_current);

        // Take the more restrictive limit
        let max_attachable = available_per_query.min(available_per_conversation);

        // Determine how many we can actually attach
        let images_to_attach = num_images_to_add.min(max_attachable);
        let excess_images = num_images_to_add.saturating_sub(images_to_attach);

        // Show toast for excess images if any
        if excess_images > 0 {
            let (limit_name, limit_value) = if available_per_query < available_per_conversation {
                ("per query", MAX_IMAGE_COUNT_FOR_QUERY)
            } else {
                ("per conversation", MAX_IMAGES_PER_CONVERSATION)
            };

            let message = if excess_images == 1 {
                format!("1 image wasn't attached - limit is {limit_value} images {limit_name}.")
            } else {
                format!(
                    "{excess_images} images weren't attached - limit is {limit_value} images {limit_name}."
                )
            };
            self.show_image_paste_error(ctx, message);
        }

        images_to_attach
    }

    pub fn set_is_processing_attached_images(
        &mut self,
        is_processing_attached_images: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.is_processing_attached_images = is_processing_attached_images;
        self.update_image_context_options(ctx);
        ctx.notify();
    }

    /// Handles backspace at the buffer boundary (empty buffer or cursor at
    /// position 0). Covers prefix-mode exit (`&` and `!`) and legacy
    /// classic-mode AI icon toggling in a single function.
    fn handle_backspace_at_buffer_boundary(&mut self, ctx: &mut ViewContext<Self>) {
        match self.prefix_mode(ctx) {
            InputPrefixMode::CloudHandoff => {
                self.exit_cloud_handoff_compose(ctx);
                ctx.notify();
                return;
            }
            InputPrefixMode::Shell => {
                let is_cli_agent_input_open =
                    CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
                if self.agent_view_controller.as_ref(ctx).is_fullscreen() || is_cli_agent_input_open
                {
                    self.exit_shell_mode_to_ai(ctx);
                    ctx.notify();
                }
                return;
            }
            InputPrefixMode::None => {}
        }

        // Legacy classic-mode AI icon toggling (only reachable when Agent View
        // is inactive and no prefix mode is set).
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
            return;
        }

        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        if is_udi_enabled {
            return;
        }

        // When the agent view is active, the classic-mode AI icon toggling and follow-up clearing
        // logic below does not apply.
        if FeatureFlag::AgentView.is_enabled() && self.agent_view_controller.as_ref(ctx).is_active()
        {
            return;
        }

        // If we have an AI follow up icon, backspace should clear that icon.
        if self
            .ai_context_model
            .as_ref(ctx)
            .is_targeting_existing_conversation()
        {
            self.ai_context_model.update(ctx, |ai_context_model, ctx| {
                ai_context_model.set_pending_query_state_for_new_conversation(
                    // This origin is unused in this codepath, which doesn't get called when
                    // AgentView is enabled.
                    AgentViewEntryOrigin::Input {
                        was_prompt_autodetected: false,
                    },
                    ctx,
                );
            });
        } else {
            // Otherwise backspace away the AI icon.
            let new_input_type = self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                let new_input_config = ai_input_model.input_config().with_toggled_type().locked();
                let new_input_type = new_input_config.input_type;
                ai_input_model.set_input_config_for_classic_mode(new_input_config, ctx);
                new_input_type
            });
            self.maybe_send_autodetection_telemetry_on_manual_toggle(new_input_type, ctx);
        }
    }

    /// Updates the tab completion menu given the current text of the editor and location of the
    /// cursor. Returns whether the input suggestions should be closed.
    ///
    /// If the original text is still within the buffer up to where the cursor is, we filter the
    /// suggestions to only show the suggestions that match the current word. If the original text
    /// is _not_ within the buffer up to the cursor, we close the input suggestions.
    fn update_tab_completion_menu(
        &self,
        replacement_start: usize,
        buffer_text_original: &str,
        completion_results: &SuggestionResults,
        ctx: &mut ViewContext<Input>,
    ) -> bool {
        let editor_text = self.editor.as_ref(ctx).buffer_text(ctx);
        let cursor_position = self.start_byte_index_of_last_selection(ctx);
        let text_up_to_cursor = &editor_text[0..cursor_position.as_usize()];

        // If the cursor position is before the start of the replacement span,
        // then we should definitely close the menu.
        if cursor_position.as_usize() < replacement_start {
            return true;
        }

        // If the buffer no longer starts with the original buffer text,
        // then we should close the completion menu because the result set
        // was based on a different query.
        //
        // For classic completions, this is a poor heuristic: when you cycle
        // through fuzzy matches, the text up to the cursor might not start
        // with the original buffer text anymore.
        // TODO: there's a bug here where if you hit tab and backspace,
        // the result set won't go away (stale).
        if !text_up_to_cursor.starts_with(buffer_text_original)
            && !self.is_classic_completions_enabled(ctx)
        {
            // Close the input suggestions since the buffer was edited to no longer
            // contain the text that triggered tab completion.
            true
        } else {
            // The current word is everything from the start of the replacement to the
            // cursor
            let current_word = &editor_text[replacement_start..cursor_position.as_usize()];

            if self.is_classic_completions_enabled(ctx) {
                let current_selected_item =
                    self.input_suggestions.as_ref(ctx).get_selected_item_text();
                if current_selected_item.is_some_and(|selected| selected == current_word) {
                    // If we're in classic completion mode and the selected item is equal
                    // to the current word, then we should keep the menu open; the user is cycling.
                    // We early-return because we don't want to filter the menu based on the
                    // selected item.
                    return false;
                }
            }

            // If the user continues to type with the tab suggestions open, we perform a
            // prefix search on the original results to filter the suggestions.
            let should_close = self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.prefix_search_for_tab_completion(
                    current_word,
                    completion_results,
                    TabCompletionsPreselectOption::Unchanged,
                    ctx,
                );

                // We should close the menu if there aren't any results
                // after filtering.
                suggestions.items().is_empty()
            });

            ctx.notify();
            should_close
        }
    }

    fn clear_screen(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.lock().clear_visible_screen();
        ctx.notify();
    }

    /// Attempts to write the EOT (End-of-Transmission) char to the PTY, which is canonically mapped
    /// to Ctrl-D. If successful, the session is terminated.
    fn ctrl_d(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::CtrlD);
    }

    fn ctrl_r(&mut self, ctx: &mut ViewContext<Self>) {
        if self.suggestions_mode_model.as_ref(ctx).is_history_up() {
            // Iterate through menu if we're already in history substring mode and
            // the user hits ctrl-r.
            self.input_suggestions
                .update(ctx, |input_suggestions, ctx| {
                    input_suggestions.select_prev(ctx);
                });
        } else if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
            self.fuzzy_history_search(ctx);
        }
    }

    fn fuzzy_history_search(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.can_query_history(ctx) {
            return;
        }

        self.focus_input_box(ctx);

        let editor = self.editor.as_ref(ctx);

        let original_cursor_point = editor.single_cursor_to_point(ctx);

        // Although we don't use suggestions_mode_model when using Voltron,
        // we still close the input suggestion menu before opening the Voltron modal,
        // which involves resetting the cursor point.
        let original_buffer = editor.buffer_text(ctx);
        let original_input_type = self.ai_input_model.as_ref(ctx).input_type();
        let original_input_was_locked = self.ai_input_model.as_ref(ctx).is_input_type_locked();
        self.suggestions_mode_model.update(ctx, |m, ctx| {
            m.set_mode(
                InputSuggestionsMode::HistoryUp {
                    original_buffer,
                    original_cursor_point,
                    search_mode: HistorySearchMode::Fuzzy,
                    original_input_type,
                    original_input_was_locked,
                },
                ctx,
            );
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::OpenSuggestionsMenu(
                self.suggestions_mode_model
                    .as_ref(ctx)
                    .mode()
                    .to_telemetry_mode(),
            ),
            ctx
        );

        self.select_and_refresh_voltron(VoltronItem::History, ctx);

        ctx.notify();
    }

    pub fn on_session_share_joined(
        &mut self,
        replica_id: ReplicaId,
        presence_manager: ModelHandle<PresenceManager>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Shared session history model should only be set if we are a viewer
        debug_assert!(self.model.lock().shared_session_status().is_viewer());
        self.set_shared_session_presence_manager(presence_manager);

        // Set the history model which is only available for a shared session viewer.
        let history_model = ctx.add_model(|_| SharedSessionHistoryModel::new());
        self.shared_session_input_state = Some(SharedSessionInputState {
            history_model,
            pending_command_execution_request: None,
        });

        // Set the server-assigned replica ID on the input buffer.
        self.editor().update(ctx, |editor, ctx| {
            editor.reinitialize_buffer(Some(replica_id), ctx);
        });
    }

    /// Returns a collection of history entries that are shell commands from
    /// the shared session (run on the sharer's machine).
    fn shared_session_history<'b>(
        &'b self,
        ctx: &'b ViewContext<Self>,
    ) -> Vec<HistoryInputSuggestion<'b>> {
        let Some(history_model) = self
            .shared_session_input_state
            .as_ref()
            .map(|state| state.history_model.clone())
        else {
            return Vec::new();
        };

        let commands = history_model
            .as_ref(ctx)
            .entries()
            .map(|entry| HistoryInputSuggestion::Command { entry })
            .collect();
        // TODO: append viewer's local shell history
        commands
    }

    /// Returns a collection of history entries that are user AI queries or shell commands in order
    /// from oldest to most recent.
    fn collate_ai_and_command_history<'a>(
        &'a self,
        ctx: &'a ViewContext<Self>,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        let input_config = self.ai_input_model.as_ref(ctx).input_config();
        let config = UpArrowHistoryConfig::for_input_config(&input_config);

        History::as_ref(ctx).up_arrow_suggestions_for_terminal_view(
            self.terminal_view_id,
            self.active_block_session_id(),
            config,
            ctx,
        )
    }

    fn update_last_word_insertion_state(&mut self) {
        // If an `InsertLastWordPrevCommand` action is received, its handler method will set
        // `is_latest_editor_event` on `self.last_word_insertion` to true, marking the following
        // EditorEvent (buffer edited) received is from this insertion.
        //
        // Any other editor event means the following "last word" insert is not consecutive, so
        // index is reset - the following insert will insert last word from most recent command
        // in history, index 0 (After that, a consecutive insertion would increment to index 1,
        // last word of second last command in history).
        //
        // If the last event was a last word insertion, we increment the
        // `insert_command_from_history_index` on `self.last_word_insertion` to indicate
        // consecutive inserts may be made (if so, insert from next earlier command in history).
        // We then set `is_latest_editor_event` to false for the following editor event; if another
        // last word insertion occurs, it is responsible for re-setting this boolean to true.
        if self.last_word_insertion.is_latest_editor_event {
            self.last_word_insertion.insert_command_from_history_index += 1;
            self.last_word_insertion.is_latest_editor_event = false;
        } else {
            self.last_word_insertion.insert_command_from_history_index = 0;
        }
    }

    fn history_commands<'b>(&self, ctx: &'b ViewContext<Input>) -> Vec<&'b HistoryEntry> {
        self.active_block_session_id()
            .map_or_else(Vec::new, |session_id| {
                History::as_ref(ctx)
                    .commands(session_id)
                    .unwrap_or_default()
            })
    }

    fn insert_last_word_previous_command(&mut self, ctx: &mut ViewContext<Input>) {
        if let Some(word_to_insert) = self.get_last_word_of_command_in_history(
            self.last_word_insertion.insert_command_from_history_index,
            ctx,
        ) {
            self.editor.update(ctx, |editor, ctx| {
                editor.insert_selected_text_to_buffer_ignoring_undo(&word_to_insert, ctx);
            });

            self.last_word_insertion.is_latest_editor_event = true;
        }
    }

    fn get_last_word_of_command_in_history(
        &mut self,
        command_history_index: usize,
        ctx: &mut ViewContext<Input>,
    ) -> Option<String> {
        let commands = self.history_commands(ctx);
        if commands.is_empty() {
            return None;
        }

        let view_command_idx = commands.len().saturating_sub(1 + command_history_index);
        let view_command = commands[view_command_idx];

        let last_word = view_command
            .command
            .rsplit_once(' ')
            .map(|(_, last_word)| last_word)
            .unwrap_or(&view_command.command);

        Some(last_word.to_string())
    }

    /// We only want to show the completions while typing menu when the cursor is
    /// positioned at the end of the buffer text
    fn is_cursor_in_valid_position_for_completions_while_typing(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let editor = self.editor.as_ref(ctx);
        editor.single_cursor_at_buffer_end(false /* respect_line_cap */, ctx)
    }

    fn should_show_completions_while_typing(&self, ctx: &mut ViewContext<Self>) -> bool {
        let editor = self.editor.as_ref(ctx);
        let buffer_text = editor.buffer_text(ctx);

        self.is_completions_while_typing_turned_on(ctx)
            && (!self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
                || should_show_completions_in_ai_input(&buffer_text))
            && buffer_text.len() >= MIN_BUFFER_LEN_TO_SHOW_COMPLETIONS_WHILE_TYPING
            && self.is_cursor_in_valid_position_for_completions_while_typing(ctx)
    }

    fn is_completions_while_typing_turned_on(&self, app: &AppContext) -> bool {
        *InputSettings::as_ref(app)
            .completions_open_while_typing
            .value()
    }

    /// Returns true if an AI context menu should be enabled at the current cursor position based
    /// on the buffer text and surrounding context. This is triggered when the user just typed '@'
    /// in a valid context and the menu is not disabled for other reasons.
    fn should_enable_ai_context(
        &self,
        buffer_text: &str,
        cursor_position: usize,
        is_alias_expansion_enabled: bool,
        session_context: Option<&SessionContext>,
        shell_family: ShellFamily,
        app: &AppContext,
    ) -> bool {
        if cursor_position == 0 {
            return false;
        }

        if buffer_text.chars().nth(cursor_position.saturating_sub(1)) != Some('@') {
            return false;
        }

        // Check if '@' is at beginning of line or after non-alphanumeric
        let is_valid_context = if cursor_position == 1 {
            true // '@' is the first character
        } else {
            buffer_text
                .chars()
                .nth(cursor_position.saturating_sub(2))
                .is_some_and(|c| !c.is_alphanumeric())
        };

        if !is_valid_context {
            return false;
        }

        let is_disabled = AtContextMenuDisabledReason::get_disable_reason(
            self.active_block_metadata.as_ref(),
            self.sessions.as_ref(app),
            &self.ai_input_model.as_ref(app).input_config(),
            app,
        )
        .is_some();

        if is_disabled {
            return false;
        }

        // Don't trigger in shell mode for common package installer prefixes, where '@' is valid input.
        let is_shell_mode = !self.ai_input_model.as_ref(app).is_ai_input_enabled();
        let looks_like_package_install = is_shell_mode
            && command_at_cursor_has_common_package_installer_prefix(
                buffer_text,
                cursor_position - 1,
                shell_family,
                is_alias_expansion_enabled,
                session_context,
            );

        !looks_like_package_install
    }

    fn is_classic_completions_enabled(&self, ctx: &AppContext) -> bool {
        (FeatureFlag::ClassicCompletions.is_enabled()
            && *InputSettings::as_ref(ctx).classic_completions_mode)
            || FeatureFlag::ForceClassicCompletions.is_enabled()
    }

    fn should_expand_aliases(&self, ctx: &mut ViewContext<Self>) -> bool {
        // Never expand aliases when in AI input mode, regardless of the setting.
        if self.ai_input_model.as_ref(ctx).input_type().is_ai() {
            return false;
        }
        *AliasExpansionSettings::as_ref(ctx)
            .alias_expansion_enabled
            .value()
    }

    fn open_completion_suggestions(
        &mut self,
        completions_trigger: CompletionsTrigger,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
            self.close_slash_commands_menu(ctx);
        }

        let editor = self.editor.as_ref(ctx);
        let buffer_text = editor.buffer_text(ctx);

        let is_command_grid_active = {
            let model = self.model.lock();
            !model.is_alt_screen_active()
                && model.block_list().active_block().is_command_grid_active()
        };

        // CLI agent rich input in shell mode (! prefix) should allow completions
        // even though the active block is a long-running command.
        // However, completions are disabled on warpified remote hosts because
        // in-band generators don't work in this context (with CLI agent).
        let is_cli_agent_shell_mode = self.is_locked_in_shell_mode(ctx)
            && CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id)
            && !self
                .active_session(ctx)
                .is_some_and(|s| matches!(s.session_type(), SessionType::WarpifiedRemote { .. }));

        // If the cursor is in a valid completion position, go into CompletionSuggestions mode
        if (is_command_grid_active || is_cli_agent_shell_mode) && self.can_query_history(ctx) {
            let matcher = MatchStrategy::Fuzzy;

            if let Some(completion_context) = self.completion_session_context(ctx) {
                let cursor_position = self.start_byte_index_of_last_selection(ctx);
                let before_cursor_text = buffer_text[..cursor_position.as_usize()].to_owned();
                let editor_model = self.editor.read(ctx, |view, ctx| view.snapshot_model(ctx));

                self.run_completions_async(
                    before_cursor_text,
                    matcher,
                    completions_trigger,
                    editor_model,
                    cursor_position,
                    completion_context,
                    ctx,
                );
            }
        }
    }

    /// _Asynchronously_ generates completions by calling into the completer.
    #[allow(clippy::too_many_arguments)]
    fn run_completions_async(
        &mut self,
        before_cursor_text: String,
        matcher: MatchStrategy,
        completions_trigger: CompletionsTrigger,
        editor_snapshot: EditorSnapshot,
        cursor_position: ByteOffset,
        completion_context: SessionContext,
        ctx: &mut ViewContext<'_, Input>,
    ) {
        let buffer_text = self.buffer_text(ctx);

        // The 'ForceNativeShellCompletions' user pref can be used to unconditionally
        // generate and show native shell completion results (i.e. regardless of whether or
        // not we have completion results via completion specs).
        let force_native_shell_completions = ctx
            .private_user_preferences()
            .read_value("ForceNativeShellCompletions")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(false);

        let use_native_shell_completions = (FeatureFlag::NativeShellCompletions.is_enabled() || force_native_shell_completions)
            && completion_context
                .session
                .shell()
                .supports_native_shell_completions()
            // For now, don't use native shell completions for multi-line commands.
            && !buffer_text.contains('\n');

        let fallback_strategy = match completions_trigger {
            CompletionsTrigger::Keybinding if !use_native_shell_completions => {
                CompletionsFallbackStrategy::FilePaths
            }
            _ => CompletionsFallbackStrategy::None,
        };

        if self.is_completions_while_typing_turned_on(ctx) {
            if let Some(last_abort_handle) = self.completions_abort_handle.take() {
                last_abort_handle.abort();
            }
        }

        let input_type = self.ai_input_model.as_ref(ctx).input_type();

        // Don't trigger completions if the last character typed is whitespace, in AI input mode.
        // The user is likely typing in a natural language word at this point, not a filepath.
        if input_type.is_ai()
            && completions_trigger == CompletionsTrigger::AsYouType
            && before_cursor_text.ends_with(char::is_whitespace)
        {
            return;
        }

        let Some(session_id) = self.completer_data().active_block_session_id() else {
            return;
        };
        let session_env_vars = self.sessions.read(ctx, |sessions, _| {
            sessions.get_env_vars_for_session(session_id)
        });

        let cursor_position = cursor_position.as_usize();
        let native_results_fut = if use_native_shell_completions {
            // If we're using native shell completions, construct a future that
            // will be resolved with any completions data provided by the shell.
            let (results_tx, results_rx) = async_channel::unbounded();
            ctx.dispatch_typed_action(&TerminalAction::RunNativeShellCompletions {
                buffer_text: buffer_text[0..cursor_position].to_owned(),
                results_tx,
            });
            async move { results_rx.recv().await.ok() }.boxed()
        } else {
            // If not, we can immediately say that there are no completion
            // results from the shell.
            futures::future::ready(None).boxed()
        };

        let completion_session = completion_context.session.clone();

        let abort_handle = ctx
            .spawn_abortable(
                async move {
                    let suggestions = completer::suggestions(
                        before_cursor_text.as_str(),
                        cursor_position,
                        session_env_vars.as_ref(),
                        CompleterOptions {
                            match_strategy: matcher,
                            fallback_strategy,
                            suggest_file_path_completions_only: input_type.is_ai(),
                            parse_quotes_as_literals: input_type.is_ai(),
                        },
                        &completion_context,
                    )
                    .await;

                    let suggestions = match suggestions {
                        Some(s) if !s.suggestions.is_empty() && !force_native_shell_completions => {
                            Some(s)
                        }
                        _ => native_results_fut.await.map(|results| {
                            let suggestions = results.into_iter().map(Into::into).collect_vec();

                            let token_end = cursor_position;
                            // Within the section of the buffer from the start
                            // to the end of this token...
                            let token_start = buffer_text[0..token_end]
                                // Find the last whitespace char before the token end.
                                .rfind(char::is_whitespace)
                                // If we find one, the token start is the next char.
                                .map(|pos| pos + 1)
                                // Otherwise, the start is the beginning of the buffer.
                                .unwrap_or_default();

                            SuggestionResults {
                                replacement_span: (token_start, token_end).into(),
                                suggestions,
                                match_strategy: MatchStrategy::Fuzzy,
                            }
                        }),
                    };

                    (suggestions, completions_trigger, editor_snapshot)
                },
                |input, (suggestions, completions_trigger, editor_model), ctx| {
                    input.handle_completion_suggestions_results(
                        suggestions,
                        completions_trigger,
                        editor_model,
                        ctx,
                    )
                },
                move |_, _| {
                    completion_session.cancel_active_commands();
                },
            )
            .abort_handle();

        self.completions_abort_handle = Some(abort_handle);
    }

    /// Asynchronously generates dynamic enum suggestions.
    fn get_enum_suggestions_async(
        &mut self,
        command: String,
        editor_snapshot: EditorSnapshot,
        ctx: &mut ViewContext<'_, Input>,
    ) {
        if let Some(completion_context) = self.completion_session_context(ctx) {
            self.suggestions_mode_model.update(ctx, |m, ctx| {
                m.set_dynamic_enum_status(DynamicEnumSuggestionStatus::Pending, ctx);
            });
            let abort_handle = ctx
                .spawn(
                    async move {
                        let variants = super::dynamic_enum_suggestions::run_dynamic_enum_command(
                            command.as_str(),
                            &completion_context,
                        )
                        .await;

                        (variants, editor_snapshot)
                    },
                    move |input, (variants, editor_model), ctx| {
                        input.handle_enum_completion_results(variants, editor_model, ctx);
                    },
                )
                .abort_handle();

            self.completions_abort_handle = Some(abort_handle);
            ctx.notify();
        }
    }

    /// When the command finishes running, update the input suggestions menu with the suggestions.
    fn handle_enum_completion_results(
        &mut self,
        results: anyhow::Result<Vec<String>>,
        editor_snapshot_when_completer_was_ran: EditorSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_editor_model = self
            .editor
            .read(ctx, |editor, ctx| editor.snapshot_model(ctx));

        let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
        // If the editor has changed since the completions trigger was hit-- noop since the
        // suggestions are no longer valid. Note that we purposely ignore attributes such as text
        // styles for the purposes of this check (we only care about the buffer text content and
        // the cursor selections state).
        if buffer_text != editor_snapshot_when_completer_was_ran.text()
            || current_editor_model.selections()
                != editor_snapshot_when_completer_was_ran.selections()
        {
            return;
        }

        let (variants, status) = match results {
            Ok(variants) => (variants, DynamicEnumSuggestionStatus::Success),
            Err(e) => {
                log::warn!("Failed to generate dynamic enum suggestions: {e:?}");
                (vec![], DynamicEnumSuggestionStatus::Failure)
            }
        };

        self.input_suggestions.update(ctx, |input, ctx| {
            input.set_enum_variants(variants.clone(), ctx);
        });

        if let InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
            menu_position,
            selected_ranges,
            cursor_point,
            command,
            ..
        } = self.suggestions_mode_model.as_ref(ctx).mode()
        {
            let updated_mode = InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                dynamic_enum_status: status,
                suggestions: variants,
                menu_position: *menu_position,
                selected_ranges: selected_ranges.clone(),
                cursor_point: *cursor_point,
                command: command.clone(),
            };
            self.suggestions_mode_model.update(ctx, |model, ctx| {
                model.set_mode(updated_mode, ctx);
            });
        }

        ctx.notify();
    }

    fn path_separators(&self, ctx: &AppContext) -> PathSeparators {
        self.active_session(ctx)
            .map(|session| session.path_separators())
            .unwrap_or(PathSeparators::for_os())
    }

    /// Returns the buffer point that the tab completion menu should be positioned relative to.
    /// If None, the menu should be positioned relative to the cursor.
    ///
    /// In regular completions mode, we want to dock the completions menu at the cursor.
    ///
    /// In classic completions mode, we want to dock the completions menu at the start of
    /// the replacement span*. This ensures that the menu doesn't jump around as the cursor
    /// moves when the user cycles through items in the menu.
    /// * The one edge case is when we're completing a file path. In this case, the menu
    ///   should be docked at the end of the last directory in the replacement span.
    ///   This is because the replacement span will include the entire file path.
    ///   For example, if the user types "cd app/D" and one of the completion display result is
    ///   "Documents", then the replacement span will be for "app/D" and the replacement will
    ///   be "app/Documents".
    fn tab_completions_menu_position(
        &self,
        results: &SuggestionResults,
        buffer_text_original: &str,
        ctx: &AppContext,
    ) -> Option<BufferPoint> {
        // In regular mode, the menu should be positioned at the cursor.
        if !self.is_classic_completions_enabled(ctx) {
            return None;
        }

        // Note: the replacement span is in terms of byte offsets.
        // But these byte offsets should correspond to valid char offsets.
        let start = results.replacement_span.start();
        let end = results.replacement_span.end();

        let all_results_are_file_completions = results
            .suggestions
            .iter()
            .all(|s| s.suggestion.file_type.is_some());

        let offset = if all_results_are_file_completions {
            // If all the results are file completions, let's find the last slash in the replacement
            // span and dock the completions menu right after it. We do this because the replacement
            // span of file path completions is relative to the beginning of the file path. For
            // example, if the user types "cd app/D" and one of the completion display result is
            // "Documents", then the replacement span will be for "app/D" and the replacement will
            // be "app/Documents".
            buffer_text_original
                .get(0..end)
                .and_then(|s| s.rfind(self.path_separators(ctx).all))
                .map(|i| i + 1)
                .unwrap_or(start)
        } else {
            start
        };

        let point = self
            .editor
            .as_ref(ctx)
            .point_for_offset(ByteOffset::from(offset), ctx);
        point.ok()
    }

    fn handle_completion_suggestions_results(
        &mut self,
        results: Option<SuggestionResults>,
        completions_trigger: CompletionsTrigger,
        editor_snapshot_when_completer_was_ran: EditorSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_editor_model = self
            .editor
            .read(ctx, |editor, ctx| editor.snapshot_model(ctx));

        let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
        // If the editor has changed since the completions trigger was hit-- noop since the
        // suggestions are no longer valid. Note that we purposely ignore attributes such as text
        // styles for the purposes of this check (we only care about the buffer text content and
        // the cursor selections state).
        if buffer_text != editor_snapshot_when_completer_was_ran.text()
            || current_editor_model.selections()
                != editor_snapshot_when_completer_was_ran.selections()
        {
            return;
        }

        match results {
            None => {
                // It's necessary to specifically set to closed in the case where we first
                // opened the tab menu and then keep typing
                self.suggestions_mode_model.update(ctx, |m, ctx| {
                    m.set_mode(InputSuggestionsMode::Closed, ctx);
                });
            }
            Some(results) if results.suggestions.is_empty() => {
                self.suggestions_mode_model.update(ctx, |m, ctx| {
                    m.set_mode(InputSuggestionsMode::Closed, ctx);
                });
            }
            Some(results) => {
                match (results.single_prefix_suggestion(), completions_trigger) {
                    (Some(only_prefix_suggestion), CompletionsTrigger::Keybinding) => {
                        // If there is exactly one prefix suggestion, just insert into the buffer.
                        self.insert_completion_result_into_editor(
                            only_prefix_suggestion.replacement(),
                            results.replacement_span.start(),
                            Executing::No,
                            ctx,
                        );
                    }
                    (_, completions_trigger) => {
                        let buffer_text_original = buffer_text
                            [0..self.start_byte_index_of_last_selection(ctx).as_usize()]
                            .to_string();

                        if completions_trigger == CompletionsTrigger::Keybinding {
                            if let Some(common_prefix) = longest_common_prefix(
                                results
                                    .suggestions
                                    .iter()
                                    .filter(|suggestion| {
                                        // Ignore fuzzy matches and case-insensitive matches
                                        // when calculating the longest common prefix, so we
                                        // are able to insert a common prefix more often.
                                        matches!(
                                            suggestion.match_type,
                                            Match::Prefix {
                                                is_case_sensitive: true
                                            } | Match::Exact {
                                                is_case_sensitive: true
                                            }
                                        )
                                    })
                                    .map(|suggestion| suggestion.replacement()),
                            ) {
                                // Insert the common prefix if it is longer than what the user has
                                // already typed. This check is necessary because the suggestions
                                // are case-insensitive, while the common prefix is necessarily
                                // case-sensitive. That can lead to the common prefix being shorter
                                // than the input, causing confusing behavior where the input is
                                // truncated. Also, only fill in the common prefix if the
                                // replacement itself is a prefix of the common prefix. If there
                                // are only fuzzy completions, then it's possible this is not the
                                // case, and we don't want to fill in the common prefix in that
                                // case.
                                let replacement_start = results.replacement_span.start();
                                let current_word = &buffer_text_original[replacement_start
                                    ..self.start_byte_index_of_last_selection(ctx).as_usize()];
                                if common_prefix.len() > results.replacement_span.distance()
                                    && common_prefix.starts_with(current_word)
                                {
                                    self.insert_completion_prefix_into_editor(
                                        ctx,
                                        common_prefix,
                                        results.replacement_span.start(),
                                    );
                                }
                            }
                        }

                        // If not using completions as you type, then
                        // clear any autosuggestions when tab completions are open.
                        // The autosuggestion will be repopulated when the menu is closed.
                        // We don't do this for completions as you type because the user would
                        // otherwise hardly see autosuggestons.
                        if FeatureFlag::RemoveAutosuggestionDuringTabCompletions.is_enabled()
                            && !self.is_completions_while_typing_turned_on(ctx)
                        {
                            self.editor.update(ctx, |view, ctx| {
                                view.clear_autosuggestion(ctx);
                            });
                        }

                        // Decide where to render the tab completion menu.
                        // If we're rendering it at a specific position, let's make sure
                        // that position exists in the position cache.
                        let position = self.tab_completions_menu_position(
                            &results,
                            &buffer_text_original,
                            ctx,
                        );
                        let menu_position = if let Some(position) = position {
                            self.editor.update(ctx, |editor, ctx| {
                                editor.cache_buffer_point(
                                    position,
                                    COMPLETIONS_START_OF_REPLACEMENT_SPAN_POSITION_ID,
                                    ctx,
                                );
                            });
                            TabCompletionsMenuPosition::AtStartOfReplacementSpan
                        } else {
                            TabCompletionsMenuPosition::AtLastCursor
                        };

                        self.suggestions_mode_model.update(ctx, |m, ctx| {
                            m.set_mode(
                                InputSuggestionsMode::CompletionSuggestions {
                                    replacement_start: results.replacement_span.start(),
                                    buffer_text_original,
                                    completion_results: results.clone(),
                                    trigger: completions_trigger,
                                    menu_position,
                                },
                                ctx,
                            );
                        });

                        send_telemetry_from_ctx!(
                            TelemetryEvent::OpenSuggestionsMenu(
                                self.suggestions_mode_model
                                    .as_ref(ctx)
                                    .mode()
                                    .to_telemetry_mode(),
                            ),
                            ctx
                        );

                        let preselect_option = if self.is_classic_completions_enabled(ctx) {
                            TabCompletionsPreselectOption::Unselected
                        } else {
                            TabCompletionsPreselectOption::First
                        };

                        self.input_suggestions
                            .update(ctx, |input_suggestions, ctx| {
                                input_suggestions.prefix_search_for_tab_completion(
                                    results.replacement_span.slice(&buffer_text),
                                    &results,
                                    preselect_option,
                                    ctx,
                                );
                            });
                    }
                }
            }
        }
        ctx.notify();
    }

    /// Replace the replacement with the common completion prefix. Note that completion prefix
    /// itself is not the completion result so we don't add a space.
    fn insert_completion_prefix_into_editor(
        &mut self,
        ctx: &mut ViewContext<Input>,
        completion_prefix: &str,
        replacement_start: usize,
    ) {
        self.editor.update(ctx, |input, ctx| {
            let cursor_end_offset = input.end_byte_index_of_last_selection(ctx);
            input.select_and_replace(
                completion_prefix,
                [ByteOffset::from(replacement_start)..cursor_end_offset],
                PlainTextEditorViewAction::AcceptCompletionSuggestion,
                ctx,
            );
        });
    }

    /// Replace the replacement with the completion result and potentially add a space after.
    fn insert_completion_result_into_editor(
        &mut self,
        completion_result: &str,
        replacement_start: usize,
        executing: Executing,
        ctx: &mut ViewContext<Input>,
    ) {
        let is_completions_as_you_type_enabled = self.is_completions_while_typing_turned_on(ctx);
        self.editor.update(ctx, |input, ctx| {
            let cursor_end_offset = input.end_byte_index_of_last_selection(ctx);

            // Add a space to the end if the end of the selection/replacement
            // is at the end of the buffer and the completion result doesn't end with a slash.
            // If completions as you type is turned on and classic completions is off, then
            // _don't_ add a space.
            let is_classic_completions_enabled = self.is_classic_completions_enabled(ctx);
            let replacement: Cow<str> = if (!is_completions_as_you_type_enabled
                || is_classic_completions_enabled)
                && cursor_end_offset.as_usize() == input.buffer_text(ctx).len()
                && !completion_result.ends_with(self.path_separators(ctx).main)
                && executing == Executing::No
            {
                format!("{completion_result} ").into()
            } else {
                completion_result.into()
            };

            input.select_and_replace(
                &replacement,
                [ByteOffset::from(replacement_start)..cursor_end_offset],
                PlainTextEditorViewAction::AcceptCompletionSuggestion,
                ctx,
            );
        });
        send_telemetry_from_ctx!(TelemetryEvent::TabSingleResultAutocompletion, ctx);
    }

    /// Whether the editor is in a state where we should tab complete instead of indenting text
    /// within the editor.
    /// The editor is considered in a state where we should tab complete if:
    ///     1) The buffer text is not empty.
    ///     2) The user is not actively selecting.
    ///     3) There is only a single selection and that selection does not take up the entire
    ///        buffer.
    fn cursor_positioned_for_completion(&self, ctx: &mut ViewContext<Self>) -> bool {
        let input = self.editor.as_ref(ctx);
        let buffer_text = input.buffer_text(ctx);

        // We can show the completion menu when there is a single cursor selection
        // and we aren't actively selecting.
        !buffer_text.trim_start().is_empty()
            && !input.is_selecting(ctx)
            && input.num_selections(ctx) == 1
            && !input.any_selections_span_entire_buffer(ctx)
    }

    /// Returns the index of the argument our cursor is currently on, if there is one,
    /// as well as any style runs computed for reuse in `highlight_selected_workflow_argument`
    fn get_current_argument(
        &self,
        ctx: &ViewContext<Self>,
    ) -> (Option<WorkflowArgumentIndex>, Vec<Range<ByteOffset>>) {
        // If we aren't in a workflow, return
        let Some(workflow_state) = &self.workflows_state.selected_workflow_state else {
            log::error!(
                "Tried to get the current argument when no workflow is loaded into the input",
            );
            return (None, Vec::new());
        };

        let cursor_position = self
            .editor
            .as_ref(ctx)
            .end_byte_index_of_last_selection(ctx);

        // Get the highlighted text style ranges, which are used to determine where the workflow arguments are
        let text_style_ranges = self.get_text_style_ranges_for_workflow(ctx);

        // Find a text range that contains the cursor position
        let highlight_index = text_style_ranges
            .iter()
            .position(|range| range.contains(&cursor_position));

        // Find the argument that corresponds with this highlight index
        let arg_index = highlight_index.and_then(|index| {
            workflow_state
                .argument_index_to_highlight_index
                .iter()
                .find(|(_, highlight)| highlight.contains(&index))
                .map(|(arg_index, _)| *arg_index)
        });

        (arg_index, text_style_ranges)
    }

    fn input_shift_tab(&mut self, ctx: &mut ViewContext<Self>) {
        match self.suggestions_mode_model.as_ref(ctx).mode() {
            // If the model selector is open and has multiple tabs,
            // shift + tab should cycle between them.
            InputSuggestionsMode::ModelSelector => {
                if self
                    .inline_model_selector_view
                    .update(ctx, |view, ctx| view.select_next_tab(ctx))
                {
                    return;
                }
            }
            // If the inline history menu is open and has multiple tabs,
            // shift + tab should cycle between them.
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    return;
                }
                if self
                    .inline_history_menu_view
                    .update(ctx, |view, ctx| view.select_next_tab(ctx))
                {
                    return;
                }
            }
            // If the conversation menu is open and has multiple tabs,
            // shift + tab should cycle between them.
            InputSuggestionsMode::ConversationMenu => {
                if self
                    .inline_conversation_menu_view
                    .update(ctx, |view, ctx| view.select_next_tab(ctx))
                {
                    return;
                }
            }
            // If we're in CompletionSuggestions mode, shift tab moves to the previous selection.
            InputSuggestionsMode::CompletionSuggestions { .. } => {
                self.input_suggestions.update(ctx, |suggestions, ctx| {
                    suggestions.select_prev(ctx);
                });
                return;
            }
            _ => {}
        }

        if let Some(workflows_info_view) = &self
            .workflows_state
            .selected_workflow_state
            .as_ref()
            .map(|state| &state.more_info_view)
        {
            // Get the index of the argument we are currently selecting, if it exists
            let (current_argument, text_style_ranges) = self.get_current_argument(ctx);

            workflows_info_view.update(ctx, |info_view, ctx| {
                // If we are selecting an argument, open that one
                if let Some(index) = current_argument {
                    info_view.selected_workflow_state.set_argument_index(index);
                }
                // If we were in history suggestion mode, select the first argument
                else if matches!(
                    self.suggestions_mode_model.as_ref(ctx).mode(),
                    InputSuggestionsMode::HistoryUp { .. }
                ) {
                    info_view
                        .selected_workflow_state
                        .set_argument_index(0.into());
                }
                // Otherwise, continue to cycle arguments
                else {
                    info_view.selected_workflow_state.increment_argument_index();
                }

                ctx.notify();
            });

            self.highlight_selected_workflow_argument(text_style_ranges, ctx);

            if let Some(a11y_text) = self.selected_workflow_a11y_text(ctx) {
                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    a11y_text,
                    WarpA11yRole::UserAction,
                ));
            }
        } else {
            self.editor.update(ctx, |input, ctx| input.unindent(ctx));
        }
    }

    pub fn completion_session_context(&self, ctx: &AppContext) -> Option<SessionContext> {
        self.active_block_session_id()
            .and_then(|active_block_session_id| {
                let current_session = self.sessions.as_ref(ctx).get(active_block_session_id);
                let pwd = self
                    .active_block_metadata
                    .as_ref()
                    .and_then(BlockMetadata::current_working_directory)
                    .map(str::to_owned);

                current_session.zip(pwd).map(|(current_session, pwd)| {
                    // TODO(abhishek): Ideally, BlockMetadata::current_working_directory should directly
                    // return a TypedPathBuf. This shouldn't happen here in the view.
                    let current_working_directory =
                        current_session.convert_directory_to_typed_path_buf(pwd);
                    SessionContext::new(
                        current_session,
                        CommandRegistry::global_instance(),
                        current_working_directory,
                        ctx,
                    )
                })
            })
    }

    pub fn active_session(&self, ctx: &AppContext) -> Option<Arc<Session>> {
        self.active_block_session_id()
            .and_then(|active_block_session_id| {
                self.sessions.as_ref(ctx).get(active_block_session_id)
            })
    }

    fn hide_x_ray(&mut self, ctx: &mut ViewContext<Self>) {
        if self.command_x_ray_description.take().is_some() {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_command_x_ray();
                ctx.notify();
            });
            ctx.notify();
        }
    }

    fn start_xray_at_offset(
        &mut self,
        pos: ByteOffset,
        trigger: CommandXRayTrigger,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(completion_context) = self.completion_session_context(ctx) {
            let buffer_text = self.buffer_text(ctx);
            let _ =
                ctx.spawn(
                    async move {
                        completer::describe(buffer_text.as_str(), pos, &completion_context).await
                    },
                    |input, description, ctx| {
                        input.show_xray(description, trigger, ctx);
                    },
                );
        }
    }

    fn show_xray(
        &mut self,
        description: Option<Description>,
        trigger: CommandXRayTrigger,
        ctx: &mut ViewContext<'_, Self>,
    ) {
        let description = description.map(Arc::new);
        self.command_x_ray_description.clone_from(&description);
        if let Some(description) = description {
            if trigger == CommandXRayTrigger::Keystroke {
                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    description.a11y_text(),
                    WarpA11yRole::UserAction,
                ));
            }
            ctx.notify();
            self.editor.update(ctx, move |editor, ctx| {
                editor.set_command_x_ray(description);
                ctx.notify();
            });
        }
        ctx.notify();
    }

    fn active_block_session_id(&self) -> Option<SessionId> {
        self.active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::session_id)
    }

    /// Handles a tab keypress from the editor.
    ///
    /// "Tab" is the default trigger to open the completion suggestions menu, but this may be
    /// overridden in settings. If the completion suggestions menu is already open, tab and
    /// shift-tab are used to select the next and previous suggestion, respectively -- this is not
    /// overridable; note that even if "open completion suggestions menu" is rebound to a non-tab
    /// key, tab and shift-tab are still used to navigate within the menu once it is open.
    ///
    /// If tab is not bound to "open completion suggestions menu" nor is the suggestions menu
    /// already open, inserts a tab char into the input editor.
    fn input_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::AIContextMenu { .. }
        ) {
            self.editor.update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |ai_context_menu, ctx| {
                        ai_context_menu.select_current_item(ctx);
                    });
                }
            });
            return;
        }
        // We have to manually check if "tab" is bound to
        // `InputAction::MaybeOpenCompletionSuggestions` here because the child `EditorView`
        // handles the actual tab keypress event -- the handler method attached to the
        // `EditableBinding` for `MaybeOpenCompletionSuggestions` is not called when the
        // binding is tab because the UI framework dictates that only one View may receive a
        // keypress event.
        let is_tab_bound_to_open_completions =
            bindings::keybinding_name_to_keystroke(OPEN_COMPLETIONS_KEYBINDING_NAME, ctx)
                .map(|keystroke| keystroke.key == "tab")
                .unwrap_or_default();

        let replacement_start_opt = if let InputSuggestionsMode::CompletionSuggestions {
            replacement_start,
            ..
        } = self.suggestions_mode_model.as_ref(ctx).mode()
        {
            Some(*replacement_start)
        } else {
            None
        };
        if let Some(replacement_start) = replacement_start_opt {
            // The completions menu is already open, in which there are two cases.
            // Case 1: There is a common prefix amongst filtered suggestions that we could fill; so
            //         we fill it in buffer.
            // Case 2: Else, tab should move to next option.
            let (common_prefix_of_filtered_suggestions, is_single_prefix_suggestion) =
                self.input_suggestions.read(ctx, |suggestions, _| {
                    // Ignore fuzzy matches when calculating longest common
                    // prefix of suggestions. So even if there are fuzzy
                    // matches, we can find a common prefix and try to insert it.
                    let suggestion_texts = suggestions
                        .items()
                        .iter()
                        .filter(|item| {
                            matches!(
                                item.match_type(),
                                MatchType::Prefix {
                                    is_case_sensitive: true
                                } | MatchType::Exact {
                                    is_case_sensitive: true
                                }
                            )
                        })
                        .map(|item| item.text())
                        .collect_vec();
                    let num_suggestions = suggestion_texts.len();
                    (
                        longest_common_prefix(suggestion_texts).map(|x| x.to_owned()),
                        num_suggestions == 1,
                    )
                });
            if let Some(common_prefix) = common_prefix_of_filtered_suggestions {
                let input_text = self.editor.as_ref(ctx).buffer_text(ctx);
                // Determine the current word in the editor that will be replaced by the tab
                // completion. We use the start index of the selection since the completer only sees
                // the text up to the start of the selection when generating completion results.
                let current_word = &input_text
                    [replacement_start..self.start_byte_index_of_last_selection(ctx).as_usize()];

                // Insert the common prefix if it is longer than what the user has currently typed
                // This check is necessary because the suggestions are case-insensitive, while the
                // common prefix logic is necessarily case-sensitive. That can lead to the common
                // prefix being shorter, causing confusing behavior where the input is shortened.
                // Also, we check if the replacement
                if common_prefix.len() > current_word.len()
                    && common_prefix.starts_with(current_word)
                {
                    self.insert_completion_prefix_into_editor(
                        ctx,
                        &common_prefix,
                        replacement_start,
                    );
                    // If there was only a single completion remaining and we just inserted it into the editor,
                    // close the completions menu.
                    if is_single_prefix_suggestion {
                        self.close_input_suggestions(true, ctx)
                    }
                    return;
                }
            }
            self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.select_next(ctx);
            });
        } else if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
                | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
        ) {
            self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.select_next(ctx);
            });
        } else if is_tab_bound_to_open_completions && self.cursor_positioned_for_completion(ctx) {
            self.open_completion_suggestions(CompletionsTrigger::Keybinding, ctx);
        } else {
            // Otherwise, pass the tab down to the editor
            self.editor.update(ctx, |input, ctx| input.handle_tab(ctx));
        }
    }

    /// Opens the completion suggestions menu if the cursor is in a valid position to generate
    /// suggestions and the menu is not already open.
    ///
    /// This is called when [`InputAction::MaybeOpenCompletionSuggestions`] is bound to a non-tab
    /// key; tab is the default binding. This is _not_ called when the binding is set to the
    /// default ("tab") because the tab keypress event is actually handled by the child
    /// [`Editor`] view, so the tab event is never actually propagated to this input view. Instead,
    /// the logic to open the completions menu when tab bound is implemented in
    /// [`Self::input_tab()`], which is called when the editor emits an
    /// `EditorEvent::Navigate(NavigationKey::Tab)`.
    ///
    /// Ultimately this weirdness is due to limitations in the UI framework preventing multiple
    /// `View`s from handling/responding to the same `Event`.
    fn maybe_open_completion_suggestions(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::CompletionSuggestions { .. },
        ) && self.cursor_positioned_for_completion(ctx)
        {
            self.open_completion_suggestions(CompletionsTrigger::Keybinding, ctx);
        }
    }

    #[cfg(test)]
    fn user_insert(&mut self, text: &str, ctx: &mut ViewContext<Self>) -> bool {
        self.insert_internal(text, EditOrigin::UserTyped, ctx)
    }

    pub fn user_replace_editor_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) -> bool {
        self.editor.update(ctx, |editor, ctx| {
            editor.select_all(ctx);
        });
        self.insert_internal(text, EditOrigin::UserTyped, ctx)
    }

    // It's the responsibility of the caller to ensure that the text submitted here
    // should be inputted into the input area (i.e. arrow keys should not be
    // included in the string).
    pub fn system_insert(&mut self, text: &str, ctx: &mut ViewContext<Self>) -> bool {
        self.insert_internal(text, EditOrigin::UserInitiated, ctx)
    }

    pub fn has_pending_command(&self) -> bool {
        self.has_pending_command
    }

    pub fn set_pending_command(&mut self, exec: &str, ctx: &mut ViewContext<Self>) {
        self.has_pending_command = true;
        self.system_insert(exec, ctx);
    }

    fn should_enter_accept_completion_suggestion(&self, app: &AppContext) -> bool {
        let InputSuggestionsMode::CompletionSuggestions {
            replacement_start, ..
        } = self.suggestions_mode_model.as_ref(app).mode()
        else {
            return false;
        };
        let completions_while_typing = self.is_completions_while_typing_turned_on(app);
        let selected_item = self.input_suggestions.as_ref(app).get_selected_item_text();

        // If classic completions is enabled, accept the suggestion if an item is selected.
        if self.is_classic_completions_enabled(app) {
            return self
                .input_suggestions
                .as_ref(app)
                .get_selected_item()
                .is_some();
        }
        // If completions as you type is disabled, accept the suggestion if an item is selected.
        if !completions_while_typing {
            return selected_item.is_some();
        }

        let path_separators = self.path_separators(app).all;

        // At this point, we know completions as you type is enabled and classic completions
        // is disabled. Accept the completion unless the buffer already matches the selected item
        // (in which case, just execute the command).
        let current_buffer_text = self.editor.as_ref(app).buffer_text(app);
        selected_item.is_none_or(|selected_item| {
            let Some(replacement) = &current_buffer_text.get(*replacement_start..) else {
                log::error!("Failed to get replacement range in current buffer text");
                return true;
            };
            if replacement == &selected_item {
                return false;
            }
            let Some(no_slash) = selected_item.strip_suffix(path_separators) else {
                return true;
            };
            replacement != &no_slash
        })
    }

    /// Determines whether to insert a newline in the buffer instead of executing a command
    /// when enter is pressed.
    fn should_insert_newline_on_enter(&self, ctx: &AppContext) -> bool {
        let editor = self.editor.as_ref(ctx);
        let shell_family = editor.shell_family();
        editor.chars_preceding_selections(ctx).any(|chars| {
            let mut preceding_chars = chars.rev();
            while let Some(c) = preceding_chars.next() {
                match shell_family {
                    Some(ShellFamily::PowerShell) => {
                        if c == '`' {
                            // Kind of a quirk, but PowerShell only inserts a
                            // newline after a backtick if the character preceding
                            // the backtick is whitespace.
                            if let Some(c) = preceding_chars.next() {
                                if !c.is_ascii_whitespace() {
                                    return false;
                                }
                            }
                            return true;
                        }
                    }
                    Some(ShellFamily::Posix) | None => {
                        if c == '\\' {
                            // Continue if there are more \ characters
                            if let Some(c) = preceding_chars.next() {
                                if c == '\\' {
                                    continue;
                                }
                            }
                            // Odd number of \ characters
                            return true;
                        }
                    }
                }
                return false;
            }
            false
        })
    }

    pub(crate) fn initiate_create_new_project(
        &mut self,
        ai_query: String,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.agent_view_controller.update(ctx, |controller, ctx| {
                let _ =
                    controller.try_enter_agent_view(None, AgentViewEntryOrigin::ProjectEntry, ctx);
            });
        }
        self.ai_controller.update(ctx, move |controller, ctx| {
            controller.send_slash_command_request(
                SlashCommandRequest::CreateNewProject { query: ai_query },
                ctx,
            )
        });
    }

    pub(crate) fn initiate_clone_repository(&mut self, url: String, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.agent_view_controller.update(ctx, |controller, ctx| {
                let _ =
                    controller.try_enter_agent_view(None, AgentViewEntryOrigin::ProjectEntry, ctx);
            });
        }
        self.ai_controller.update(ctx, move |controller, ctx| {
            controller.send_slash_command_request(SlashCommandRequest::CloneRepository { url }, ctx)
        });
    }

    /// Handles the user's 'Enter' keypress.
    ///
    /// Depending on input state, this method may either execute a command, accept an input
    /// suggestion, or add a newline to the input buffer contents.  If there is an active and long
    /// running command, exits early and does nothing. This method should not be callable if there
    /// is an active and long running command; in such a state, the enter keypress should be
    /// handled by the ongoing process corresponding to the active/long running command.
    pub(crate) fn input_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id) {
            // If the @ context menu is open, Enter selects the highlighted item
            // instead of submitting the CLI agent input.
            if matches!(
                self.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::AIContextMenu { .. }
            ) {
                self.editor.update(ctx, |editor, ctx| {
                    if let Some(ai_context_menu) = editor.ai_context_menu() {
                        ai_context_menu.update(ctx, |ai_context_menu, ctx| {
                            ai_context_menu.select_current_item(ctx);
                        });
                    }
                });
                return;
            }

            // If the prompts menu is open, Enter selects the highlighted prompt.
            if self.suggestions_mode_model.as_ref(ctx).is_prompts_menu() {
                self.inline_prompts_menu_view
                    .update(ctx, |view, ctx| view.accept_selected_item(ctx));
                return;
            }

            // If the skill selector menu is open, Enter selects the highlighted skill.
            if self.suggestions_mode_model.as_ref(ctx).is_skill_menu() {
                self.inline_skill_selector_view
                    .update(ctx, |view, ctx| view.accept_selected_item(ctx));
                return;
            }

            // If the slash commands menu is open, accept the selected item
            // (e.g. /prompts or /skills). However, don't intercept detected
            // slash/skill commands in the buffer — those should be submitted
            // directly to the CLI agent so it can handle them natively.
            if matches!(
                self.suggestions_mode_model.as_ref(ctx).mode(),
                InputSuggestionsMode::SlashCommands
            ) {
                self.inline_slash_commands_view.update(ctx, |view, ctx| {
                    view.accept_selected_item(false, ctx);
                });
                return;
            }

            // When the `!` prefix was stripped (shell mode in CLI agent input),
            // prepend it back so the CLI agent receives the mode-switch prefix,
            // then exit shell mode so the next prompt starts in AI mode.
            let mut text = self.editor.as_ref(ctx).buffer_text(ctx);
            if self.is_locked_in_shell_mode(ctx) {
                text = format!("{TERMINAL_INPUT_PREFIX}{text}");
                self.exit_shell_mode_to_ai(ctx);
            }
            ctx.emit(Event::SubmitCLIAgentInput { text });
            return;
        }
        let command = self.editor.as_ref(ctx).buffer_text(ctx);

        ctx.emit(Event::Enter);

        if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_model_selector()
        {
            self.inline_model_selector_view
                .update(ctx, |view, ctx| view.accept_selected_item(false, ctx));
            return;
        }

        if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_profile_selector()
        {
            self.inline_profile_selector_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        }

        if self.suggestions_mode_model.as_ref(ctx).is_prompts_menu() {
            self.inline_prompts_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        }

        if self.should_insert_newline_on_enter(ctx) {
            self.editor.update(ctx, |editor, ctx| {
                editor.user_initiated_insert("\n", PlainTextEditorViewAction::NewLine, ctx)
            });
        } else if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::AIContextMenu { .. }
        ) {
            self.editor.update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |ai_context_menu, ctx| {
                        ai_context_menu.select_current_item(ctx);
                    });
                }
            });
            return;
        } else if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_conversation_menu()
        {
            self.inline_conversation_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_skill_menu() {
            self.inline_skill_selector_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_user_query_menu() {
            self.user_query_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(false, ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_rewind_menu() {
            self.rewind_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        } else if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_history_menu()
            && self.is_cloud_mode_input_v2_composing(ctx)
            && self
                .cloud_mode_v2_history_menu_view
                .as_ref()
                .is_some_and(|view| view.as_ref(ctx).has_selection(ctx))
        {
            if let Some(view) = self.cloud_mode_v2_history_menu_view.clone() {
                view.update(ctx, |view, ctx| view.accept_selected(ctx));
            }
            return;
        } else if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_history_menu()
            && self
                .inline_history_menu_view
                .as_ref(ctx)
                .model()
                .as_ref(ctx)
                .selected_item()
                .is_some()
        {
            self.inline_history_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_repos_menu() {
            self.inline_repos_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(false, ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_plan_menu() {
            self.inline_plan_menu_view
                .update(ctx, |view, ctx| view.accept_selected_item(ctx));
            return;
        } else if self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
            if self.is_cloud_mode_input_v2_composing(ctx) {
                if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
                    view.update(ctx, |view, ctx| {
                        view.accept_selected_item(false, ctx);
                    });
                }
            } else {
                self.inline_slash_commands_view.update(ctx, |view, ctx| {
                    view.accept_selected_item(false, ctx);
                });
            }
            return;
        } else if self.maybe_launch_cloud_handoff_request(ctx)
            || self.maybe_queue_input_for_in_progress_conversation(ctx)
            || self.maybe_handle_enter_for_slash_command(ctx)
        {
            return;
        } else if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::CompletionSuggestions { .. }
        ) && self.should_enter_accept_completion_suggestion(ctx)
        {
            self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.confirm(ctx);
            })
        } else if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
                | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
        ) {
            self.input_suggestions.update(ctx, |suggestions, ctx| {
                suggestions.confirm(ctx);
            });
        } else if self.should_block_cloud_mode_setup_submission(ctx) {
            return;
        } else if FeatureFlag::HandoffCloudCloud.is_enabled()
            && self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(ctx)
                        .is_ready_for_cloud_followup_prompt()
                })
        {
            ctx.emit(Event::SubmitCloudFollowup {
                prompt: command.trim().to_owned(),
            });
            return;
        } else if FeatureFlag::AgentMode.is_enabled()
            && AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
            && (self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
                || self.is_cloud_mode_input_v2_composing(ctx))
        {
            // If we're submitting an AI query, we want to send telemetry for the input type.
            let input_model = self.ai_input_model.as_ref(ctx);
            let input_type = input_model.input_type();
            let is_locked = input_model.is_input_type_locked();
            let was_lock_set_with_empty_buffer = input_model.was_lock_set_with_empty_buffer();
            send_telemetry_from_ctx!(
                TelemetryEvent::InputBufferSubmitted {
                    input_type,
                    is_locked,
                    was_lock_set_with_empty_buffer,
                },
                ctx
            );

            // Check if we're configuring an ambient agent and spawn it instead of submitting a regular AI query.
            if self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(ctx)
                        .is_configuring_ambient_agent()
                })
            {
                if FeatureFlag::AgentHarness.is_enabled() {
                    let availability = HarnessAvailabilityModel::as_ref(ctx);
                    if !availability.has_any_enabled_harness() {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                            ts.add_ephemeral_toast(
                                DismissibleToast::error(
                                    "No agent harnesses are available. Contact your team admin."
                                        .to_string(),
                                ),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    }
                }

                let prompt = command.trim().to_owned();
                if prompt.is_empty() {
                    return;
                }

                if self.is_cloud_mode_input_v2_composing(ctx) {
                    if let Some(ambient_agent_view_model) = self.ambient_agent_view_model() {
                        let needs_env_modal = ambient_agent_view_model
                            .as_ref(ctx)
                            .selected_environment_id()
                            .is_none();
                        if needs_env_modal {
                            ctx.emit(Event::OpenCloudModeV2EnvironmentCreationModal);
                            return;
                        }
                    }
                }

                #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
                let attachments = self
                    .collect_cloud_launch_attachments(ctx)
                    .request_attachments;
                #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
                let attachments = vec![];

                // For local-to-cloud handoff panes, gate the buffer clear on the
                // async `derive_touched_workspace` derivation having completed and
                // no orchestrator already being in flight. If we cleared early and
                // then bailed inside `submit_handoff`, the user's prompt and
                // pending attachments would be silently dropped. Surface a toast
                // so the user gets some feedback instead of seeing the submit do
                // nothing — the prompt and attachments are intentionally left
                // intact so the next submit picks them back up.
                if let Some(ambient_agent_view_model) = self.ambient_agent_view_model().cloned() {
                    let is_handoff_not_ready = {
                        let model = ambient_agent_view_model.as_ref(ctx);
                        model.is_local_to_cloud_handoff() && !model.is_handoff_ready_to_submit()
                    };
                    if is_handoff_not_ready {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                            ts.add_ephemeral_toast(
                                DismissibleToast::default(
                                    "Preparing handoff — try again in a moment.".to_owned(),
                                )
                                .with_object_id("local-to-cloud-handoff-not-ready".to_owned()),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    }
                }

                // Clear the buffer and pending attachments after collecting them.
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.clear_pending_attachments(ctx);
                });

                if let Some(ambient_agent_view_model) = self.ambient_agent_view_model() {
                    ambient_agent_view_model.update(ctx, |state, ctx| {
                        if state.is_local_to_cloud_handoff() {
                            state.submit_handoff(prompt, attachments, ctx);
                        } else {
                            state.spawn_agent(prompt, attachments, ctx);
                        }
                    });
                }
                return;
            }

            self.submit_ai_query(None, ctx);
        } else {
            // If we're submitting a shell command, we want to send telemetry for the input type.
            let input_model = self.ai_input_model.as_ref(ctx);
            let input_type = input_model.input_type();
            let is_locked = input_model.is_input_type_locked();
            let was_lock_set_with_empty_buffer = input_model.was_lock_set_with_empty_buffer();
            send_telemetry_from_ctx!(
                TelemetryEvent::InputBufferSubmitted {
                    input_type,
                    is_locked,
                    was_lock_set_with_empty_buffer,
                },
                ctx
            );

            if FeatureFlag::WorkflowAliases.is_enabled() {
                let mut command_string = self.editor.as_ref(ctx).buffer_text(ctx);
                // If the alias was inserted from the completions menu, it will have trailing
                // whitespace - trim it in-place.
                command_string.truncate(command_string.trim_end().len());

                if let Some(alias) = WorkflowAliases::as_ref(ctx).match_alias(&command_string) {
                    if let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(&alias.workflow_id)
                    {
                        let owner = workflow.clone().permissions.owner.into();

                        let workflow_type = WorkflowType::Cloud(Box::new(workflow.clone()));
                        let env_vars = alias.env_vars.or(workflow.model().data.default_env_vars());

                        self.insert_workflow_into_input(
                            workflow_type,
                            owner,
                            WorkflowSelectionSource::Alias,
                            alias.arguments,
                            None,
                            env_vars,
                            true,
                            ctx,
                        );
                        return;
                    } else {
                        log::warn!(
                            "Tried to execute workflow for id {:?} but it does not exist",
                            alias.workflow_id
                        );
                    };
                }
            }

            let command = self.get_command(ctx);
            if !self.try_execute_command(&command, ctx) {
                return;
            }

            if FeatureFlag::AgentMode.is_enabled()
                && AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx)
            {
                self.ai_input_model.update(ctx, |input, ctx| {
                    input.abort_in_progress_detection();

                    // The default input state after executing a shell command is Shell mode with
                    // autodetection enabled.
                    input.set_input_config_for_classic_mode(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: true,
                        }
                        .unlocked_if_autodetection_enabled(false, ctx),
                        ctx,
                    );
                });
            }

            // Cancel actively streaming conversations if we're able to run the command.
            // This is possible in persistent input mode.
            self.ai_controller.update(ctx, |controller, ctx| {
                let active_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                    .active_conversation(self.terminal_view_id)
                    .filter(|conversation| conversation.status().is_in_progress())
                    .map(|conversation| conversation.id());
                if let Some(active_conversation_id) = active_conversation_id {
                    controller.cancel_conversation_progress(
                        active_conversation_id,
                        CancellationReason::UserCommandExecuted,
                        ctx,
                    );
                }
            });

            self.ai_input_model.update(ctx, |model, ctx| {
                model.handle_input_buffer_submitted(ctx);
            });

            if SyncedInputState::as_ref(ctx).is_syncing_any_inputs(ctx.window_id()) {
                ctx.emit(Event::SyncInput(SyncInputType::RanCommand));
            }

            self.model.lock().set_is_input_dirty(false);
        }

        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            // Don't show the quota banner once a user has run a command or AI query.
            ai_settings.mark_quota_banner_as_dismissed(ctx);
            ctx.notify();
        });
    }

    fn input_cmd_enter(&mut self, ctx: &mut ViewContext<Self>) {
        // NaturalLanguageCommandSearch has its own `cmd+enter` behaviour, not expected to execute here
        let mode = self.suggestions_mode_model.as_ref(ctx).mode().clone();
        match &mode {
            InputSuggestionsMode::CompletionSuggestions { .. }
            | InputSuggestionsMode::HistoryUp { .. }
                // If FeatureFlag::AgentView is enabled, cmd-enter should unconditionally enter the
                // agent view with the current buffer contents as agent input.
                //
                // I'm (ZB) not even sure what this legacy behavior is for, because if you have any
                // selected completion or history suggestion, that suggestion has already been
                // inserted into the buffer so enter (without cmd- prefix) would directly execute
                // it anyway.
                if !FeatureFlag::AgentView.is_enabled() =>
            {
                self.input_suggestions.update(ctx, |suggestions, ctx| {
                    suggestions.confirm_and_execute(ctx);
                });
            }
            InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
                dynamic_enum_status: DynamicEnumSuggestionStatus::Unapproved,
                command,
                ..
            } => {
                let editor_model = self.editor.read(ctx, |view, ctx| view.snapshot_model(ctx));
                self.get_enum_suggestions_async(command.clone(), editor_model, ctx);
            }
            InputSuggestionsMode::ModelSelector
                if FeatureFlag::InlineMenuHeaders.is_enabled() =>
            {
                self.inline_model_selector_view
                    .update(ctx, |view, ctx| view.accept_selected_item(true, ctx));
            }
            InputSuggestionsMode::UserQueryMenu { .. } => {
                self.user_query_menu_view
                    .update(ctx, |view, ctx| view.accept_selected_item(true, ctx));
            }
            InputSuggestionsMode::IndexedReposMenu => {
                self.inline_repos_menu_view
                    .update(ctx, |view, ctx| view.accept_selected_item(true, ctx));
            }
            _ => {
                if FeatureFlag::AgentView.is_enabled()
                    && self.maybe_handle_cmd_or_ctrl_shift_enter_for_slash_command(ctx)
                {
                    return;
                }
                // In cloud mode (ambient agent), Cmd+Enter should exit cloud mode entirely and start a
                // new *local* agent conversation in the root terminal. This should work whether the
                // buffer is empty (blank convo) or non-empty (prefill draft, but don't auto-send).
                if self
                    .ambient_agent_view_model()
                    .is_some_and(|ambient_agent_model| {
                        ambient_agent_model.as_ref(ctx).is_ambient_agent()
                    })
                {
                    let mut draft = self.editor.as_ref(ctx).buffer_text(ctx);
                    // Normalize draft for empty-checks and for prefill.
                    draft.truncate(draft.trim_end().len());

                    let is_empty = draft.trim().is_empty();
                    ctx.emit(Event::ExitCloudModeAndStartLocalAgent {
                        initial_prompt: (!is_empty).then_some(draft),
                    });
                    return;
                }

                // If there is a slash command bound to cmd-enter, we'll execute it.
                let cmd_enter_slash_command = {
                    self.slash_command_data_source
                        .as_ref(ctx)
                        .active_commands()
                        .find_map(|(_, command)| {
                            let binding = keybinding_name_to_normalized_string(command.name, ctx)?;
                            (binding == CMD_ENTER_KEYBINDING).then_some(command)
                        })
                        .cloned()
                };


                if let Some(command) = cmd_enter_slash_command {
                    self.select_slash_command(&command, SlashCommandTrigger::keybinding(), ctx);
                    return;
                }

                // For viewers in a shared session, send the prompt to the sharer via
                // submit_viewer_ai_query instead of emitting UnhandledCmdEnter. This keeps
                // all viewer AI query logic in input.rs.
                let shared_session_status = self.model.lock().shared_session_status().clone();
                if FeatureFlag::AgentView.is_enabled()
                    && shared_session_status.is_viewer()
                    && shared_session_status.is_executor()
                {
                    let prompt = self.editor.as_ref(ctx).buffer_text(ctx);
                    if !prompt.trim().is_empty() {
                        self.submit_viewer_ai_query(ctx);
                        return;
                    }
                }

                ctx.emit(Event::UnhandledCmdEnter)
            }
        }
    }

    fn predict_am_query(&mut self, ctx: &mut ViewContext<Self>) {
        // Cancel any pending requests.
        if let Some(future_handle) = self.predict_am_queries_future_handle.take() {
            future_handle.abort();
        }

        let block = &self.last_user_block_completed;
        if block.is_none() {
            return;
        }
        let block = block.as_ref().unwrap();
        let (exit_code, working_dir) = (
            block.serialized_block.exit_code,
            block.serialized_block.pwd.as_ref(),
        );
        let number_of_top_lines_per_grid = 100;
        let number_of_bottom_lines_per_grid = 200;

        let (processed_input, processed_output) = {
            let model = self.model.lock();
            let terminal_width = model.block_list().size().columns;

            if let Some(current_block) =
                model.block_list().block_with_id(&block.serialized_block.id)
            {
                current_block.get_block_content_summary(
                    terminal_width,
                    number_of_top_lines_per_grid,
                    number_of_bottom_lines_per_grid,
                )
            } else {
                log::error!(
                    "Failed to fetch predicted queries, could not find block with ID: {:?}",
                    block.serialized_block.id
                );
                return;
            }
        };

        let json_message = json!({
            "command": processed_input,
            "output": processed_output,
            "exit_code": exit_code,
            "pwd": working_dir,
        });

        let am_query_input_buffer = self.editor.as_ref(ctx).buffer_text(ctx);
        let Some(session) = self.active_session(ctx) else {
            return;
        };
        let context = WarpAiExecutionContext::new(&session);

        let request = PredictAMQueriesRequest {
            context_messages: vec![json_message.to_string()],
            partial_query: am_query_input_buffer.clone(),
            system_context: context.to_json_string(),
        };

        let server_api = self.server_api.clone();

        self.predict_am_queries_future_handle = Some(ctx.spawn(
            async move {
                match server_api.predict_am_queries(&request).await {
                    Ok(resp) => Some(resp.suggestion),
                    Err(err) => {
                        log::error!("Failed to fetch predicted queries: {err:?}");
                        None
                    }
                }
            },
            move |me: &mut Self, maybe_suggestion: Option<String>, ctx: &mut ViewContext<Self>| {
                // Only set the autosuggestion if the input buffer hasn't changed, since we made the original request
                // i.e. verify the suggestion is still relevant.
                if am_query_input_buffer != me.editor.as_ref(ctx).buffer_text(ctx) {
                    return;
                }

                if let Some(suggestion) = maybe_suggestion {
                    me.set_autosuggestion(
                        suggestion,
                        AutosuggestionType::AgentModeQuery {
                            context_block_ids: vec![],
                            was_intelligent_autosuggestion: true,
                        },
                        ctx,
                    );
                }
            },
        ));
    }

    /// Re-submits a queued prompt through the correct handler (slash, skill, or regular AI query),
    /// without touching the input buffer or triggering NLD / autosuggestion side-effects.
    ///
    /// Cancels the in-flight stream first so slash/skill paths don't trip the in-flight assertion.
    /// `is_for_same_conversation: true` keeps the conversation status `InProgress` so the warping
    /// indicator stays visible.
    pub(crate) fn submit_queued_prompt(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        if let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            self.ai_controller.update(ctx, |controller, ctx| {
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::FollowUpSubmitted {
                        is_for_same_conversation: true,
                    },
                    ctx,
                );
            });
        }

        let detected = self
            .slash_command_model
            .as_ref(ctx)
            .detect_command(&prompt, ctx);

        // Try slash command or skill command first. Some slash commands
        // (e.g. /plan, /compact) return false to indicate the full text
        // should be sent as a regular AI query — fall through in that case.
        let handled = match detected {
            SlashCommandEntryState::SlashCommand(detected_command) => {
                self.execute_slash_command(
                    &detected_command.command,
                    detected_command.argument.as_ref(),
                    SlashCommandTrigger::input(),
                    /*is_queued_prompt*/ true,
                    ctx,
                )
            }
            SlashCommandEntryState::SkillCommand(detected_skill) => {
                self.execute_skill_command(
                    detected_skill.reference,
                    detected_skill.argument,
                    /*is_queued_prompt*/ true,
                    ctx,
                )
            }
            _ => false,
        };

        if handled {
            return;
        }

        if let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            self.ai_controller.update(ctx, move |controller, ctx| {
                controller.send_queued_user_query_in_conversation(
                    prompt,
                    conversation_id,
                    None,
                    ctx,
                );
            });
        } else {
            self.ai_controller.update(ctx, move |controller, ctx| {
                controller.send_queued_user_query_in_new_conversation(
                    prompt,
                    None,
                    EntrypointType::UserInitiated,
                    None,
                    ctx,
                );
            });
        }

        ctx.emit(Event::ExecuteAIQuery);
    }

    /// Checks whether the current input should be queued instead of executed.
    /// Returns true (and queues the prompt) when the queue-next-prompt toggle is
    /// on and the active conversation is still in progress.
    /// Only queues when AI input is active — if the user is in shell mode the
    /// input is not queued (so e.g. `ls` still runs in the terminal).
    fn maybe_queue_input_for_in_progress_conversation(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !FeatureFlag::QueueSlashCommand.is_enabled() {
            return false;
        }

        if !self
            .ai_context_model
            .as_ref(ctx)
            .is_queue_next_prompt_enabled()
        {
            return false;
        }

        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
            return false;
        }

        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return false;
        };

        let history = BlocklistAIHistoryModel::handle(ctx);
        let should_queue = history
            .as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| {
                !c.is_empty() && (c.status().is_in_progress() || c.status().is_blocked())
            });

        if !should_queue {
            return false;
        }

        let prompt = self.editor.as_ref(ctx).buffer_text(ctx);
        if prompt.is_empty() {
            return false;
        }

        // If the input is itself a /queue command, unwrap the argument so we
        // queue "fix the tests" directly instead of "/queue fix the tests"
        // (which would double-hop through the /queue handler on re-submission).
        let prompt = if let SlashCommandEntryState::SlashCommand(ref detected) = self
            .slash_command_model
            .as_ref(ctx)
            .detect_command(&prompt, ctx)
        {
            if detected.command.name == commands::QUEUE.name {
                match detected.argument.as_ref().filter(|a| !a.is_empty()) {
                    Some(arg) => arg.clone(),
                    // /queue with no argument — bail and let the normal slash command
                    // handler show the error toast.
                    None => return false,
                }
            } else {
                prompt
            }
        } else {
            prompt
        };

        self.ai_input_model.update(ctx, |model, ctx| {
            model.handle_input_buffer_submitted(ctx);
        });
        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
        ctx.dispatch_typed_action(&WorkspaceAction::QueuePromptForConversation { prompt });

        true
    }

    /// Submit the input buffer contents as an AI query.
    fn submit_ai_query(
        &mut self,
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |editor, ctx| {
            editor.abort_attached_images_future_handle(ctx);
        });

        // If this is a viewer in a shared session, send the agent prompt
        // to the sharer instead of executing locally.
        let shared_session_status = self.model.lock().shared_session_status().clone();
        if shared_session_status.is_viewer() {
            if shared_session_status.is_executor() {
                // This will return false if we should execute the given command locally instead
                // of sending it to the sharer (which is the case for slash commands like fork
                // and fork-and-compact).
                if self.submit_viewer_ai_query(ctx) {
                    return;
                }
            } else {
                log::warn!("Viewer tried to submit AI query without executor role");
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(
                            "Cannot send queries as a read-only viewer.".to_string(),
                        ),
                        window_id,
                        ctx,
                    );
                });
                return;
            }
        }

        // If the agent view is inactive but the current input is detected as AI, submitting
        // this query triggers entering the agent view.
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            let prompt = self.editor.as_ref(ctx).buffer_text(ctx);
            let prompt = prompt.trim().to_owned();
            // Don't enter the agent view if input is autodetected as AI but the input is empty.
            //
            // This may happen because the input mode must be set to either shell or agent, and
            // when the buffer cleared the input remains in whichever mode it was in previously
            // until new input is entered.
            if prompt.is_empty() {
                return;
            }
            ctx.emit(Event::EnterAgentView {
                initial_prompt: Some(prompt),
                conversation_id: None,
                origin: AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: !self
                        .ai_input_model
                        .as_ref(ctx)
                        .is_input_type_locked(),
                },
            });
            return;
        }

        let has_requests_remaining = AIRequestUsageModel::as_ref(ctx).has_requests_remaining();

        let has_any_ai = AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx);
        if !has_any_ai {
            AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                model.enable_buy_credits_banner(ctx);
            });
        }

        if PromptAlertView::does_alert_block_ai_requests(ctx) {
            if !has_requests_remaining {
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeUserAttemptedQueryAtRequestLimit {
                        limit: AIRequestUsageModel::as_ref(ctx).request_limit()
                    },
                    ctx
                );
            }

            AIRequestUsageModel::handle(ctx).update(ctx, |usage_model, ctx| {
                // Rate limit requests to fetch the user's AI usage if triggered by enter
                // keypress.
                const USAGE_LIMIT_UPDATE_REQUEST_RATE_LIMIT: Duration = Duration::from_secs(10);

                let last_update_time = usage_model.last_update_time();
                if last_update_time
                    .is_some_and(|time| time.elapsed() >= USAGE_LIMIT_UPDATE_REQUEST_RATE_LIMIT)
                    || last_update_time.is_none()
                {
                    usage_model.refresh_request_usage_async(ctx);
                }
            });

            return;
        }

        if let Some(zero_state_prompt_suggestion_type) = zero_state_prompt_suggestion_type {
            return self.ai_controller.update(ctx, move |controller, ctx| {
                controller.send_zero_state_prompt_suggestion(zero_state_prompt_suggestion_type, ctx)
            });
        }

        let ai_query = self.editor.as_ref(ctx).buffer_text(ctx);
        // We don't send AI requests with empty queries, even if the context is non-empty. We
        // also don't send a query when the input (query plus context) is over the length limit.
        // If we haven't calculated the input length, we optimistically as if it is within the
        // limit. We always check the length before sending making the API request.
        if ai_query.is_empty() {
            return;
        }

        IgnoredSuggestionsModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_ignored_suggestion(ai_query.clone(), SuggestionType::AIQuery, ctx);
        });

        self.ai_input_model.update(ctx, |model, ctx| {
            model.handle_input_buffer_submitted(ctx);
        });

        if let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            self.ai_controller.update(ctx, move |controller, ctx| {
                controller.send_user_query_in_conversation(ai_query, conversation_id, None, ctx)
            });
        } else {
            self.ai_controller.update(ctx, move |controller, ctx| {
                controller.send_user_query_in_new_conversation(
                    ai_query,
                    None,
                    EntrypointType::UserInitiated,
                    None,
                    ctx,
                );
            });
        }

        ctx.emit(Event::ExecuteAIQuery);

        if let Some(workflow_state) = self.workflows_state.selected_workflow_state.as_ref() {
            if let WorkflowType::Cloud(workflow) = &workflow_state.workflow_type {
                send_telemetry_from_ctx!(
                    TelemetryEvent::ExecutedWarpDrivePrompt {
                        id: workflow.id.into_server().map(Into::into),
                        selection_source: workflow_state.workflow_selection_source,
                    },
                    ctx
                );

                UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                    update_manager.record_object_action(
                        workflow.cloud_object_type_and_id(),
                        ObjectActionType::Execute,
                        None,
                        ctx,
                    )
                });
            }
        }
    }

    /// Send the given query to the session sharer for them to execute on their machine.
    /// Returns false if the query should be run locally instead of being sent to the sharer
    /// (which is the case for slash commands like fork and fork-and-compact).
    fn submit_viewer_ai_query(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let prompt = self.editor.as_ref(ctx).buffer_text(ctx);
        if prompt.is_empty() {
            return true;
        }

        // Fork slash commands should be run locally instead of being sent to the sharer
        // (as the viewer running the slash command wants to fork on their local machine).
        if prompt.starts_with(commands::FORK_AND_COMPACT.name)
            || prompt.starts_with(commands::FORK.name)
        {
            return false;
        }

        // Freeze the editor and put it in a loading state
        self.freeze_input_in_loading_state(ctx);

        // Look up the conversation's server token from the conversation metadata.
        let selected_conv_id = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx);
        let server_conversation_token = selected_conv_id
            .and_then(|id| {
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&id)
                    .and_then(|conv| conv.server_conversation_token().cloned())
            })
            .and_then(|token| {
                token
                    .as_str()
                    .parse()
                    .ok()
                    .map(ServerConversationToken::from_uuid)
            });

        let ambient_agent_task_id = self
            .ambient_agent_view_model()
            .and_then(|ambient_agent_model| ambient_agent_model.as_ref(ctx).task_id());

        // Collect attachments from ai_context_model
        let attachments: Vec<AgentAttachment> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_context(ctx, true)
            .into_iter()
            .filter_map(|context| match context {
                AIAgentContext::Block(block) => Some(AgentAttachment::BlockReference {
                    block_id: block.id.into(),
                }),
                AIAgentContext::SelectedText(text) => {
                    Some(AgentAttachment::PlainText { content: text })
                }
                // For now, only AgentAttachment context is supported.
                // TODO: Add support for other context types.
                _ => None,
            })
            .collect();

        let pending_images: Vec<_> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .into_iter()
            .cloned()
            .collect();
        let pending_files: Vec<_> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_files()
            .into_iter()
            .cloned()
            .collect();

        let has_uploads = (!pending_images.is_empty() || !pending_files.is_empty())
            && FeatureFlag::CloudModeImageContext.is_enabled();

        if let Some(task_id) = ambient_agent_task_id.filter(|_| has_uploads) {
            // Upload files first, then send prompt with file references in callback
            Self::upload_files_then_send_prompt(
                task_id,
                server_conversation_token,
                prompt,
                attachments,
                &pending_images,
                &pending_files,
                ctx,
            );
        } else {
            // No files to upload, send prompt immediately
            if !pending_images.is_empty() || !pending_files.is_empty() {
                log::warn!("Cannot upload files: no task_id available");
            }
            ctx.emit(Event::SendAgentPrompt {
                server_conversation_token,
                prompt,
                attachments,
            });
        }

        true
    }

    /// Uploads image and file attachments to GCS via presigned URLs, then emits `SendAgentPrompt`
    /// with the resulting `FileReference` attachments appended.
    fn upload_files_then_send_prompt(
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        server_conversation_token: Option<
            session_sharing_protocol::common::ServerConversationToken,
        >,
        prompt: String,
        base_attachments: Vec<AgentAttachment>,
        pending_images: &[crate::ai::agent::ImageContext],
        pending_files: &[crate::ai::blocklist::PendingFile],
        ctx: &mut ViewContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let server_api = ServerApiProvider::as_ref(ctx).get();

        // Decode all images upfront; drop any that fail so that file_infos
        // and files_to_upload stay in sync (they're zipped later).
        let mut files_to_upload: Vec<(String, String, Vec<u8>)> = pending_images
            .iter()
            .filter_map(|img| {
                base64::engine::general_purpose::STANDARD
                    .decode(&img.data)
                    .map(|bytes| (img.file_name.clone(), img.mime_type.clone(), bytes))
                    .map_err(|e| {
                        log::error!("Failed to decode base64 image {}: {e}", img.file_name)
                    })
                    .ok()
            })
            .collect();

        // Also read non-image files from disk and add them to the upload list.
        for file in pending_files {
            match std::fs::read(&file.file_path) {
                Ok(bytes) => {
                    if bytes.len() > MAX_ATTACHMENT_SIZE_BYTES {
                        log::warn!(
                            "Skipping file {} ({} bytes) — exceeds 10MB limit",
                            file.file_name,
                            bytes.len()
                        );
                        continue;
                    }
                    files_to_upload.push((file.file_name.clone(), file.mime_type.clone(), bytes));
                }
                Err(e) => {
                    log::error!("Failed to read file {}: {e}", file.file_path.display());
                }
            }
        }

        let file_infos: Vec<AttachmentFileInfo> = files_to_upload
            .iter()
            .map(|(name, mime, _)| AttachmentFileInfo {
                filename: name.clone(),
                mime_type: mime.clone(),
            })
            .collect();

        ctx.spawn(
            async move {
                let response = match ai_client
                    .prepare_attachments_for_upload(&task_id, &file_infos)
                    .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        log::error!(
                            "Failed to prepare attachment uploads for task {task_id}: {e:?}"
                        );
                        return None;
                    }
                };

                let mut uploaded = Vec::new();
                for ((file_name, mime_type, file_bytes), upload_info) in
                    files_to_upload.iter().zip(response.attachments.iter())
                {
                    let result = server_api
                        .http_client()
                        .put(&upload_info.upload_url)
                        .header("Content-Type", mime_type.as_str())
                        .body(file_bytes.clone())
                        .send()
                        .await;

                    match result {
                        Ok(resp) if resp.status().is_success() => {
                            uploaded.push(AgentAttachment::FileReference {
                                attachment_id: upload_info.attachment_id.clone(),
                                file_name: file_name.clone(),
                            });
                        }
                        Ok(resp) => {
                            log::error!(
                                "Failed to upload attachment {}: HTTP {}",
                                file_name,
                                resp.status()
                            );
                        }
                        Err(e) => {
                            log::error!("Failed to upload attachment {file_name}: {e:?}");
                        }
                    }
                }

                if uploaded.len() < files_to_upload.len() {
                    log::warn!(
                        "Only {}/{} attachments uploaded successfully",
                        uploaded.len(),
                        files_to_upload.len()
                    );
                }

                Some(uploaded)
            },
            move |input, maybe_uploaded, ctx| {
                let Some(uploaded_files) = maybe_uploaded else {
                    // Prepare request failed (e.g. attachment limit exceeded).
                    // Keep pending attachments so the user can retry, unfreeze input,
                    // and show an error toast.
                    input.unfreeze_and_clear_agent_input(ctx);
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Too many attachments for this conversation.".to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                    return;
                };

                // Upload succeeded — clear pending attachments now.
                input.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.clear_pending_attachments(ctx);
                });

                let mut all_attachments = base_attachments;
                all_attachments.extend(uploaded_files);

                ctx.emit(Event::SendAgentPrompt {
                    server_conversation_token,
                    prompt,
                    attachments: all_attachments,
                });
            },
        );
    }

    /// Returns true if toggling the input mode is disabled.
    fn is_input_mode_toggle_disabled(&self, ctx: &ViewContext<Self>) -> bool {
        // Don't allow input mode changes for:
        // - read-only viewers in shared sessions.
        // - long-running commands with an agent tagged in or in control.
        // - local -> cloud handoff prompts (these must be agent mode prompts)
        let terminal_model = self.model.lock();
        let active_block = terminal_model.block_list().active_block();
        terminal_model.shared_session_status().is_reader()
            || active_block.is_agent_in_control_or_tagged_in()
            || self.prefix_mode(ctx) == InputPrefixMode::CloudHandoff
    }

    /// Set input mode to natural language detection (auto-detection)
    pub fn set_input_mode_natural_language_detection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_input_mode_toggle_disabled(ctx) {
            return;
        }

        let is_autodetection_enabled = AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);

        if !is_autodetection_enabled {
            return;
        }

        let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);

        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            // If we're already configured to do autodetection, there's nothing to do here.
            if ai_input_model.should_run_input_autodetection(ctx) {
                return;
            }

            // Update the input mode to remove any locks and re-enable autodetection.
            // If the buffer is empty, this returns the input mode to the default.
            let input_type = if buffer_text.is_empty() {
                InputType::default()
            } else {
                ai_input_model.input_config().input_type
            };
            ai_input_model.enable_autodetection(input_type, ctx);
        });

        // If the buffer is non-empty, we should kick off the autodetection process, in case the
        // classification doesn't match the previous locked mode.
        if !buffer_text.is_empty() {
            if let Some(completion_context) = self.completion_session_context(ctx) {
                let ai_input_model = self.ai_input_model.clone();

                ctx.spawn(
                    async move {
                        (
                            parse_current_commands_and_tokens(buffer_text, &completion_context)
                                .await,
                            completion_context,
                        )
                    },
                    move |_input, (parsed_tokens, completion_context), ctx| {
                        let session_id = completion_context.session.id();
                        ai_input_model.update(ctx, |model, ctx| {
                            model.detect_and_set_input_type(
                                parsed_tokens,
                                completion_context,
                                Some(session_id),
                                ctx,
                            );
                        });
                    },
                );
            }
        }
    }

    /// Set input mode to Agent Mode (AI input)
    pub fn set_input_mode_agent(
        &mut self,
        ensure_input_is_focused: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't allow input mode changes for read-only viewers in shared sessions
        if self.model.lock().shared_session_status().is_reader() {
            return;
        }

        let is_input_buffer_empty = self.editor.as_ref(ctx).buffer_text(ctx).is_empty();

        // When AgentView is enabled, reverting to AI mode in an active agent view with an empty
        // buffer should unlock (re-enable autodetection) - semantically like clearing the "!".
        //
        // If there is a pending image / file attachment, do NOT unlock. The user's intent is
        // unambiguously "talk to the agent"; letting the classifier flip the input back to
        // shell mode would be a bug.
        let has_locking_attachment = self.ai_context_model.as_ref(ctx).has_locking_attachment();
        let should_unlock = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_fullscreen()
            && is_input_buffer_empty
            && AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx)
            && !has_locking_attachment;

        if should_unlock {
            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                ai_input_model.enable_autodetection(InputType::AI, ctx);
            });
        } else {
            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                let new_config = InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                };
                ai_input_model.set_input_config(new_config, is_input_buffer_empty, ctx);
            });
        }

        if ensure_input_is_focused {
            self.focus_input_box(ctx);
        }
    }

    /// Set input mode to Terminal Mode (shell command input)
    pub fn set_input_mode_terminal(&mut self, steal_focus: bool, ctx: &mut ViewContext<Self>) {
        if self.is_input_mode_toggle_disabled(ctx) {
            return;
        }

        let is_input_buffer_empty = self.editor.as_ref(ctx).buffer_text(ctx).is_empty();
        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            let new_config = InputConfig {
                input_type: InputType::Shell,
                is_locked: true,
            };
            ai_input_model.set_input_config(new_config, is_input_buffer_empty, ctx);
        });

        if steal_focus {
            self.focus_input_box(ctx);
        }
    }

    /// Applies an input config update from an external source (e.g., session sharing).
    pub fn apply_external_input_config_update(
        &mut self,
        config: InputConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        // do nothing if the config is the same as the current config
        if config == self.ai_input_model.as_ref(ctx).input_config() {
            return;
        }

        let is_input_buffer_empty = self.editor.as_ref(ctx).buffer_text(ctx).is_empty();
        self.ai_input_model.update(ctx, |model, ctx| {
            model.set_input_config(config, is_input_buffer_empty, ctx);
        });
    }

    /// Returns true if the input is locked in shell mode
    fn is_locked_in_shell_mode(&self, ctx: &ViewContext<Self>) -> bool {
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        ai_input_model.is_input_type_locked() && !ai_input_model.input_type().is_ai()
    }

    /// Exits `!` shell mode by switching back to AI mode. For CLI agent input
    /// the mode is always locked (the `!` prefix is the explicit toggle). For
    /// the agent view, the autodetection setting is respected.
    fn exit_shell_mode_to_ai(&mut self, ctx: &mut ViewContext<Self>) {
        let is_cli_agent_input_open =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);
        let new_config = if is_cli_agent_input_open {
            InputConfig {
                input_type: InputType::AI,
                is_locked: true,
            }
        } else {
            InputConfig {
                input_type: InputType::AI,
                is_locked: true,
            }
            .unlocked_if_autodetection_enabled(true, ctx)
        };
        self.ai_input_model.update(ctx, |ai_input_model, ctx| {
            ai_input_model.set_input_config(new_config, true, ctx);
        });
    }

    /// Returns true if the input is locked in AI mode
    fn is_locked_in_ai_mode(&self, ctx: &ViewContext<Self>) -> bool {
        let ai_input_model = self.ai_input_model.as_ref(ctx);
        ai_input_model.is_input_type_locked() && ai_input_model.input_type().is_ai()
    }

    fn get_command(&mut self, ctx: &mut ViewContext<Self>) -> String {
        // Expand valid abbreviations or aliases, if any
        if let Some(expanded_command) = self.get_expanded_command_on_execute(ctx) {
            return expanded_command;
        }
        self.editor.as_ref(ctx).buffer_text(ctx)
    }

    /// Inserts the given text into the input buffer. Note that this requires a TerminalModel lock
    /// because when not in Agent Mode, we clear all active selections when inserting text into the
    /// editor! Any upstream caller should NOT be holding a lock on the TerminalModel when calling
    /// this method, to avoid a deadlock.
    fn insert_internal(
        &mut self,
        text: &str,
        edit_origin: EditOrigin,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if matches!(edit_origin, EditOrigin::UserTyped) {
            self.model.lock().set_is_input_dirty(true);
        }
        // If not in Agent Mode, clear any active text selections in the blocklist when inserting
        // new text. Note that the TerminalModel lock is instantly dropped after this expression,
        // since it's stored in a temporary variable.
        //
        // When `FeatureFlag::AgentView` is enabled, blocks are attachable as AI context in terminal
        // mode. Selections are preserved so they can be attached to the query when entering the
        // agent view.
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
            && !FeatureFlag::AgentView.is_enabled()
        {
            self.model.lock().block_list_mut().clear_selection();
        }

        ctx.focus(&self.editor);
        self.editor.update(ctx, |editor, ctx| match edit_origin {
            EditOrigin::UserTyped => editor.user_insert(text, ctx),
            EditOrigin::UserInitiated => {
                editor.user_initiated_insert(text, PlainTextEditorViewAction::SystemInsert, ctx)
            }
            EditOrigin::SystemEdit => {
                editor.system_insert(text, PlainTextEditorViewAction::SystemInsert, ctx)
            }
            EditOrigin::SyncedTerminalInput | EditOrigin::RemoteEdit => (),
        });
        ctx.notify();
        true
    }

    /// Returns the operations for any edits made to the latest buffer.
    pub fn latest_buffer_operations(&self) -> impl Iterator<Item = &CrdtOperation> {
        self.latest_buffer_operations.iter()
    }

    /// Applies the `operations` if the block ID of this buffer
    /// is equal to `block_id`. Otherwise, queues up these operations
    /// to be processed eventually when the block IDs are equal.
    pub fn process_remote_edits(
        &mut self,
        block_id: &BlockId,
        operations: Vec<CrdtOperation>,
        ctx: &mut ViewContext<Self>,
    ) {
        // We check the `block_id` against the cached latest block ID
        // rather than the latest terminal model state because the terminal
        // model can be updated off of the main thread. This can cause
        // scenarios where the terminal model has a new active block ID but
        // we haven't processed block completed events yet.
        //
        // Although we're checking against a potentially old block ID here,
        // we'll flush the right ops when we handle the block completed events.
        if block_id == &self.deferred_remote_operations.latest_block_id {
            self.editor.update(ctx, |editor, ctx| {
                editor.apply_remote_operations(operations, ctx);
            });
        } else {
            self.deferred_remote_operations
                .defer(block_id.clone(), operations);
        }
    }

    /// Updates the latest block ID to be equal to the latest block ID known to the terminal model
    /// and flushes any previously-deferred operations for this new block ID.
    pub fn refresh_deferred_remote_operations(&mut self, ctx: &mut ViewContext<Self>) {
        let latest_block_id = self.model.lock().block_list().active_block_id().clone();
        self.deferred_remote_operations.latest_block_id = latest_block_id;
        self.flush_deferred_remote_operations(ctx);
    }

    /// Flushes any deferred remote operations for the latest known block ID.
    fn flush_deferred_remote_operations(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(operations) = self.deferred_remote_operations.flush() {
            self.editor.update(ctx, |editor, ctx| {
                editor.apply_remote_operations(operations, ctx);
            });
        }
    }

    /// Resets state in the input box that depends on the block lifecycle.
    /// This is on a performance-sensitive path.
    ///
    /// If the newly created block is for an executed user command, the input buffer is cleared.
    pub fn handle_block_completed_event(
        &mut self,
        block_completed_event: BlockCompletedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // We clear the input box after executing a command here instead of where we
        // execute a command to avoid the input box flashing when its contents are
        // cleared. For the multiline input box case, this also caused contents to go
        // off the screen because we were forcing the long running command to be the same
        // size of the cleared input box.
        if let BlockType::User(user_block) = &block_completed_event.block_type {
            // Only clear the input buffer for user-executed commands, not agent-executed ones.
            let should_clear_buffer = !user_block.was_part_of_agent_interaction;
            let latest_block_id = self.model.lock().block_list().active_block_id().clone();
            let input_contents_before_prompt_chip_command =
                self.input_contents_before_prompt_chip_command.take();

            if should_clear_buffer {
                // We want to reinitialize the buffer whenever a command is completed so that
                // state does not leak from buffer to buffer (e.g. edit history).
                if self.deferred_remote_operations.latest_block_id != latest_block_id {
                    self.deferred_remote_operations.latest_block_id = latest_block_id;
                    self.editor
                        .update(ctx, |editor, ctx| editor.reinitialize_buffer(None, ctx));
                    self.latest_buffer_operations = Vec::new();

                    // If we have a pending input restore (from a prompt chip command like cd),
                    // restore the input contents instead of leaving the buffer empty.
                    if let Some(restore_text) = input_contents_before_prompt_chip_command {
                        self.editor.update(ctx, |editor, ctx| {
                            editor.set_buffer_text(&restore_text, ctx);
                        });
                        self.is_editor_empty_on_last_edit = false;
                    } else {
                        // This is the one place where buffer contents can change without an `Edit`
                        // -- this is because the buffer semantically isn't being edited, a new one is
                        // being constructed. We can guarantee in this case that the buffer was previously
                        // non-empty and should emit this event, because this code path is executed upon block
                        // completion in response to an executed command, though this guarantee is not explicitly
                        // enforced by the code.
                        self.is_editor_empty_on_last_edit = true;
                        ctx.emit(Event::InputEmptyStateChanged {
                            is_empty: true,
                            reason: InputEmptyStateChangeReason::UserCommandCompleted,
                        });
                    }
                }
            } else {
                // For agent-executed commands, still update the latest block ID but don't clear the buffer
                if self.deferred_remote_operations.latest_block_id != latest_block_id {
                    self.deferred_remote_operations.latest_block_id = latest_block_id;
                }
            }

            // Make sure the viewer's interaction state is correct based on their role.
            // We may have locked up their input if they tried to execute a command.
            if let SharedSessionStatus::ActiveViewer { role } =
                self.model.lock().shared_session_status()
            {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_interaction_state(role.into(), ctx);

                    // Also need to set the text colors back to normal.
                    let appearance: &Appearance = Appearance::as_ref(ctx);
                    editor.set_text_colors(TextColors::from_appearance(appearance), ctx);
                });

                if let Some(shared_session_input_state) = self.shared_session_input_state.as_mut() {
                    shared_session_input_state.pending_command_execution_request = None;
                };
            }

            // Update the segmented control disabled state based on the new state.
            self.universal_developer_input_button_bar
                .update(ctx, |button_bar, ctx| {
                    button_bar.update_segmented_control_disabled_state(ctx);
                });

            // Generate autosuggestion if the input is not empty (user had type-ahead).
            self.maybe_generate_autosuggestion(ctx);
        }

        self.input_render_state_model_handle
            .update(ctx, |input_render_state_model, _| {
                input_render_state_model.set_editor_modified_since_block_finished(false);
            });

        // Re-render for anything that depends on the block list (e.g. zero state AM chips).
        ctx.notify();
    }

    /// Performs any post-block completion processing that's relevant to the input.
    ///
    /// This is triggered after [`Self::handle_block_completed_event`] as
    /// the handling of the main block completed event is a sensitive path.
    pub fn handle_after_block_completed_event(
        &mut self,
        block: BlockType,
        ctx: &mut ViewContext<Self>,
    ) {
        if let BlockType::User(block_completed) = block {
            self.last_user_block_completed = Some(block_completed.clone());

            let is_in_fullscreen_agent_view =
                self.agent_view_controller.as_ref(ctx).is_fullscreen();
            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                // If the user has autodetection enabled, unlock the input mode.
                // Otherwise, keep it locked in the current mode.
                let new_config = ai_input_model
                    .input_config()
                    .unlocked_if_autodetection_enabled(is_in_fullscreen_agent_view, ctx);
                ai_input_model.set_input_config(new_config, false, ctx);
            });

            let viewing_shared_session = self.model.lock().shared_session_status().is_viewer();
            if viewing_shared_session {
                // As we switch to the new block ID, if there were any remote
                // edits that were pending for that block ID, we should flush them.
                // Today, we only expect this to be the case with session-sharing viewers.
                self.flush_deferred_remote_operations(ctx);

                // Update shared session history model
                if let Some(shared_session_history_model) = self
                    .shared_session_input_state
                    .as_ref()
                    .map(|state| state.history_model.clone())
                {
                    shared_session_history_model.update(ctx, |history_model, _ctx| {
                        history_model.push(HistoryEntry::for_completed_block(
                            block_completed.command,
                            &block_completed.serialized_block,
                        ))
                    })
                } else {
                    log::warn!("Tried to access non-existent shared session history model")
                }
            } else if is_next_command_enabled(ctx) {
                self.maybe_predict_next_action_ai(block_completed, ctx);
            }

            ctx.emit(Event::InputStateChanged(InputState::Enabled));
        } else if block.is_bootstrap_block()
            && self
                .model
                .lock()
                .block_list()
                .is_bootstrapping_precmd_done()
        {
            // When a bootstrap block is completed and the session is now
            // post-bootstrap, post-precmd, we know that the active block ID
            // is the block ID that we want to key input updates off of
            // (the block IDs during bootstrap are meaningless).
            self.refresh_deferred_remote_operations(ctx);

            // If the user typed ahead during bootstrap, the autosuggestion and
            // completions-as-you-type requests were silently skipped (history
            // wasn't queryable, session ID was absent). Now that bootstrap is
            // done, retry them so ghost text appears without the user having to
            // re-type.
            if !self.buffer_text(ctx).is_empty() {
                self.maybe_generate_autosuggestion(ctx);

                if self.should_show_completions_while_typing(ctx) {
                    self.open_completion_suggestions(CompletionsTrigger::AsYouType, ctx);
                }
            }
        }
    }

    /// 'Starts' the active block and sends its command bytes to the pty.
    ///
    /// Additionally, the executed command is recorded to history if appropriate.
    fn start_block_and_write_command_to_pty(
        &mut self,
        command: &str,
        source: CommandExecutionSource,
        ctx: &mut ViewContext<Self>,
    ) {
        start_trace!("command_execution:start");

        // Abort running completions since we're about to execute a command.
        if let Some(abort_handle) = self.completions_abort_handle.take() {
            abort_handle.abort();
        }
        self.abort_latest_autosuggestion_future();

        if let Some(future_handle) = self.decorations_future_handle.take() {
            future_handle.abort_handle().abort();
        }

        let session_id = self
            .active_block_session_id()
            .expect("session_id should be set (via bootstrap) before executing command");

        // If the SelectedWorkflowState is populated with a workflow, we count this as a workflow execution.
        let (workflow_id, workflow_command) = {
            match self.workflows_state.selected_workflow_state.as_ref() {
                Some(selected_workflow_state) => {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::WorkflowExecuted(WorkflowTelemetryMetadata {
                            workflow_source: selected_workflow_state.workflow_source,
                            workflow_categories: selected_workflow_state
                                .workflow_type
                                .as_workflow()
                                .tags()
                                .cloned(),
                            workflow_selection_source: selected_workflow_state
                                .workflow_selection_source,
                            // This is only `Some()` for WarpDrive workflows; we don't track
                            // ID for execution of local workflows because they have no such
                            // unique ID.
                            workflow_id: selected_workflow_state.workflow_type.server_id(),
                            workflow_space: match &selected_workflow_state.workflow_type {
                                WorkflowType::Cloud(workflow) => Some(workflow.space(ctx).into()),
                                _ => None,
                            },
                            enum_ids: selected_workflow_state
                                .workflow_type
                                .as_workflow()
                                .get_server_enum_ids()
                        }),
                        ctx
                    );

                    let workflow_type = &selected_workflow_state.workflow_type;
                    let workflow_id = match workflow_type {
                        WorkflowType::Cloud(workflow) => Some(workflow.id),
                        _ => None,
                    };

                    // If the SelectedWorkflowState is populated, then we're always able to return the workflow command.
                    // The case where workflow_id = None but workflow_command = Some() is when it's a local workflow, which
                    // don't have ids and are tracked just by persisting the workflow contents. This is a little janky and would
                    // be fixed if we could identify all workflows under a unified id system, not just cloud ones.
                    (
                        workflow_id,
                        workflow_type
                            .as_workflow()
                            .command()
                            .map(|command| command.to_owned()),
                    )
                }
                None => (None, None),
            }
        };

        ctx.emit(Event::ExecuteCommand(Box::new(ExecuteCommandEvent {
            command: command.to_string(),
            workflow_id,
            session_id,
            workflow_command,
            should_add_command_to_history: true,
            source,
        })));
        end_trace!();
    }

    pub fn notify_and_notify_children(&self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        // The left notch may have been updated due to the prompt updating, in the case of
        // same-line prompt!
        self.editor.update(ctx, |_editor, ctx| {
            ctx.notify();
        });
    }

    /// Returns a tuple (prompt_text, rprompt_text).
    pub fn prompt_and_rprompt_text(&self, app: &AppContext) -> (String, Option<String>) {
        let model = self.model.lock();
        let appearance = Appearance::as_ref(app);
        let (lprompt_top, lprompt_bottom, rprompt) = self
            .prompt_render_helper
            .render_prompt(&model, appearance, app);
        // Separate this into a helper (follow-up PR?)

        let show_universal_developer_input = self.should_show_universal_developer_input(app);

        let lprompt_top_text = lprompt_top.map(|rendered| rendered.element.text(app));
        let lprompt_bottom_text = lprompt_bottom.map(|rendered| rendered.element.text(app));
        let rprompt_text = rprompt.map(|rendered| rendered.element.text(app));
        if should_render_prompt_on_same_line(show_universal_developer_input, &model, app) {
            if let Some(lprompt_top_text) = lprompt_top_text {
                (
                    lprompt_top_text + "\n" + &lprompt_bottom_text.unwrap_or_default(),
                    rprompt_text,
                )
            } else {
                (lprompt_bottom_text.unwrap_or_default(), rprompt_text)
            }
        } else {
            (lprompt_top_text.unwrap_or_default(), rprompt_text)
        }
    }

    pub fn create_prompt_elements(&self, app: &AppContext) -> SessionNavigationPromptElements {
        let model = self.model.lock();
        let block = self.prompt_render_helper.prompt_block(&model);
        let is_udi = InputSettings::as_ref(app).is_universal_developer_input_enabled(app);
        let mut prompt_elements = SessionNavigationPromptElements {
            ps1_prompt_grid: None,
            prompt_chip_snapshot: None,
        };

        if let Some(block) = block {
            if !is_udi && block.honor_ps1() && model.block_list().is_bootstrapped() {
                // PS1 mode: capture the raw prompt grid so the command palette
                // can render it with full fidelity (CORE-1683).
                prompt_elements.ps1_prompt_grid = Some(block.prompt_grid().clone());
            }
        }

        // Always capture a chip snapshot as the fallback prompt representation.
        // This covers both UDI mode and any edge cases where PS1 is not available
        // (e.g. not yet bootstrapped, block-level honor_ps1 mismatch).
        if prompt_elements.ps1_prompt_grid.is_none() {
            prompt_elements.prompt_chip_snapshot = Some(self.prompt_type.as_ref(app).snapshot(app));
        }
        prompt_elements
    }

    /// This function determines if the subshell flag should be in the input editor. The flag
    /// should show here if there are no blocks in the block list for this subshell session, which
    /// will be the case if no non-hidden blocks have been executed yet or the block list was
    /// cleared.
    fn get_subshell_flag_render_state(
        &self,
        model: &TerminalModel,
        spacing_is_compact: bool,
        app: &AppContext,
    ) -> Option<SubshellRenderState> {
        if spacing_is_compact {
            return None;
        }
        let session_id = self.active_block_session_id()?;
        let should_render = self
            .sessions
            .as_ref(app)
            .get(session_id)
            .and_then(|session| {
                session.subshell_info().as_ref().map(|info| {
                    if let Some(env_var_collection_name) = &info.env_var_collection_name {
                        Some(SubshellRenderState::Flag(SubshellSource::EnvVarCollection(
                            env_var_collection_name.to_owned(),
                        )))
                    } else {
                        info.spawning_command.split_whitespace().next().map(|exec| {
                            SubshellRenderState::Flag(SubshellSource::Command(exec.to_owned()))
                        })
                    }
                })
            })?;

        let block_list = model.block_list();
        let block_before_active_block = block_list
            .prev_non_hidden_block_from_index(block_list.active_block_index())
            .and_then(|index| block_list.block_at(index));

        match block_before_active_block {
            // If there is a block before the editor, and it belongs to this same subshell session,
            // the flag will be in the block list, and hence doesn't need to be in the editor.
            // Only extend the flag into the editor.
            Some(block) if block.session_id() == Some(session_id) => {
                Some(SubshellRenderState::Flagpole)
            }
            // Otherwise, this editor (the active block) is the first in this subshell session, and
            // we should show the flag here.
            _ => should_render,
        }
    }

    pub fn set_active_block_metadata(
        &mut self,
        active_block_metadata: BlockMetadata,
        is_after_in_band_command: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let active_session = active_block_metadata
            .session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id));
        if let Some(session) = active_session {
            let transformer: Option<PathTransformerFn> = session
                .windows_path_converter()
                .map(|convert| Box::new(convert) as PathTransformerFn);
            self.editor.update(ctx, |editor, _| {
                editor.set_shell_family(session.shell().shell_type().into());
                editor.set_drag_drop_path_transformer(transformer);
            });
            self.input_suggestions.update(ctx, |input_suggestions, _| {
                input_suggestions.set_path_separators(session.path_separators());
            });
        }
        self.active_block_metadata = Some(active_block_metadata);

        // If needed, update the prompt display with the now-available session
        // context. In-band commands don't meaningfully change block metadata,
        // so only update prompt display chips if the previous block was not an
        // in-band command (i.e.: was probably a user-executed block).
        //
        // If we update the prompt display chips here, we can get into infinite
        // loops where we run an in-band command to compute an updated value for
        // a chip (e.g.: listing the files in the current directory), which
        // triggers another in-band command, etc. etc.
        if !is_after_in_band_command {
            self.update_prompt_display_chips(ctx);
        }
    }

    pub fn update_prompt_display_chips(&mut self, ctx: &mut ViewContext<Self>) {
        let session_context = self.completion_session_context(ctx);

        self.prompt_render_helper
            .prompt_view()
            .update(ctx, |prompt, prompt_ctx| {
                prompt.update_session_context(session_context.clone(), prompt_ctx);
            });

        self.agent_input_footer.update(ctx, |footer, footer_ctx| {
            footer.update_session_context(session_context, footer_ctx);
        });
    }

    pub fn update_repo_path(&mut self, repo_path: Option<PathBuf>, ctx: &mut ViewContext<Self>) {
        self.prompt_render_helper
            .prompt_view()
            .update(ctx, |prompt, prompt_ctx| {
                prompt.update_repo_path(repo_path.clone(), prompt_ctx);
            });

        self.agent_input_footer.update(ctx, |footer, footer_ctx| {
            footer.set_current_repo_path(repo_path.clone(), footer_ctx);
        });

        self.slash_command_data_source.update(ctx, {
            let repo_path = repo_path.clone();
            |data_source, ctx| {
                data_source.set_active_repo_root(repo_path, ctx);
            }
        });
        if let Some(data_source) = self.cloud_mode_composer_slash_command_data_source.as_ref() {
            data_source.update(ctx, |data_source, ctx| {
                data_source.set_active_repo_root(repo_path, ctx);
            });
        }
    }

    fn active_session_path_if_local(&self, ctx: &ViewContext<Self>) -> Option<&Path> {
        self.active_block_session_id().and_then(|session_id| {
            let current_session = self.sessions.as_ref(ctx).get(session_id)?;
            if current_session.is_local() {
                self.active_block_metadata
                    .as_ref()
                    .and_then(BlockMetadata::current_working_directory)
                    .map(Path::new)
            } else {
                None
            }
        })
    }

    fn apply_input_banner_padding(
        &self,
        banner: Box<dyn Element>,
        is_compact_mode: bool,
        input_mode: InputMode,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let constrained_banner = ConstrainedBox::new(banner)
            .with_height(2. * appearance.line_height_ratio() * appearance.monospace_font_size())
            .finish();
        let should_use_udi_spacing = self.should_show_universal_developer_input(app)
            || (FeatureFlag::AgentView.is_enabled()
                && self.agent_view_controller.as_ref(app).is_active());
        let mut container: Container = Container::new(constrained_banner);
        let (suggestion_to_prompt_padding, suggestion_to_input_border_padding) =
            if should_use_udi_spacing {
                (0., 0.)
            } else if is_compact_mode {
                (0., 8.)
            } else {
                (-12., 8.)
            };

        container = match input_mode {
            InputMode::PinnedToTop => container
                .with_padding_top(suggestion_to_prompt_padding)
                .with_padding_bottom(suggestion_to_input_border_padding),
            InputMode::PinnedToBottom | InputMode::Waterfall => container
                .with_padding_bottom(suggestion_to_prompt_padding)
                .with_padding_top(suggestion_to_input_border_padding),
        };

        container.finish()
    }

    /// Renders a banner that should stay next to the input box.
    fn render_input_banner(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        input_mode: InputMode,
        is_compact_mode: bool,
    ) -> Option<Box<dyn Element>> {
        if let Some(prompt_suggestions_banner_state) = &self.prompt_suggestions_banner_state {
            if prompt_suggestions_banner_state.should_hide {
                return None;
            }

            let prompt_suggestions_banner = ChildView::new(&self.prompt_suggestions_view).finish();

            Some(self.apply_input_banner_padding(
                prompt_suggestions_banner,
                is_compact_mode,
                input_mode,
                appearance,
                app,
            ))
        } else {
            None
        }
    }

    fn render_attachment_chips(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if self.attachment_chips.is_empty() {
            None
        } else {
            let chips = self
                .attachment_chips
                .iter()
                .map(|chip| self.render_attached_chip(chip, appearance));

            Some(
                Wrap::row()
                    .with_run_spacing(spacing::UDI_CHIP_MARGIN)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_children(chips)
                    .finish(),
            )
        }
    }

    fn render_attached_chip(
        &self,
        chip: &AttachmentChip,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let chip_index = chip.index;
        let close_button = appearance
            .ui_builder()
            .close_button(
                appearance.monospace_font_size(),
                chip.mouse_state_handle.clone(),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::DeleteAttachment { index: chip_index });
            })
            .finish();

        let icon = match chip.attachment_type {
            AttachmentType::Image => Icon::Image,
            AttachmentType::File => Icon::File,
        };

        Chip::new(
            chip.file_name.clone(),
            UiComponentStyles {
                margin: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: 0.,
                    right: 6.,
                }),
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(appearance.monospace_font_size()),
                font_color: Some(blended_colors::text_main(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                border_width: Some(1.),
                border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
                ..Default::default()
            },
        )
        .with_icon(icon.to_warpui_icon(
            blended_colors::text_main(appearance.theme(), appearance.theme().background()).into(),
        ))
        .with_close_button(close_button)
        .build()
        .finish()
    }

    fn render_input_box(
        &self,
        show_vim_status: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Set editor height to be half of the terminal view height
        let editor_height = self.size_info(app).pane_height_px() / 2.0.into_pixels();

        // Round down editor height to be divisible by line height so we do not see partial lines
        let line_height = self
            .editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance)
            .into_pixels();
        let editor_height_rounded_down =
            (editor_height / line_height).round().max(1.0.into_pixels()) * line_height;

        let terminal_settings = TerminalSettings::as_ref(app);
        let terminal_spacing =
            terminal_settings.terminal_input_spacing(appearance.line_height_ratio(), app);
        let mut bottom_padding = terminal_spacing.editor_bottom_padding;

        // When `FeatureFlag::AgentView` is enabled, always render with UDI-style spacing values,
        // regardless of terminal/agent mode or prompt setting.
        let is_udi_style_spacing =
            self.should_show_universal_developer_input(app) || FeatureFlag::AgentView.is_enabled();

        let is_compact_mode =
            matches!(terminal_settings.spacing_mode.value(), SpacingMode::Compact)
                && !is_udi_style_spacing;

        // In compact mode, allocate some extra padding for the Vim status bar.
        if is_compact_mode && show_vim_status {
            bottom_padding = VIM_STATUS_BAR_BOTTOM_PADDING;
        }

        if is_udi_style_spacing {
            bottom_padding = terminal_spacing.editor_bottom_padding - 4.;
        }

        let input_box = Container::new(
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_max_height(editor_height_rounded_down.as_f32())
                .finish(),
        )
        .with_padding_right(*TERMINAL_VIEW_PADDING_LEFT)
        .with_padding_bottom(bottom_padding)
        .finish();

        let input_editor_save_position_id = self.editor_save_position_id();
        SavePosition::new(
            EventHandler::new(input_box)
                .on_right_mouse_down(move |ctx, _, position| {
                    let input_rect = ctx
                        .element_position_by_id(input_editor_save_position_id.clone())
                        .expect("input editor position id should be saved");
                    let offset_position = position - input_rect.origin();
                    ctx.dispatch_typed_action(TerminalAction::OpenInputContextMenu {
                        position: offset_position,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            &self.editor_save_position_id(),
        )
        .finish()
    }

    // TODO remove voltron from the code given we are not using it anymore, and we have universal search instead.
    fn select_and_refresh_voltron(
        &mut self,
        feature_item: VoltronItem,
        ctx: &mut ViewContext<Input>,
    ) {
        // View-only sessions should not show workflows menu
        if self.model.lock().shared_session_status().is_reader() {
            return;
        }

        let welcome_tip_feature = match feature_item {
            VoltronItem::AiCommands => Some(Tip::Action(TipAction::AiCommandSearch)),
            VoltronItem::History => Some(Tip::Action(TipAction::HistorySearch)),
            VoltronItem::Workflows => None,
        };

        if let Some(welcome_tip_feature) = welcome_tip_feature {
            self.tips_completed.update(ctx, |tips_completed, ctx| {
                mark_feature_used_and_write_to_user_defaults(
                    welcome_tip_feature,
                    tips_completed,
                    ctx,
                );
                ctx.notify();
            });
        }
        // If input suggestions are opened we should close them when opening voltron
        if self.suggestions_mode_model.as_ref(ctx).is_visible() {
            self.close_input_suggestions_and_restore_buffer(true, true, ctx);
        }
        let active_session_path_if_local = self.active_session_path_if_local(ctx);
        let menu_positioning = self.menu_positioning(ctx);
        let metadata = VoltronMetadata {
            active_session_path_if_local: active_session_path_if_local.map(|path| path.into()),
            starting_editor_text: Some(self.editor.as_ref(ctx).buffer_text(ctx)),
            keymap_context: Self::keymap_context(self, ctx),
            menu_positioning,
        };

        self.voltron_view.update(ctx, |voltron, ctx| {
            voltron.select_and_refresh_by_name(feature_item, metadata, ctx);
            self.is_voltron_open = true;
        });
        ctx.notify();
    }

    /// Returns whether AI command search should be displayed for the given
    /// editor contents.
    fn editor_starts_with_command_search_trigger(&self, ctx: &AppContext) -> bool {
        self.buffer_text(ctx).starts_with(AI_COMMAND_SEARCH_TRIGGER)
    }

    /// Returns whether the buffer contains any attachment patterns (blocks, drive objects, or diffs).
    /// These patterns indicate the user is referencing context that requires AI mode.
    fn buffer_contains_attachment_patterns(buffer_text: &str) -> bool {
        BLOCK_CONTEXT_ATTACHMENT_REGEX.is_match(buffer_text)
            || DRIVE_OBJECT_ATTACHMENT_REGEX.is_match(buffer_text)
            || DIFF_HUNK_ATTACHMENT_REGEX.is_match(buffer_text)
    }

    /// Shows the AI command search panel.
    ///
    /// This modifies the input buffer as needed to display the panel (i.e.:
    /// inserting a leading #, which is the trigger when typed manually by the
    /// user).
    fn show_ai_command_search(&mut self, ctx: &mut ViewContext<Input>) {
        // Should not show ai command search for read-only viewers
        if self.model.lock().shared_session_status().is_reader() {
            return;
        }
        // If the editor doesn't contain the necessary trigger for AI command
        // search, update its buffer accordingly.
        let buffer_starts_with_trigger = self.editor_starts_with_command_search_trigger(ctx);
        if !buffer_starts_with_trigger {
            let updated_text = format!("{AI_COMMAND_SEARCH_TRIGGER} {}", self.buffer_text(ctx));
            self.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&updated_text, ctx);
            });
        }

        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::AiCommandSearch),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        ctx.emit(Event::ShowCommandSearch(Default::default()));

        let entrypoint = if buffer_starts_with_trigger {
            AICommandSearchEntrypoint::ShortHandTrigger
        } else {
            AICommandSearchEntrypoint::Keybinding
        };
        send_telemetry_from_ctx!(TelemetryEvent::AICommandSearchOpened { entrypoint }, ctx);
        ctx.notify();
    }

    /// Returns the SavePosition ID for the input.
    ///
    /// This may be used by parent views to position UI elements relative to the input.
    pub fn save_position_id(&self) -> String {
        format!("input_{}", self.view_id)
    }

    /// Returns the position ID for the input editor
    pub fn editor_save_position_id(&self) -> String {
        format!("input_editor_{}", self.view_id)
    }

    /// Returns the position ID for the (left) prompt.
    pub fn prompt_save_position_id(&self) -> String {
        format!("prompt_area_{}", self.view_id)
    }

    /// A save position for the bordered input alone,
    /// not including the status bar.
    pub fn status_free_input_save_position_id(&self) -> String {
        format!("status_free_input_{}", self.view_id)
    }

    /// Returns a reference to the universal developer input button bar, if it exists
    pub fn universal_developer_input_button_bar(
        &self,
    ) -> &ViewHandle<UniversalDeveloperInputButtonBar> {
        &self.universal_developer_input_button_bar
    }

    pub fn should_show_universal_developer_input(&self, app: &AppContext) -> bool {
        InputSettings::as_ref(app).is_universal_developer_input_enabled(app)
    }

    fn handle_prompt_suggestions_event(
        &mut self,
        event: &PromptSuggestionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PromptSuggestionsEvent::SignupAnonymousUser => ctx.emit(Event::SignupAnonymousUser {
                entrypoint: AnonymousUserSignupEntrypoint::SignUpAIPrompt,
            }),
            PromptSuggestionsEvent::OpenBillingAndUsagePage => {
                ctx.emit(Event::OpenSettings(SettingsSection::BillingAndUsage))
            }
            PromptSuggestionsEvent::OpenPrivacyPage => {
                ctx.emit(Event::OpenSettings(SettingsSection::Privacy))
            }
            PromptSuggestionsEvent::OpenBillingPortal { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
        }
    }

    /// Returns whether the input box is currently pinned to the top of the screen.
    fn is_input_at_top(&self, model: &TerminalModel, ctx: &AppContext) -> bool {
        match InputModeSettings::as_ref(ctx).input_mode.value() {
            InputMode::PinnedToBottom => false,
            InputMode::PinnedToTop => true,
            InputMode::Waterfall => model.is_block_list_empty(),
        }
    }
}

impl Entity for Input {
    type Event = Event;
}

impl TypedActionView for Input {
    type Action = InputAction;

    fn action_accessibility_contents(
        &mut self,
        action: &InputAction,
        _: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            InputAction::FocusInputBox => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new(
                    INPUT_A11Y_LABEL,
                    // TODO (a11y) use bindings from user settings
                    INPUT_A11Y_HELPER,
                    WarpA11yRole::TextareaRole,
                ))
            }
            _ => ActionAccessibilityContent::Empty,
        }
    }

    fn handle_action(&mut self, action: &InputAction, ctx: &mut ViewContext<Self>) {
        match action {
            InputAction::FocusInputBox => self.focus_input_box(ctx),
            InputAction::Up => self.editor_up(ctx),
            InputAction::PageUp => self.editor_page_up(ctx),
            InputAction::PageDown => self.editor_page_down(ctx),
            InputAction::CtrlD => self.ctrl_d(ctx),
            InputAction::CtrlR => self.ctrl_r(ctx),
            InputAction::ClearScreen => self.clear_screen(ctx),
            InputAction::SelectAndRefreshVoltron(feature_name) => {
                self.select_and_refresh_voltron(*feature_name, ctx);
            }
            InputAction::ShowAiCommandSearch => self.show_ai_command_search(ctx),
            InputAction::MaybeOpenCompletionSuggestions => {
                self.maybe_open_completion_suggestions(ctx);
            }
            InputAction::HideWorkflowInfoCard => self.hide_workflows_info_box(ctx),
            InputAction::ResetWorkflowState => self.reset_workflow_state(None, ctx),
            InputAction::ToggleClassicCompletionsMode => {
                InputSettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings.classic_completions_mode.toggle_and_save_value(ctx) {
                        log::warn!(
                            "Failed to toggle and save classic completions mode setting: {e}."
                        )
                    }
                });
            }
            InputAction::ToggleConversationsMenu => {
                if self
                    .suggestions_mode_model
                    .as_ref(ctx)
                    .is_conversation_menu()
                {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.close_and_restore_buffer(ctx);
                    });
                    ctx.notify();
                } else {
                    self.open_conversation_menu(ctx);
                }
            }
            InputAction::ToggleInputAutoDetection => {
                if let Ok(new_value) =
                    AISettings::handle(ctx).update(ctx, |ai_settings, model_ctx| {
                        ai_settings
                            .ai_autodetection_enabled_internal
                            .toggle_and_save_value(model_ctx)
                    })
                {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeToggleAutoDetectionSetting {
                            is_autodetection_enabled: new_value,
                            origin: AgentModeAutoDetectionSettingOrigin::Banner
                        },
                        ctx
                    );
                }
            }
            InputAction::CycleNextCommandSuggestion => {
                self.cycle_next_command_suggestion(ctx);
            }
            InputAction::InsertZeroStatePromptSuggestion(suggestion_type) => {
                self.insert_zero_state_prompt_suggestion(
                    *suggestion_type,
                    ZeroStatePromptSuggestionTriggeredFrom::InputBar,
                    ctx,
                );
            }
            InputAction::EnableAutoDetection => {
                // Call the same logic that clicking the lightbulb icon triggers
                self.handle_universal_developer_input_button_bar_event(
                    &UniversalDeveloperInputButtonBarEvent::EnableAutoDetection,
                    ctx,
                );
            }
            InputAction::TryHandlePassiveCodeDiff(action) => {
                ctx.emit(Event::TryHandlePassiveCodeDiff(action.clone()));
            }
            InputAction::ToggleAgentViewShortcuts => {
                self.agent_shortcut_view_model.update(ctx, |model, ctx| {
                    if model.is_shortcut_view_open() {
                        model.hide_shortcut_view(ctx);
                    } else {
                        model.open_shortcut_view(ctx);
                    }
                });
            }
            InputAction::ClearAndResetAIContextMenuQuery => {
                self.clear_and_reset_ai_context_menu_query(ctx);
            }
            InputAction::SetUDIHovered(is_hovered) => {
                self.universal_developer_input_button_bar
                    .update(ctx, |button_bar, ctx| {
                        button_bar.set_udi_hovered(*is_hovered, ctx);
                    });
            }
            InputAction::UpdateCompletionsMenuWidth(width) => {
                InputSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.completions_menu_width.set_value(*width, ctx));
                });
            }
            InputAction::UpdateCompletionsMenuHeight(height) => {
                InputSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.completions_menu_height.set_value(*height, ctx));
                });
            }
            InputAction::ToggleSlashCommandsMenu => {
                self.toggle_legacy_slash_commands_menu(ctx);
            }
            InputAction::TriggerSlashCommandFromKeybinding(command_name) => {
                let Some(command) = COMMAND_REGISTRY.get_command_with_name(command_name) else {
                    return;
                };
                self.select_slash_command(command, SlashCommandTrigger::keybinding(), ctx);
            }
            InputAction::StartNewAgentConversation => {
                // Block starting a new conversation if the agent is in control of a long-running command
                if !self
                    .ai_context_model
                    .as_ref(ctx)
                    .can_start_new_conversation()
                {
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Cannot start a new conversation while agent is monitoring a command.".to_string()
                            ),
                            window_id,
                            ctx,
                        );
                    });
                    return;
                }

                if FeatureFlag::AgentView.is_enabled() {
                    if let Err(e) = self.agent_view_controller.update(ctx, |controller, ctx| {
                        controller.try_enter_agent_view(
                            None,
                            AgentViewEntryOrigin::Input {
                                was_prompt_autodetected: false,
                            },
                            ctx,
                        )
                    }) {
                        log::warn!("Failed to start new agent conversation from zero-state: {e:?}");
                    }
                } else if self.should_show_universal_developer_input(ctx) {
                    // Clear follow-up state (start a fresh conversation)
                    self.ai_context_model.update(ctx, |ai_context_model, ctx| {
                        ai_context_model.set_pending_query_state_for_new_conversation(
                            // This is a placeholder origin, this codepath is dead when AgentView is enabled.
                            AgentViewEntryOrigin::Input {
                                was_prompt_autodetected: false,
                            },
                            ctx,
                        );
                    });
                    self.enter_ai_mode(ctx);
                }
            }
            InputAction::OpenInlineHistoryMenu => {
                self.open_inline_history_menu(ctx);
            }
            InputAction::DismissCloudModeV2SlashCommandsMenu => {
                if self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                    self.slash_command_model
                        .update(ctx, |model, ctx| model.disable(ctx));
                    self.close_slash_commands_menu(ctx);
                }
            }
            InputAction::OpenModelSelector => {
                self.open_model_selector(ctx);
            }
            InputAction::FigmaAddButtonClicked => {
                TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.install_figma_from_gallery(ctx);
                });
            }
            InputAction::FigmaEnableButtonClicked => {
                TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.enable_figma_mcp(ctx);
                });
            }
            InputAction::ClearAttachedContext => {
                self.clear_attached_context(ctx);
            }
            InputAction::ActivateCloudHandoff => {
                self.activate_cloud_handoff_compose(HandoffEntryPoint::Ampersand, ctx);
            }
        }
    }
}

impl View for Input {
    fn ui_name() -> &'static str {
        "Input"
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            INPUT_A11Y_LABEL,
            // TODO (a11y) use bindings from user settings
            INPUT_A11Y_HELPER,
            WarpA11yRole::TextareaRole,
        ))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            if self.is_voltron_open {
                ctx.focus(&self.voltron_view);
            } else if self.prompt_render_helper.has_open_chip_menu(ctx) {
                // Focus the PromptDisplay, which will in turn focus any open chip menu
                ctx.focus(self.prompt_render_helper.prompt_view());
            } else if self.agent_input_footer.as_ref(ctx).has_open_chip_menu(ctx) {
                // Focus the AgentInputFooter, which will in turn focus any open chip menu
                ctx.focus(&self.agent_input_footer);
            } else {
                self.close_voltron(ctx);
                ctx.focus(&self.editor);
                ctx.notify();
            }
            ctx.dispatch_typed_action(&PaneGroupAction::HandleFocusChange);
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut ctx = Self::default_keymap_context();
        let ai_settings = AISettings::as_ref(app);

        if self.is_voltron_open {
            ctx.set.insert("VoltronActive");
        }

        if self.ai_input_model.as_ref(app).is_ai_input_enabled() {
            ctx.set.insert("AIInput");
        }

        if InputSettings::as_ref(app).is_universal_developer_input_enabled(app) {
            ctx.set.insert("UniversalDeveloperInput");
        }

        if self.ai_input_model.as_ref(app).is_ai_input_enabled() {
            ctx.set.insert(flags::AGENT_MODE_INPUT);
        } else {
            ctx.set.insert(flags::TERMINAL_MODE_INPUT);
        }

        if self.ai_input_model.as_ref(app).is_input_type_locked() {
            ctx.set.insert(flags::LOCKED_INPUT);
        }

        // Keep Input's keymap context in sync with TerminalView's context for AgentView-related
        // bindings (e.g. cmd-i).
        if FeatureFlag::AgentView.is_enabled() {
            ctx.set.insert(flags::AGENT_VIEW_ENABLED);
            let agent_view_state = self.agent_view_controller.as_ref(app).agent_view_state();
            if agent_view_state.is_fullscreen() {
                ctx.set.insert(flags::ACTIVE_AGENT_VIEW);
            } else if agent_view_state.is_inline() {
                ctx.set.insert(flags::ACTIVE_INLINE_AGENT_VIEW);
            }
        }

        if self.buffer_text(app).is_empty() {
            ctx.set.insert(flags::EMPTY_INPUT_BUFFER);
        }

        if ai_settings.is_any_ai_enabled(app) {
            ctx.set.insert(flags::IS_ANY_AI_ENABLED);
        }

        if *InputSettings::as_ref(app)
            .enable_slash_commands_in_terminal
            .value()
        {
            ctx.set.insert(flags::SLASH_COMMANDS_IN_TERMINAL_FLAG);
        }

        if ai_settings.is_ai_autodetection_enabled(app) {
            ctx.set.insert(flags::AI_INPUT_AUTODETECTION_FLAG);
        }

        if ai_settings.is_code_suggestions_enabled(app) {
            ctx.set.insert(flags::CODE_SUGGESTIONS_FLAG);
        }

        if let Some(workflow) = self.workflows_state.selected_workflow_state.clone() {
            if workflow.should_show_more_info_view {
                ctx.set.insert("WorkflowInfoBox");
            }
        }

        let is_profile_model_selector_open = self.should_show_universal_developer_input(app)
            && self
                .universal_developer_input_button_bar
                .as_ref(app)
                .is_profile_model_selector_open(app);
        let is_agent_footer_model_selector_open = self
            .agent_input_footer
            .as_ref(app)
            .is_model_selector_open(app);
        let is_v2_model_selector_open = self
            .agent_input_footer
            .as_ref(app)
            .is_v2_model_selector_open(app);
        let is_v2_host_selector_open = self
            .host_selector()
            .is_some_and(|view| view.as_ref(app).is_menu_open());
        let is_v2_harness_selector_open = self
            .harness_selector()
            .is_some_and(|view| view.as_ref(app).is_menu_open());
        let is_v2_environment_selector_open = self
            .agent_input_footer
            .as_ref(app)
            .is_v2_environment_selector_open(app);
        if is_profile_model_selector_open
            || is_agent_footer_model_selector_open
            || is_v2_model_selector_open
            || is_v2_host_selector_open
            || is_v2_harness_selector_open
            || is_v2_environment_selector_open
        {
            ctx.set.insert("ProfileModelSelectorOpen");
        }

        if self.prompt_render_helper.has_open_chip_menu(app)
            || self.agent_input_footer.as_ref(app).has_open_chip_menu(app)
        {
            ctx.set.insert("PromptChipMenuOpen");
        }

        if BlocklistAIHistoryModel::as_ref(app)
            .all_live_conversations_for_terminal_view(self.terminal_view_id)
            .any(|conversation| conversation.initial_user_query().is_some())
        {
            ctx.set.insert("ActiveAIConversationHasHistory");
        }

        if AppEditorSettings::as_ref(app).vim_mode_enabled() {
            ctx.set.insert("VimModeEnabled");
        }

        if let Some(VimMode::Normal) = self.editor.as_ref(app).vim_mode(app) {
            ctx.set.insert("VimNormalMode");
        }

        if matches!(
            self.suggestions_mode_model.as_ref(app).mode(),
            InputSuggestionsMode::AIContextMenu { .. }
        ) {
            ctx.set.insert("AIContextMenuOpen");
        } else if self
            .suggestions_mode_model
            .as_ref(app)
            .is_conversation_menu()
        {
            ctx.set.insert(flags::OPEN_INLINE_CONVERSATION_MENU);
        }

        if self
            .buy_credits_banner
            .as_ref(app)
            .is_denomination_dropdown_open(app)
        {
            ctx.set.insert("BuyCreditsBannerOpen");
        }

        let model_lock = self.model.lock();
        ctx.set
            .insert(model_lock.shared_session_status().as_keymap_context());

        if model_lock
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            ctx.set.insert("LongRunningCommand");
        }

        if model_lock.is_block_list_empty() {
            ctx.set.insert("TerminalView_EmptyBlockList");
        } else {
            ctx.set.insert("TerminalView_NonEmptyBlockList");
        }

        // Only enable keybindings for passive code diffs when there is one pending in the
        // blocklist that is undismissed (i.e. keybindings are shown in the banner/block).
        // This is to prevent any keybinding conflicts (with actions such as split pane
        // down on non-Macs).
        let has_undismissed_passive_code_diff = model_lock
            .block_list()
            .last_non_hidden_ai_block_handle(app)
            .is_some_and(|ai_block| {
                let block = ai_block.as_ref(app);
                block.is_passive_conversation(app)
                    && block.find_undismissed_code_diff(app).is_some()
            });
        if has_undismissed_passive_code_diff {
            ctx.set.insert(flags::PASSIVE_CODE_DIFF_KEYBINDINGS_ENABLED);
        }

        for (_, command) in self.slash_command_data_source.as_ref(app).active_commands() {
            ctx.set.insert(command.name);
        }

        ctx
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if CLIAgentSessionsModel::as_ref(app).is_input_open(self.terminal_view_id) {
            return self.render_cli_agent_input(app);
        }
        let is_universal_input = self.should_show_universal_developer_input(app);
        let should_show_status_footer =
            self.ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model.as_ref(app).should_show_status_footer()
                });

        if FeatureFlag::CloudMode.is_enabled() && should_show_status_footer {
            self.render_ambient_agent_status_footer(app)
        } else if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_active()
        {
            self.render_agent_input(app)
        } else if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(app).is_active()
            && !should_render_ps1_prompt(&self.model.lock(), app)
        {
            self.render_terminal_input(app)
        } else if !FeatureFlag::AgentView.is_enabled() && is_universal_input {
            self.render_universal_developer_input(app)
        } else {
            self.render_classic_input(app)
        }
    }
}

impl Autosuggester for Input {
    fn on_autosuggestion_result(
        &mut self,
        result: AutoSuggestionResult,
        ctx: &mut ViewContext<Self>,
    ) {
        let buffer_text = result.buffer_text;
        if self.editor.as_ref(ctx).buffer_text(ctx) != buffer_text {
            return;
        }

        let autosuggestion_result_substring = result
            .autosuggestion_result
            .as_ref()
            .and_then(|result| result.strip_prefix(buffer_text.as_str()));

        if let Some(autosuggestion) = autosuggestion_result_substring {
            self.set_autosuggestion(
                autosuggestion,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
        }
    }

    fn abort_latest_autosuggestion_future(&mut self) {
        if let Some(last_abort_handle) = self.autosuggestions_abort_handle.take() {
            last_abort_handle.abort();
        }
    }

    fn set_autosuggestion_future(&mut self, abort_handle: AbortHandle) {
        self.autosuggestions_abort_handle = Some(abort_handle);
    }
}

/// Returns an optional element to be rendered at the start of the editor buffer, almost like a
/// rich UI 'prefix'.
///
/// When AgentView is enabled, this is responsible for rendering the '!' shell mode indicator.
///
/// When Agent View is disabled, this renders the agent mode icon and optional follow-up icon when
/// classic input is enabled.
fn render_prefix_mode_indicator(
    prefix: &'static str,
    color: ColorU,
    appearance: &Appearance,
    em_width: f32,
    app: &AppContext,
) -> Box<dyn Element> {
    let indicator_size = ai_indicator_height(app);
    Container::new(
        ConstrainedBox::new(
            Align::new(
                Text::new(
                    prefix,
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(color)
                .finish(),
            )
            .finish(),
        )
        .with_height(indicator_size)
        .with_width(indicator_size)
        .finish(),
    )
    .with_margin_right(em_width)
    .finish()
}
fn maybe_render_ai_input_indicators(
    ai_input_model: &ModelHandle<BlocklistAIInputModel>,
    ai_context_model: &ModelHandle<BlocklistAIContextModel>,
    agent_view_controller: &ModelHandle<AgentViewController>,
    handoff_compose_state: &ModelHandle<HandoffComposeState>,
    ai_follow_up_icon_mouse_state: MouseStateHandle,
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let ai_input_model = ai_input_model.as_ref(app);
    let appearance = Appearance::as_ref(app);
    let em_width = app.font_cache().em_width(
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    );

    let is_agent_view_active = agent_view_controller.as_ref(app).is_fullscreen();
    let is_ai_input_enabled = ai_input_model.is_ai_input_enabled();
    let is_input_type_locked = ai_input_model.is_input_type_locked();

    // Show the `!` shell mode indicator when in locked shell mode inside the
    // agent view OR inside the CLI agent rich input (e.g. Claude Code bash mode).
    let is_locked_shell = !is_ai_input_enabled && is_input_type_locked;
    let is_cli_agent_input_open =
        CLIAgentSessionsModel::as_ref(app).is_input_open(terminal_view_id);
    if handoff_compose_state.as_ref(app).is_active() && is_agent_view_active {
        return Some(render_prefix_mode_indicator(
            CLOUD_HANDOFF_INPUT_PREFIX,
            ai_brand_color(appearance.theme()),
            appearance,
            em_width,
            app,
        ));
    }

    if is_locked_shell && (is_agent_view_active || is_cli_agent_input_open) {
        return Some(render_prefix_mode_indicator(
            TERMINAL_INPUT_PREFIX,
            appearance.theme().ansi_fg_blue(),
            appearance,
            em_width,
            app,
        ));
    }

    if is_agent_view_active {
        return None;
    }

    if !ai_input_model.is_ai_input_enabled() || FeatureFlag::AgentView.is_enabled() {
        return None;
    }

    let is_universal_developer_input_enabled =
        InputSettings::as_ref(app).is_universal_developer_input_enabled(app);

    // If universal developer input is enabled, don't show any AI indicators
    if is_universal_developer_input_enabled {
        return None;
    }

    let ai_icon = render_ai_agent_mode_icon(
        app,
        AnsiColorIdentifier::Yellow.to_ansi_color(&appearance.theme().terminal_colors().normal),
    );

    let all_icons = if ai_context_model
        .as_ref(app)
        .is_targeting_existing_conversation()
    {
        let reply_icon = render_ai_follow_up_icon(ai_follow_up_icon_mouse_state, app);
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ai_icon)
            .with_child(reply_icon)
            .finish()
    } else {
        ai_icon
    };
    Some(
        Container::new(all_icons)
            .with_margin_right(em_width)
            .finish(),
    )
}

#[cfg(feature = "integration_tests")]
impl Input {}

#[cfg(test)]
impl Input {
    pub fn agent_footer_chip_kinds(
        &self,
        app: &AppContext,
    ) -> (
        Vec<crate::context_chips::ContextChipKind>,
        Vec<crate::context_chips::ContextChipKind>,
    ) {
        self.agent_input_footer
            .as_ref(app)
            .displayed_chip_kinds(app)
    }

    pub fn cli_footer_chip_kinds(
        &self,
        app: &AppContext,
    ) -> Vec<crate::context_chips::ContextChipKind> {
        self.agent_input_footer
            .as_ref(app)
            .cli_display_chip_kinds(app)
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
