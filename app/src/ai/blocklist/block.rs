//! Implementation of "AI blocks" used to render AI queries and outputs in the blocklist.
pub mod cli;
pub mod cli_controller;
pub mod compact_agent_input;
pub(super) mod find;
pub mod keyboard_navigable_buttons;
pub mod model;
pub mod number_shortcut_buttons;
pub mod numbered_button;
pub mod pending_user_query_block;
pub mod secret_redaction;
pub mod status_bar;
pub mod toggleable_items;
pub mod view_impl;

use std::cell::OnceCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use ai::agent::action::{AskUserQuestionItem, InsertReviewComment, RunAgentsRequest};
use base64::Engine as _;
use chrono::Duration;
use cli_controller::{CLISubagentController, CLISubagentEvent};
use find::FindState;
use indexmap::IndexMap;
use itertools::Itertools;
use model::AIBlockOutputStatus;
use parking_lot::{FairMutex, Mutex, RwLock};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
pub use pending_user_query_block::{PendingUserQueryBlock, PendingUserQueryBlockEvent};
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use secret_redaction::*;
use serde::Serialize;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_editor::content::buffer::InitialBufferState;
#[cfg(feature = "local_fs")]
use warp_editor::content::edit::resolve_asset_source_relative_to_directory;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::path::ShellFamily;
use warpui::assets::asset_cache::AssetCache;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    get_rich_content_position_id, ClippedScrollStateHandle, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, SecretRange, SelectionBound, SelectionHandle, TableStateHandle,
};
use warpui::image_cache::ImageType;
use warpui::keymap::FixedBinding;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::text::SelectionType;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::radio_buttons::RadioButtonStateHandle;
use warpui::{
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle, WindowId,
};

#[cfg(feature = "agent_mode_debug")]
use self::code_diff_view::FileDiff;
use self::model::{AIBlockModel, AIBlockModelHelper};
use super::action_model::{AIActionStatus, BlocklistAIActionEvent, RequestFileEditsFormatKind};
use super::code_block::CodeSnippetButtonHandles;
use super::controller::ClientIdentifiers;
use super::inline_action::code_diff_view::{
    CodeDiffState, CodeDiffView, CodeDiffViewAction, CodeDiffViewEvent,
};
use super::inline_action::requested_action::{CTRL_C_KEYSTROKE, ENTER_KEYSTROKE};
use super::inline_action::requested_command_attribution::is_command_copied_from_document;
use super::permissions::is_agent_mode_autonomy_allowed;
use super::suggested_agent_mode_workflow_modal::SuggestedAgentModeWorkflowAndId;
use super::suggested_rule_modal::SuggestedRuleAndId;
use super::telemetry_banner::should_collect_ai_ugc_telemetry;
use super::{
    BlocklistAIActionModel, BlocklistAIController, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, BlocklistAIPermissions, ResponseStreamId,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::redaction::redact_secrets;
use crate::ai::agent::telemetry::ForTelemetry as _;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType, AIAgentAttachment,
    AIAgentCitation, AIAgentContext, AIAgentInput, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentTextSection, AIIdentifiers, CancellationReason,
    CreateDocumentsRequest, CreateDocumentsResult, DocumentToCreate, EditDocumentsResult,
    MessageId, PassiveSuggestionTrigger, ProgrammingLanguage, RenderableAIError,
    RequestCommandOutputResult, RequestFileEditsResult, SearchCodebaseResult, ServerOutputId,
    SubagentCall, SubagentType, SuggestPromptRequest, SuggestPromptResult, SuggestedLoggingId,
    SummarizationType, TodoOperation,
};
use crate::ai::agent_conversations_model::{AgentConversationsModel, AgentConversationsModelEvent};
use crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::action_model::NewConversationDecision;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewEntryOrigin};
use crate::ai::blocklist::block::keyboard_navigable_buttons::{
    KeyboardNavigableButtonBuilder, KeyboardNavigableButtons,
};
use crate::ai::blocklist::context_model::AttachmentType;
use crate::ai::blocklist::inline_action::ask_user_question_view::{
    self, AskUserQuestionView, AskUserQuestionViewEvent,
};
use crate::ai::blocklist::inline_action::aws_bedrock_credentials_error::{
    AwsBedrockCredentialsErrorEvent, AwsBedrockCredentialsErrorView,
};
use crate::ai::blocklist::inline_action::code_diff_view;
use crate::ai::blocklist::inline_action::code_diff_view::convert_file_edits_to_file_diffs;
use crate::ai::blocklist::inline_action::requested_command::{
    self, RequestedActionViewType, RequestedCommand, RequestedCommandView,
    RequestedCommandViewEvent,
};
use crate::ai::blocklist::inline_action::run_agents_card_view::{
    self, RunAgentsCardView, RunAgentsCardViewEvent,
};
use crate::ai::blocklist::inline_action::search_codebase::{
    SearchCodebaseView, SearchCodebaseViewEvent,
};
use crate::ai::blocklist::inline_action::suggested_unit_tests::{
    SuggestedUnitTestsEvent, SuggestedUnitTestsView,
};
use crate::ai::blocklist::inline_action::web_fetch::WebFetchView;
use crate::ai::blocklist::inline_action::web_search::WebSearchView;
use crate::ai::blocklist::permissions::{
    CommandExecutionPermission, CommandExecutionPermissionDeniedReason,
};
use crate::ai::blocklist::suggestion_chip_view::{SuggestedChipViewEvent, SuggestionChipView};
use crate::ai::blocklist::{
    BlocklistAIContextEvent, BlocklistAIContextModel, SuggestionDismissButtonTheme,
};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::facts::{AIFact, AIMemory, CloudAIFactModel};
use crate::ai::get_relevant_files::controller::{
    GetRelevantFilesController, GetRelevantFilesControllerEvent,
};
#[cfg(feature = "local_fs")]
use crate::ai::skills::SkillOpenOrigin;
use crate::ai::skills::{SkillManager, SkillTelemetryEvent};
use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::{AuthStateProvider, UserUid};
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::comment_editor::create_readonly_comment_markdown_editor;
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::editor_management::CodeSource;
use crate::code_review::comment_rendering::{CommentViewCard, HeaderClickHandler};
use crate::code_review::comments::{
    attach_pending_imported_comments, convert_insert_review_comments, AttachedReviewComment,
    CommentId, CommentOrigin,
};
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use crate::code_review::CodeReviewTelemetryEvent;
use crate::editor::InteractionState;
use crate::notebooks::editor::model::FileLinkResolutionContext;
use crate::notebooks::editor::view::{EditorViewEvent, RichTextEditorView};
use crate::server::ids::SyncId;
use crate::server::telemetry::{
    AgentModeRewindEntrypoint, AutonomySettingToggleSource, InteractionSource, TelemetryEvent,
};
use crate::settings::{
    AISettings, AISettingsChangedEvent, AgentModeCodingPermissionsType, FontSettings,
    InputModeSettings, InputModeSettingsChangedEvent, InputSettings,
    OrchestrationMessageDisplayMode,
};
use crate::settings_view::SettingsSection;
use crate::terminal::find::TerminalFindModel;
use crate::terminal::model::secrets::RichContentSecretTooltipInfo;
use crate::terminal::model::session::active_session::{ActiveSession, ActiveSessionEvent};
use crate::terminal::model::BlockId;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::safe_mode_settings::{
    get_secret_obfuscation_mode, SafeModeSettings, SafeModeSettingsChangedEvent,
};
use crate::terminal::view::ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent};
use crate::terminal::view::{
    CodeDiffAction, RichContentLink, RichContentLinkTooltipInfo, TerminalAction,
};
use crate::terminal::{ShellLaunchData, TerminalModel, TerminalView};
use crate::ui_components::icons::Icon;
use crate::util::link_detection::*;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{is_supported_image_file, FileTarget};
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme,
    SecondaryTheme,
};
use crate::view_components::compactible_action_button::CompactibleActionButton;
use crate::view_components::find::FindEvent;
use crate::view_components::DismissibleToast;
use crate::workspace::{ForkAIConversationParams, ForkedConversationDestination, WorkspaceAction};
use crate::workspaces::user_profiles::{UserProfileWithUID, UserProfiles};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    report_error, report_if_error, send_telemetry_from_ctx, AIAgentTodoList, Appearance, FileEdit,
    LLMPreferences, PrivacySettings, ToastStack,
};

/// The default display name used for the user if they have no associated display name.
const DEFAULT_USER_DISPLAY_NAME: &str = "User";

const HAS_PENDING_ACTION: &str = "HasPendingAction";
const DISPATCHED_REQUESTED_EDIT_KEYMAP_CONTEXT: &str = "PendingAIRequestedEdits";

const AUTO_EXPAND_REQUESTED_COMMAND_DELAY: std::time::Duration =
    std::time::Duration::from_millis(3000);

pub const RICH_CONTENT_SECRET_FIRST_CHAR_POSITION_ID: &str =
    "ai_block:rich_content_secret_first_char_position";

#[derive(Clone, Debug, PartialEq, Eq)]
struct UserAvatarInfo {
    display_name: String,
    profile_image_path: Option<String>,
}

fn current_user_avatar_info(app: &AppContext) -> UserAvatarInfo {
    let auth_state = AuthStateProvider::as_ref(app).get().clone();
    UserAvatarInfo {
        display_name: auth_state
            .username_for_display()
            .unwrap_or_else(|| DEFAULT_USER_DISPLAY_NAME.to_owned()),
        profile_image_path: auth_state.user_photo_url(),
    }
}

fn non_empty_photo_url(photo_url: &str) -> Option<String> {
    (!photo_url.is_empty()).then(|| photo_url.to_string())
}

fn display_name_for_user_profile(profile: &UserProfileWithUID) -> String {
    profile
        .display_name
        .as_ref()
        .filter(|name| !name.is_empty())
        .or_else(|| (!profile.email.is_empty()).then_some(&profile.email))
        .cloned()
        .unwrap_or_else(|| profile.firebase_uid.to_string())
}

fn user_avatar_info_for_conversation_creator(
    creator: Option<&UserProfileWithUID>,
    creator_uid: Option<&str>,
    fallback: UserAvatarInfo,
    app: &AppContext,
) -> UserAvatarInfo {
    if let Some(creator) = creator {
        return UserAvatarInfo {
            display_name: display_name_for_user_profile(creator),
            profile_image_path: non_empty_photo_url(&creator.photo_url),
        };
    }

    if let Some(creator_uid) = creator_uid {
        if let Some(profile) = UserProfiles::as_ref(app).profile_for_uid(UserUid::new(creator_uid))
        {
            return UserAvatarInfo {
                display_name: profile.displayable_identifier(),
                profile_image_path: non_empty_photo_url(&profile.photo_url),
            };
        }
    }

    fallback
}

fn user_avatar_info_for_ai_block(
    model: &dyn AIBlockModel<View = AIBlock>,
    app: &AppContext,
) -> UserAvatarInfo {
    let fallback = current_user_avatar_info(app);
    let server_metadata = model
        .conversation(app)
        .and_then(|conversation| conversation.server_metadata());

    user_avatar_info_for_conversation_creator(
        server_metadata.and_then(|metadata| metadata.creator.as_ref()),
        server_metadata.and_then(|metadata| metadata.metadata.creator_uid.as_deref()),
        fallback,
        app,
    )
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            AIBlockAction::ExecuteNextPendingAction,
            id!(AIBlock::ui_name()) & id!(HAS_PENDING_ACTION),
        ),
        FixedBinding::new(
            "numpadenter",
            AIBlockAction::ExecuteNextPendingAction,
            id!(AIBlock::ui_name()) & id!(HAS_PENDING_ACTION),
        ),
    ]);

    ask_user_question_view::init(app);
    code_diff_view::init(app);
    requested_command::init(app);
    run_agents_card_view::init(app);
    cli::init(app);
}

#[cfg(feature = "local_fs")]
impl AIBlock {
    fn detected_file_path_target_override(&self, absolute_path: &Path) -> Option<FileTarget> {
        is_supported_image_file(absolute_path).then_some(FileTarget::SystemGeneric)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Complete,

    /// The block was finished with an error.
    Error,

    /// The block was finished with manual cancellation.
    Cancelled,

    /// The block was finished due to the requested command being cancelled via ctrl-c.
    CancelledDuringRequestedCommandExecution,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextLocation {
    Output {
        section_index: usize,
        /// Note that this does ***not*** correspond to the frame index of a text frame after layout;
        /// Instead, it represents the line index of the `FormattedTextLine` after markdown parsing.
        line_index: usize,
    },
    Query {
        input_index: usize,
    },
    Action {
        action_index: usize,
        line_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AIBlockResponseRating {
    Positive,
    Negative,
}

impl AIBlockResponseRating {
    pub fn name(&self) -> &'static str {
        match self {
            AIBlockResponseRating::Positive => "positive",
            AIBlockResponseRating::Negative => "negative",
        }
    }
}

#[derive(Clone)]
struct ActionButtons {
    run_button: CompactibleActionButton,
    cancel_button: CompactibleActionButton,
}

/// Like `SecondaryTheme` but with grey text instead of white.
struct RewindButtonTheme;

impl ActionButtonTheme for RewindButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(appearance.theme().surface_3())
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_2())
            .into_solid()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_4(appearance.theme()))
    }
}

#[derive(Clone, Default)]
pub(super) struct TableSectionHandles {
    pub scroll_handle: ClippedScrollStateHandle,
    pub state_handle: TableStateHandle,
}

#[derive(Default)]
pub(super) struct AIBlockStateHandles {
    normal_response_code_snippet_buttons: Vec<CodeSnippetButtonHandles>,
    table_section_handles: Vec<TableSectionHandles>,

    /// Per-image `MouseStateHandle` for hover tooltips on rendered markdown
    /// images. These must persist across frames — `tool_tip_on_element`
    /// wraps the image in a `Hoverable` whose `is_hovered()` state would
    /// reset every frame if we built the handle inline.
    image_section_tooltip_handles: Vec<MouseStateHandle>,

    /// Only applies to text selections made at the `AIBlock` level. Child views of the `AIBlock`
    /// are responsible for managing their own text selection states.
    selection_handle: SelectionHandle,

    /// Mouse state handle for interacting with the attached blocks.
    attached_blocks_chip_state_handle: MouseStateHandle,

    /// Mouse state handle for the continue conversation button
    continue_conversation_handle: MouseStateHandle,

    /// Mouse state handle for the resume conversation button
    resume_conversation_handle: MouseStateHandle,

    /// Mouse state handle for the fork conversation button
    fork_conversation_handle: MouseStateHandle,

    /// Mouse state handle for the usage button
    usage_button_handle: MouseStateHandle,

    /// Mouse state handles per citation.
    /// A given citation should only appear once per block.
    footer_citation_chip_handles: HashMap<AIAgentCitation, MouseStateHandle>,
    orchestration_navigation_card_handles: HashMap<AIAgentActionId, MouseStateHandle>,
    /// Persistent mouse-state handles per received-message transcript row,
    /// used by the clickable sender avatar.
    pub(super) transcript_avatar_handles: HashMap<MessageId, MouseStateHandle>,

    references_section_collapsible_handle: MouseStateHandle,

    autoread_files_speedbump_checkbox_handle: MouseStateHandle,
    codebase_search_speedbump_option_handles: Vec<MouseStateHandle>,
    codebase_search_speedbump_radio_button_handle: RadioButtonStateHandle,
    manage_autonomy_settings_link_handle: MouseStateHandle,

    ask_user_question_speedbump_settings_link_handle: MouseStateHandle,

    /// Mouse state handles for rating the AI block.
    thumbs_up_handle: MouseStateHandle,
    thumbs_down_handle: MouseStateHandle,

    /// Mouse state handle for the overflow menu button
    overflow_menu_handle: MouseStateHandle,

    menu_accept_button_handle: MouseStateHandle,
    menu_reject_button_handle: MouseStateHandle,

    /// Mouse state handle for the debug ID copy button
    debug_copy_button_handle: MouseStateHandle,
    /// Mouse state handle for the submit issue button
    submit_issue_button_handle: MouseStateHandle,

    /// Mouse state handle for the invalid API key button
    invalid_api_key_button_handle: MouseStateHandle,

    /// Mouse state handle for AI document created block
    ai_document_handle: MouseStateHandle,

    /// Per-action mouse state handles for the 'open skill' button shown on
    /// ReadSkill and ReadFiles action banners. Keyed by action id so that
    /// multiple skill banners in the same block don't share hover/click state.
    skill_button_handles: HashMap<AIAgentActionId, MouseStateHandle>,
}

#[derive(Default, Clone, Debug)]
pub struct DirectoryContext {
    pub pwd: Option<String>,
    pub home_dir: Option<String>,
}

/// Convenience wrapper around a [`CodeDiffView`].
/// TODO(Simon): For consistency with other inline actions, let's move this to code_diff_view.rs.
#[derive(Debug)]
pub(crate) struct RequestedEdit {
    view: ViewHandle<CodeDiffView>,
}

impl RequestedEdit {
    fn new(view: ViewHandle<CodeDiffView>) -> Self {
        Self { view }
    }
}

#[derive(Clone, Debug, Default)]
pub enum AutonomySettingSpeedbump {
    /// There's no speedbump to show.
    #[default]
    None,
    /// Show a checkbox-based speedbump for auto-executing read-only commands.
    ShouldShowForAutoexecutingReadonlyCommands {
        /// Which action this corresponds to.
        action_id: AIAgentActionId,
        /// Whether the setting in the speedbump is checked or not.
        checked: bool,
        /// Whether or not the speedbump is actually shown.
        ///
        /// Set at render-time.
        shown: Arc<Mutex<bool>>,
    },
    /// Show a checkbox-based speedbump for file access.
    ShouldShowForFileAccess {
        /// Which action this corresponds to.
        action_id: AIAgentActionId,
        /// Whether the setting in the speedbump is checked or not.
        checked: bool,
        /// Whether or not the speedbump is actually shown.
        ///
        /// Set at render-time.
        shown: Arc<Mutex<bool>>,
    },
    /// Show a radio-button-based speedbump for file access during codebase search.
    ShouldShowForCodebaseSearchFileAccess {
        /// Which action this corresponds to.
        action_id: AIAgentActionId,
        /// Which radio option is selected.
        /// 0 => Always allow file access.
        /// 1 => Allowlist this repo for file access.
        selected_option: Option<usize>,
        /// Whether or not the speedbump is actually shown.
        ///
        /// Set at render-time.
        shown: Arc<Mutex<bool>>,
    },
    /// Show an informational footer for execution profile command autoexecution settings.
    ShouldShowForProfileCommandAutoexecution {
        /// Which action this corresponds to.
        action_id: AIAgentActionId,
        /// Whether or not the speedbump is actually shown.
        ///
        /// Set at render-time.
        shown: Arc<Mutex<bool>>,
    },
    /// Show a one-shot dropdown-based speedbump for ask-user-question permission.
    ShouldShowForAskUserQuestion {
        /// Which action this corresponds to.
        action_id: AIAgentActionId,
        /// Whether or not the speedbump is actually shown.
        ///
        /// Set at render-time.
        shown: Arc<Mutex<bool>>,
    },
}

/// State for the todo list preview element in the AI block.
/// Displayed whenever an agent creates a todo list.
struct TodoListElementState {
    header_toggle_mouse_state: MouseStateHandle,
    is_expanded: bool,
}

impl Default for TodoListElementState {
    fn default() -> Self {
        Self {
            header_toggle_mouse_state: MouseStateHandle::new(Default::default()),
            is_expanded: true,
        }
    }
}

pub(super) struct ImportedCommentElementState {
    pub(super) open_in_github_button: Option<ViewHandle<ActionButton>>,
    pub(super) open_in_code_review_button: ViewHandle<ActionButton>,
    pub(super) chevron_button: ViewHandle<ActionButton>,
    pub(super) header_click_handler: HeaderClickHandler,
}

impl ImportedCommentElementState {
    fn new(
        action_id: AIAgentActionId,
        comment_index: usize,
        html_url: Option<String>,
        ctx: &mut ViewContext<AIBlock>,
    ) -> Self {
        let open_in_github_button = html_url.map(|url| {
            ctx.add_typed_action_view(move |_| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::Github)
                    .with_size(ButtonSize::Small)
                    .with_tooltip("Open in GitHub")
                    .on_click({
                        let url = url.clone();
                        move |ctx| {
                            ctx.dispatch_typed_action(AIBlockAction::OpenCommentInGitHub {
                                url: url.clone(),
                            });
                        }
                    })
            })
        });

        let action_id_for_open_button = action_id.clone();
        let open_in_code_review_button = ctx.add_typed_action_view(move |_| {
            ActionButton::new("Open in code review", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::OpenImportedCommentInCodeReview {
                        action_id: action_id_for_open_button.clone(),
                        comment_index,
                    });
                })
        });

        let chevron_button = ctx.add_view(|_| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::ChevronDown)
                .with_size(ButtonSize::Small)
        });

        let header_click_handler = HeaderClickHandler {
            mouse_state: MouseStateHandle::default(),
            on_click: Rc::new(move |ctx| {
                ctx.dispatch_typed_action(AIBlockAction::ToggleImportedCommentCollapsed {
                    action_id: action_id.clone(),
                    comment_index,
                });
            }),
        };

        Self {
            open_in_github_button,
            open_in_code_review_button,
            chevron_button,
            header_click_handler,
        }
    }
}

pub(super) struct ImportedCommentGroup {
    repo_path: LocalOrRemotePath,
    base_branch: Option<String>,
    cards: Vec<CommentViewCard>,
    element_states: Vec<ImportedCommentElementState>,
}

impl ImportedCommentGroup {
    fn new(
        repo_path: LocalOrRemotePath,
        base_branch: Option<String>,
        cards: Vec<CommentViewCard>,
        element_states: Vec<ImportedCommentElementState>,
    ) -> Self {
        Self {
            repo_path,
            base_branch,
            cards,
            element_states,
        }
    }

    fn card_mut(&mut self, comment_index: usize) -> Option<&mut CommentViewCard> {
        self.cards.get_mut(comment_index)
    }

    fn set_buttons_disabled(&self, should_disable: bool, ctx: &mut ViewContext<AIBlock>) {
        for state in &self.element_states {
            set_imported_comment_button_disabled(
                &state.open_in_code_review_button,
                should_disable,
                Some(&self.repo_path),
                ctx,
            );
        }
    }
}

pub(super) struct CommentElementState {
    pub(super) header_toggle_mouse_state: MouseStateHandle,
    pub(super) maximize_minimize_button: ViewHandle<ActionButton>,
    pub(super) rich_text_editor: ViewHandle<RichTextEditorView>,
    pub(super) is_expanded: bool,
}

impl CommentElementState {
    fn new(comment_id: CommentId, content: &str, ctx: &mut ViewContext<AIBlock>) -> Self {
        let action_button = ctx.add_view(move |_| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::Maximize)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::CommentExpanded { id: comment_id });
                })
                .with_size(ButtonSize::Small)
        });

        let rich_text_editor = create_readonly_comment_markdown_editor(
            content, true, /* disable_scrolling */
            None, /* allow comments to expand to full width */
            ctx,
        );

        CommentElementState {
            header_toggle_mouse_state: MouseStateHandle::default(),
            maximize_minimize_button: action_button,
            rich_text_editor,
            is_expanded: false,
        }
    }
}

/// Expansion state for collapsible blocks with streamed content.
pub enum CollapsibleExpansionState {
    Expanded {
        /// Whether streaming/generation is complete
        is_finished: bool,
        /// Whether to auto-scroll to bottom during streaming.
        /// Disabled when user manually scrolls to read previous content.
        scroll_pinned_to_bottom: bool,
    },
    Collapsed,
}

/// State for collapsible elements with scroll support.
pub struct CollapsibleElementState {
    pub expansion_toggle_mouse_state: MouseStateHandle,
    pub expansion_state: CollapsibleExpansionState,
    pub scroll_state: ClippedScrollStateHandle,
    last_known_is_finished: bool,
    user_toggled_while_streaming: bool,
}

impl Default for CollapsibleElementState {
    fn default() -> Self {
        Self {
            expansion_toggle_mouse_state: MouseStateHandle::new(Default::default()),
            expansion_state: CollapsibleExpansionState::Expanded {
                is_finished: false,
                scroll_pinned_to_bottom: true,
            },
            scroll_state: ClippedScrollStateHandle::new(),
            last_known_is_finished: false,
            user_toggled_while_streaming: false,
        }
    }
}

impl CollapsibleElementState {
    fn collapsed() -> Self {
        Self {
            expansion_state: CollapsibleExpansionState::Collapsed,
            ..Default::default()
        }
    }
    fn expand(&mut self) {
        self.expansion_state = CollapsibleExpansionState::Expanded {
            is_finished: self.last_known_is_finished,
            scroll_pinned_to_bottom: true,
        };
    }

    /// Mirrors the latest streamed finished state into the cached state and any expanded UI.
    fn sync_finished_state(&mut self, is_finished: bool) {
        self.last_known_is_finished = is_finished;

        if let CollapsibleExpansionState::Expanded {
            is_finished: expanded_is_finished,
            ..
        } = &mut self.expansion_state
        {
            *expanded_is_finished = is_finished;
        }
    }

    fn finish_reasoning(&mut self, app: &AppContext) {
        let should_auto_collapse = self.should_auto_collapse_on_finish();
        let thinking_mode = AISettings::as_ref(app).thinking_display_mode;

        self.sync_finished_state(true);

        if !thinking_mode.should_keep_expanded() && should_auto_collapse {
            self.expansion_state = CollapsibleExpansionState::Collapsed;
        } else if let CollapsibleExpansionState::Expanded {
            scroll_pinned_to_bottom,
            ..
        } = &mut self.expansion_state
        {
            *scroll_pinned_to_bottom = false;
        }
    }

    fn finish_summarization(&mut self) {
        let should_auto_collapse = matches!(
            self.expansion_state,
            CollapsibleExpansionState::Expanded {
                is_finished: false,
                ..
            }
        );

        self.sync_finished_state(true);

        if should_auto_collapse {
            self.expansion_state = CollapsibleExpansionState::Collapsed;
        }
    }

    /// Applies orchestration message display behavior after streaming finishes.
    fn finish_orchestration_message(&mut self, display_mode: OrchestrationMessageDisplayMode) {
        let should_auto_collapse = self.should_auto_collapse_on_finish();

        self.sync_finished_state(true);

        if display_mode.should_collapse_agent_message_body_on_finish() && should_auto_collapse {
            self.expansion_state = CollapsibleExpansionState::Collapsed;
        } else if let CollapsibleExpansionState::Expanded {
            scroll_pinned_to_bottom,
            ..
        } = &mut self.expansion_state
        {
            *scroll_pinned_to_bottom = false;
        }
    }

    fn toggle_expansion(&mut self) {
        if !self.last_known_is_finished {
            self.user_toggled_while_streaming = true;
        }

        if matches!(
            self.expansion_state,
            CollapsibleExpansionState::Expanded { .. }
        ) {
            self.expansion_state = CollapsibleExpansionState::Collapsed;
        } else {
            self.expand();
        }
    }

    fn should_auto_collapse_on_finish(&self) -> bool {
        !self.user_toggled_while_streaming
            && matches!(
                self.expansion_state,
                CollapsibleExpansionState::Expanded {
                    is_finished: false,
                    scroll_pinned_to_bottom: true
                }
            )
    }
}

const RECEIVED_MESSAGE_COLLAPSIBLE_ID_PREFIX: &str = "received-message:";

pub(crate) fn received_message_collapsible_id(message_id: &str) -> MessageId {
    MessageId::new(format!(
        "{RECEIVED_MESSAGE_COLLAPSIBLE_ID_PREFIX}{message_id}"
    ))
}

fn default_collapsible_state_for_orchestration_action(
    action: &AIAgentActionType,
    display_mode: OrchestrationMessageDisplayMode,
) -> Option<CollapsibleElementState> {
    match action {
        AIAgentActionType::StartAgent { .. } => Some(CollapsibleElementState::default()),
        AIAgentActionType::SendMessageToAgent { .. } => {
            Some(default_orchestration_collapsible_state(
                display_mode.should_expand_agent_message_body(),
            ))
        }
        _ => None,
    }
}

fn default_collapsible_state_for_orchestration_message(
    display_mode: OrchestrationMessageDisplayMode,
) -> CollapsibleElementState {
    default_orchestration_collapsible_state(display_mode.should_expand_agent_message_body())
}

fn default_orchestration_collapsible_state(expanded: bool) -> CollapsibleElementState {
    if expanded {
        CollapsibleElementState::default()
    } else {
        CollapsibleElementState::collapsed()
    }
}

pub struct AIBlock {
    model: Rc<dyn AIBlockModel<View = AIBlock>>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    client_ids: ClientIdentifiers,
    profile_image_path: Option<String>,
    user_display_name: String,

    /// Only applies to text selections made at the `AIBlock` level. Child views of the `AIBlock`
    /// are responsible for managing their own text selection states. We need this in an RwLock so
    /// that the SelectableArea can modify this synchronously when a selection ends. This is necessary
    /// so the terminal view can read the updated value when the selection ends in the copy-on-select case.
    selected_text: Arc<RwLock<Option<String>>>,
    state_handles: AIBlockStateHandles,
    controller: ModelHandle<BlocklistAIController>,
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    window_id: warpui::WindowId,

    /// The current working directory at the time the AI block was created. Note that this
    /// is different from `directory_context`, which represents the directory-related contexts
    /// to render the block header. `directory_context` might be empty if the input type does not
    /// require context (e.g. ActionResult).
    current_working_directory: Option<String>,

    /// The shell launch data at the time the AI block was created.
    shell_launch_data: Option<ShellLaunchData>,

    action_model: ModelHandle<BlocklistAIActionModel>,
    context_model: ModelHandle<BlocklistAIContextModel>,

    /// The IDs of requested blocking actions rendered in this block.
    requested_action_ids: HashSet<AIAgentActionId>,

    /// Map from a requested command action ID to its view handle and status.
    requested_commands: HashMap<AIAgentActionId, RequestedCommand>,

    /// Map from a requested MCP tool call action ID to its view handle and status.
    requested_mcp_tools: HashMap<AIAgentActionId, RequestedCommand>,

    /// Map from a requested edit action ID to its view handle and status.
    /// Uses IndexMap to preserve insertion order for correct revert ordering.
    requested_edits: IndexMap<AIAgentActionId, RequestedEdit>,

    /// Map from a search codebase action ID to its view handle and status.
    search_codebase_view: HashMap<AIAgentActionId, ViewHandle<SearchCodebaseView>>,

    /// Map from web search message IDs to their view handles.
    web_search_views: HashMap<MessageId, ViewHandle<WebSearchView>>,

    /// Map from web fetch message IDs to their view handles.
    web_fetch_views: HashMap<MessageId, ViewHandle<WebFetchView>>,

    /// Map from todo list IDs to their states.
    todo_list_states: HashMap<MessageId, TodoListElementState>,

    comment_states: HashMap<CommentId, CommentElementState>,

    /// Map from collapsible block message IDs (reasoning or summarization) to their states.
    collapsible_block_states: HashMap<MessageId, CollapsibleElementState>,

    /// Map from suggested prompt action ID to its view handle and status.
    unit_tests_suggestions: HashMap<AIAgentActionId, ViewHandle<SuggestedUnitTestsView>>,

    /// Task to auto expand an executed requested command or requested action after it has
    /// been running for a while. This applies to both the [`RequestedCommandView`] and
    /// non-[`View`] inline actions.
    auto_expand_requested_command_timer_handle: Option<SpawnedFutureHandle>,

    time_to_first_token: OnceCell<Duration>,
    time_to_last_token: Option<Duration>,

    /// The number of blocks that were attached as context to this AI block's query.
    num_attached_context_blocks: usize,

    /// Whether selected text was attached as context to this AI block's query.
    has_attached_context_selected_text: bool,

    /// This is only set to Some if no new content is expected to be received and all
    /// requested actions and requested commands are completed or cancelled.
    finish_reason: Option<FinishReason>,

    directory_context: DirectoryContext,
    view_id: EntityId,

    detected_links_state: DetectedLinksState,
    find_state: FindState,
    find_model: ModelHandle<TerminalFindModel>,

    /// The CodeEditorViews associated with this block.
    code_editor_views: Vec<EmbeddedCodeEditorView>,

    secret_redaction_state: SecretRedactionState,

    /// Whether or not the references section at the bottom of the block is toggled open.
    is_references_section_open: bool,

    /// A speedbump, or notice, about an autonomy setting.
    ///
    /// Assumes we only have 1 action per AI block.
    autonomy_setting_speedbump: AutonomySettingSpeedbump,

    /// The suggested rules to render in the block.
    suggested_rules: Vec<ViewHandle<SuggestionChipView>>,

    /// The suggested agent mode workflows to render in the block.
    suggested_agent_mode_workflow: Option<ViewHandle<SuggestionChipView>>,

    manage_rules_button: ViewHandle<ActionButton>,

    action_buttons: HashMap<AIAgentActionId, ActionButtons>,

    /// A user menu presenting an accept and reject choice.
    ///
    /// This UI is used for the new conversation suggestion multi-select.
    keyboard_navigable_buttons: Option<ViewHandle<KeyboardNavigableButtons>>,

    /// The thumbs up/down rating of the AI block response.
    response_rating: OnceCell<AIBlockResponseRating>,

    /// The number of requests that have been refunded.
    /// Right now, this happens when a user thumbs down a response.
    request_refunded_count: Option<i32>,

    /// Requested commands that were auto-expanded,
    /// and should thus be auto-collapsed when the block is finished.
    requested_commands_to_auto_collapse: HashSet<AIAgentActionId>,

    review_changes_button: ViewHandle<ActionButton>,
    open_all_comments_button: ViewHandle<ActionButton>,

    dismiss_suggestion_button: ViewHandle<ActionButton>,
    disable_rule_suggestions_button: ViewHandle<ActionButton>,

    /// Rewind button to revert to before this block.
    rewind_button: ViewHandle<ActionButton>,

    /// Per-action button components for "View screenshot" buttons on UseComputer actions.
    view_screenshot_buttons: HashMap<AIAgentActionId, ui_components::button::Button>,

    /// Stores the last command that was right-clicked by a child component.
    /// When set, CopyCommand will copy this specific command instead of all commands.
    last_right_clicked_command: Option<String>,

    /// Whether the usage summary footer is expanded.
    is_usage_footer_expanded: bool,

    /// Controller for reading/modifying `AgentView` state for this terminal pane (e.g. if there is
    /// an active agent view or not, which affects whether or not this block should be hidden).
    ///
    /// Only used when `FeatureFlag::AgentView` is enabled.
    agent_view_controller: ModelHandle<AgentViewController>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,

    /// View for AWS Bedrock credentials error, created lazily when the error occurs.
    aws_bedrock_credentials_error_view: Option<ViewHandle<AwsBedrockCredentialsErrorView>>,

    imported_comments: HashMap<AIAgentActionId, ImportedCommentGroup>,
    has_imported_comments: bool,

    /// Per-action `RunAgentsCardView`, lazily created.
    run_agents_card_views: HashMap<AIAgentActionId, ViewHandle<RunAgentsCardView>>,

    /// Handle for the background link detection task, kept so we can abort a previous
    /// detection when a new one is spawned (e.g. on shell data change).
    link_detection_handle: Option<SpawnedFutureHandle>,

    /// Cache of resolved code block file paths, keyed by the original path from the AI output.
    /// Populated by the background file path detection task so that `render_code_output_section`
    /// does not need to call `fs::metadata` on every render.
    #[cfg(feature = "local_fs")]
    resolved_code_block_paths: HashMap<PathBuf, Option<PathBuf>>,
    #[cfg(feature = "local_fs")]
    resolved_blocklist_image_sources: view_impl::common::ResolvedBlocklistImageSources,
    terminal_view_handle: WeakViewHandle<TerminalView>,

    ask_user_question_view: Option<ViewHandle<AskUserQuestionView>>,
}

struct EmbeddedCodeEditorView {
    view: ViewHandle<CodeEditorView>,
    language: Option<ProgrammingLanguage>,
    length: usize,
}

impl AIBlock {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Rc<dyn AIBlockModel<View = AIBlock>>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        client_ids: ClientIdentifiers,
        controller: ModelHandle<BlocklistAIController>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        current_working_directory: Option<String>,
        shell_launch_data: Option<ShellLaunchData>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        context_model: ModelHandle<BlocklistAIContextModel>,
        find_model: ModelHandle<TerminalFindModel>,
        active_session: ModelHandle<ActiveSession>,
        cli_subagent_controller: &ModelHandle<CLISubagentController>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        terminal_view_handle: WeakViewHandle<TerminalView>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let user_avatar_info = user_avatar_info_for_ai_block(model.as_ref(), ctx);
        let num_attached_context_blocks = num_attached_context_blocks(model.inputs_to_render(ctx));
        let has_attached_context_selected_text =
            has_attached_context_selected_text(model.inputs_to_render(ctx));

        let (pwd, home_dir) = model
            .inputs_to_render(ctx)
            .iter()
            .find_map(AIAgentInput::context)
            .and_then(|context| {
                context.iter().find_map(|agent_context| {
                    if let AIAgentContext::Directory { pwd, home_dir, .. } = agent_context {
                        Some((pwd.clone(), home_dir.clone()))
                    } else {
                        None
                    }
                })
            })
            .unwrap_or((None, None));

        let font_settings_handle = FontSettings::handle(ctx);
        ctx.subscribe_to_model(&font_settings_handle, |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &AISettings::handle(ctx),
            move |me, settings_model, event, ctx| match event {
                AISettingsChangedEvent::AgentModeExecuteReadonlyCommands { .. } => {
                    if let AutonomySettingSpeedbump::ShouldShowForAutoexecutingReadonlyCommands {
                        checked,
                        ..
                    } = &mut me.autonomy_setting_speedbump
                    {
                        *checked = *settings_model
                            .as_ref(ctx)
                            .agent_mode_execute_read_only_commands;
                    } else {
                        me.autonomy_setting_speedbump = AutonomySettingSpeedbump::None;
                    }
                    ctx.notify();
                }
                AISettingsChangedEvent::AgentModeCodingPermissions { .. } => {
                    match &mut me.autonomy_setting_speedbump {
                        AutonomySettingSpeedbump::ShouldShowForFileAccess { checked, .. } => {
                            *checked = matches!(
                                *settings_model.as_ref(ctx).agent_mode_coding_permissions,
                                AgentModeCodingPermissionsType::AlwaysAllowReading
                            );
                        }
                        AutonomySettingSpeedbump::ShouldShowForCodebaseSearchFileAccess {
                            selected_option,
                            ..
                        } => {
                            *selected_option =
                                match *settings_model.as_ref(ctx).agent_mode_coding_permissions {
                                    AgentModeCodingPermissionsType::AlwaysAllowReading => Some(0),
                                    AgentModeCodingPermissionsType::AllowReadingSpecificFiles => {
                                        Some(1)
                                    }
                                    AgentModeCodingPermissionsType::AlwaysAskBeforeReading => None,
                                };
                        }
                        _ => {}
                    }
                    ctx.notify();
                }
                AISettingsChangedEvent::ThinkingDisplayMode { .. }
                | AISettingsChangedEvent::OrchestrationMessageDisplayMode { .. } => {
                    ctx.notify();
                }
                _ => {}
            },
        );

        ctx.subscribe_to_model(
            &AgentConversationsModel::handle(ctx),
            |me, _, event, ctx| {
                me.handle_agent_conversations_model_event(event, ctx);
            },
        );

        let safe_mode_settings = SafeModeSettings::handle(ctx);
        ctx.subscribe_to_model(&safe_mode_settings, |me, _, event, ctx| {
            me.handle_safe_mode_settings_changed_event(event, ctx)
        });

        ctx.subscribe_to_model(&AIExecutionProfilesModel::handle(ctx), |me, _, _, ctx| {
            let terminal_view_id = me.terminal_view_id;
            if let Some(view) = me.ask_user_question_view.clone() {
                view.update(ctx, |v, ctx| {
                    v.refresh_speedbump_dropdown_selection(terminal_view_id, ctx);
                });
            }
        });

        let detected_links_state: DetectedLinksState = Default::default();
        let secret_redaction_state = SecretRedactionState::default();

        ctx.subscribe_to_model(&find_model, |me, _, event, ctx| match event {
            FindEvent::UpdatedFocusedMatch => {
                me.handle_find_match_focus_change(ctx);
                ctx.notify();
            }
            FindEvent::RanFind => ctx.notify(),
        });

        // Input Mode affects styling of AI blocks -- in particular, the spacing and border between
        // a user query block and a subsequent action (e.g. requested command, suggested code diff)
        // block.
        ctx.subscribe_to_model(
            &InputModeSettings::handle(ctx),
            |_, _, event, ctx| match event {
                InputModeSettingsChangedEvent::InputModeState { .. } => ctx.notify(),
            },
        );

        Self::register_action_model_subscription(&action_model, ctx);

        ctx.subscribe_to_model(&active_session, |me, _, event, ctx| match event {
            ActiveSessionEvent::UpdatedPwd => {
                me.update_imported_comments_disabled_state(ctx);
            }
            ActiveSessionEvent::Bootstrapped => {}
        });

        ctx.subscribe_to_model(&get_relevant_files_controller, |me, _, event, ctx| {
            if let GetRelevantFilesControllerEvent::Success { action_id, .. } = event {
                if me.requested_action_ids.contains(action_id) {
                    ctx.notify();
                }
            }
        });

        let manage_rules_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Manage rules", NakedTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(AIBlockAction::OpenAIFactCollection))
        });

        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |me, _, event, ctx| {
            if let AIRequestUsageModelEvent::RequestBonusRefunded {
                requests_refunded,
                server_conversation_id,
                request_id,
            } = event
            {
                let server_conversation_token = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&me.client_ids.conversation_id)
                    .and_then(|conversation| conversation.server_conversation_token())
                    .cloned();

                let server_output_id = me.model.server_output_id(ctx);

                if let (Some(server_conversation_token), Some(server_output_id)) =
                    (server_conversation_token, server_output_id)
                {
                    if request_id.eq(server_output_id.to_string().as_str())
                        && server_conversation_id.eq(server_conversation_token.as_str())
                    {
                        me.request_refunded_count = Some(*requests_refunded);
                        ctx.notify();
                    }
                }
            }
        });

        // Note: UpdatedStreamingExchange is handled by the dedicated on_updated_output()
        // callback in model_impl.rs, so we don't need to respond to it here.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if event
                    .terminal_surface_id()
                    .is_none_or(|id| id == me.terminal_view_id)
                {
                    match event {
                        BlocklistAIHistoryEvent::AppendedExchange { .. }
                        | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                        | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
                        | BlocklistAIHistoryEvent::StartedNewConversation { .. }
                        | BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                            ctx.notify();
                        }
                        _ => {}
                    }
                }
            },
        );

        ctx.subscribe_to_model(
            cli_subagent_controller,
            move |me, _, event, ctx| match event {
                CLISubagentEvent::SpawnedSubagent {
                    initial_requested_command_action_id: Some(initial_requested_command_action_id),
                    ..
                } => {
                    me.expand_requested_command_view(initial_requested_command_action_id, ctx);
                }
                CLISubagentEvent::FinishedSubagent {
                    initial_requested_command_action_id: Some(initial_requested_command_action_id),
                    ..
                } => {
                    me.collapse_requested_command_view(initial_requested_command_action_id, ctx);
                }
                CLISubagentEvent::UpdatedControl {
                    requested_command_action_id: Some(requested_command_action_id),
                    ..
                } => {
                    if let Some(requested_command_view) =
                        me.requested_commands.get(requested_command_action_id)
                    {
                        requested_command_view
                            .view
                            .update(ctx, |_, ctx| ctx.notify());
                    }
                }
                _ => {}
            },
        );

        ctx.subscribe_to_model(model_event_dispatcher, |me, _, event, ctx| {
            if let ModelEvent::BlockCompleted(block_completed_event) = event {
                let terminal_model = me.terminal_model.lock();
                if terminal_model
                    .block_list()
                    .block_with_id(&block_completed_event.block_id)
                    .and_then(|block| block.agent_interaction_metadata())
                    .is_some_and(|metadata| {
                        metadata
                            .requested_command_action_id()
                            .is_some_and(|id| me.requested_action_ids.contains(id))
                    })
                {
                    ctx.notify();
                }
            }
        });

        if FeatureFlag::AgentView.is_enabled() {
            ctx.subscribe_to_model(&agent_view_controller, |_, _, _, ctx| ctx.notify());
        }

        // Handoff prep emits ambient-agent events before submit, while still composing.
        // Only the run lifecycle events can change `is_cloud_agent_pre_first_exchange`
        // for this block's footer.
        if let Some(ambient_agent_view_model) = ambient_agent_view_model.as_ref() {
            ctx.subscribe_to_model(ambient_agent_view_model, |_, _, event, ctx| match event {
                AmbientAgentViewModelEvent::DispatchedAgent
                | AmbientAgentViewModelEvent::FollowupDispatched
                | AmbientAgentViewModelEvent::SessionReady { .. }
                | AmbientAgentViewModelEvent::ExecutionSessionReady { .. }
                | AmbientAgentViewModelEvent::Failed { .. }
                | AmbientAgentViewModelEvent::NeedsGithubAuth
                | AmbientAgentViewModelEvent::Cancelled
                | AmbientAgentViewModelEvent::HarnessCommandStarted { .. } => ctx.notify(),
                _ => {}
            });
        }

        ctx.subscribe_to_model(&context_model, |_, _, event, ctx| {
            if let BlocklistAIContextEvent::UpdatedPendingContext { .. } = event {
                ctx.notify();
            }
        });

        let review_changes_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Review changes", SecondaryTheme)
                .with_icon(Icon::Diff)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::ToggleCodeReviewPane);
                })
        });

        let open_all_comments_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Open all in code review", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::OpenAllImportedCommentsInCodeReview);
                })
        });

        let dismiss_suggestion_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Dismiss", SuggestionDismissButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::DismissSuggestionsSection);
                })
        });

        let disable_rule_suggestions_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Don't show again", SuggestionDismissButtonTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::DisableRuleSuggestions);
                })
        });

        let ai_block_view_id = ctx.view_id();
        let exchange_id = client_ids.client_exchange_id;
        let conversation_id = client_ids.conversation_id;
        let rewind_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Rewind", RewindButtonTheme)
                .with_size(ButtonSize::XSmall)
                .with_tooltip("Rewind to before this block")
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(TerminalAction::RewindAIConversation {
                        ai_block_view_id,
                        exchange_id,
                        conversation_id,
                        entrypoint: AgentModeRewindEntrypoint::Button,
                    });
                })
        });

        let comment_data = model
            .inputs_to_render(ctx)
            .iter()
            .find_map(|input| match input {
                AIAgentInput::CodeReview {
                    review_comments, ..
                } => Some(&review_comments.comments),
                _ => None,
            })
            .into_iter()
            .flatten()
            .map(|comment| (comment.id, comment.content.clone()))
            .collect_vec();

        let mut comment_states = HashMap::new();
        for (id, content) in comment_data {
            let state = CommentElementState::new(id, &content, ctx);
            ctx.subscribe_to_view(&state.rich_text_editor, |me, view, event, ctx| {
                if matches!(event, EditorViewEvent::TextSelectionChanged)
                    && view.as_ref(ctx).selected_text(ctx).is_some()
                {
                    me.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                    ctx.emit(AIBlockEvent::ChildViewTextSelected);
                }
            });
            comment_states.insert(id, state);
        }

        let mut me = Self {
            model,
            terminal_model,
            client_ids,
            profile_image_path: user_avatar_info.profile_image_path,
            user_display_name: user_avatar_info.display_name,
            controller,
            action_model,
            context_model,
            current_working_directory,
            shell_launch_data,
            requested_action_ids: Default::default(),
            auto_expand_requested_command_timer_handle: None,
            selected_text: Arc::new(RwLock::new(None)),
            window_id: ctx.window_id(),
            state_handles: Default::default(),
            time_to_first_token: OnceCell::new(),
            time_to_last_token: None,
            num_attached_context_blocks,
            has_attached_context_selected_text,
            finish_reason: None,
            directory_context: DirectoryContext { pwd, home_dir },
            view_id: ctx.view_id(),
            detected_links_state,
            code_editor_views: Default::default(),
            requested_commands: Default::default(),
            requested_mcp_tools: Default::default(),
            requested_edits: Default::default(),
            todo_list_states: Default::default(),
            comment_states,
            collapsible_block_states: Default::default(),
            unit_tests_suggestions: Default::default(),
            secret_redaction_state,
            find_state: FindState::default(),
            find_model,
            is_references_section_open: false,
            active_session,
            autonomy_setting_speedbump: Default::default(),
            suggested_rules: Default::default(),
            suggested_agent_mode_workflow: Default::default(),
            manage_rules_button,
            keyboard_navigable_buttons: None,
            response_rating: OnceCell::new(),
            terminal_view_id,
            request_refunded_count: None,
            action_buttons: Default::default(),
            search_codebase_view: Default::default(),
            web_search_views: Default::default(),
            web_fetch_views: Default::default(),
            requested_commands_to_auto_collapse: Default::default(),
            review_changes_button,
            open_all_comments_button,
            dismiss_suggestion_button,
            disable_rule_suggestions_button,
            rewind_button,
            view_screenshot_buttons: Default::default(),
            last_right_clicked_command: None,
            is_usage_footer_expanded: false,
            agent_view_controller,
            ambient_agent_view_model,
            aws_bedrock_credentials_error_view: None,
            imported_comments: Default::default(),
            has_imported_comments: false,
            run_agents_card_views: Default::default(),
            link_detection_handle: None,
            #[cfg(feature = "local_fs")]
            resolved_code_block_paths: Default::default(),
            #[cfg(feature = "local_fs")]
            resolved_blocklist_image_sources: Default::default(),
            terminal_view_handle,
            ask_user_question_view: None,
        };
        me.run_secret_redaction_on_user_query(me.client_ids.conversation_id, ctx);
        me.spawn_link_detection(ctx);

        if me.model.status(ctx).is_streaming() {
            me.model
                .on_updated_output(Box::new(Self::on_output_status_update), ctx);
        } else if let Some(output) = me.model.status(ctx).output_to_render() {
            // "Simulate" receiving this output if output is already complete.
            let output = output.get();
            me.handle_updated_output(&output, ctx);
            me.handle_complete_output(&output, ctx);
        }

        match me.model.status(ctx) {
            AIBlockOutputStatus::Complete { .. } => {
                me.finish(FinishReason::Complete, ctx);
            }
            AIBlockOutputStatus::Failed { error, .. } => {
                me.maybe_create_aws_bedrock_credentials_error_view(&error, ctx);
                me.finish(FinishReason::Error, ctx);
            }
            AIBlockOutputStatus::Cancelled { .. } => {
                me.finish(FinishReason::Cancelled, ctx);
            }
            AIBlockOutputStatus::PartiallyReceived { .. } | AIBlockOutputStatus::Pending => (),
        }
        me
    }

    /// Update this block's directory context (pwd and home_dir) after creation.
    pub fn update_directory_context(
        &mut self,
        pwd: Option<String>,
        home_dir: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.directory_context.pwd == pwd && self.directory_context.home_dir == home_dir {
            return;
        }

        self.directory_context.pwd = pwd;
        self.directory_context.home_dir = home_dir;
        ctx.notify();
    }

    /// Set the shell launch data for this block, re-running link detection on the
    /// user query and output. This is used to populate shell launch data on restored
    /// AI blocks after the session finishes bootstrapping.
    pub fn set_shell_launch_data(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.shell_launch_data == shell_launch_data {
            return;
        }
        self.shell_launch_data = shell_launch_data;

        // Re-run secret redaction on the user query with the new shell launch data.
        self.run_secret_redaction_on_user_query(self.client_ids.conversation_id, ctx);

        // Re-run link detection with the new shell launch data.
        self.spawn_link_detection(ctx);

        ctx.notify();
    }

    fn run_secret_redaction_on_user_query(
        &mut self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) {
        self.detected_links_state
            .detected_links_by_location
            .retain(|location, _| !matches!(location, TextLocation::Query { .. }));
        if self
            .detected_links_state
            .currently_hovered_link_location
            .as_ref()
            .is_some_and(|location| matches!(location.location, TextLocation::Query { .. }))
        {
            self.detected_links_state.currently_hovered_link_location = None;
        }
        if self
            .detected_links_state
            .link_location_open_tooltip
            .as_ref()
            .is_some_and(|location| matches!(location.location, TextLocation::Query { .. }))
        {
            self.detected_links_state.link_location_open_tooltip = None;
        }

        self.secret_redaction_state.clear_user_query_locations();

        let initial_conversation_query = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&conversation_id)
            .and_then(|conversation| conversation.initial_user_query());
        let secret_redaction_mode = get_secret_obfuscation_mode(app);
        for (input_index, input) in self.model.inputs_to_render(app).iter().enumerate() {
            let Some(query) = input.display_user_query(initial_conversation_query.as_ref()) else {
                continue;
            };
            if secret_redaction_mode.should_redact_secret() {
                self.secret_redaction_state.run_redaction_for_location(
                    &query,
                    TextLocation::Query { input_index },
                    secret_redaction_mode.is_visually_obfuscated(),
                );
            }
        }
    }

    /// Detects all links (URLs + file paths) in both the output and user query inputs.
    /// Reads the current output from the model internally.
    ///
    /// On `local_fs`, this spawns a background task via `spawn_blocking` to avoid blocking
    /// the main thread with filesystem I/O (file path detection + code block path resolution).
    /// On other targets (e.g. WASM), `spawn_blocking` is unavailable so detection runs
    /// synchronously. This is fine because it's only URL detection which is cheap.
    fn spawn_link_detection(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(handle) = self.link_detection_handle.take() {
            handle.abort();
        }

        let shared_output = self.model.status(ctx).output_to_render();
        let output_guard = shared_output.as_ref().map(|o| o.get());
        let output = output_guard.as_deref();

        let (mut texts, hyperlinks) = match output {
            Some(output) => collect_output_data_for_link_detection(
                output,
                self.current_working_directory.as_ref(),
                self.shell_launch_data.as_ref(),
            ),
            None => (Vec::new(), Vec::new()),
        };

        // Include user query texts for link detection.
        let initial_conversation_query = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&self.client_ids.conversation_id)
            .and_then(|conversation| conversation.initial_user_query());
        for (input_index, input) in self.model.inputs_to_render(ctx).iter().enumerate() {
            if let Some(query) = input.display_user_query(initial_conversation_query.as_ref()) {
                texts.push((query, TextLocation::Query { input_index }));
            }
        }

        #[cfg(feature = "local_fs")]
        {
            // Collect code block paths that need resolution for the render cache.
            let code_block_paths: Vec<PathBuf> = output
                .map(|output| {
                    output
                        .all_text()
                        .flat_map(|text| text.sections.iter())
                        .filter_map(|section| {
                            if let AIAgentTextSection::Code {
                                source: Some(source),
                                ..
                            } = section
                            {
                                source.path()
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let image_sources: Vec<String> = output
                .map(|output| {
                    output
                        .all_text()
                        .flat_map(|text| text.sections.iter())
                        .filter_map(|section| {
                            if let AIAgentTextSection::Image { image } = section {
                                Some(image.source.clone())
                            } else {
                                None
                            }
                        })
                        .unique()
                        .collect()
                })
                .unwrap_or_default();

            let cwd = self.current_working_directory.clone();
            let shell_data = self.shell_launch_data.clone();

            self.link_detection_handle = Some(ctx.spawn(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let all_links = detect_all_links(
                            &texts,
                            hyperlinks,
                            cwd.as_ref(),
                            shell_data.as_ref(),
                        );

                        // Resolve code block paths for the render cache.
                        let mut resolved_paths: HashMap<PathBuf, Option<PathBuf>> =
                            HashMap::new();
                        if let Some(home_dir) = dirs::home_dir() {
                            for path in &code_block_paths {
                                let resolved = crate::ai::blocklist::block::view_impl::common::resolve_absolute_file_path(
                                    path.clone(),
                                    cwd.as_ref(),
                                    shell_data.as_ref(),
                                    home_dir.clone(),
                                );
                                resolved_paths.insert(path.clone(), resolved);
                            }
                        }

                        let mut resolved_image_sources = HashMap::new();
                        for source in image_sources {
                            resolved_image_sources.insert(
                                source.clone(),
                                Some(
                                    resolve_asset_source_relative_to_directory(
                                        &source,
                                        cwd.as_deref().map(Path::new),
                                    )
                                    .with_local_file_content_version(),
                                ),
                            );
                        }

                        (all_links, resolved_paths, resolved_image_sources)
                    })
                    .await
                },
                |me, result, ctx| {
                    if let Ok((all_links, resolved_paths, resolved_image_sources)) = result {
                        me.detected_links_state.replace_all_links(all_links);
                        me.resolved_code_block_paths = resolved_paths;
                        me.resolved_blocklist_image_sources = resolved_image_sources;
                        ctx.notify();
                    }
                    me.link_detection_handle = None;
                },
            ));
        }

        #[cfg(not(feature = "local_fs"))]
        {
            // No filesystem I/O, so detection is cheap and runs synchronously.
            let all_links = detect_all_links(&texts, hyperlinks, None, None);
            self.detected_links_state.replace_all_links(all_links);
        }
    }

    /// Updates which conversation this block is pointing to
    pub fn reset_conversation_id(
        &mut self,
        new_conversation_id: AIConversationId,
        new_model: Rc<dyn AIBlockModel<View = AIBlock>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let conversation_contains_exchange = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&new_conversation_id)
            .map(|conversation| {
                conversation
                    .exchange_with_id(self.client_ids.client_exchange_id)
                    .is_some()
            })
            .unwrap_or(false);

        if !conversation_contains_exchange {
            log::error!(
                "Reassigning AIBlock to a new conversation but the new conversation does not contain the AI exchange"
            );
            return;
        }

        self.client_ids.conversation_id = new_conversation_id;
        self.model = new_model;
        let user_avatar_info = user_avatar_info_for_ai_block(self.model.as_ref(), ctx);
        self.profile_image_path = user_avatar_info.profile_image_path;
        self.user_display_name = user_avatar_info.display_name;
        self.run_secret_redaction_on_user_query(new_conversation_id, ctx);

        // Re-detect all links for the new conversation.
        self.spawn_link_detection(ctx);

        ctx.notify();
    }

    pub fn contains_actions(&self) -> bool {
        !self.requested_action_ids.is_empty()
    }

    pub fn contains_action(&self, action_id: &AIAgentActionId) -> bool {
        self.requested_action_ids.contains(action_id)
    }

    pub fn contains_action_result(&self, action_id: &AIAgentActionId, app: &AppContext) -> bool {
        self.model.inputs_to_render(app).iter().any(|input| {
            input
                .action_result()
                .is_some_and(|result| result.id == *action_id)
        })
    }

    fn on_output_status_update(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(latency) = self.model.time_since_request_start(ctx) {
            // Since this is a OnceCell, we'll only set time_to_first_token to the
            // latency of the first output received.
            if self.time_to_first_token.set(latency).is_ok() {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                    history.set_exchange_time_to_first_token(
                        self.client_ids.conversation_id,
                        self.client_ids.client_exchange_id,
                        latency.num_milliseconds(),
                    );
                });
            }
            self.time_to_last_token = Some(latency);
        }

        let was_autodetected_ai_query = self.model.was_autodetected_ai_query(ctx);
        let client_exchange_id = self.client_ids.client_exchange_id.to_string();
        let conversation_id = self.client_ids.conversation_id;
        let time_to_first_token_ms = self
            .time_to_first_token
            .get()
            .map(|duration| duration.num_milliseconds() as u128);
        let time_to_last_token_ms = self
            .time_to_last_token
            .map(|duration| duration.num_milliseconds() as u128);
        let status = self.model.status(ctx);
        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);

        match status {
            AIBlockOutputStatus::Pending => {
                self.requested_action_ids.clear();
                self.secret_redaction_state.reset();
            }
            AIBlockOutputStatus::PartiallyReceived { output } => {
                let output = output.get();
                self.handle_updated_output(&output, ctx);
            }
            AIBlockOutputStatus::Complete { output } => {
                let output = output.get();
                let server_output_id = self.model.server_output_id(ctx);
                self.handle_updated_output(&output, ctx);
                self.handle_complete_output(&output, ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeCreatedAIBlock {
                        client_exchange_id,
                        was_autodetected_ai_query,
                        conversation_id,
                        time_to_first_token_ms,
                        time_to_last_token_ms,
                        server_output_id,
                        was_user_facing_error: false,
                        cancelled: false,
                        is_udi_enabled,
                    },
                    ctx
                );
            }
            AIBlockOutputStatus::Cancelled { partial_output, .. } => {
                if let Some(output) = partial_output.as_ref() {
                    let output = output.get();
                    self.handle_updated_output(&output, ctx);
                }
                self.spawn_link_detection(ctx);
                self.finish(FinishReason::Cancelled, ctx);

                let server_output_id = self.model.server_output_id(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeCreatedAIBlock {
                        client_exchange_id,
                        conversation_id,
                        was_autodetected_ai_query,
                        time_to_first_token_ms,
                        time_to_last_token_ms,
                        server_output_id,
                        was_user_facing_error: false,
                        cancelled: true,
                        is_udi_enabled,
                    },
                    ctx
                );
            }
            AIBlockOutputStatus::Failed { error, .. } => {
                let server_output_id = self.model.server_output_id(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeCreatedAIBlock {
                        client_exchange_id,
                        was_autodetected_ai_query,
                        conversation_id,
                        time_to_first_token_ms,
                        time_to_last_token_ms,
                        server_output_id,
                        was_user_facing_error: true,
                        cancelled: false,
                        is_udi_enabled,
                    },
                    ctx
                );
                self.maybe_create_aws_bedrock_credentials_error_view(&error, ctx);
                // There are no actions to be taken in this block, it is finished.
                self.finish(FinishReason::Error, ctx);
            }
        }
        ctx.emit(AIBlockEvent::AIOutputUpdated);
        ctx.notify();
    }

    fn handle_updated_output(&mut self, output: &AIAgentOutput, ctx: &mut ViewContext<Self>) {
        // Ensure ui state handles are initialized for todo operation output messages.
        for message in &output.messages {
            if let AIAgentOutputMessageType::TodoOperation(TodoOperation::UpdateTodos { .. }) =
                &message.message
            {
                self.todo_list_states.entry(message.id.clone()).or_default();
            }
        }

        if FeatureFlag::WebSearchUI.is_enabled() {
            // Handle WebSearch messages
            self.handle_web_search_messages(&output.messages, ctx);
        }

        if FeatureFlag::WebFetchUI.is_enabled() {
            // Handle WebFetch messages
            self.handle_web_fetch_messages(&output.messages, ctx);
        }

        self.fetch_conversation_search_agent_run_titles(output, ctx);

        for action in output.actions() {
            let new_action_ids: HashSet<AIAgentActionId> =
                output.actions().map(|action| action.id.clone()).collect();

            #[cfg(feature = "integration_tests")]
            {
                // Log action IDs that were cached from a previous version of `output` that are not
                // in the updated `output` to assist with debugging evals.
                //
                // This is a short-term means to observe if the requested_action_ids logic is
                // faulty.
                // TODO(zachbai): Remove caching of action ids on the block entirely.
                let invalid_action_ids = self
                    .requested_action_ids
                    .difference(&new_action_ids)
                    .collect_vec();
                if !invalid_action_ids.is_empty() {
                    log::warn!("AIBlock has invalid cached action IDs: {invalid_action_ids:?}");
                }
            }
            self.requested_action_ids = new_action_ids;

            if !self.action_buttons.contains_key(&action.id) {
                let run_button = CompactibleActionButton::new(
                    "Run".to_string(),
                    Some(KeystrokeSource::Fixed(ENTER_KEYSTROKE.clone())),
                    ButtonSize::InlineActionHeader,
                    AIBlockAction::ExecuteRequestedAction {
                        action_id: action.id.clone(),
                    },
                    Icon::Play,
                    Arc::new(PrimaryTheme),
                    ctx,
                );

                let cancel_button = CompactibleActionButton::new(
                    "Cancel".to_string(),
                    Some(KeystrokeSource::Fixed(CTRL_C_KEYSTROKE.clone())),
                    ButtonSize::InlineActionHeader,
                    AIBlockAction::CancelRequestedAction {
                        action_id: action.id.clone(),
                    },
                    Icon::X,
                    Arc::new(NakedTheme),
                    ctx,
                );

                self.action_buttons.insert(
                    action.id.clone(),
                    ActionButtons {
                        run_button,
                        cancel_button,
                    },
                );
            }
            if matches!(&action.action, AIAgentActionType::StartAgent { .. }) {
                self.state_handles
                    .orchestration_navigation_card_handles
                    .entry(action.id.clone())
                    .or_default();
            }

            if matches!(
                &action.action,
                AIAgentActionType::ReadSkill(_) | AIAgentActionType::ReadFiles(_)
            ) {
                self.state_handles
                    .skill_button_handles
                    .entry(action.id.clone())
                    .or_default();
            }

            if let AIAgentActionType::RunAgents(req) = &action.action {
                self.ensure_run_agents_card_view(&action.id, req, ctx);
            }

            // Ensure a button component exists for UseComputer actions.
            if matches!(&action.action, AIAgentActionType::UseComputer(_)) {
                self.view_screenshot_buttons
                    .entry(action.id.clone())
                    .or_default();
            }

            match action {
                AIAgentAction {
                    id: action_id,
                    action:
                        AIAgentActionType::RequestCommandOutput {
                            command, citations, ..
                        },
                    ..
                } => {
                    self.handle_requested_command_stream_update(action_id, command, citations, ctx);
                }
                AIAgentAction {
                    id: action_id,
                    action:
                        AIAgentActionType::CallMCPTool {
                            server_id,
                            name,
                            input,
                        },
                    ..
                } => {
                    // Coerce the display value the same way dispatch does, so the
                    // rendered MCP tool call detail shows `5` instead of `5.0` for
                    // integer-typed fields. The raw `input` from the stream has
                    // `f64` values because `structpb.NumberValue` erases the
                    // integer/float distinction; see `coerce_integer_args` for
                    // the full rationale.
                    let display_input = match input {
                        serde_json::Value::Object(map) => {
                            let mut map = map.clone();
                            if let Some(schema) =
                                crate::ai::mcp::TemplatableMCPServerManager::as_ref(ctx)
                                    .tool_input_schema(*server_id, name.as_str())
                            {
                                crate::ai::blocklist::action_model::coerce_integer_args(
                                    &mut map, &schema,
                                );
                            }
                            serde_json::Value::Object(map)
                        }
                        other => other.clone(),
                    };
                    let command_text = if display_input.is_null() {
                        format!("MCP Tool: {name}")
                    } else {
                        format!("MCP Tool: {name} ({display_input})")
                    };
                    self.handle_mcp_tool_stream_update(
                        action_id,
                        &command_text,
                        display_input,
                        ctx,
                    );
                }
                AIAgentAction {
                    id: action_id,
                    action: AIAgentActionType::CreateDocuments(CreateDocumentsRequest { documents }),
                    ..
                } => {
                    self.handle_create_documents_stream_update(action_id, documents, ctx);
                }
                AIAgentAction {
                    id: action_id,
                    action: AIAgentActionType::AskUserQuestion { questions },
                    ..
                } if FeatureFlag::AskUserQuestion.is_enabled() => {
                    self.handle_ask_user_question_stream_update(action_id, questions, ctx);
                }
                AIAgentAction {
                    id: action_id,
                    action: AIAgentActionType::SuggestNewConversation { .. },
                    ..
                } => {
                    let start_new_conversation_button_text = "Start a new conversation".to_owned();
                    let continue_current_conversation_button_text =
                        "Continue current conversation".to_owned();

                    let server_output_id = self.model.server_output_id(ctx);
                    let accept_action = AIBlockAction::StartNewConversationButtonClicked {
                        action_id: action_id.clone(),
                        server_output_id: server_output_id.clone(),
                    };
                    let reject_action = AIBlockAction::ContinueCurrentConversationButtonClicked {
                        action_id: action_id.clone(),
                        server_output_id: server_output_id.clone(),
                    };

                    self.set_keyboard_navigable_buttons(
                        start_new_conversation_button_text,
                        continue_current_conversation_button_text,
                        accept_action,
                        reject_action,
                        ctx,
                    );
                }
                _ => (),
            }
        }
        // Build the views and stream new content for suggested code snippets.
        output
            .all_text()
            .flat_map(|text| text.sections.iter())
            .filter_map(|section| match section {
                AIAgentTextSection::Code {
                    code,
                    language,
                    source,
                } => Some((code, language, source)),
                _ => None,
            })
            .enumerate()
            .for_each(|(index, (code, language, source))| {
                self.handle_code_section_stream_update(index, code, language, source, ctx);
            });

        // Register the mouse state handles for citations.
        for citation in &output.citations {
            self.state_handles
                .footer_citation_chip_handles
                .entry(citation.clone())
                .or_default();
        }
        // Also register handles for memory citations derived from fetched_memories,
        // which are synthesized at render time and never go through output.citations.
        // Only register for the first exchange since that's the only one that shows them.
        if let Some(conversation) = self.model.conversation(ctx) {
            let is_first_exchange = conversation
                .first_exchange()
                .map(|e| Some(e.id) == self.model.exchange_id(ctx))
                .unwrap_or(false);
            if is_first_exchange {
                for memory in conversation.fetched_memories() {
                    if memory.memory_store_id.is_empty() || memory.memory_id.is_empty() {
                        continue;
                    }
                    self.state_handles
                        .footer_citation_chip_handles
                        .entry(AIAgentCitation::AgentMemory {
                            memory_store_id: memory.memory_store_id.clone(),
                            memory_id: memory.memory_id.clone(),
                            content: memory.content.clone(),
                        })
                        .or_default();
                }
            }
        }

        // Register element state for reasoning messages and track summarization timing.
        for message in &output.messages {
            if let AIAgentOutputMessageType::Reasoning {
                finished_duration, ..
            } = &message.message
            {
                let entry = self
                    .collapsible_block_states
                    .entry(message.id.clone())
                    .or_default();
                if finished_duration.is_some() {
                    entry.finish_reasoning(ctx);
                } else {
                    entry.sync_finished_state(false);
                }
            }

            // Track summarization start time and token count when summarization message arrives
            if let AIAgentOutputMessageType::Summarization {
                finished_duration,
                summarization_type,
                ..
            } = &message.message
            {
                // Only track conversation summarization, not tool call result summarization
                if matches!(summarization_type, SummarizationType::ConversationSummary) {
                    let entry = self
                        .collapsible_block_states
                        .entry(message.id.clone())
                        .or_default();
                    if finished_duration.is_some() {
                        entry.finish_summarization();
                    } else {
                        entry.sync_finished_state(false);
                    }
                }
            }

            // Register element state for debug output messages - start collapsed
            if matches!(
                &message.message,
                AIAgentOutputMessageType::DebugOutput { .. }
            ) {
                self.collapsible_block_states
                    .entry(message.id.clone())
                    .or_insert_with(CollapsibleElementState::collapsed);
            }

            // Register collapsible state for orchestration action messages.
            let orchestration_message_display_mode =
                AISettings::as_ref(ctx).orchestration_message_display_mode;
            match &message.message {
                AIAgentOutputMessageType::Action(AIAgentAction { action, .. }) => {
                    if let Some(state) = default_collapsible_state_for_orchestration_action(
                        action,
                        orchestration_message_display_mode,
                    ) {
                        self.collapsible_block_states
                            .entry(message.id.clone())
                            .or_insert(state);
                    }
                }
                AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                    for received_message in messages {
                        let collapsible_id =
                            received_message_collapsible_id(&received_message.message_id);
                        self.collapsible_block_states
                            .entry(collapsible_id.clone())
                            .or_insert_with(|| {
                                default_collapsible_state_for_orchestration_message(
                                    orchestration_message_display_mode,
                                )
                            });
                        self.state_handles
                            .transcript_avatar_handles
                            .entry(collapsible_id)
                            .or_default();
                    }
                }
                _ => {}
            }
        }

        if get_secret_obfuscation_mode(ctx).should_redact_secret() {
            self.secret_redaction_state
                .run_incremental_redaction_on_partial_output(
                    output,
                    get_secret_obfuscation_mode(ctx).is_visually_obfuscated(),
                );
        }
    }

    fn fetch_conversation_search_agent_run_titles(
        &self,
        output: &AIAgentOutput,
        ctx: &mut ViewContext<Self>,
    ) {
        for task_id in Self::conversation_search_agent_run_ids(output) {
            AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(&task_id, ctx);
            });
        }
    }

    fn handle_agent_conversations_model_event(
        &mut self,
        event: &AgentConversationsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Conversation-search labels may render with an agent-run fallback before the
        // async task title fetch completes. `TasksUpdated` is the signal that the real
        // title may now be cached, so only notify blocks that render those labels.
        if matches!(event, AgentConversationsModelEvent::TasksUpdated)
            && self.output_references_agent_run_for_conversation_search(ctx)
        {
            ctx.notify();
        }
    }

    fn output_references_agent_run_for_conversation_search(&self, app: &AppContext) -> bool {
        self.model
            .status(app)
            .output_to_render()
            .is_some_and(|output| {
                !Self::conversation_search_agent_run_ids(&output.get()).is_empty()
            })
    }

    fn conversation_search_agent_run_ids(output: &AIAgentOutput) -> Vec<AmbientAgentTaskId> {
        let mut task_ids = HashSet::new();
        for message in &output.messages {
            let AIAgentOutputMessageType::Subagent(SubagentCall {
                subagent_type:
                    SubagentType::ConversationSearch {
                        agent_run_id: Some(agent_run_id),
                        ..
                    },
                ..
            }) = &message.message
            else {
                continue;
            };
            if let Ok(task_id) = agent_run_id.parse() {
                task_ids.insert(task_id);
            }
        }
        task_ids.into_iter().collect()
    }

    fn set_keyboard_navigable_buttons(
        &mut self,
        accept_text: String,
        reject_text: String,
        accept_action: AIBlockAction,
        reject_action: AIBlockAction,
        ctx: &mut ViewContext<Self>,
    ) {
        let accept_button_handle = self.state_handles.menu_accept_button_handle.clone();
        let reject_button_handle = self.state_handles.menu_reject_button_handle.clone();
        let buttons = vec![
            KeyboardNavigableButtonBuilder::new(
                move |is_selected, app| {
                    let appearance = Appearance::handle(app).as_ref(app);
                    let mut button = appearance
                        .ui_builder()
                        .button(ButtonVariant::Secondary, accept_button_handle.clone())
                        .with_style(UiComponentStyles {
                            font_size: Some(appearance.monospace_font_size()),
                            ..UiComponentStyles::default()
                        })
                        .with_hovered_styles(UiComponentStyles {
                            font_size: Some(appearance.monospace_font_size()),
                            ..UiComponentStyles::default()
                        });
                    if is_selected {
                        let selected_styles = UiComponentStyles {
                            border_color: Some(appearance.theme().accent().into()),
                            border_width: Some(1.0),
                            background: Some(appearance.theme().surface_2().into()),
                            ..UiComponentStyles::default()
                        };
                        button = button.with_style(selected_styles);
                        button = button.with_text_and_icon_label(TextAndIcon::new(
                            TextAndIconAlignment::TextFirst,
                            accept_text.clone(),
                            Icon::CornerDownLeft.to_warpui_icon(appearance.theme().foreground()),
                            MainAxisSize::Max,
                            MainAxisAlignment::SpaceBetween,
                            vec2f(
                                appearance.monospace_font_size(),
                                appearance.monospace_font_size(),
                            ),
                        ));
                    } else {
                        button = button.with_text_label(accept_text.clone());
                    }
                    button
                },
                move |ctx: &mut ViewContext<KeyboardNavigableButtons>| {
                    ctx.dispatch_typed_action(&accept_action);
                },
            ),
            KeyboardNavigableButtonBuilder::new(
                move |is_selected, app| {
                    let appearance = Appearance::handle(app).as_ref(app);
                    let mut button = appearance
                        .ui_builder()
                        .button(ButtonVariant::Secondary, reject_button_handle.clone())
                        .with_style(UiComponentStyles {
                            font_size: Some(appearance.monospace_font_size()),
                            ..UiComponentStyles::default()
                        })
                        .with_hovered_styles(UiComponentStyles {
                            font_size: Some(appearance.monospace_font_size()),
                            ..UiComponentStyles::default()
                        });
                    if is_selected {
                        let selected_styles = UiComponentStyles {
                            border_color: Some(appearance.theme().accent().into()),
                            border_width: Some(1.0),
                            background: Some(appearance.theme().surface_2().into()),
                            ..UiComponentStyles::default()
                        };
                        button = button.with_style(selected_styles);
                        button = button.with_text_and_icon_label(TextAndIcon::new(
                            TextAndIconAlignment::TextFirst,
                            reject_text.clone(),
                            Icon::CornerDownLeft.to_warpui_icon(appearance.theme().foreground()),
                            MainAxisSize::Max,
                            MainAxisAlignment::SpaceBetween,
                            vec2f(
                                appearance.monospace_font_size(),
                                appearance.monospace_font_size(),
                            ),
                        ));
                    } else {
                        button = button.with_text_label(reject_text.clone());
                    }
                    button
                },
                move |ctx: &mut ViewContext<KeyboardNavigableButtons>| {
                    ctx.dispatch_typed_action(&reject_action);
                },
            ),
        ];
        let menu = ctx.add_typed_action_view(|_| KeyboardNavigableButtons::new(buttons));
        self.keyboard_navigable_buttons = Some(menu);
    }

    /// Applies final display behavior to orchestration message bodies.
    fn finish_orchestration_message_collapsible_states(
        &mut self,
        output: &AIAgentOutput,
        ctx: &mut ViewContext<Self>,
    ) {
        let display_mode = AISettings::as_ref(ctx).orchestration_message_display_mode;
        for message in &output.messages {
            match &message.message {
                AIAgentOutputMessageType::Action(AIAgentAction {
                    action: AIAgentActionType::SendMessageToAgent { .. },
                    ..
                }) => {
                    self.collapsible_block_states
                        .entry(message.id.clone())
                        .or_insert_with(|| {
                            default_orchestration_collapsible_state(
                                display_mode.should_expand_agent_message_body(),
                            )
                        })
                        .finish_orchestration_message(display_mode);
                }
                AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                    for received_message in messages {
                        let collapsible_id =
                            received_message_collapsible_id(&received_message.message_id);
                        self.collapsible_block_states
                            .entry(collapsible_id)
                            .or_insert_with(|| {
                                default_collapsible_state_for_orchestration_message(display_mode)
                            })
                            .finish_orchestration_message(display_mode);
                    }
                }
                AIAgentOutputMessageType::Text(_)
                | AIAgentOutputMessageType::Reasoning { .. }
                | AIAgentOutputMessageType::Summarization { .. }
                | AIAgentOutputMessageType::Subagent(_)
                | AIAgentOutputMessageType::Action(_)
                | AIAgentOutputMessageType::TodoOperation(_)
                | AIAgentOutputMessageType::WebSearch(_)
                | AIAgentOutputMessageType::WebFetch(_)
                | AIAgentOutputMessageType::CommentsAddressed { .. }
                | AIAgentOutputMessageType::DebugOutput { .. }
                | AIAgentOutputMessageType::ArtifactCreated(_)
                | AIAgentOutputMessageType::SkillInvoked(_)
                | AIAgentOutputMessageType::EventsFromAgents { .. } => {}
            }
        }
    }

    fn handle_complete_output(&mut self, output: &AIAgentOutput, ctx: &mut ViewContext<Self>) {
        self.finish_orchestration_message_collapsible_states(output, ctx);
        let mut suggestions = BlocklistAIHistoryModel::as_ref(ctx)
            .existing_suggestions_for_conversation(self.client_ids.conversation_id)
            .cloned()
            .unwrap_or_default();
        if let Some(output_suggestions) = &output.suggestions {
            suggestions.extend(output_suggestions);
        }

        if FeatureFlag::SuggestedRules.is_enabled()
            && AISettings::as_ref(ctx).is_rule_suggestions_enabled(ctx)
        {
            // Ensure we don't suggest rules that were already suggested and saved by checking the logging id.
            let existing_suggestions = self
                .suggested_rules
                .iter()
                .map(|rule| rule.read(ctx, |rule, _| rule.logging_id()))
                .collect_vec();

            let existing_rules: HashSet<SuggestedLoggingId> = {
                CloudModel::as_ref(ctx)
                    .get_all_objects_of_type::<GenericStringObjectId, CloudAIFactModel>()
                    .filter_map(|fact| {
                        let AIFact::Memory(AIMemory {
                            suggested_logging_id,
                            ..
                        }) = fact.model().string_model.clone();
                        suggested_logging_id
                    })
                    .collect()
            };

            for rule in suggestions.rules.into_iter() {
                if existing_rules.contains(&rule.logging_id)
                    || existing_suggestions.contains(&rule.logging_id)
                {
                    continue;
                }

                let rule_view =
                    ctx.add_typed_action_view(|ctx| SuggestionChipView::new_rule_chip(rule, ctx));
                ctx.subscribe_to_view(&rule_view, |_me, _view, event, ctx| match event {
                    SuggestedChipViewEvent::OpenAIFactCollection { sync_id } => {
                        ctx.emit(AIBlockEvent::OpenAIFactCollection { sync_id: *sync_id });
                    }
                    SuggestedChipViewEvent::ShowSuggestedRuleDialog { rule_and_id } => {
                        ctx.emit(AIBlockEvent::OpenSuggestedRuleDialog {
                            rule_and_id: rule_and_id.clone(),
                        });
                    }
                    _ => {}
                });
                self.suggested_rules.push(rule_view);
            }
        }

        // Only show the agent mode workflow if there are no rules.
        if FeatureFlag::SuggestedAgentModeWorkflows.is_enabled() && self.suggested_rules.is_empty()
        {
            if let Some(workflow) = suggestions.agent_mode_workflows.first() {
                let workflow_view = ctx.add_typed_action_view(|ctx| {
                    SuggestionChipView::new_agent_mode_workflow_chip(workflow.clone(), ctx)
                });
                ctx.subscribe_to_view(&workflow_view, |_me, _view, event, ctx| match event {
                    SuggestedChipViewEvent::OpenWorkflow { sync_id } => {
                        ctx.emit(AIBlockEvent::OpenWorkflow { sync_id: *sync_id });
                    }
                    SuggestedChipViewEvent::ShowSuggestedAgentModeWorkflowModal {
                        workflow_and_id,
                    } => {
                        ctx.emit(AIBlockEvent::OpenSuggestedAgentModeWorkflowModal {
                            workflow_and_id: workflow_and_id.clone(),
                        });
                    }
                    _ => {}
                });
                self.suggested_agent_mode_workflow = Some(workflow_view);
            }
        }

        for action in output.actions() {
            match action {
                AIAgentAction {
                    id,
                    action:
                        AIAgentActionType::RequestFileEdits {
                            title, file_edits, ..
                        },
                    ..
                } => {
                    self.handle_requested_edit_complete(
                        id,
                        title,
                        file_edits.clone(),
                        output.server_output_id.clone(),
                        ctx,
                    );
                }
                AIAgentAction {
                    id,
                    action: AIAgentActionType::SearchCodebase(request),
                    ..
                } => {
                    self.handle_search_codebase_complete(
                        id,
                        &request.query,
                        request.codebase_path.clone(),
                        output.server_output_id.clone(),
                        ctx,
                    );
                }
                AIAgentAction {
                    id,
                    action:
                        AIAgentActionType::SuggestPrompt(SuggestPromptRequest::UnitTestsSuggestion {
                            query,
                            title,
                            description,
                        }),
                    ..
                } => {
                    if !self.model.is_restored() {
                        self.handle_unit_test_suggestion_complete(
                            id,
                            output.server_output_id.as_ref(),
                            query.clone(),
                            title.clone(),
                            description.clone(),
                            ctx,
                        );
                    }
                }
                AIAgentAction {
                    id,
                    action:
                        AIAgentActionType::InsertCodeReviewComments {
                            repo_path,
                            comments,
                            base_branch,
                        },
                    ..
                } => {
                    if self.model.is_restored() && FeatureFlag::PRCommentsV2.is_enabled() {
                        self.handle_insert_code_review_comments(
                            id.clone(),
                            repo_path,
                            comments,
                            base_branch.as_deref(),
                            ctx,
                        );
                    }
                }
                _ => (),
            }
        }

        // Collect UI state handles for code snippets, tables, and image
        // tooltips in a single pass. Each handle type is collected in the
        // order its section type appears, matching the indices used during
        // rendering.
        let (code_buttons, table_handles, image_tooltip_handles) = output
            .all_text()
            .flat_map(|text| text.sections.iter())
            .fold(
                (Vec::new(), Vec::new(), Vec::new()),
                |(mut code_buttons, mut table_handles, mut image_tooltip_handles), section| {
                    match section {
                        AIAgentTextSection::Code { .. } => {
                            code_buttons.push(CodeSnippetButtonHandles::default());
                        }
                        AIAgentTextSection::Table { .. } => {
                            table_handles.push(TableSectionHandles::default());
                        }
                        AIAgentTextSection::Image { .. } => {
                            image_tooltip_handles.push(MouseStateHandle::default());
                        }
                        AIAgentTextSection::PlainText { .. }
                        | AIAgentTextSection::MermaidDiagram { .. } => {}
                    }
                    (code_buttons, table_handles, image_tooltip_handles)
                },
            );
        self.state_handles.normal_response_code_snippet_buttons = code_buttons;
        self.state_handles.table_section_handles = table_handles;
        self.state_handles.image_section_tooltip_handles = image_tooltip_handles;

        self.spawn_link_detection(ctx);

        let shell_type = self.active_session.as_ref(ctx).shell_type(ctx);
        let escape_char = shell_type.map(|s| ShellFamily::from(s).escape_char());

        for (requested_command_action_id, command, is_read_only, is_risky) in
            output.actions().filter_map(|action| {
                if let AIAgentAction {
                    action:
                        AIAgentActionType::RequestCommandOutput {
                            command,
                            is_read_only,
                            is_risky,
                            ..
                        },
                    ..
                } = action
                {
                    Some((
                        &action.id,
                        command,
                        is_read_only.unwrap_or(false),
                        *is_risky,
                    ))
                } else {
                    None
                }
            })
        {
            if is_agent_mode_autonomy_allowed(ctx) {
                let autoexecute_decision = escape_char.map(|escape_char| {
                    BlocklistAIPermissions::as_ref(ctx).can_autoexecute_command(
                        &self.client_ids.conversation_id,
                        command,
                        escape_char,
                        is_read_only,
                        is_risky,
                        Some(self.terminal_view_id),
                        ctx,
                    )
                });

                match autoexecute_decision {
                    Some(CommandExecutionPermission::Denied(
                        CommandExecutionPermissionDeniedReason::AlwaysAskEnabled,
                    )) if !*AISettings::as_ref(ctx)
                        .has_shown_agent_mode_profile_command_autoexecution_speedbump =>
                    {
                        // Show the default command autonomy setting if set to Always Ask.
                        self.autonomy_setting_speedbump =
                            AutonomySettingSpeedbump::ShouldShowForProfileCommandAutoexecution {
                                action_id: requested_command_action_id.clone(),
                                shown: Default::default(),
                            };
                        self.update_requested_command_autonomy_speedbump(
                            requested_command_action_id.clone(),
                            ctx,
                        );
                        // Mark the speedbump as shown in settings so that we do not render it again.
                        AISettings::handle(ctx)
                            .update(ctx, |ai_settings, ctx| {
                                if let Err(err) = ai_settings.has_shown_agent_mode_profile_command_autoexecution_speedbump.set_value(true, ctx) {
                                    log::warn!("Could not mark profile command autoexecution speedbump as shown {err}");
                                }
                            }
                        )
                    }
                    Some(CommandExecutionPermission::Denied(
                        CommandExecutionPermissionDeniedReason::Inconclusive,
                    )) if *AISettings::as_ref(ctx)
                        .should_show_agent_mode_autoexecute_readonly_commands_speedbump
                        && is_read_only =>
                    {
                        // Try to show the speedbump for the readonly command setting
                        // if we haven't shown it enough before and this command is
                        // considered readonly.
                        self.autonomy_setting_speedbump =
                            AutonomySettingSpeedbump::ShouldShowForAutoexecutingReadonlyCommands {
                                action_id: requested_command_action_id.clone(),
                                checked: true,
                                shown: Arc::new(Mutex::new(false)),
                            };
                        self.update_requested_command_autonomy_speedbump(
                            requested_command_action_id.clone(),
                            ctx,
                        );
                        // Mark the speedbump as shown in settings so that we do not render it again.
                        AISettings::handle(ctx)
                            .update(ctx, |ai_settings, ctx| {
                                if let Err(err) = ai_settings.should_show_agent_mode_autoexecute_readonly_commands_speedbump.set_value(false, ctx) {
                                    log::warn!("Could not mark autoexecute read-only commands speedbump as shown {err}");
                                }
                            }
                        )
                    }
                    _ => (),
                }
            }

            for citation in &output.citations {
                if is_command_copied_from_document(command, citation, shell_type, ctx) {
                    if let Some(requested_command) =
                        self.requested_commands.get(requested_command_action_id)
                    {
                        requested_command.view.update(ctx, |view, ctx| {
                            view.update_copied_from_citation(citation);
                            ctx.notify();
                        });
                    }
                }
            }
        }

        for action_id in output
            .actions()
            .filter_map(|action| (action.is_get_relevant_files()).then_some(&action.id))
        {
            if is_agent_mode_autonomy_allowed(ctx)
                && *AISettings::as_ref(ctx).should_show_agent_mode_autoread_files_speedbump
            {
                // Try to show the speedbump for codebase search.
                self.state_handles.codebase_search_speedbump_option_handles =
                    vec![Default::default(), Default::default()];
                self.state_handles
                    .codebase_search_speedbump_radio_button_handle
                    .set_selected_idx(0);
                self.autonomy_setting_speedbump =
                    AutonomySettingSpeedbump::ShouldShowForCodebaseSearchFileAccess {
                        action_id: action_id.clone(),
                        selected_option: Some(0),
                        shown: Arc::new(Mutex::new(false)),
                    };
                // Mark the speedbump as shown in settings so that we do not render it again.
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    if let Err(err) = ai_settings
                        .should_show_agent_mode_autoread_files_speedbump
                        .set_value(false, ctx)
                    {
                        log::warn!("Could not mark autoread files speedbump as shown {err}");
                    }
                })
            }
        }

        // One-shot flag is consumed only after the footer is attached, so restored
        // blocks and views that haven't arrived yet don't burn the flag.
        for action in output.actions() {
            let is_file_access =
                action.is_get_specific_files() || action.is_grep() || action.is_file_glob();
            if is_file_access {
                if is_agent_mode_autonomy_allowed(ctx)
                    && *AISettings::as_ref(ctx).should_show_agent_mode_autoread_files_speedbump
                {
                    // Try to show the speedbump for autoread files setting
                    // if we haven't shown it enough before.
                    self.autonomy_setting_speedbump =
                        AutonomySettingSpeedbump::ShouldShowForFileAccess {
                            action_id: action.id.clone(),
                            checked: true,
                            shown: Arc::new(Mutex::new(false)),
                        };
                    // Mark the speedbump as shown in settings so that we do not render it again.
                    AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                        if let Err(err) = ai_settings
                            .should_show_agent_mode_autoread_files_speedbump
                            .set_value(false, ctx)
                        {
                            log::warn!(
                                "Error with marking autoread files speedbump as shown {err}"
                            );
                        }
                    })
                }
            } else if matches!(action.action, AIAgentActionType::AskUserQuestion { .. })
                && !self.model.is_restored()
                && FeatureFlag::AskUserQuestion.is_enabled()
                && is_agent_mode_autonomy_allowed(ctx)
                && *AISettings::as_ref(ctx).should_show_agent_mode_ask_user_question_speedbump
            {
                self.autonomy_setting_speedbump =
                    AutonomySettingSpeedbump::ShouldShowForAskUserQuestion {
                        action_id: action.id.clone(),
                        shown: Arc::new(Mutex::new(false)),
                    };
                if self.sync_ask_user_question_speedbump_footer(ctx) {
                    Self::mark_ask_user_question_speedbump_as_shown(ctx);
                }
            }
        }

        // Run secret detection at the end of the stream to catch any secrets we might've missed while streaming,
        // due to secret patterns that may include whitespace within them (we delimit on whitespace with the optimized
        // secret detection approach).
        if get_secret_obfuscation_mode(ctx).is_visually_obfuscated() {
            self.secret_redaction_state
                .run_redaction_on_complete_output(output);
        }

        let surfaced_citations = output
            .citations
            .iter()
            .filter_map(|citation| citation.for_telemetry(ctx))
            .collect_vec();
        if !surfaced_citations.is_empty() {
            send_telemetry_from_ctx!(
                TelemetryEvent::AgentModeSurfacedCitations {
                    citations: surfaced_citations,
                    block_id: self.client_ids.client_exchange_id.to_string(),
                    conversation_id: self.client_ids.conversation_id,
                    server_output_id: output.server_output_id.clone(),
                },
                ctx
            );
        }

        // This is used to trigger the theme chooser opening when the theme chooser onboarding block is active.
        if let Some(text_message) = output.text_from_agent_output().last() {
            if text_message.sections.iter().any(|section| {
                if let AIAgentTextSection::PlainText { text } = section {
                    text.text().contains("The matrix theme is now available at")
                } else {
                    false
                }
            }) {
                ctx.emit(AIBlockEvent::OpenThemeChooser);
            }
        }
        if self.requested_action_ids.is_empty() {
            // There are no actions to be taken in this block, it is finished.
            self.finish(FinishReason::Complete, ctx);
        }
    }

    /// Returns `true` if the AI block should be hidden.
    pub fn is_hidden(&self, app: &AppContext) -> bool {
        let is_for_hidden_exchange = BlocklistAIHistoryModel::as_ref(app).is_exchange_hidden(
            self.client_ids.conversation_id,
            self.client_ids.client_exchange_id,
        );
        // If the AI Block's exchange is hidden, return true.
        //
        // This is typically the case for the initial exchange in a conversation started for a
        // 'passive' AI feature like suggested prompts.
        if is_for_hidden_exchange {
            return true;
        }
        if !FeatureFlag::AgentView.is_enabled() {
            return false;
        }

        if let Some(active_conversation_id) = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
        {
            // If the agent view is active, only AI blocks for the active agent view conversation
            // should be visible.
            active_conversation_id != self.client_ids.conversation_id
        } else {
            // If there is no active agent view, only passive, non-hidden (we checked for if the
            // exchange is hidden already above) exchanges are rendered.
            //
            // These correspond to AI blocks with a successfully received suggested code diff or
            // unit test suggestion.
            !self.model.request_type(app).is_passive()
        }
    }

    pub fn is_passive_conversation(&self, app: &AppContext) -> bool {
        self.model.request_type(app).is_passive()
    }

    fn handle_code_section_stream_update(
        &mut self,
        index: usize,
        code: &str,
        language: &Option<ProgrammingLanguage>,
        source: &Option<CodeSource>,
        ctx: &mut ViewContext<Self>,
    ) {
        match self.code_editor_views.get_mut(index) {
            Some(embedded_view) => {
                embedded_view.view.update(ctx, |view, ctx| {
                    // The language and starting line number may not be specified in the output for the first iteration.
                    // Only set the language/starting line number the first time that they are specified or if they change.
                    if embedded_view.language != *language {
                        embedded_view.language = language.clone();
                        if let Some(extension) = language
                            .as_ref()
                            .and_then(|language| language.to_extension())
                        {
                            // Since this is a code snippet, construct a fake path name for looking up the language.
                            let fake_path = format!("/snippet.{extension}");
                            view.set_language_with_local_path(Path::new(&fake_path), ctx);
                        }
                    }
                    let starting_line_number = source.as_ref().and_then(|s| {
                        if let CodeSource::Link { range_start, .. } = s {
                            range_start.as_ref().map(|ls| ls.line_num)
                        } else {
                            None
                        }
                    });
                    if view.starting_line_number() != starting_line_number {
                        view.set_starting_line_number(starting_line_number);
                    }

                    // Update the buffer with just the new or deleted range.
                    // Assumption: Only the end of the string is updated.
                    // Assumption: The only time text is deleted is at the end of parsing, where it has partially
                    // received the ``` end marker.
                    // Ex: Iteration 57: "a += 12\n``"
                    // Ex: Iteration 58: "a += 12"
                    match code.len().cmp(&embedded_view.length) {
                        Ordering::Greater => {
                            view.append_at_end(&code[embedded_view.length..], ctx);
                            ctx.notify();
                        }
                        Ordering::Less => {
                            view.truncate(code.len(), ctx);
                            ctx.notify();
                        }
                        Ordering::Equal => {}
                    }
                    embedded_view.length = code.len();
                });
            }
            None => {
                let view = ctx.add_typed_action_view(|ctx| {
                    CodeEditorView::new(
                        self.shell_launch_data.clone().map(|data| data.into()),
                        None,
                        CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight),
                        ctx,
                    )
                    .with_can_show_diff_ui(false)
                });
                view.update(ctx, |view, ctx| {
                    view.set_starting_line_number({
                        source.as_ref().and_then(|s| match s {
                            CodeSource::Link { range_start, .. } => {
                                range_start.as_ref().map(|ls| ls.line_num)
                            }
                            _ => None,
                        })
                    });
                    view.set_show_current_line_highlights(false, ctx);
                    view.set_interaction_state(InteractionState::Selectable, ctx);
                    let state = InitialBufferState::plain_text(code);
                    view.reset(state, ctx);

                    // Apply language immediately on initial creation so restored blocks get syntax highlighting.
                    if let Some(ext) = language.as_ref().and_then(|lang| lang.to_extension()) {
                        let fake_path = format!("/snippet.{ext}");
                        view.set_language_with_local_path(Path::new(&fake_path), ctx);
                    }

                    ctx.notify();
                });
                ctx.subscribe_to_view(&view, |me, view, event, ctx| match event {
                    CodeEditorEvent::SelectionChanged => {
                        // If there's an ongoing text selection, clear all other selections within the
                        // `AIBlock`'s view sub-hierarchy to ensure only one component has a selection at a time.
                        //
                        // The `is_some` check is necessary because `CodeEditorEvent::SelectionChanged` is
                        // also emitted when the editor's selection is cleared via external means
                        // (i.e. when a text selection is made outside the `CodeEditorView`).
                        if view.as_ref(ctx).selected_text(ctx).is_some() {
                            me.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                            ctx.emit(AIBlockEvent::ChildViewTextSelected);
                        }
                    }
                    CodeEditorEvent::CopiedEmptyText => {
                        ctx.emit(AIBlockEvent::CopiedEmptyText);
                    }
                    #[cfg(windows)]
                    CodeEditorEvent::WindowsCtrlC { .. } => {
                        ctx.emit(AIBlockEvent::WindowsCtrlC);
                    }
                    _ => {}
                });
                self.code_editor_views.push(EmbeddedCodeEditorView {
                    view,
                    language: language.clone(),
                    length: code.len(),
                });
            }
        }
    }

    pub fn current_todo_list<'a>(&'a self, app: &'a AppContext) -> Option<&'a AIAgentTodoList> {
        self.model.conversation_id(app).and_then(|id| {
            BlocklistAIHistoryModel::as_ref(app)
                .conversation(&id)
                .and_then(|conversation| conversation.active_todo_list())
        })
    }

    fn enable_autoexecute_override(&mut self, ctx: &mut ViewContext<Self>) {
        let is_on: bool = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&self.client_ids.conversation_id)
            .map(|c| c.autoexecute_any_action())
            .unwrap_or(false);
        if !is_on {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(
                    &self.client_ids.conversation_id,
                    self.terminal_view_id,
                    ctx,
                );
            });
        }
    }

    fn mark_ask_user_question_speedbump_as_shown(ctx: &mut ViewContext<Self>) {
        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            if let Err(err) = ai_settings
                .should_show_agent_mode_ask_user_question_speedbump
                .set_value(false, ctx)
            {
                log::warn!("Could not mark ask-user-question speedbump as shown: {err}");
            }
        });
    }

    fn handle_requested_edit_complete(
        &mut self,
        action_id: &AIAgentActionId,
        title: &Option<String>,
        file_edits: Vec<FileEdit>,
        server_output_id: Option<ServerOutputId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let identifiers = AIIdentifiers {
            client_conversation_id: Some(self.client_ids.conversation_id),
            client_exchange_id: Some(self.client_ids.client_exchange_id),
            server_output_id,
            server_conversation_id: None,
            model_id: self.model.model_id(ctx),
        };
        let contains_str_replace = file_edits.iter().any(|file_edit| {
            matches!(
                file_edit,
                FileEdit::Edit(ai::diff_validation::ParsedDiff::StrReplaceEdit { .. })
            )
        });
        let contains_v4a = file_edits.iter().any(|file_edit| {
            matches!(
                file_edit,
                FileEdit::Edit(ai::diff_validation::ParsedDiff::V4AEdit { .. })
            )
        });
        let edit_format_kind = match (contains_str_replace, contains_v4a) {
            (true, false) => RequestFileEditsFormatKind::StrReplace,
            (false, true) => RequestFileEditsFormatKind::V4A,
            (true, true) => RequestFileEditsFormatKind::Mixed,
            (false, false) => RequestFileEditsFormatKind::Unknown,
        };

        // Only show the speedbump once, update the setting afterwards.
        let should_show_code_suggestion_speedbump =
            self.model.request_type(ctx).is_passive_code_diff()
                && UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable()
                && AISettings::as_ref(ctx).show_code_suggestion_speedbump(ctx);
        if should_show_code_suggestion_speedbump {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                if let Err(e) = settings
                    .show_code_suggestion_speedbump
                    .set_value(false, ctx)
                {
                    log::error!("Failed to persist 'Show code suggestion speedbump' setting: {e}");
                }
            });
        }

        let view = ctx.add_typed_action_view(|ctx| {
            CodeDiffView::new(
                action_id,
                self.model.as_ref(),
                title.clone(),
                identifiers,
                edit_format_kind,
                should_show_code_suggestion_speedbump,
                self.action_model.clone(),
                self.shell_launch_data.clone().map(|data| data.into()),
                ctx,
            )
        });
        let executor = self
            .action_model
            .as_ref(ctx)
            .request_file_edits_executor(ctx);
        executor.update(ctx, |executor, _| {
            executor.register_requested_edits(action_id, &view);
        });

        // If the diff is being viewed in a shared session (read-only mode), populate diffs from the payload.
        if self.action_model.as_ref(ctx).is_view_only() {
            let active_session = self.active_session.as_ref(ctx);
            let file_diffs = convert_file_edits_to_file_diffs(
                file_edits,
                &active_session.shell_launch_data(ctx),
                &active_session.current_working_directory().cloned(),
            );
            view.update(ctx, |diff_view, ctx| {
                diff_view.set_candidate_diffs(file_diffs, ctx);
            });
        }

        let action_id_clone = action_id.clone();
        ctx.subscribe_to_view(&view, move |me, view, event, ctx| {
            match event {
                CodeDiffViewEvent::TryAccept => {
                    me.action_model.update(ctx, |action_model, ctx| {
                        action_model.execute_action(
                            &action_id_clone,
                            me.client_ids.conversation_id,
                            ctx,
                        );
                    });
                }
                CodeDiffViewEvent::EnableAutoexecuteMode => {
                    me.enable_autoexecute_override(ctx);
                }
                CodeDiffViewEvent::Rejected => {
                    me.cancel_action(&action_id_clone, ctx);
                }
                CodeDiffViewEvent::EditModeChanged { enabled } => {
                    if *enabled {
                        ctx.emit(AIBlockEvent::OpenCodeWithDiff { view: view.clone() })
                    } else {
                        ctx.notify()
                    }
                }
                CodeDiffViewEvent::ToggledEditVisibility => {
                    ctx.emit(AIBlockEvent::ToggleCodeDiffVisibility);
                    ctx.notify();
                }
                CodeDiffViewEvent::EditorFocused => {
                    // Actions within the editor should clear all other text selections
                    me.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                }
                CodeDiffViewEvent::TextSelected => {
                    // If there's an ongoing text selection, clear all other selections within the
                    // `AIBlock`'s view sub-hierarchy to ensure only one component has a selection at a time.
                    me.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                    ctx.emit(AIBlockEvent::ChildViewTextSelected);
                }
                CodeDiffViewEvent::CopiedEmptyText => {
                    ctx.emit(AIBlockEvent::CopiedEmptyText);
                }
                CodeDiffViewEvent::Blur => {
                    ctx.emit(AIBlockEvent::FocusTerminal);
                }
                CodeDiffViewEvent::DisplayModeChanged => {
                    ctx.notify();
                }
                CodeDiffViewEvent::OpenSettings => {
                    ctx.emit(AIBlockEvent::OpenSettings);
                }
                CodeDiffViewEvent::CancelPassive => {
                    ctx.emit(AIBlockEvent::DismissedPassiveBlock);
                }
                CodeDiffViewEvent::ViewDetails => {
                    // We only need to set the selected conversation when agent view is disabled;
                    // when agent view is enabled, you have to enter the agent view for the code diff
                    // conversation to follow-up in the first place, and hitting 'view details'
                    // shouldn't auto-enter the agent view.
                    if !FeatureFlag::AgentView.is_enabled() {
                        me.context_model.update(ctx, |context_model, ctx| {
                            context_model.set_pending_query_state_for_existing_conversation(
                                me.client_ids.conversation_id,
                                AgentViewEntryOrigin::ViewPassiveCodeDiffDetails,
                                ctx,
                            );
                        });
                    }
                    ctx.emit(AIBlockEvent::FocusTerminal);
                    ctx.notify();
                }
                CodeDiffViewEvent::ContinuePassiveCodeDiffWithAgent {
                    accepted: auto_resume,
                    ..
                } => {
                    let trigger_block_id = me.model.inputs_to_render(ctx).iter().find_map(|i| {
                        if let Some(PassiveSuggestionTrigger::ShellCommandCompleted(trigger)) =
                            i.passive_suggestion_trigger()
                        {
                            Some(trigger.executed_shell_command.id.clone())
                        } else {
                            None
                        }
                    });
                    ctx.emit(AIBlockEvent::ContinuePassiveCodeDiffWithAgent {
                        conversation_id: me.client_ids.conversation_id,
                        trigger_block_id,
                        auto_resume: *auto_resume,
                    });
                    ctx.emit(AIBlockEvent::FocusTerminal);
                    ctx.notify();
                }
                CodeDiffViewEvent::ToggleCodeReviewPane { entrypoint } => {
                    ctx.emit(AIBlockEvent::ToggleCodeReviewPane {
                        entrypoint: *entrypoint,
                    });
                }
                CodeDiffViewEvent::LoadedDiffs => {
                    if me.model.request_type(ctx).is_passive_code_diff() {
                        ctx.emit(AIBlockEvent::PassiveCodeDiffLoaded);
                    }
                }
                #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
                CodeDiffViewEvent::OpenSkill { reference, path } => {
                    #[cfg(feature = "local_fs")]
                    {
                        ctx.emit(AIBlockEvent::OpenCodeInWarp {
                            source: CodeSource::Skill {
                                reference: reference.clone(),
                                location: path.clone(),
                                origin: SkillOpenOrigin::EditFiles,
                            },
                            layout: *crate::util::file::external_editor::EditorSettings::as_ref(
                                ctx,
                            )
                            .open_file_layout
                            .value(),
                        });
                    }
                }
                #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
                CodeDiffViewEvent::OpenMCPConfig { path, .. } => {
                    #[cfg(feature = "local_fs")]
                    {
                        ctx.emit(AIBlockEvent::OpenCodeInWarp {
                            source: CodeSource::Link {
                                path: path.clone(),
                                range_start: None,
                                range_end: None,
                            },
                            layout: *crate::util::file::external_editor::EditorSettings::as_ref(
                                ctx,
                            )
                            .open_file_layout
                            .value(),
                        });
                    }
                }
                _ => (),
            }
        });

        self.requested_edits
            .insert(action_id.clone(), RequestedEdit::new(view));

        if self.model.request_type(ctx).is_passive() {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.set_exchange_hidden_status(
                    self.terminal_view_id,
                    self.client_ids.conversation_id,
                    self.client_ids.client_exchange_id,
                    false,
                    ctx,
                );
            });
            self.terminal_model
                .lock()
                .block_list_mut()
                .mark_rich_content_dirty(ctx.view_id());
        }
        ctx.notify();
    }

    pub fn set_restored_file_edits(
        &mut self,
        action_id: &AIAgentActionId,
        file_edits: Vec<crate::ai::agent::FileEdit>,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();

        let shell_launch_data = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        if let Some(code_diff_view) = self.requested_edits.get(action_id).map(|edit| &edit.view) {
            let file_diffs = crate::ai::blocklist::inline_action::code_diff_view::convert_file_edits_to_file_diffs(
                file_edits,
                &shell_launch_data,
                &current_working_directory,
            );

            code_diff_view.update(ctx, |diff_view, ctx| {
                diff_view.set_candidate_diffs(file_diffs, ctx);

                // For restored conversations that include a passive code diff, we assume the diff
                // is no longer live, so we display it as embedded instead of inline.
                if self.model.request_type(ctx).is_passive_code_diff() {
                    diff_view.set_embedded_display_mode(true, ctx);
                }

                // Set the state based on the action status from the action model
                let action_status = self.action_model.as_ref(ctx).get_action_status(action_id);

                let is_reverted = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&self.client_ids.conversation_id)
                    .map(|conv| conv.is_action_reverted(action_id))
                    .unwrap_or(false);

                let state = if is_reverted {
                    CodeDiffState::Reverted
                } else {
                    match action_status {
                        Some(AIActionStatus::Finished(result)) => {
                            if result.result.is_successful() {
                                CodeDiffState::Accepted(None)
                            } else {
                                // For other finished states, default to rejected
                                CodeDiffState::Rejected
                            }
                        }
                        _ => {
                            // When we quit in the middle of an action being completed, it is expected for that action to be saved as in progress.
                            // However, it does not make sense to interact with a restored in-progress action,
                            // so we mark the action as cancelled/rejected on restore.
                            CodeDiffState::Rejected
                        }
                    }
                };
                diff_view.set_state(state, ctx);
            });
        }
    }

    /// Handle a new requested command received from the server. This will update the existing
    /// requested command block if one exists, or insert a new one otherwise.
    fn handle_requested_command_stream_update(
        &mut self,
        action_id: &AIAgentActionId,
        command: &str,
        citations: &[AIAgentCitation],
        ctx: &mut ViewContext<Self>,
    ) {
        match self.requested_commands.get_mut(action_id) {
            Some(requested_command) => {
                requested_command.view.update(ctx, |view, ctx| {
                    view.apply_streamed_update(command, ctx);
                    view.update_derived_from_citations(citations);
                    ctx.notify();
                });
            }
            None => {
                let view = ctx.add_typed_action_view(|ctx| {
                    let mut view = RequestedCommandView::new(
                        action_id.clone(),
                        self.client_ids.clone(),
                        RequestedActionViewType::Command,
                        self.model.clone(),
                        &self.action_model,
                        self.terminal_model.clone(),
                        self.autonomy_setting_speedbump.clone(),
                        self.state_handles
                            .manage_autonomy_settings_link_handle
                            .clone(),
                        self.view_id,
                        ctx,
                    );
                    view.apply_streamed_update(command, ctx);
                    view.update_derived_from_citations(citations);
                    view
                });
                let action_id_clone = action_id.clone();
                ctx.subscribe_to_view(&view, move |me, view, event, ctx| {
                    me.handle_requested_command_view_event(&action_id_clone, view, event, ctx);
                });

                self.requested_commands
                    .insert(action_id.clone(), RequestedCommand { view });
            }
        }
    }

    fn handle_requested_command_view_event(
        &mut self,
        action_id: &AIAgentActionId,
        view: ViewHandle<RequestedCommandView>,
        event: &RequestedCommandViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Short-circuit if this is no longer a requested command we're tracking.
        if !self.requested_commands.contains_key(action_id) {
            return;
        }
        match event {
            RequestedCommandViewEvent::Accepted => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.handle_requested_command_accepted(
                        action_id,
                        view.as_ref(ctx).command_text().to_string(),
                        ctx,
                    );
                });
                self.yield_requested_action_focus_if_focused(&view, ctx);
                ctx.notify();
            }
            RequestedCommandViewEvent::EnableAutoexecuteMode => {
                self.enable_autoexecute_override(ctx);
            }
            RequestedCommandViewEvent::Rejected => {
                self.cancel_action(action_id, ctx);
                self.yield_requested_action_focus_if_focused(&view, ctx);
            }
            RequestedCommandViewEvent::UpdatedExpansionState { is_expanded } => {
                // We only care about expansion state updates when the command
                // is running or finished (i.e. when it has a block).
                let action_status = self.action_model.as_ref(ctx).get_action_status(action_id);
                let has_finished_command_block = {
                    let terminal_model = self.terminal_model.lock();
                    terminal_model
                        .block_list()
                        .block_for_ai_action_id(action_id)
                        .is_some_and(|block| block.finished())
                };
                if !has_finished_command_block
                    && !action_status.is_some_and(|a| a.is_running() || a.is_done())
                {
                    return;
                }

                // If the requested command is being expanded, we don't need to auto-expand anymore.
                if *is_expanded {
                    self.abort_auto_expand_requested_command_timer();
                } else {
                    // Remove requested command from list of requested commands to auto-collapse if the user manually collapses it.
                    // In the edge-case where the user then manually expands the requested command, we won't auto-collapse it again.
                    self.requested_commands_to_auto_collapse.remove(action_id);
                }

                ctx.emit(AIBlockEvent::UpdateInlineActionVisibility {
                    action_id: action_id.clone(),
                    is_visible: *is_expanded,
                });
                ctx.notify();
            }
            RequestedCommandViewEvent::TextSelected => {
                // If there's an ongoing text selection, clear all other selections within the
                // `AIBlock`'s view sub-hierarchy to ensure only one component has a selection at a time.
                self.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                ctx.emit(AIBlockEvent::ChildViewTextSelected);
            }
            RequestedCommandViewEvent::CopiedEmptyText => {
                ctx.emit(AIBlockEvent::CopiedEmptyText);
            }
            RequestedCommandViewEvent::EditorFocused => {
                // Actions within the editor should clear all other text selections
                self.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
            }
            RequestedCommandViewEvent::OpenActiveAgentProfileEditor => {
                ctx.emit(AIBlockEvent::OpenActiveAgentProfileEditor);
            }
        }
    }

    /// Update the autonomy setting speedbump in requested command and MCP tool views that match the given action ID.
    fn update_requested_command_autonomy_speedbump(
        &self,
        action_id: AIAgentActionId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(requested_command) = self.requested_commands.get(&action_id) {
            requested_command.view.update(ctx, |view, ctx| {
                view.set_autonomy_setting_speedbump(self.autonomy_setting_speedbump.clone(), ctx);
            });
        }
        if let Some(requested_mcp_tool) = self.requested_mcp_tools.get(&action_id) {
            requested_mcp_tool.view.update(ctx, |view, ctx| {
                view.set_autonomy_setting_speedbump(self.autonomy_setting_speedbump.clone(), ctx);
            });
        }
    }

    /// Handle a new MCP tool call received from the server. This will update the existing
    /// MCP tool call block if one exists, or insert a new one otherwise.
    fn handle_mcp_tool_stream_update(
        &mut self,
        action_id: &AIAgentActionId,
        command_text: &str,
        mcp_args: serde_json::Value,
        ctx: &mut ViewContext<Self>,
    ) {
        match self.requested_mcp_tools.get_mut(action_id) {
            Some(requested_mcp_tool) => {
                requested_mcp_tool.view.update(ctx, |view, ctx| {
                    view.apply_streamed_update(command_text, ctx);
                    view.update_mcp_request(mcp_args);
                    ctx.notify();
                });
            }
            None => {
                let view = ctx.add_typed_action_view(|ctx| {
                    let mut view = RequestedCommandView::new(
                        action_id.clone(),
                        self.client_ids.clone(),
                        RequestedActionViewType::McpTool,
                        self.model.clone(),
                        &self.action_model,
                        self.terminal_model.clone(),
                        self.autonomy_setting_speedbump.clone(),
                        self.state_handles
                            .manage_autonomy_settings_link_handle
                            .clone(),
                        self.view_id,
                        ctx,
                    );
                    view.apply_streamed_update(command_text, ctx);
                    view.update_mcp_request(mcp_args);
                    view
                });
                let action_id_clone = action_id.clone();
                ctx.subscribe_to_view(&view, move |me, view, event, ctx| {
                    me.handle_mcp_tool_view_event(&action_id_clone, view, event, ctx);
                });

                self.requested_mcp_tools
                    .insert(action_id.clone(), RequestedCommand { view });
            }
        }
    }

    fn handle_mcp_tool_view_event(
        &mut self,
        action_id: &AIAgentActionId,
        view: ViewHandle<RequestedCommandView>,
        event: &RequestedCommandViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Short-circuit if this is no longer an MCP tool call we're tracking.
        if !self.requested_mcp_tools.contains_key(action_id) {
            return;
        }
        match event {
            RequestedCommandViewEvent::Accepted => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.execute_action(action_id, self.client_ids.conversation_id, ctx);
                });
                self.yield_requested_action_focus_if_focused(&view, ctx);
                ctx.notify();
            }
            RequestedCommandViewEvent::Rejected => {
                self.cancel_action(action_id, ctx);
                self.yield_requested_action_focus_if_focused(&view, ctx);
            }
            RequestedCommandViewEvent::TextSelected => {
                // If there's an ongoing text selection, clear all other selections within the
                // `AIBlock`'s view sub-hierarchy to ensure only one component has a selection at a time.
                self.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                ctx.emit(AIBlockEvent::ChildViewTextSelected);
            }
            RequestedCommandViewEvent::CopiedEmptyText => {
                ctx.emit(AIBlockEvent::CopiedEmptyText);
            }
            RequestedCommandViewEvent::EditorFocused => {
                // Actions within the editor should clear all other text selections
                self.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
            }
            RequestedCommandViewEvent::EnableAutoexecuteMode => {
                self.enable_autoexecute_override(ctx);
            }
            // There's nothing to do here for MCP tool calls; their expanded state
            // doesn't change the blocklist like it does for requested commands.
            RequestedCommandViewEvent::UpdatedExpansionState { .. } => {}
            RequestedCommandViewEvent::OpenActiveAgentProfileEditor => {
                ctx.emit(AIBlockEvent::OpenActiveAgentProfileEditor);
            }
        }
    }

    fn handle_ask_user_question_stream_update(
        &mut self,
        action_id: &AIAgentActionId,
        questions: &[AskUserQuestionItem],
        ctx: &mut ViewContext<Self>,
    ) {
        let needs_init = match self.ask_user_question_view.as_ref() {
            Some(view) => !view.as_ref(ctx).matches_action(action_id, questions),
            None => true,
        };
        if !needs_init {
            return;
        }

        let action_model = self.action_model.clone();
        let conversation_id = self.client_ids.conversation_id;
        let action_id_for_view = action_id.clone();
        let questions_for_view = questions.to_vec();
        let view = ctx.add_typed_action_view(move |ctx| {
            AskUserQuestionView::new(
                action_model.clone(),
                conversation_id,
                action_id_for_view.clone(),
                questions_for_view.clone(),
                ctx,
            )
        });
        let action_id_clone = action_id.clone();
        ctx.subscribe_to_view(&view, move |me, _, event, ctx| {
            me.handle_ask_user_question_view_event(&action_id_clone, event, ctx);
        });

        self.ask_user_question_view = Some(view.clone());
        if self
            .action_model
            .as_ref(ctx)
            .get_action_status(action_id)
            .is_some_and(|status| status.is_blocked())
        {
            ctx.focus(&view);
        }
        if self.sync_ask_user_question_speedbump_footer(ctx)
            && *AISettings::as_ref(ctx).should_show_agent_mode_ask_user_question_speedbump
        {
            Self::mark_ask_user_question_speedbump_as_shown(ctx);
        }
        ctx.notify();
    }

    /// Attaches the footer to the view if the speedbump variant matches. Called from
    /// both the variant-seeding and view-creation paths.
    fn sync_ask_user_question_speedbump_footer(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(view) = self.ask_user_question_view.clone() else {
            return false;
        };
        let speedbump_matches = matches!(
            &self.autonomy_setting_speedbump,
            AutonomySettingSpeedbump::ShouldShowForAskUserQuestion { action_id, .. }
                if action_id == view.as_ref(ctx).action_id()
        );
        if speedbump_matches {
            let settings_link_handle = self
                .state_handles
                .ask_user_question_speedbump_settings_link_handle
                .clone();
            if let AutonomySettingSpeedbump::ShouldShowForAskUserQuestion { shown, .. } =
                &self.autonomy_setting_speedbump
            {
                *shown.lock() = true;
            }
            let terminal_view_id = self.terminal_view_id;
            view.update(ctx, |view, ctx| {
                view.set_speedbump_settings_link(Some(settings_link_handle), ctx);
                view.init_speedbump_dropdown(ctx);
                view.refresh_speedbump_dropdown_selection(terminal_view_id, ctx);
            });
            true
        } else {
            false
        }
    }

    fn handle_ask_user_question_view_event(
        &mut self,
        action_id: &AIAgentActionId,
        event: &AskUserQuestionViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self
            .ask_user_question_view
            .as_ref()
            .is_some_and(|view| view.as_ref(ctx).action_id() == action_id)
        {
            return;
        }

        match event {
            AskUserQuestionViewEvent::Updated => {
                ctx.notify();
            }
            AskUserQuestionViewEvent::SpeedbumpPermissionChanged(permission) => {
                let permission = *permission;
                let profile_id = *AIExecutionProfilesModel::as_ref(ctx)
                    .active_profile(Some(self.terminal_view_id), ctx)
                    .id();
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    model.set_ask_user_question(profile_id, permission, ctx);
                });
                send_telemetry_from_ctx!(
                    TelemetryEvent::ChangedAgentModeAskUserQuestionPermission {
                        src: AutonomySettingToggleSource::Speedbump,
                        new: permission,
                    },
                    ctx
                );
                Self::mark_ask_user_question_speedbump_as_shown(ctx);
                self.autonomy_setting_speedbump = AutonomySettingSpeedbump::None;
                ctx.notify();
            }
        }
    }

    fn handle_create_documents_stream_update(
        &mut self,
        action_id: &AIAgentActionId,
        documents: &[DocumentToCreate],
        ctx: &mut ViewContext<Self>,
    ) {
        let model_handle = AIDocumentModel::handle(ctx);
        let conversation_id = self.client_ids.conversation_id;
        // If the conversation stream has already been stopped, don't process the updates.
        // We need to do this to avoid marking the document as streaming again in the AIDocumentModel on
        // get_or_create_streaming_document_for_create_documents below after the stream has already been stopped.
        // This might throw away the last update for a normally completed stream, but that's okay because
        // we'll reset using the full content in the CreateDocumentsExecutor.
        if !self.model.status(ctx).is_streaming() {
            return;
        }
        let active_session_ref = self.active_session.as_ref(ctx);
        let file_link_resolution_context =
            active_session_ref
                .current_working_directory()
                .map(|working_directory| FileLinkResolutionContext {
                    working_directory: working_directory.clone(),
                    shell_launch_data: active_session_ref.shell_launch_data(ctx),
                });

        let mut opened_first = false;

        for (index, document) in documents.iter().enumerate() {
            let title = if document.title.is_empty() {
                DEFAULT_PLANNING_DOCUMENT_TITLE.to_string()
            } else {
                document.title.clone()
            };

            let (document_id, created_new) = model_handle.update(ctx, |model, model_ctx| {
                let (document_id, created_new) = model
                    .get_or_create_streaming_document_for_create_documents(
                        conversation_id,
                        action_id,
                        index,
                        &title,
                        document.content.clone(),
                        file_link_resolution_context.clone(),
                        model_ctx,
                    );
                if !created_new {
                    model.apply_streamed_agent_update(
                        &document_id,
                        &title,
                        &document.content,
                        model_ctx,
                    );
                }
                (document_id, created_new)
            });

            if created_new && !opened_first {
                ctx.emit(AIBlockEvent::OpenAIDocumentPane {
                    document_id,
                    document_version: AIDocumentVersion::default(),
                    is_auto_open: true,
                });
                opened_first = true;
            }
        }
    }

    fn calculate_renderable_action_index(
        &self,
        target_action_id: &AIAgentActionId,
        app: &AppContext,
    ) -> Option<usize> {
        let output = self.model.status(app).output_to_render()?;
        let output = output.get();
        output.calculate_action_index(target_action_id)
    }

    fn handle_web_search_messages(
        &mut self,
        messages: &[AIAgentOutputMessage],
        ctx: &mut ViewContext<Self>,
    ) {
        for message in messages {
            // Check if this is a WebSearch message
            let AIAgentOutputMessageType::WebSearch(status) = &message.message else {
                continue;
            };

            if let Some(view) = self.web_search_views.get(&message.id) {
                // Update existing view
                view.update(ctx, |view, ctx| {
                    view.set_status(status);
                    ctx.notify();
                });
            } else {
                let view = ctx.add_typed_action_view(|_ctx| {
                    let mut view = WebSearchView::new(String::new());
                    view.set_status(status);
                    view
                });

                self.web_search_views.insert(message.id.clone(), view);
                ctx.notify();
            }
        }
    }

    fn handle_web_fetch_messages(
        &mut self,
        messages: &[AIAgentOutputMessage],
        ctx: &mut ViewContext<Self>,
    ) {
        for message in messages {
            // Check if this is a WebFetch message
            let AIAgentOutputMessageType::WebFetch(status) = &message.message else {
                continue;
            };

            if let Some(view) = self.web_fetch_views.get(&message.id) {
                // Update existing view
                view.update(ctx, |view, ctx| {
                    view.set_status(status);
                    ctx.notify();
                });
            } else {
                let view = ctx.add_typed_action_view(|_ctx| {
                    let mut view = WebFetchView::new(Vec::new());
                    view.set_status(status);
                    view
                });

                self.web_fetch_views.insert(message.id.clone(), view);
                ctx.notify();
            }
        }
    }

    /// Note this is called when the search codebase tool call definition finishes streaming, not when the search actually completes.
    fn handle_search_codebase_complete(
        &mut self,
        action_id: &AIAgentActionId,
        query: &str,
        repo_path: Option<String>,
        _server_output_id: Option<ServerOutputId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::SearchCodebaseUI.is_enabled() {
            return;
        }

        let Some(action_index) = self.calculate_renderable_action_index(action_id, ctx) else {
            return;
        };

        let view = ctx.add_typed_action_view(|_ctx| {
            SearchCodebaseView::new(
                self.find_model.clone(),
                vec![],
                query.to_string(),
                repo_path,
                &self.shell_launch_data,
                &self.current_working_directory,
                action_index,
            )
        });

        // Subscribe to events from SearchCodebaseView and convert them to AIBlockActions
        ctx.subscribe_to_view(&view, |me, view, event, ctx| match event {
            #[cfg(feature = "local_fs")]
            SearchCodebaseViewEvent::OpenLinkTooltip { rich_content_link } => {
                let rich_content_link = match rich_content_link {
                    RichContentLink::FilePath {
                        absolute_path,
                        line_and_column_num,
                        ..
                    } => RichContentLink::FilePath {
                        absolute_path: absolute_path.clone(),
                        line_and_column_num: *line_and_column_num,
                        target_override: me.detected_file_path_target_override(absolute_path),
                    },
                    RichContentLink::Url(url) => RichContentLink::Url(url.clone()),
                };
                ctx.emit(AIBlockEvent::ShowLinkTooltip(RichContentLinkTooltipInfo {
                    link: rich_content_link,
                    position_id: RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID.to_owned(),
                }));
                ctx.notify();
            }
            #[cfg(not(feature = "local_fs"))]
            SearchCodebaseViewEvent::OpenLinkTooltip { rich_content_link } => {
                ctx.emit(AIBlockEvent::ShowLinkTooltip(RichContentLinkTooltipInfo {
                    link: rich_content_link.clone(),
                    position_id: RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID.to_owned(),
                }));
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            SearchCodebaseViewEvent::OpenDetectedFilePath {
                absolute_path,
                line_and_column_num,
            } => {
                ctx.emit(AIBlockEvent::OpenDetectedFilePath {
                    absolute_path: absolute_path.clone(),
                    line_and_column_num: *line_and_column_num,
                    target_override: me.detected_file_path_target_override(absolute_path),
                });
            }
            SearchCodebaseViewEvent::TextSelected => {
                me.clear_other_selections(Some(view.id()), ctx.window_id(), ctx);
                ctx.emit(AIBlockEvent::ChildViewTextSelected);
            }
        });

        self.search_codebase_view.insert(action_id.clone(), view);

        // Initialize the view with the action status and file contexts from the action model, if populated.
        // The action is not expected to exist already in live conversations since search is just beginning,
        // but it's not incorrect to populate if it is, and we rely on this for
        // for restored conversations because action model events don't re-fire
        // after the view is created.
        let action_status = self.action_model.as_ref(ctx).get_action_status(action_id);
        if let Some(view) = self.search_codebase_view.get(action_id) {
            let files = if let Some(AIActionStatus::Finished(ref result)) = action_status {
                if let AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Success {
                    files,
                }) = &result.result
                {
                    Some(files.clone())
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(files) = files {
                let find_state = self.find_state.clone();
                view.update(ctx, |view, ctx| {
                    view.update_render_read_file_args(&find_state, files, action_status);
                    ctx.notify();
                });
            } else if action_status.is_some() {
                view.update(ctx, |view, ctx| {
                    view.update_status(action_status);
                    ctx.notify();
                });
            }
        }

        ctx.notify();
    }

    /// Creates the AWS Bedrock credentials error view if the error is `AwsBedrockCredentialsExpiredOrInvalid`
    /// and we don't already have one. If auto-login is enabled, automatically runs the login command.
    fn maybe_create_aws_bedrock_credentials_error_view(
        &mut self,
        error: &RenderableAIError,
        ctx: &mut ViewContext<Self>,
    ) {
        // Only create the view for AWS Bedrock credentials errors
        let RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid { model_name } = error else {
            return;
        };

        // Don't recreate if we already have a view
        if self.aws_bedrock_credentials_error_view.is_some() {
            return;
        }

        let ai_settings = AISettings::as_ref(ctx);
        let login_command = ai_settings.aws_bedrock_auth_refresh_command.value().clone();
        let auto_login_enabled = *ai_settings.aws_bedrock_auto_login.value();

        // If auto-login is enabled, run the login command automatically
        if auto_login_enabled {
            ctx.emit(AIBlockEvent::RunAwsLoginCommand);
        }

        let model_name = model_name.clone();
        let view = ctx.add_typed_action_view(|ctx| {
            AwsBedrockCredentialsErrorView::new(model_name, login_command, auto_login_enabled, ctx)
        });

        // Subscribe to events from the view and emit AIBlockEvents directly
        // Note: We emit events here rather than dispatch actions because we're in a
        // subscription callback where the context is already for AIBlock
        ctx.subscribe_to_view(&view, |_me, _view, event, ctx| match event {
            AwsBedrockCredentialsErrorEvent::RunLoginCommand => {
                ctx.emit(AIBlockEvent::RunAwsLoginCommand);
            }
            AwsBedrockCredentialsErrorEvent::ConfigureLoginCommand => {
                ctx.dispatch_typed_action(&WorkspaceAction::ShowSettingsPageWithSearch {
                    search_query: "aws bedrock".to_string(),
                    section: Some(SettingsSection::WarpAgent),
                });
            }
        });

        self.aws_bedrock_credentials_error_view = Some(view);
        ctx.notify();
    }

    pub fn accept_pending_unit_test_suggestion(
        &mut self,
        interaction_source: InteractionSource,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(suggested_prompt) = self.pending_unit_test_suggestion(ctx) else {
            return false;
        };
        self.accept_unit_test_suggestion(suggested_prompt.clone(), interaction_source, ctx)
    }

    pub fn dismiss_pending_suggested_prompt(
        &mut self,
        interaction_source: InteractionSource,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(suggested_prompt) = self.pending_unit_test_suggestion(ctx) else {
            return false;
        };
        let identifiers = suggested_prompt.as_ref(ctx).identifiers().clone();

        // Complete the suggest prompt executor with Cancelled so the async action
        // finishes cleanly (the action auto-executes and is no longer in pending_actions).
        self.action_model.update(ctx, |action_model, ctx| {
            let executor = action_model.suggest_prompt_executor(ctx).clone();
            executor.update(ctx, |executor, _ctx| {
                executor.complete_suggest_prompt_action(SuggestPromptResult::Cancelled);
            });
        });

        // Hide the view so pending_unit_test_suggestion() won't find it again,
        // preventing a double-dismiss race from emitting DismissedPassiveBlock twice.
        suggested_prompt.clone().update(ctx, |view, _ctx| {
            view.set_is_hidden(true);
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::UnitTestSuggestionCancelled {
                identifiers,
                interaction_source,
            },
            ctx
        );
        ctx.emit(AIBlockEvent::DismissedPassiveBlock);
        true
    }

    fn accept_unit_test_suggestion(
        &mut self,
        view: ViewHandle<SuggestedUnitTestsView>,
        interaction_source: InteractionSource,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(query) = view.as_ref(ctx).query() else {
            return false;
        };

        if FeatureFlag::AgentView.is_enabled()
            && self
                .agent_view_controller
                .update(ctx, |controller, ctx| {
                    controller.try_enter_agent_view(
                        Some(self.client_ids.conversation_id),
                        AgentViewEntryOrigin::AcceptedUnitTestSuggestion,
                        ctx,
                    )
                })
                .is_err()
        {
            return false;
        }

        let action_id = view.as_ref(ctx).action_id().clone();

        self.action_model.update(ctx, |action_model, ctx| {
            action_model.execute_action(&action_id, self.client_ids.conversation_id, ctx);
            let executor = action_model.suggest_prompt_executor(ctx).clone();
            executor.update(ctx, |executor, _ctx| {
                executor.complete_suggest_prompt_action(SuggestPromptResult::Accepted { query });
            });
        });
        // When accepted, we only want to hide the banner portion of the exchange.
        view.update(ctx, |view, _ctx| {
            view.set_is_hidden(true);
        });

        let identifiers = view.as_ref(ctx).identifiers().clone();
        let query = view.as_ref(ctx).query().unwrap_or_default();

        let should_collect_ugc =
            should_collect_ai_ugc_telemetry(ctx, PrivacySettings::as_ref(ctx).is_telemetry_enabled);
        let redacted_query = if should_collect_ugc {
            let mut redacted_query = query.clone();
            redact_secrets(&mut redacted_query);
            Some(redacted_query)
        } else {
            None
        };
        send_telemetry_from_ctx!(
            TelemetryEvent::UnitTestSuggestionAccepted {
                identifiers,
                query: redacted_query,
                interaction_source,
            },
            ctx
        );
        ctx.notify();
        true
    }

    fn handle_unit_test_suggestion_complete(
        &mut self,
        action_id: &AIAgentActionId,
        server_output_id: Option<&ServerOutputId>,
        query: String,
        title: String,
        description: String,
        ctx: &mut ViewContext<Self>,
    ) {
        // Short-circuit if we've already handled the suggested prompt correspoding to this action
        // id.
        if self.unit_tests_suggestions.contains_key(action_id) {
            return;
        }

        let identifiers = AIIdentifiers {
            client_conversation_id: Some(self.client_ids.conversation_id),
            client_exchange_id: Some(self.client_ids.client_exchange_id),
            server_output_id: server_output_id.cloned(),
            server_conversation_id: None,
            model_id: self.model.model_id(ctx),
        };

        // Only show the speedbump once, update the setting afterwards.
        let should_show_speedbump = self
            .model
            .request_type(ctx)
            .is_passive_unit_test_suggestion()
            && UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable()
            && AISettings::as_ref(ctx).show_code_suggestion_speedbump(ctx);
        if should_show_speedbump {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                if let Err(e) = settings
                    .show_code_suggestion_speedbump
                    .set_value(false, ctx)
                {
                    log::error!("Failed to persist 'Show code suggestion speedbump' setting: {e}");
                }
            });
        }

        let view = ctx.add_typed_action_view(|ctx| {
            SuggestedUnitTestsView::new(
                identifiers.clone(),
                action_id.clone(),
                query,
                title,
                description,
                should_show_speedbump,
                ctx,
            )
        });

        let action_id_clone = action_id.clone();
        ctx.subscribe_to_view(&view, move |me, view, event, ctx| {
            me.handle_suggested_prompt_view_event(&action_id_clone, event, view, ctx);
        });

        self.unit_tests_suggestions.insert(action_id.clone(), view);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.set_exchange_hidden_status(
                self.terminal_view_id,
                self.client_ids.conversation_id,
                self.client_ids.client_exchange_id,
                false,
                ctx,
            );
        });
        self.terminal_model
            .lock()
            .block_list_mut()
            .mark_rich_content_dirty(ctx.view_id());
        ctx.notify();

        send_telemetry_from_ctx!(TelemetryEvent::UnitTestSuggestionShown { identifiers }, ctx);
    }

    fn handle_suggested_prompt_view_event(
        &mut self,
        action_id: &AIAgentActionId,
        event: &SuggestedUnitTestsEvent,
        view: ViewHandle<SuggestedUnitTestsView>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Short-circuit if this is no longer a suggested prompt we're tracking.
        if !self.unit_tests_suggestions.contains_key(action_id) {
            return;
        }

        match event {
            SuggestedUnitTestsEvent::Accept => {
                self.accept_unit_test_suggestion(view, InteractionSource::Button, ctx);
            }
            SuggestedUnitTestsEvent::Cancel => {
                self.dismiss_pending_suggested_prompt(InteractionSource::Button, ctx);
            }
            SuggestedUnitTestsEvent::Blur => {
                ctx.emit(AIBlockEvent::FocusTerminal);
            }
            SuggestedUnitTestsEvent::OpenSettings => {
                ctx.emit(AIBlockEvent::OpenSettings);
            }
        }
    }

    #[cfg(feature = "integration_tests")]
    pub fn selection_type(&self) -> SelectionType {
        self.state_handles.selection_handle.selection_type()
    }

    /// Handles find match focus changes by auto-expanding collapsed reasoning blocks
    /// that contain the focused match.
    /// The number of cached find matches for this AI block. Test-only; used to
    /// assert that find highlights are cleared when the find bar closes.
    #[cfg(test)]
    pub(crate) fn find_match_count(&self) -> usize {
        self.find_state.match_count()
    }

    fn handle_find_match_focus_change(&mut self, ctx: &mut ViewContext<Self>) {
        // Get the currently focused match ID from the terminal's find model.
        // The helper handles both the sync and async find paths.
        let Some(match_id) = self.find_model.as_ref(ctx).focused_rich_content_match_id() else {
            return;
        };

        // Resolve the match ID to a location in this block's find state
        let Some(match_location) = self.find_state.location_for_match(match_id) else {
            return;
        };

        // If the match has a message ID, expand that message's block if it's collapsed
        let Some(message_id) = &match_location.message_id else {
            return;
        };

        if let Some(state) = self.collapsible_block_states.get_mut(message_id) {
            if let CollapsibleExpansionState::Collapsed = state.expansion_state {
                state.expand();
                ctx.notify();
            }
        }
    }
}

impl AIBlock {
    pub fn conversation_id(&self) -> AIConversationId {
        self.client_ids.conversation_id
    }

    /// Reverts all file diffs (CodeDiffViews) in this AIBlock, from newest to oldest (order matters)
    pub fn revert_all_diffs(&mut self, ctx: &mut ViewContext<Self>) {
        for edit in self.requested_edits.values().rev() {
            edit.view.update(ctx, |diff_view, ctx| {
                diff_view.handle_action(&CodeDiffViewAction::RevertChanges, ctx);
            });
        }
    }

    pub fn response_stream_id(&self) -> Option<&ResponseStreamId> {
        self.client_ids.response_stream_id.as_ref()
    }

    pub fn current_working_directory(&self) -> Option<&String> {
        self.current_working_directory.as_ref()
    }

    pub fn output_status(&self, app: &AppContext) -> AIBlockOutputStatus {
        self.model.status(app)
    }

    /// Returns `true` if this AI block contains user input.
    pub fn has_user_input(&self, app: &AppContext) -> bool {
        self.model
            .inputs_to_render(app)
            .iter()
            .any(|input| input.display_query().is_some())
    }

    /// `true` if the AI block is "finished".
    /// An AI block is not finished if it has any pending requested command(s) or pending
    /// requested action(s) that are still receiving output.
    pub fn is_finished(&self) -> bool {
        self.finish_reason.is_some()
    }

    pub fn is_manually_cancelled(&self) -> bool {
        self.finish_reason == Some(FinishReason::Cancelled)
    }

    pub fn is_complete(&self) -> bool {
        self.finish_reason == Some(FinishReason::Complete)
    }

    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish_reason
    }

    pub fn is_restored(&self) -> bool {
        self.model.is_restored()
    }

    /// Returns the rich-content link currently hovered by the mouse, if any.
    ///
    /// This is used to build a link-specific right-click context menu (e.g. "Copy URL") when the
    /// user right-clicks a hyperlink rendered inside an AI response. Returns `None` if the mouse
    /// is not over a detected link.
    pub fn hovered_rich_content_link(&self) -> Option<RichContentLink> {
        let hovered = self
            .detected_links_state
            .currently_hovered_link_location
            .as_ref()?;
        let link_type = self
            .detected_links_state
            .link_at(&hovered.location, &hovered.link_range)?;
        let rich_content_link = match link_type {
            DetectedLinkType::Url(link) => RichContentLink::Url(link.clone()),
            #[cfg(feature = "local_fs")]
            DetectedLinkType::FilePath {
                absolute_path,
                line_and_column_num,
            } => RichContentLink::FilePath {
                absolute_path: absolute_path.to_owned(),
                line_and_column_num: *line_and_column_num,
                target_override: self.detected_file_path_target_override(absolute_path),
            },
        };
        Some(rich_content_link)
    }

    /// `true` if the AI output in the block finished streaming.
    ///
    /// Note that this is different from `is_finished` since user could still have pending
    /// actions to execute.
    pub fn is_ai_output_complete(&self, app: &AppContext) -> bool {
        self.model.status(app).is_complete()
    }

    /// Returns `true` if the block contains any actions that are blocked on user confirmation.
    pub fn is_blocked_on_user_confirmation(&self, app: &AppContext) -> bool {
        self.requested_action_ids
            .iter()
            .filter_map(|id| self.action_model.as_ref(app).get_action_status(id))
            .any(|status| status.is_blocked())
    }

    /// Returns the server output ID for this AI block, if available.
    pub fn server_output_id(&self, app: &AppContext) -> Option<ServerOutputId> {
        self.model.server_output_id(app)
    }

    fn finish(&mut self, finish_reason: FinishReason, ctx: &mut ViewContext<Self>) {
        if self.finish_reason.is_some() {
            return;
        }
        self.finish_reason = Some(finish_reason);
        ctx.emit(AIBlockEvent::Finished);
    }

    /// Registers a subscription on the `BlocklistAIActionModel` that updates view state in response
    /// to AI action updates.
    fn register_action_model_subscription(
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_model(action_model, |me, action_model, event, ctx| {
            let action_id = event.action_id();

            if me.is_finished() || !me.requested_action_ids.contains(action_id) {
                // Technically, this subscription should be unregistered after `is_finished` is
                // set to true, but it seems that the callback is called once more after the `unsubscribe_to_model`
                // call, so early return here if this is errantly being called.
                return;
            }

            match event {
                BlocklistAIActionEvent::ExecutingAction(..) => {
                    match &me.autonomy_setting_speedbump {
                        AutonomySettingSpeedbump::ShouldShowForAutoexecutingReadonlyCommands {
                            action_id: speedbump_action_id,
                            shown,
                            checked,
                            ..
                        } if speedbump_action_id == action_id && *shown.lock() => {
                            BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, ctx| {
                                report_if_error!(permissions
                                    .set_should_autoexecute_readonly_commands(*checked, ctx));
                            });
                        }
                        AutonomySettingSpeedbump::ShouldShowForFileAccess {
                            action_id: speedbump_action_id,
                            shown,
                            checked,
                            ..
                        } if speedbump_action_id == action_id && *shown.lock() => {
                            let permission = if *checked {
                                AgentModeCodingPermissionsType::AlwaysAllowReading
                            } else {
                                AgentModeCodingPermissionsType::AlwaysAskBeforeReading
                            };
                            BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, ctx| {
                                report_if_error!(
                                    permissions.set_coding_permissions(permission, ctx)
                                );
                            });
                        }
                        AutonomySettingSpeedbump::ShouldShowForCodebaseSearchFileAccess {
                            action_id: speedbump_action_id,
                            shown,
                            selected_option,
                            ..
                        } if speedbump_action_id == action_id && *shown.lock() => {
                            let Some(root_repo_path) = me
                                .action_model
                                .as_ref(ctx)
                                .search_codebase_executor(ctx)
                                .as_ref(ctx)
                                .root_repo_for_action(action_id)
                                .map(Path::to_owned)
                            else {
                                return;
                            };

                            let permission = match selected_option {
                                Some(0) => AgentModeCodingPermissionsType::AlwaysAllowReading,
                                Some(1) => {
                                    AgentModeCodingPermissionsType::AllowReadingSpecificFiles
                                }
                                _ => AgentModeCodingPermissionsType::AlwaysAskBeforeReading,
                            };
                            BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, ctx| {
                                report_if_error!(
                                    permissions.set_coding_permissions(permission, ctx)
                                );
                                if matches!(
                                    permission,
                                    AgentModeCodingPermissionsType::AllowReadingSpecificFiles
                                ) {
                                    report_if_error!(permissions
                                        .add_filepath_to_code_read_allowlist(root_repo_path, ctx));
                                }
                            });
                        }
                        _ => {}
                    }
                }
                BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(..) => {
                    ctx.emit(AIBlockEvent::ActionBlockedOnUserConfirmation);
                }
                BlocklistAIActionEvent::FinishedAction { action_id, .. } => {
                    me.abort_auto_expand_requested_command_timer();

                    // Handle auto-collapsing of requested commands that were marked for auto-collapse
                    // Skip collapsing for interrupted commands (exit code 130) so user can see partial output
                    if let Some(requested_command) = me
                        .requested_commands_to_auto_collapse
                        .contains(action_id)
                        .then(|| me.requested_commands.get(action_id))
                        .flatten()
                    {
                        let should_collapse = action_model
                            .as_ref(ctx)
                            .get_action_result(action_id)
                            .is_none_or(|result| match &result.result {
                                AIAgentActionResultType::RequestCommandOutput(
                                    RequestCommandOutputResult::Completed { exit_code, .. },
                                ) => exit_code.value() != 130, // Keep original expansion state on ctrl-c during long running command
                                _ => true, // Collapse for other results
                            }); // Default to collapse if can't determine

                        if should_collapse {
                            requested_command.force_collapse(ctx);
                        }
                        me.requested_commands_to_auto_collapse.remove(action_id);
                    }

                    if let Some(view) = me.search_codebase_view.get(action_id) {
                        let new_status = action_model.as_ref(ctx).get_action_status(action_id);
                        view.update(ctx, |view, ctx| {
                            view.update_status(new_status);
                            ctx.notify();
                        });
                    }

                    let action_statuses = me
                        .requested_action_ids
                        .iter()
                        .filter_map(|id| action_model.as_ref(ctx).get_action_status(id))
                        .collect_vec();

                    // Detecting links on SearchCodebase tool call outputs
                    for (action_index, status) in action_statuses.iter().enumerate() {
                        let AIActionStatus::Finished(result) = status else {
                            continue;
                        };
                        if let AIAgentActionResultType::SearchCodebase(
                            SearchCodebaseResult::Success { files },
                        ) = &result.result
                        {
                            if !FeatureFlag::SearchCodebaseUI.is_enabled() {
                                for (line_index, file) in files.iter().enumerate() {
                                    let text_location = TextLocation::Action {
                                        action_index,
                                        line_index,
                                    };
                                    detect_links(
                                        &mut me.detected_links_state,
                                        &file.to_string(),
                                        text_location,
                                        me.current_working_directory.as_ref(),
                                        me.shell_launch_data.as_ref(),
                                    );
                                }
                            }

                            if let Some(view) = me.search_codebase_view.get(action_id) {
                                view.update(ctx, |view, ctx| {
                                    view.update_render_read_file_args(
                                        &me.find_state,
                                        files.clone(),
                                        action_model.as_ref(ctx).get_action_status(action_id),
                                    );
                                    ctx.notify();
                                })
                            }
                        }
                    }

                    // Open the AI document pane when documents are created or edited
                    if let Some(action_result) =
                        action_model.as_ref(ctx).get_action_result(action_id)
                    {
                        match &action_result.result {
                            AIAgentActionResultType::CreateDocuments(
                                CreateDocumentsResult::Success { created_documents },
                            ) => {
                                if let Some(first_doc) = created_documents.first() {
                                    ctx.emit(AIBlockEvent::OpenAIDocumentPane {
                                        document_id: first_doc.document_id,
                                        document_version: first_doc.document_version,
                                        is_auto_open: true,
                                    });
                                }
                            }
                            AIAgentActionResultType::EditDocuments(
                                EditDocumentsResult::Success { updated_documents },
                            ) => {
                                if let Some(first_doc) = updated_documents.first() {
                                    ctx.emit(AIBlockEvent::OpenAIDocumentPane {
                                        document_id: first_doc.document_id,
                                        document_version: first_doc.document_version,
                                        is_auto_open: true,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }

                    if action_statuses.iter().any(AIActionStatus::is_blocked) && !me.is_hidden(ctx)
                    {
                        // TODO (suraj): figure out focus behaviour for multi-action responses.
                        me.try_steal_focus(ctx);
                        ctx.emit(AIBlockEvent::ActionFinished);
                    } else if action_statuses
                        .iter()
                        .all(|action| action.is_cancelled_during_requested_command_execution())
                    {
                        me.finish(FinishReason::CancelledDuringRequestedCommandExecution, ctx);
                        ctx.unsubscribe_to_model(&action_model);
                    } else if action_statuses.iter().all(|action| action.is_cancelled()) {
                        me.finish(FinishReason::Cancelled, ctx);
                        ctx.unsubscribe_to_model(&action_model);
                    } else {
                        // Only finish the block when all tool calls in this block are done.
                        if action_statuses.iter().all(|s| s.is_done()) {
                            me.finish(FinishReason::Complete, ctx);
                            ctx.unsubscribe_to_model(&action_model);
                        }
                    }
                    ctx.notify();
                }
                BlocklistAIActionEvent::QueuedAction(action_id) => {
                    // Update search codebase view status when action is queued
                    if let Some(view) = me.search_codebase_view.get(action_id) {
                        view.update(ctx, |view, ctx| {
                            view.update_status(Some(AIActionStatus::Queued));
                            ctx.notify();
                        });
                    }
                    ctx.notify();
                }
                BlocklistAIActionEvent::InsertCodeReviewComments {
                    action_id,
                    repo_path,
                    comments,
                    base_branch,
                } => {
                    if FeatureFlag::PRCommentsV2.is_enabled() {
                        me.handle_insert_code_review_comments(
                            action_id.clone(),
                            repo_path,
                            comments,
                            base_branch.as_deref(),
                            ctx,
                        );
                        ctx.notify();
                    }
                }

                BlocklistAIActionEvent::InitProject(_)
                | BlocklistAIActionEvent::ToggleCodeReview(_) => {}
            }
        });
    }

    /// Cleans up state for this block, to be called before the block is `Drop`ped (e.g. deleted from the blocklist).
    pub fn cleanup_block(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_finished() {
            return;
        }
        self.controller.update(ctx, |controller, ctx| {
            controller.cancel_conversation_progress(
                self.client_ids.conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
        self.finish(FinishReason::Cancelled, ctx);
    }

    fn handle_insert_code_review_comments(
        &mut self,
        action_id: AIAgentActionId,
        repo_path: &Path, // TODO: this should be migrated to str
        comments: &[InsertReviewComment],
        base_branch: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(repo_location) = self
            .active_session
            .as_ref(ctx)
            .location_for_path(repo_path.to_string_lossy().as_ref(), ctx)
        else {
            log::warn!(
                "Cannot import review comments for repo path without an active session location: {}",
                repo_path.display()
            );
            return;
        };

        let raw_count = comments.len();
        let pending = convert_insert_review_comments(comments);
        let converted_count = pending.len();
        let flattened = attach_pending_imported_comments(pending, &repo_location);
        let thread_count = flattened.len();

        if !self.model.is_restored() {
            send_telemetry_from_ctx!(
                CodeReviewTelemetryEvent::CommentsReceived {
                    is_local: Some(repo_location.is_local()),
                    raw_count,
                    converted_count,
                    thread_count,
                },
                ctx
            );
        }

        let cards: Vec<CommentViewCard> = flattened
            .into_iter()
            .map(|comment| {
                CommentViewCard::new(comment, true, true, None, Some(&repo_location), ctx)
            })
            .collect();

        let element_states = cards
            .iter()
            .enumerate()
            .map(|(comment_index, card)| {
                let html_url = match &card.source().origin {
                    CommentOrigin::ImportedFromGitHub(details) => details.html_url.clone(),
                    CommentOrigin::Native => None,
                };
                ImportedCommentElementState::new(action_id.clone(), comment_index, html_url, ctx)
            })
            .collect();

        let base_branch = base_branch.map(String::from);

        if !cards.is_empty() {
            self.has_imported_comments = true;
            let conversation_id = self.client_ids.conversation_id;
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, _| {
                model.mark_conversation_has_imported_comments(conversation_id);
            });
        }

        self.imported_comments.insert(
            action_id,
            ImportedCommentGroup::new(repo_location, base_branch, cards, element_states),
        );

        self.update_imported_comments_disabled_state(ctx);
    }

    fn cancel_action(&mut self, action_id: &AIAgentActionId, ctx: &mut ViewContext<Self>) {
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_action_with_id(
                self.client_ids.conversation_id,
                action_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
    }

    fn yield_requested_action_focus_if_focused(
        &self,
        view: &ViewHandle<RequestedCommandView>,
        ctx: &mut ViewContext<Self>,
    ) {
        if view.is_self_or_child_focused(ctx) {
            ctx.emit(AIBlockEvent::FocusTerminal);
        }
    }
    /// Tries to focus the AI block or one of its parts, if applicable.
    /// If the block doesn't need to be focused, focus is yielded
    /// back to the owning [`TerminalView`].
    ///
    /// WARNING: take care to only use this API when you are sure the AI block or its
    /// children should steal focus (e.g. on user action). For example, be careful
    /// not to steal focus away from another terminal session just because a requested
    /// command needs attention.
    pub fn try_steal_focus(&self, ctx: &mut ViewContext<Self>) {
        if self.is_hidden(ctx) {
            return;
        }

        // If there's a blocking passive code diff, focus that.
        // We special case this since get_pending_action only focuses on active conversations,
        // and passive code diffs are not part of an active conversation, when they initially appear.
        if self.model.request_type(ctx).is_passive_code_diff() {
            if let Some(diff) = self.find_undismissed_code_diff(ctx) {
                ctx.focus(&diff.view);
                return;
            }
        }

        if self
            .model
            .request_type(ctx)
            .is_passive_unit_test_suggestion()
            && self.pending_unit_test_suggestion(ctx).is_some()
        {
            ctx.emit(AIBlockEvent::FocusTerminal);
            return;
        }

        if self.focus_subview_if_necessary(ctx) {
            return;
        }

        let should_focus_block =
            !self.requested_action_ids.is_empty() || self.keyboard_navigable_buttons.is_some();
        if should_focus_block {
            // Otherwise, just focus the block.
            ctx.focus_self();
        } else {
            // Else, delegate focus back to the terminal view.
            ctx.emit(AIBlockEvent::FocusTerminal);
        }
    }

    pub fn focus_subview_if_necessary(&self, ctx: &mut ViewContext<Self>) -> bool {
        let mut did_focus_subview = false;
        let pending_action_id = self
            .action_model
            .as_ref(ctx)
            .get_pending_action(ctx)
            .map(|a| &a.id);

        if let Some(diff) = pending_action_id.and_then(|id| self.requested_edits.get(id)) {
            // If there's a blocking code diff, focus that.
            ctx.focus(&diff.view);
            did_focus_subview = true;
        } else if let Some(command) =
            pending_action_id.and_then(|id| self.requested_commands.get(id))
        {
            // If there's a blocking requested command, focus that.
            ctx.focus(&command.view);
            did_focus_subview = true;
        } else if let Some(mcp_tool) =
            pending_action_id.and_then(|id| self.requested_mcp_tools.get(id))
        {
            // If there's a blocking MCP tool call, focus that.
            ctx.focus(&mcp_tool.view);
            did_focus_subview = true;
        } else if let Some(ask_user_question_view) =
            self.ask_user_question_view.as_ref().filter(|view| {
                pending_action_id.is_some_and(|id| {
                    let view = view.as_ref(ctx);
                    view.action_id() == id && view.is_editing()
                })
            })
        {
            ctx.focus(ask_user_question_view);
            did_focus_subview = true;
        } else if let Some(card_view) =
            pending_action_id.and_then(|id| self.run_agents_card_views.get(id))
        {
            // If there's a blocking RunAgents card, focus it so its
            // own keybindings (`enter -> Accept`, `cmdorctrl-e ->
            // ToggleEdit`, etc.) resolve.
            ctx.focus(card_view);
            did_focus_subview = true;
        } else if let Some(keyboard_navigable_buttons) = self.keyboard_navigable_buttons.as_ref() {
            // If there's buttons to take action on, focus those.
            ctx.focus(keyboard_navigable_buttons);
            did_focus_subview = true;
        }
        did_focus_subview
    }

    /// Returns the currently selected text within the entire `AIBlock` view sub-hierarchy.
    /// There **shouldn't** be more than one instance of selected text at any given time across
    /// any view within the same `AIBlock` view sub-hierarchy.
    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        self.code_editor_views
            .iter()
            .find_map(|editor_view| editor_view.view.as_ref(ctx).selected_text(ctx))
            .or_else(|| {
                self.requested_commands
                    .values()
                    .find_map(|command| command.view.as_ref(ctx).selected_text(ctx))
            })
            .or_else(|| {
                self.requested_mcp_tools
                    .values()
                    .find_map(|tool| tool.view.as_ref(ctx).selected_text(ctx))
            })
            .or_else(|| {
                self.requested_edits
                    .values()
                    .find_map(|edit| edit.view.as_ref(ctx).selected_text(ctx))
            })
            .or_else(|| {
                self.search_codebase_view
                    .values()
                    .find_map(|search_view| search_view.as_ref(ctx).selected_text(ctx))
            })
            .or_else(|| {
                self.comment_states
                    .values()
                    .find_map(|comment| comment.rich_text_editor.as_ref(ctx).selected_text(ctx))
            })
            .or_else(|| self.selected_text.read().clone())
            .filter(|selection| !selection.is_empty())
    }

    /// Test-only helper to set the block-level text selection, which is normally
    /// written by the `SelectableArea` selection callback during a drag.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn set_block_level_selected_text_for_test(&self, text: Option<String>) {
        *self.selected_text.write() = text;
    }

    /// Test-only helper that simulates a block-level text selection in this AI
    /// block: it writes the selected text and emits the same
    /// [`AIBlockEvent::SelectionChanged`] signal that
    /// [`AIBlockAction::SelectText`] does, so the terminal view mirrors it into
    /// the model's rich content selection (which the copy/insert paths read).
    ///
    /// This lets integration tests exercise the in-AI-block copy path without
    /// depending on layout-sensitive pixel coordinates.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn simulate_text_selection_for_test(
        &mut self,
        text: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        *self.selected_text.write() = text;
        ctx.emit(AIBlockEvent::SelectionChanged);
    }

    /// Start a selection at the top left corner of the block's SelectableArea.
    pub fn start_selection_at_min_point(&self, selection_type: SelectionType, x_pos: Option<f32>) {
        self.state_handles.selection_handle.start_selection_outside(
            match x_pos {
                Some(x_bound) => SelectionBound::Top { x_bound },
                None => SelectionBound::TopLeft,
            },
            selection_type,
        )
    }

    /// Start a selection at the bottom right corner of the block's SelectableArea.
    pub fn start_selection_at_max_point(&self, selection_type: SelectionType, x_pos: Option<f32>) {
        self.state_handles.selection_handle.start_selection_outside(
            match x_pos {
                Some(x_bound) => SelectionBound::Bottom { x_bound },
                None => SelectionBound::BottomRight,
            },
            selection_type,
        )
    }

    /// Clears all text selections in all components within this `AIBlock`'s view sub-hierarchy.
    /// This includes the `AIBlock` level and all child views (code blocks, etc.).
    pub fn clear_all_selections(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_other_selections(None, ctx.window_id(), ctx);
        self.clear_block_level_selection();
    }

    /// Clears text selections at the `AIBlock` level (e.g. selected reasoning paragraph text).
    /// This does _not_ clear the selection of the child views (code blocks, etc.)!
    fn clear_block_level_selection(&mut self) {
        self.state_handles.selection_handle.clear();
        *self.selected_text.write() = None;
    }

    /// Clears all text selections in all components within this `AIBlock`'s view sub-hierarchy
    /// _other_ than the one that triggered a selection change.
    ///
    /// Call this after text is selected in one part of the AI block (e.g. a code diff), to ensure
    /// that there's only one active selection at a time.
    fn clear_other_selections(
        &mut self,
        source_view_id: Option<EntityId>,
        source_window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        if source_window_id != ctx.window_id() {
            return;
        }

        for editor_view in self.code_editor_views.iter() {
            // Don't clear selections for the view that triggered this change.
            if source_view_id.is_some_and(|entity_id| editor_view.view.id() == entity_id)
                && editor_view.view.window_id(ctx) == source_window_id
            {
                continue;
            }
            editor_view
                .view
                .update(ctx, |view, ctx| view.clear_selection(ctx));
        }

        for command in self.requested_commands.values() {
            // Don't clear selections for the requested command view that triggered this change.
            if source_view_id.is_some_and(|entity_id| command.view.id() == entity_id)
                && command.view.window_id(ctx) == source_window_id
            {
                continue;
            }
            command
                .view
                .update(ctx, |view, ctx| view.clear_selection(ctx));
        }

        for mcp_tool in self.requested_mcp_tools.values() {
            // Don't clear selections for the MCP tool view that triggered this change.
            if source_view_id.is_some_and(|entity_id| mcp_tool.view.id() == entity_id)
                && mcp_tool.view.window_id(ctx) == source_window_id
            {
                continue;
            }
            mcp_tool
                .view
                .update(ctx, |view, ctx| view.clear_selection(ctx));
        }

        for diff in self.requested_edits.values() {
            // Don't clear selections for the diff view that triggered this change.
            if source_view_id.is_some_and(|entity_id| diff.view.id() == entity_id)
                && diff.view.window_id(ctx) == source_window_id
            {
                continue;
            }
            diff.view
                .update(ctx, |view, ctx| view.clear_all_selections(ctx));
        }

        for search_view in self.search_codebase_view.values() {
            // Don't clear selections for the search codebase view that triggered this change.
            if source_view_id.is_some_and(|entity_id| search_view.id() == entity_id)
                && search_view.window_id(ctx) == source_window_id
            {
                continue;
            }
            search_view.update(ctx, |view, ctx| view.clear_selection(ctx));
        }

        for comment in self.comment_states.values() {
            if source_view_id.is_some_and(|entity_id| comment.rich_text_editor.id() == entity_id)
                && comment.rich_text_editor.window_id(ctx) == source_window_id
            {
                continue;
            }
            comment
                .rich_text_editor
                .update(ctx, |view, ctx| view.clear_text_selection(ctx));
        }

        // If the event was dispatched by a nested view (i.e. code block, requested command, etc.),
        // clear the text selection at the `AIBlock` level (outside the code block).
        // We want to have only 1 selection active at any one point in time.
        if source_view_id.is_some() {
            self.clear_block_level_selection();
        }
    }

    pub fn dismiss_ai_tooltips(&mut self, ctx: &mut ViewContext<Self>) {
        self.detected_links_state.link_location_open_tooltip = None;
        ctx.emit(AIBlockEvent::DismissLinkTooltip);
        self.secret_redaction_state.dismiss_tooltip();
        ctx.emit(AIBlockEvent::DismissSecretTooltip);
        for search_view in self.search_codebase_view.values() {
            search_view.update(ctx, |view, ctx| {
                view.clear_link_tooltip(ctx);
            });
        }

        // The hover state for the "open" button in linked code blocks should be reset on a focus change.
        for button_handles in self
            .state_handles
            .normal_response_code_snippet_buttons
            .iter()
        {
            button_handles.reset_hover_state_on_focus_change();
        }

        ctx.notify();
    }

    fn open_link(
        &self,
        location: &TextLocation,
        link_range: &Range<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        match self.detected_links_state.link_at(location, link_range) {
            Some(DetectedLinkType::Url(link)) => {
                ctx.open_url(link);
            }
            #[cfg(feature = "local_fs")]
            Some(DetectedLinkType::FilePath {
                absolute_path,
                line_and_column_num,
            }) => ctx.emit(AIBlockEvent::OpenDetectedFilePath {
                absolute_path: absolute_path.clone(),
                line_and_column_num: *line_and_column_num,
                target_override: self.detected_file_path_target_override(absolute_path),
            }),
            None => (),
        }
    }

    fn show_link_tooltip(
        &mut self,
        location: &TextLocation,
        link_range: &Range<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(link_type) = self.detected_links_state.link_at(location, link_range) else {
            return;
        };
        let rich_content_link = match link_type {
            DetectedLinkType::Url(link) => RichContentLink::Url(link.clone()),
            #[cfg(feature = "local_fs")]
            DetectedLinkType::FilePath {
                absolute_path,
                line_and_column_num,
            } => RichContentLink::FilePath {
                absolute_path: absolute_path.to_owned(),
                line_and_column_num: *line_and_column_num,
                target_override: self.detected_file_path_target_override(absolute_path),
            },
        };
        self.detected_links_state.link_location_open_tooltip = Some(LinkLocation {
            link_range: link_range.clone(),
            location: *location,
        });
        ctx.emit(AIBlockEvent::ShowLinkTooltip(RichContentLinkTooltipInfo {
            link: rich_content_link,
            position_id: RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID.to_owned(),
        }));
    }

    fn show_secret_tooltip(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(hoverable_secret) = self
            .secret_redaction_state
            .show_secret_tooltip(location, secret_range)
        {
            ctx.emit(AIBlockEvent::ShowSecretTooltip(
                RichContentSecretTooltipInfo {
                    secret: hoverable_secret.secret.clone(),
                    is_obfuscated: hoverable_secret.is_obfuscated,
                    position_id: RICH_CONTENT_SECRET_FIRST_CHAR_POSITION_ID.to_owned(),
                    secret_range: secret_range.clone(),
                    location: *location,
                    view_id: ctx.view_id(),
                    secret_level: hoverable_secret.secret_level,
                },
            ));
        }
    }

    pub fn set_secret_redaction_state(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
        is_obfuscated: bool,
    ) {
        self.secret_redaction_state
            .set_obfuscated(location, secret_range, is_obfuscated);
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
                ctx.notify();
            }
        }
    }

    pub fn on_requested_command_execution_started(
        &mut self,
        action_id: AIAgentActionId,
        ctx: &mut ViewContext<Self>,
    ) {
        // After executing a requested command, auto expand it after some delay (for long-running commands).
        self.auto_expand_requested_command_timer_handle = Some(ctx.spawn(
            async move {
                Timer::after(AUTO_EXPAND_REQUESTED_COMMAND_DELAY).await;
                action_id
            },
            |me, requested_command_id, ctx| {
                me.auto_expand_requested_command_timer_handle = None;

                // Avoid auto-expanding while voice input is active.
                let voice_active = {
                    #[cfg(feature = "voice_input")]
                    {
                        voice_input::VoiceInput::as_ref(ctx).is_active()
                    }
                    #[cfg(not(feature = "voice_input"))]
                    {
                        false
                    }
                };

                // If user has typed since the last submit, do not auto-expand while they are editing.
                if me.terminal_model.lock().is_input_dirty() || voice_active {
                    return;
                }

                // We should only attempt to auto-expand the requested command if it is still running.
                let is_command_still_active = {
                    let terminal_model = me.terminal_model.lock();
                    let active_block = terminal_model.block_list().active_block();
                    active_block.is_active_and_long_running()
                        && active_block
                            .agent_interaction_metadata()
                            .is_some_and(|metadata| {
                                metadata
                                    .requested_command_action_id()
                                    .is_some_and(|id| id == &requested_command_id)
                            })
                };
                if is_command_still_active {
                    me.expand_requested_command_view(&requested_command_id, ctx);
                    // Mark this command to be auto-collapsed when the command execution is complete.
                    me.requested_commands_to_auto_collapse
                        .insert(requested_command_id);
                }
            },
        ));
    }

    pub fn expand_requested_command_view(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Auto expansion timers only apply to requested commands, as requested actions that
        // don't lean on Views (i.e. file retrieval, grep, MCP, etc.) are non-expandable.
        let Some(requested_command) = self.requested_commands.get(action_id) else {
            return;
        };
        requested_command.force_expand(ctx);
        ctx.notify();
    }

    fn collapse_requested_command_view(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(requested_command) = self.requested_commands.get(action_id) else {
            return;
        };
        requested_command.force_collapse(ctx);
        ctx.notify();
    }

    /// Terminates any active requested command auto-expansion timer.
    fn abort_auto_expand_requested_command_timer(&mut self) {
        if let Some(auto_expand_requested_command_timer_handle) =
            self.auto_expand_requested_command_timer_handle.take()
        {
            auto_expand_requested_command_timer_handle.abort();
        }
    }

    /// Returns the document that the requested command was copied from, if any.
    pub fn requested_command_copied_from_doc(
        &self,
        action_id: &AIAgentActionId,
        ctx: &ViewContext<TerminalView>,
    ) -> Option<AIAgentCitation> {
        let requested_command = self.requested_commands.get(action_id)?;
        let requested_command_view = requested_command.view.as_ref(ctx);
        requested_command_view.copied_from_citation().cloned()
    }

    pub fn handle_passive_code_diff_action(
        &mut self,
        action: CodeDiffAction,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(edit) = self.find_undismissed_code_diff(ctx) else {
            return false;
        };
        edit.view.update(ctx, |view, ctx| match action {
            CodeDiffAction::Accept => view.try_accept_action(ctx),
            CodeDiffAction::Reject => view.reject(ctx),
            CodeDiffAction::Edit => view.expand_and_edit(ctx),
            CodeDiffAction::ScrollToExpand => view.expand_inline_banner(ctx),
        });
        ctx.notify();
        true
    }

    /// Marks all pending passive actions (code diffs and suggested prompts) as dismissed/ignored.
    /// This hides their keybindings in the UI and makes them less interactive.
    pub fn ignore_passive_actions(&mut self, ctx: &mut ViewContext<Self>) {
        self.action_model.update(ctx, |action_model, ctx| {
            for action in action_model.get_pending_actions() {
                if let Some(edit) = self.requested_edits.get(&action.id) {
                    edit.view.update(ctx, |view, ctx| view.dismiss(ctx));
                } else if let Some(suggested_prompt) = self.unit_tests_suggestions.get(&action.id) {
                    suggested_prompt.update(ctx, |view, ctx| view.hide_keybindings(ctx));
                }
            }
        });
    }

    fn pending_requested_edit(&self, app: &AppContext) -> Option<&RequestedEdit> {
        self.action_model
            .as_ref(app)
            .get_pending_action(app)
            .and_then(|action| self.requested_edits.get(&action.id))
    }

    /// Accepts the latest pending (blocked) action, if any.
    /// Includes code diffs, requested commands, and MCP tool calls.
    pub fn accept_pending_action(&mut self, ctx: &mut ViewContext<Self>) {
        self.accept_pending_requested_edit(ctx);
        self.accept_pending_requested_command(ctx);
        self.accept_pending_requested_mcp_tool(ctx);
    }

    /// Accepts the latest pending (blocked) requested code diff, if any.
    fn accept_pending_requested_edit(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(edit) = self.pending_requested_edit(ctx) {
            edit.view
                .update(ctx, |view, ctx| view.try_accept_action(ctx));
            ctx.notify();
        }
    }

    fn has_pending_requested_edit(&self, app: &AppContext) -> bool {
        self.pending_requested_edit(app).is_some()
    }

    /// Accepts the latest pending (blocked) requested command, if any.
    fn accept_pending_requested_command(&mut self, ctx: &mut ViewContext<Self>) {
        let pending_action_id = {
            self.action_model
                .as_ref(ctx)
                .get_pending_action(ctx)
                .map(|a| a.id.clone())
        };

        if let Some(action_id) = pending_action_id {
            if let Some(requested_command) = self.requested_commands.get(&action_id) {
                let command_text = requested_command
                    .view
                    .update(ctx, |view, ctx| view.commit_and_get_command_text(ctx));
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.handle_requested_command_accepted(&action_id, command_text, ctx);
                });
                ctx.notify();
            }
        }
    }
    /// Accepts the latest pending (blocked) requested MCP tool call, if any.
    fn accept_pending_requested_mcp_tool(&mut self, ctx: &mut ViewContext<Self>) {
        let pending_action_id = {
            self.action_model
                .as_ref(ctx)
                .get_pending_action(ctx)
                .map(|a| a.id.clone())
        };

        if let Some(action_id) = pending_action_id {
            if self.requested_mcp_tools.contains_key(&action_id) {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.execute_action(&action_id, self.client_ids.conversation_id, ctx);
                });
                ctx.notify();
            }
        }
    }

    /// Finds the undismissed passive code diff across all pending actions.
    /// This is needed because passive code diffs are NOT added to the active conversation by default, when they first appear.
    pub(crate) fn find_undismissed_code_diff(&self, app: &AppContext) -> Option<&RequestedEdit> {
        let all_pending_actions = self.action_model.as_ref(app).get_pending_actions();

        // Find any RequestFileEdits action that has a corresponding passive code diff view.
        // Note that we only expect a maximum of 1 passive code diff to be undismissed at any given time.
        all_pending_actions
            .iter()
            .find_map(|action| match &action.action {
                AIAgentActionType::RequestFileEdits {
                    file_edits: _,
                    title: _,
                } => self.requested_edits.get(&action.id).and_then(|edit| {
                    let view = edit.view.as_ref(app);
                    if view.is_passive() && !view.is_inline_banner_dismissed() {
                        Some(edit)
                    } else {
                        None
                    }
                }),
                _ => None,
            })
    }

    pub fn pending_unit_test_suggestion(
        &self,
        app: &AppContext,
    ) -> Option<&ViewHandle<SuggestedUnitTestsView>> {
        self.unit_tests_suggestions
            .values()
            .find(|view| !view.as_ref(app).is_hidden())
    }

    /// Inspects the state of the AI output stream and determines if we are currently at a point where
    /// we should render the floating AI control panel. This is purely a UX decision.
    pub fn should_show_ai_control_panel(&self, app: &AppContext) -> bool {
        // Returns true if we're not blocked on user input at this moment.
        // A stream of regular text output is not blocking. Generating a command is not blocking. Generating a code
        // diff is not blocking. In those cases, we return true. Waiting for a user to accept a requested command
        // is blocking. So is waiting for them to accept a file read. In those cases, we return false since the action
        // takes responsiblity for showing a cancel option.
        self.model
            .status(app)
            .output_to_render()
            .is_none_or(|output| {
                output.get().actions().last().is_none_or(|action| {
                    let is_streaming = self.model.status(app).is_streaming();
                    let status = self.action_model.as_ref(app).get_action_status(&action.id);
                    is_streaming || status.is_some_and(|status| status.is_running())
                })
            })
    }

    pub fn saved_position_id(&self) -> String {
        get_rich_content_position_id(&self.view_id)
    }

    pub fn get_pending_action_type(&self, app: &AppContext) -> Option<AIAgentActionType> {
        self.action_model
            .as_ref(app)
            .get_pending_action(app)
            .map(|action| action.action.clone())
    }

    pub fn status(&self, app: &AppContext) -> AIBlockOutputStatus {
        self.model.status(app)
    }

    pub fn has_expanded_running_commands(&self, app: &AppContext) -> bool {
        self.requested_commands
            .iter()
            .any(|(action_id, requested_command)| {
                self.action_model
                    .as_ref(app)
                    .get_action_status(action_id)
                    .is_some_and(|status| status.is_running())
                    && requested_command.view.as_ref(app).is_header_expanded()
            })
    }

    pub fn output_model_display_name(&self, app: &AppContext) -> String {
        let Some(base_model_id) = self.model.base_model(app) else {
            log::warn!("No base model found for output model display name");
            return String::default();
        };

        // Get the model name from the input metadata.
        let mut model_name = LLMPreferences::as_ref(app)
            .get_llm_info(base_model_id)
            .map(|info| info.display_name.clone())
            .unwrap_or_default();

        // If the input model is "auto", always display that, otherwise use the actual output model if available.
        if model_name != "auto" {
            let model_id = self.model.model_id(app);
            if let Some(model_id) = model_id {
                if let Some(output_model_name) = LLMPreferences::as_ref(app)
                    .get_llm_info(&model_id)
                    .map(|info| info.display_name.clone())
                {
                    model_name = output_model_name;
                }
            }
        }
        model_name
    }

    #[cfg(feature = "agent_mode_debug")]
    pub fn set_diffs(&self, diffs: Vec<FileDiff>, ctx: &mut ViewContext<Self>) {
        if let Some(edit) = self.requested_edits.values().next() {
            edit.view.update(ctx, |view, ctx| {
                view.set_candidate_diffs(diffs, ctx);
            });
        }
    }

    /// Gets the prompt text for copying
    pub fn get_prompt_text(&self, app: &AppContext) -> String {
        self.model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Gets the prompt text from the preceding user query (where the overflow menu would appear)
    pub fn get_preceding_user_query(&self, app: &AppContext) -> String {
        let history = BlocklistAIHistoryModel::as_ref(app);
        let current_exchange_id = self.model.exchange_id(app);

        let Some(exchange_id) = current_exchange_id else {
            return self.get_prompt_text(app);
        };

        let Some(conversation_id) =
            history.conversation_id_for_exchange(exchange_id, self.terminal_view_id)
        else {
            return self.get_prompt_text(app);
        };

        let Some(conversation) = history.conversation(&conversation_id) else {
            return self.get_prompt_text(app);
        };

        let exchanges: Vec<_> = conversation.root_task_exchanges().collect();

        // Find the current exchange index
        let current_index = exchanges
            .iter()
            .position(|exchange| exchange.id == exchange_id);

        let Some(current_idx) = current_index else {
            return self.get_prompt_text(app);
        };

        // Find the preceding user query (where the overflow menu would be)
        for i in (0..=current_idx).rev() {
            let formatted_input = exchanges[i].format_input_for_copy();
            if !formatted_input.is_empty() {
                return formatted_input;
            }
        }

        // Fallback to current block's prompt
        self.get_prompt_text(app)
    }

    /// Gets the AI output text for copying
    pub fn get_output_text(&self, app: &AppContext) -> String {
        let Some(output) = self.model.status(app).output_to_render() else {
            return String::new();
        };
        let output = output.get();
        output.format_for_copy(Some(self.action_model.as_ref(app)))
    }

    /// Gets AI output text for copying from the preceding user query until the next user query
    /// This copies all AI outputs that would be contained within one "overflow menu scope"
    pub fn get_output_text_since_preceding_user_query(&self, app: &AppContext) -> String {
        let history = BlocklistAIHistoryModel::as_ref(app);
        let current_exchange_id = self.model.exchange_id(app);

        let Some(exchange_id) = current_exchange_id else {
            return self.get_output_text(app);
        };

        let Some(conversation_id) =
            history.conversation_id_for_exchange(exchange_id, self.terminal_view_id)
        else {
            return self.get_output_text(app);
        };

        let Some(conversation) = history.conversation(&conversation_id) else {
            return self.get_output_text(app);
        };

        let exchanges: Vec<_> = conversation.root_task_exchanges().collect();

        // Find the current exchange index
        let current_index = exchanges
            .iter()
            .position(|exchange| exchange.id == exchange_id);

        let Some(current_idx) = current_index else {
            return self.get_output_text(app);
        };

        // Find the preceding user query (where the overflow menu would be)
        let mut start_idx = current_idx;
        for i in (0..=current_idx).rev() {
            if exchanges[i].has_user_query() {
                start_idx = i;
                break;
            }
        }

        // Find the next user query (where the next overflow menu would be)
        let mut end_idx = exchanges.len();
        for (i, exchange) in exchanges.iter().enumerate().skip(start_idx + 1) {
            if exchange.has_user_query() {
                end_idx = i;
                break;
            }
        }

        // Collect all AI outputs from start_idx to end_idx (exclusive)
        let mut combined_result = Vec::new();
        for exchange in exchanges.iter().take(end_idx).skip(start_idx) {
            let formatted_output =
                exchange.format_output_for_copy(Some(self.action_model.as_ref(app)));
            if !formatted_output.is_empty() {
                combined_result.push(formatted_output);
            }
        }

        // Not expected to be needed, but get the output of just this block as a fallback.
        if combined_result.is_empty() {
            self.get_output_text(app)
        } else {
            combined_result.join("\n\n") // Separate different exchanges with double newlines
        }
    }

    fn has_accepted_file_edits_since_last_query(&self, app: &AppContext) -> bool {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(app).conversation(&self.conversation_id())
        else {
            return false;
        };

        // Check any finished actions, from the most recent AI output, for accepted file edits.
        if let Some(finished_action_results) = self
            .action_model
            .as_ref(app)
            .get_finished_action_results(conversation.id())
        {
            if finished_action_results.iter().any(|result| {
                matches!(
                    result.result,
                    AIAgentActionResultType::RequestFileEdits(
                        RequestFileEditsResult::Success { .. }
                    )
                )
            }) {
                return true;
            }
        }

        // Otherwise, we also check all past exchanges since the last user query for accepted file edits.
        conversation
            .exchanges_reversed()
            .take_while(|exchange| !exchange.has_user_query())
            .any(|exchange| exchange.has_accepted_file_edit())
    }

    pub fn num_requested_commands(&self) -> usize {
        self.requested_commands.len()
    }

    pub fn requested_commands_iter(
        &self,
    ) -> impl Iterator<Item = (&AIAgentActionId, &RequestedCommand)> {
        self.requested_commands.iter()
    }

    /// Collects all imported review comments stored in this block
    /// and the base branch (if any). Returns `None` when this block has no imported comments.
    pub(crate) fn collect_imported_comments(&self) -> Option<ImportedBlockComments> {
        if !self.has_imported_comments {
            return None;
        }
        let mut comments = Vec::new();
        let mut base_branch = None;

        for group in self.imported_comments.values() {
            if let Some(group_base_branch) = &group.base_branch {
                if base_branch.is_none() {
                    base_branch = Some(group_base_branch.clone());
                } else if base_branch.as_ref() != Some(group_base_branch) {
                    log::warn!(
                        "Encountered multiple base branches while opening imported review comments from a single AI block",
                    );
                }
            }

            comments.extend(group.cards.iter().map(|card| card.source().clone()));
        }

        if comments.is_empty() {
            return None;
        }

        Some(ImportedBlockComments {
            comments,
            base_branch,
        })
    }

    /// Returns `true` if this block has any imported review comments.
    pub(crate) fn has_any_imported_comments(&self) -> bool {
        self.has_imported_comments
    }

    /// Returns `true` if the current working directory is within any of this block's
    /// imported comment group repo roots.
    fn cwd_matches_any_imported_comment_repo(&self, cwd: &LocalOrRemotePath) -> bool {
        self.imported_comments
            .values()
            .any(|group| group.repo_path.strip_repo_prefix(cwd).is_some())
    }

    /// Returns the repo path associated with this block's imported comments, if any.
    ///
    /// All imported comment groups in a single block share the same repo
    /// (they were fetched in the same terminal context), so any group's
    /// path is representative.
    pub(crate) fn imported_comment_repo_path(&self) -> Option<&LocalOrRemotePath> {
        self.imported_comments
            .values()
            .next()
            .map(|group| &group.repo_path)
    }

    fn current_working_directory_location(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<LocalOrRemotePath> {
        self.active_session
            .as_ref(ctx)
            .current_working_directory_location(ctx)
    }

    /// Disables or enables the per-comment "Open in code review" buttons and the
    /// bulk "Open all in code review" button based on whether the current working
    /// directory is still within the imported comments' repository.
    fn update_imported_comments_disabled_state(&mut self, ctx: &mut ViewContext<Self>) {
        if self.has_imported_comments {
            let cwd_location = self.current_working_directory_location(ctx);
            self.update_own_imported_comments_disabled_state(cwd_location.as_ref(), ctx);
        } else if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation_has_imported_comments(&self.client_ids.conversation_id)
            && self.model.is_latest_visible_exchange_in_root_task(ctx)
        {
            // The "Open all" button is rendered by the latest visible exchange when the
            // current thread has imported comments but this block does not own them directly.
            // Update that block's button state from its CWD so the button disables when the
            // user navigates outside the imported comments' repository.
            let cwd_location = self.current_working_directory_location(ctx);
            self.update_open_all_button_disabled_state(cwd_location.as_ref(), ctx);
        } else {
            return;
        }

        ctx.notify();
    }

    /// Updates the per-comment and "Open all" buttons for a block that owns
    /// imported comments. We assume all comment groups share the same repo.
    fn update_own_imported_comments_disabled_state(
        &mut self,
        cwd_location: Option<&LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        let cwd_matches_repo =
            cwd_location.is_some_and(|cwd| self.cwd_matches_any_imported_comment_repo(cwd));
        let should_disable = !cwd_matches_repo;

        for group in self.imported_comments.values() {
            group.set_buttons_disabled(should_disable, ctx);
        }

        let repo_path = if should_disable {
            self.imported_comment_repo_path().cloned()
        } else {
            None
        };
        set_imported_comment_button_disabled(
            &self.open_all_comments_button,
            should_disable,
            repo_path.as_ref(),
            ctx,
        );
    }

    /// Updates the "Open all" button for a block that has no imported comments
    /// of its own but is the latest exchange (and therefore renders the button).
    /// Derives the repo root from the block's CWD via `DetectedRepositories`.
    fn update_open_all_button_disabled_state(
        &self,
        cwd_location: Option<&LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        let repo_path =
            cwd_location.and_then(|cwd| DetectedRepositories::as_ref(ctx).get_root_for_path(cwd));
        #[cfg(target_family = "wasm")]
        let repo_path = cwd_location.cloned();

        let cwd_matches_repo = match (cwd_location, repo_path.as_ref()) {
            (Some(cwd), Some(rp)) => rp.strip_repo_prefix(cwd).is_some(),
            _ => false,
        };

        set_imported_comment_button_disabled(
            &self.open_all_comments_button,
            !cwd_matches_repo,
            repo_path.as_ref(),
            ctx,
        );
    }
    /// A "thread" covers all exchanges since and including the most recent user query. This
    /// method checks across all AI blocks for this conversation — not just this block — by
    /// querying the `TerminalView`, which in turn calls `has_any_imported_comments` on each
    /// block.
    pub(crate) fn has_imported_comments_in_current_thread(&self, app: &AppContext) -> bool {
        self.terminal_view_handle
            .upgrade(app)
            .is_some_and(|terminal_view_handle| {
                terminal_view_handle
                    .as_ref(app)
                    .has_imported_comments_in_thread(&self.client_ids.conversation_id, app)
            })
    }
}

pub(crate) struct ImportedBlockComments {
    pub(crate) comments: Vec<AttachedReviewComment>,
    pub(crate) base_branch: Option<String>,
}

fn set_imported_comment_button_disabled(
    handle: &ViewHandle<ActionButton>,
    should_disable: bool,
    repo_path: Option<&LocalOrRemotePath>,
    ctx: &mut ViewContext<AIBlock>,
) {
    handle.update(ctx, |button, ctx| {
        button.set_disabled(should_disable, ctx);
        if should_disable {
            let tooltip = repo_path
                .map(|path| format!("Navigate to {} to open these comments", path.display_path()));
            button.set_tooltip(tooltip, ctx);
        } else {
            button.set_tooltip(None::<String>, ctx);
        }
    });
}

fn num_attached_context_blocks(inputs: &[AIAgentInput]) -> usize {
    inputs.iter().fold(0, |count, input| {
        if let Some(context) = input.context() {
            count
                + context
                    .iter()
                    .filter(|context| matches!(context, AIAgentContext::Block { .. }))
                    .count()
        } else {
            count
        }
    })
}

fn has_attached_context_selected_text(inputs: &[AIAgentInput]) -> bool {
    inputs.iter().any(|input| {
        input.context().is_some_and(|context| {
            context
                .iter()
                .any(|context| matches!(context, AIAgentContext::SelectedText(_)))
        })
    })
}

pub(super) fn attachment_names(inputs: &[AIAgentInput]) -> Vec<(AttachmentType, String)> {
    let mut names: Vec<(AttachmentType, String)> = inputs
        .iter()
        .filter_map(|input| {
            input.context().map(|contexts| {
                contexts.iter().filter_map(|context| match context {
                    AIAgentContext::Image(image) => {
                        Some((AttachmentType::Image, image.file_name.clone()))
                    }
                    _ => None,
                })
            })
        })
        .flatten()
        .collect_vec();

    // Also collect file names from FilePathReference attachments.
    // Use the map key (clean filename) rather than the file_name field
    // (which may contain a UUID prefix from the VM filesystem path).
    for input in inputs {
        if let AIAgentInput::UserQuery {
            referenced_attachments,
            ..
        } = input
        {
            for (key, attachment) in referenced_attachments {
                if matches!(attachment, AIAgentAttachment::FilePathReference { .. }) {
                    names.push((AttachmentType::File, key.clone()));
                }
            }
        }
    }

    names
}

#[derive(Clone, Debug)]
pub enum AIBlockEvent {
    /// Emitted when the AI block is "finished", meaning it will no longer receive any more output
    /// (either the output is complete or the request was cancelled) and it is no longer
    /// interactable (if any requested commands or requested actions were included, all requested
    /// commands and requested actions have been executed or cancelled).
    Finished,

    /// Emitted when we want to show or hide the usage footer.
    UsageFooterToggled {
        conversation_id: AIConversationId,
        is_expanded: bool,
    },

    /// Emitted when the AI block requires user confirmation to execute.
    ActionBlockedOnUserConfirmation,

    /// Emitted when the visibility of the command block is toggled for a requested action or a
    /// requested command. This covers both [`View`] and non-[`View`] inline action components.
    UpdateInlineActionVisibility {
        action_id: AIAgentActionId,
        is_visible: bool,
    },
    ToggleCodeDiffVisibility,

    /// Open a Warp Text instance with the requested code diff.
    OpenCodeWithDiff {
        view: ViewHandle<CodeDiffView>,
    },

    #[cfg(feature = "local_fs")]
    OpenDetectedFilePath {
        absolute_path: PathBuf,
        line_and_column_num: Option<warp_util::path::LineAndColumnArg>,
        target_override: Option<FileTarget>,
    },
    ShowLinkTooltip(RichContentLinkTooltipInfo),
    DismissLinkTooltip,
    ShowSecretTooltip(RichContentSecretTooltipInfo),
    DismissSecretTooltip,
    OpenCitation(AIAgentCitation),
    OpenAIFactCollection {
        /// If set, open the fact collection to the specific rule.
        sync_id: Option<SyncId>,
    },
    OpenWorkflow {
        sync_id: SyncId,
    },
    /// Emitted when the continue conversation button is clicked
    ContinueConversation {
        conversation_id: AIConversationId,
    },
    /// Emitted when a passive code diff should be injected into an agent context.
    ContinuePassiveCodeDiffWithAgent {
        conversation_id: AIConversationId,
        /// If the auto code diff was generated as a result of a block trigger,
        /// this is the ID of that block.
        trigger_block_id: Option<BlockId>,
        auto_resume: bool,
    },
    OpenSuggestedAgentModeWorkflowModal {
        workflow_and_id: SuggestedAgentModeWorkflowAndId,
    },
    OpenSuggestedRuleDialog {
        rule_and_id: SuggestedRuleAndId,
    },
    FocusTerminal,
    OpenThemeChooser,
    #[cfg(windows)]
    WindowsCtrlC,
    AIOutputUpdated,
    ActionFinished,
    /// Emitted when text is selected within any `ChildView` of an `AIBlock`. This includes
    /// selections made within inline action views (such as requested edits, embedded code blocks,
    /// etc.) but not selections at the `AIBlock` level itself. Providing this distinction is
    /// important because selecting across multiple blocks only supports text selections at the
    /// `AIBlock` level.
    ChildViewTextSelected,
    /// Emitted when the `AIBlock`'s own (block-level) text selection state may
    /// have changed. The terminal view uses this to keep the model's record of
    /// which rich content block has an active selection in sync, so copy/insert
    /// paths can find the selected text. Rich content selections are not tied to
    /// the point-based model selection, so this signal is required.
    SelectionChanged,
    CopiedEmptyText,
    OpenSettings,
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        source: CodeSource,
        layout: crate::util::file::external_editor::settings::EditorLayout,
    },
    /// Emitted when the resume conversation button is clicked
    ResumeConversation {
        conversation_id: AIConversationId,
    },
    InsertForkSlashCommand,
    ToggleCodeReviewPane {
        entrypoint: CodeReviewPaneEntrypoint,
    },
    DismissedPassiveBlock,
    OpenAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        is_auto_open: bool,
    },
    OpenActiveAgentProfileEditor,
    /// Run the configured AWS auth refresh command to fix expired Bedrock credentials
    RunAwsLoginCommand,
    /// Emitted when a passive code diff has loaded its diffs and is ready to display.
    /// This is used to trigger height recalculation since the diffs are loaded asynchronously
    /// after the initial output completes.
    PassiveCodeDiffLoaded,
    OpenImportedCommentInCodeReview {
        repo_path: LocalOrRemotePath,
        comment: Box<AttachedReviewComment>,
        base_branch: Option<String>,
    },
    /// Emitted when the "Open all in code review" button is clicked on a block that does not
    /// itself hold imported comments. The terminal view handles this by collecting imported
    /// comments from all blocks belonging to the same conversation's current thread.
    OpenAllImportedCommentsForConversation {
        conversation_id: AIConversationId,
    },
}

impl Entity for AIBlock {
    type Event = AIBlockEvent;
}

/// User's final response to an AI-suggested code edit.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum RequestedEditResolution {
    Accept,
    Reject,
}

#[derive(Debug, Clone)]
pub enum AIBlockAction {
    /// Only applies to text selections made at the `AIBlock` level. Child views of the `AIBlock`
    /// are responsible for managing their own text selection states.
    SelectText,

    CopyAIBlockCodeSnippet(String),

    /// Continue the conversation using this response
    ContinueConversation,

    /// Resume the stopped conversation
    ResumeConversation,

    /// Fork the conversation
    ForkConversation,

    /// Manually cancel sending an AI request or streaming an AI response for a requested action.
    /// View-based inline actions (`RequestedCommandView`, etc.) should be handling AI block
    /// cancellation via their own View events.
    CancelRequestedAction {
        action_id: AIAgentActionId,
    },

    /// Executes the requested action with the given ID. View-based inline actions (i.e. `PlanView`,
    /// `RequestedCommandView`, etc.) should be handling AI block execution via their own View events.
    ExecuteRequestedAction {
        action_id: AIAgentActionId,
    },

    /// Execute the next pending action, if any.
    ExecuteNextPendingAction,

    ChangedHoverOnLink {
        link_range: Range<usize>,
        location: TextLocation,
        is_hovering: bool,
    },
    OpenLink {
        link_range: Range<usize>,
        location: TextLocation,
    },
    ChangedHoverOnSecret {
        secret_range: SecretRange,
        location: TextLocation,
        is_hovering: bool,
    },
    OpenLinkTooltip {
        link_range: Range<usize>,
        location: TextLocation,
    },
    OpenSecretTooltip {
        secret_range: SecretRange,
        location: TextLocation,
    },
    OpenCitation(AIAgentCitation),
    OpenAIFactCollection,
    ToggleReferencesSection,
    ToggleAutoexecuteReadonlyCommandsSpeedbumpCheckbox,
    ToggleAutoreadFilesSpeedbumpCheckbox,
    ToggleAwsBedrockAutoLogin,
    ToggleCodebaseSearchSpeedbump(Option<usize>),
    StartNewConversationButtonClicked {
        action_id: AIAgentActionId,
        server_output_id: Option<ServerOutputId>,
    },
    ContinueCurrentConversationButtonClicked {
        action_id: AIAgentActionId,
        server_output_id: Option<ServerOutputId>,
    },
    Rated {
        is_positive: bool,
    },
    /// Clear the selections of all other views **except** for the source view that dispatched the event.
    /// The `source_view_id` will be `None` if the event is dispatched by the [`warpui::elements::SelectableArea`]
    /// instead of a nested view (i.e. code block, requested command, etc.), which means all nested views
    /// should have their selections cleared.
    ClearOtherSelections {
        source_view_id: Option<EntityId>,
        source_window_id: WindowId,
    },
    /// Copy both query and AI output (see below)
    Copy,
    /// Copy the content from the previous user query.
    /// Note that this block may not have the user query.
    CopyQuery,
    /// Copy all AI output from the previous user query to the next user query.
    /// Note that this contains more than just this block, since from the user perspective everything after the user query appears like one block.
    CopyOutput,
    /// Copy the ai block's command
    CopyCommand,
    /// Store a command that was right-clicked for later copying
    StoreRightClickedCommand {
        command: String,
    },
    OpenCodeInWarp {
        source: CodeSource,
    },
    ToggleTodoListExpanded(MessageId),
    ToggleCollapsibleBlockExpanded(MessageId),
    SetCollapsibleBlockPinnedToBottom {
        message_id: MessageId,
        pinned_to_bottom: bool,
    },
    ToggleCodeReviewPane,
    DismissSuggestionsSection,
    DisableRuleSuggestions,
    /// Copy the debug ID to clipboard
    CopyDebugId(String),
    /// Open Warp feedback documentation
    OpenFeedbackDocs,
    /// Toggle the usage summary footer expansion state
    ToggleIsUsageFooterExpanded,
    CommentExpanded {
        id: CommentId,
    },
    /// Run the configured AWS auth refresh command to fix expired Bedrock credentials
    RunAwsLoginCommand,
    /// Open settings to configure the AWS auth refresh command
    ConfigureAwsLoginCommand,
    /// Open the screenshot lightbox for a UseComputer action.
    ViewScreenshot {
        action_id: AIAgentActionId,
    },
    /// Open the lightbox for an image attached to an already-submitted user query
    /// rendered inside this AI block.
    OpenSubmittedAttachmentLightbox {
        image_index: usize,
    },
    ToggleImportedCommentCollapsed {
        action_id: AIAgentActionId,
        comment_index: usize,
    },
    OpenImportedCommentInCodeReview {
        action_id: AIAgentActionId,
        comment_index: usize,
    },
    OpenAllImportedCommentsInCodeReview,
    OpenCommentInGitHub {
        url: String,
    },
}

impl TypedActionView for AIBlock {
    type Action = AIBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AIBlockAction::SetCollapsibleBlockPinnedToBottom {
                message_id,
                pinned_to_bottom,
            } => {
                if let Some(state) = self.collapsible_block_states.get_mut(message_id) {
                    if let CollapsibleExpansionState::Expanded {
                        scroll_pinned_to_bottom,
                        ..
                    } = &mut state.expansion_state
                    {
                        *scroll_pinned_to_bottom = *pinned_to_bottom;
                        ctx.notify();
                    }
                }
            }
            AIBlockAction::ContinueConversation => {
                // Get the current conversation ID from this block
                let conversation_id = self.client_ids.conversation_id;

                // Emit an event for the terminal view to handle
                // The terminal view will handle setting active conversation,
                // updating context model, setting input mode, and focusing
                ctx.emit(AIBlockEvent::ContinueConversation { conversation_id });

                // Also emit focus terminal event to ensure input is focused
                ctx.emit(AIBlockEvent::FocusTerminal);
                ctx.notify();
            }
            AIBlockAction::ResumeConversation => {
                ctx.emit(AIBlockEvent::ResumeConversation {
                    conversation_id: self.client_ids.conversation_id,
                });
            }
            AIBlockAction::ForkConversation => {
                // Fully reset the fork button's interaction state before navigation.
                // This avoids an immediate re-hover (and stuck tooltip) from synthetic mouse events
                // that can occur while the new pane is being created.
                if let Ok(mut state) = self.state_handles.fork_conversation_handle.lock() {
                    state.reset_interaction_state();
                }

                let is_read_only = self.terminal_model.lock().is_read_only();
                if FeatureFlag::AgentView.is_enabled() && !is_read_only {
                    ctx.emit(AIBlockEvent::InsertForkSlashCommand);
                } else {
                    ctx.dispatch_global_action(
                        "workspace:fork_ai_conversation",
                        ForkAIConversationParams {
                            conversation_id: self.client_ids.conversation_id,
                            fork_from_exchange: None,
                            summarize_after_fork: false,
                            summarization_prompt: None,
                            initial_prompt: None,
                            destination: ForkedConversationDestination::SplitPane,
                        },
                    );
                }
                ctx.notify();
            }
            AIBlockAction::SelectText => {
                // If there's an ongoing text selection, clear all other selections within the
                // `AIBlock`'s view sub-hierarchy to ensure only one component has a selection at a time.
                self.clear_other_selections(None, ctx.window_id(), ctx);
                // If we have a selection, we should use the default cursor, even if it's over a link.
                ctx.reset_cursor();
                self.dismiss_ai_tooltips(ctx);
                // Notify the terminal view so it can keep the model's record of which rich
                // content block has an active selection in sync (rich content selections are
                // not tied to the point-based model selection used for regular blocks).
                ctx.emit(AIBlockEvent::SelectionChanged);
            }
            AIBlockAction::CopyAIBlockCodeSnippet(text) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::success(String::from("Copied to clipboard")),
                        window_id,
                        ctx,
                    );
                });
            }
            AIBlockAction::CopyDebugId(debug_id) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(debug_id.clone()));
            }
            AIBlockAction::OpenFeedbackDocs => {
                ctx.open_url("https://docs.warp.dev/support-and-community/troubleshooting-and-support/sending-us-feedback");
            }
            AIBlockAction::CancelRequestedAction { action_id } => {
                self.cancel_action(action_id, ctx);
            }
            AIBlockAction::ExecuteNextPendingAction => {
                // If the next pending action is a RunAgents tool call,
                // delegate to the per-card view's Accept handler so
                // Enter routes through the executor-backed dispatch
                // path. (Focus normally goes to the card view via
                // `focus_subview_if_necessary`, in which case the
                // card's own keybinding fires; this handler covers
                // the case where focus is still on AIBlock.)
                let run_agents_id = self
                    .action_model
                    .as_ref(ctx)
                    .get_pending_actions_for_conversation(&self.client_ids.conversation_id)
                    .filter(|action| matches!(action.action, AIAgentActionType::RunAgents(_)))
                    .last()
                    .map(|action| action.id.clone());
                if let Some(run_agents_id) = run_agents_id {
                    if let Some(card_view) = self.run_agents_card_views.get(&run_agents_id).cloned()
                    {
                        card_view.update(ctx, |view, ctx_view| view.accept(ctx_view));
                    } else {
                        log::warn!(
                            "ExecuteNextPendingAction: no RunAgentsCardView for {run_agents_id:?}"
                        );
                    }
                } else {
                    self.action_model.update(ctx, |action_model, ctx| {
                        action_model.execute_next_action_for_user(self.conversation_id(), ctx)
                    });
                }
            }
            AIBlockAction::ExecuteRequestedAction { action_id } => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.execute_action(action_id, self.client_ids.conversation_id, ctx)
                });
            }
            AIBlockAction::ChangedHoverOnLink {
                link_range,
                location,
                is_hovering,
            } => {
                // If we're currently selecting, don't consider the link hovered.
                self.detected_links_state.update_hovered_link(
                    *is_hovering,
                    self.state_handles.selection_handle.is_selecting(),
                    link_range,
                    location,
                );
            }
            AIBlockAction::ChangedHoverOnSecret {
                secret_range,
                location,
                is_hovering,
            } => {
                self.secret_redaction_state.set_hover_state_for_secret(
                    location,
                    secret_range,
                    *is_hovering,
                );
            }
            AIBlockAction::OpenLink {
                location,
                link_range,
            } => {
                self.open_link(location, link_range, ctx);
            }
            AIBlockAction::OpenLinkTooltip {
                location,
                link_range,
            } => {
                self.show_link_tooltip(location, link_range, ctx);
            }
            AIBlockAction::OpenSecretTooltip {
                location,
                secret_range,
            } => {
                self.show_secret_tooltip(location, secret_range, ctx);
            }
            AIBlockAction::OpenCitation(citation) => {
                ctx.emit(AIBlockEvent::OpenCitation(citation.clone()));
                let server_output_id = self
                    .model
                    .status(ctx)
                    .output_to_render()
                    .and_then(|output| output.get().server_output_id.clone());
                if let Some(citation) = citation.for_telemetry(ctx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeOpenedCitation {
                            citation,
                            block_id: self.client_ids.client_exchange_id.to_string(),
                            conversation_id: self.client_ids.conversation_id,
                            server_output_id,
                        },
                        ctx
                    );
                }
            }
            AIBlockAction::OpenAIFactCollection => {
                ctx.emit(AIBlockEvent::OpenAIFactCollection { sync_id: None });
            }
            AIBlockAction::ToggleReferencesSection => {
                self.is_references_section_open = !self.is_references_section_open;
            }
            AIBlockAction::ToggleIsUsageFooterExpanded => {
                self.is_usage_footer_expanded = !self.is_usage_footer_expanded;
                ctx.emit(AIBlockEvent::UsageFooterToggled {
                    conversation_id: self.client_ids.conversation_id,
                    is_expanded: self.is_usage_footer_expanded,
                });
            }
            AIBlockAction::CommentExpanded { id } => {
                let Some(comment) = self.comment_states.get_mut(id) else {
                    return;
                };
                comment.is_expanded = !comment.is_expanded;

                let new_icon = if comment.is_expanded {
                    Icon::Minimize
                } else {
                    Icon::Maximize
                };

                comment.maximize_minimize_button.update(ctx, |button, ctx| {
                    button.set_icon(Some(new_icon), ctx);
                });

                ctx.notify()
            }
            AIBlockAction::DismissSuggestionsSection => {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, _| {
                    if let Some(conversation) =
                        model.conversation_mut(&self.client_ids.conversation_id)
                    {
                        conversation.dismiss_current_suggestions();
                    }
                });
                ctx.notify();
            }
            AIBlockAction::DisableRuleSuggestions => {
                // Dismiss the current suggestions and permanently disable future ones.
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, _| {
                    if let Some(conversation) =
                        model.conversation_mut(&self.client_ids.conversation_id)
                    {
                        conversation.dismiss_current_suggestions();
                    }
                });
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .rule_suggestions_enabled_internal
                        .set_value(false, ctx));
                });
                ctx.notify();
            }
            AIBlockAction::ToggleAutoexecuteReadonlyCommandsSpeedbumpCheckbox => {
                if let AutonomySettingSpeedbump::ShouldShowForAutoexecutingReadonlyCommands {
                    checked,
                    ..
                } = &mut self.autonomy_setting_speedbump
                {
                    *checked = !*checked;
                    BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                                match model.set_should_autoexecute_readonly_commands(*checked, ctx) {
                                    Ok(_) => {
                                        send_telemetry_from_ctx!(
                                            TelemetryEvent::ToggledAgentModeAutoexecuteReadonlyCommandsSetting {
                                                src: AutonomySettingToggleSource::Speedbump,
                                                enabled: *checked,
                                            },
                                            ctx);
                                    }
                                    Err(e) => report_error!(e),
                                }
                            });
                }
            }
            AIBlockAction::ToggleAutoreadFilesSpeedbumpCheckbox => {
                if let AutonomySettingSpeedbump::ShouldShowForFileAccess { checked, .. } =
                    &mut self.autonomy_setting_speedbump
                {
                    *checked = !*checked;
                    let permission = if *checked {
                        AgentModeCodingPermissionsType::AlwaysAllowReading
                    } else {
                        AgentModeCodingPermissionsType::AlwaysAskBeforeReading
                    };
                    BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                        match model.set_coding_permissions(permission, ctx) {
                            Ok(_) => {
                                send_telemetry_from_ctx!(
                                    TelemetryEvent::ChangedAgentModeCodingPermissions {
                                        src: AutonomySettingToggleSource::Speedbump,
                                        new: permission,
                                    },
                                    ctx
                                );
                            }
                            Err(e) => report_error!(e),
                        }
                    });
                }
            }
            AIBlockAction::ToggleCodebaseSearchSpeedbump(new) => {
                if let AutonomySettingSpeedbump::ShouldShowForCodebaseSearchFileAccess {
                    selected_option,
                    ..
                } = &mut self.autonomy_setting_speedbump
                {
                    *selected_option = *new;
                    let permission = match new {
                        Some(0) => AgentModeCodingPermissionsType::AlwaysAllowReading,
                        Some(1) => AgentModeCodingPermissionsType::AllowReadingSpecificFiles,
                        _ => AgentModeCodingPermissionsType::AlwaysAskBeforeReading,
                    };
                    BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                        match model.set_coding_permissions(permission, ctx) {
                            Ok(_) => {
                                send_telemetry_from_ctx!(
                                    TelemetryEvent::ChangedAgentModeCodingPermissions {
                                        src: AutonomySettingToggleSource::Speedbump,
                                        new: permission,
                                    },
                                    ctx
                                );
                            }
                            Err(e) => report_error!(e),
                        }
                    });
                }
            }
            AIBlockAction::StartNewConversationButtonClicked {
                action_id,
                server_output_id: _,
            } => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.suggest_new_conversation_executor(ctx).update(
                        ctx,
                        |executor, _| {
                            executor.complete_suggest_new_conversation_action(
                                NewConversationDecision::Accept,
                            );
                        },
                    );
                    action_model.execute_action(action_id, self.client_ids.conversation_id, ctx);
                });
            }
            AIBlockAction::ContinueCurrentConversationButtonClicked {
                action_id,
                server_output_id: _,
            } => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.suggest_new_conversation_executor(ctx).update(
                        ctx,
                        |executor, _| {
                            executor.complete_suggest_new_conversation_action(
                                NewConversationDecision::Reject,
                            );
                        },
                    );
                    action_model.execute_action(action_id, self.client_ids.conversation_id, ctx);
                });
            }
            AIBlockAction::Rated { is_positive } => {
                let output_id = self.model.server_output_id(ctx);
                let rating = if *is_positive {
                    AIBlockResponseRating::Positive
                } else {
                    AIBlockResponseRating::Negative
                };
                if self.response_rating.set(rating).is_err() {
                    // A rating was already set for this block. This should be unreachable.
                    return;
                }

                if matches!(rating, AIBlockResponseRating::Negative) {
                    if let Some(output_id) = output_id.clone() {
                        let request_usage_model = AIRequestUsageModel::handle(ctx);
                        request_usage_model.update(ctx, |request_usage_model, ctx| {
                            request_usage_model
                                .provide_negative_feedback_response_for_ai_conversation(
                                    self.client_ids.conversation_id,
                                    output_id.to_string(),
                                    self.client_ids.client_exchange_id,
                                    ctx,
                                );
                        });
                    }
                }

                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast =
                        DismissibleToast::default(String::from("Thank you for the feedback!"));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });

                send_telemetry_from_ctx!(
                    TelemetryEvent::AgentModeRatedResponse {
                        server_output_id: output_id,
                        conversation_id: self.client_ids.conversation_id,
                        rating,
                    },
                    ctx
                );
            }
            AIBlockAction::ClearOtherSelections {
                source_view_id,
                source_window_id,
            } => {
                self.clear_other_selections(*source_view_id, *source_window_id, ctx);
            }
            AIBlockAction::CopyQuery => {
                // Copy the prompt from the preceding user query (where overflow menu would appear)
                let prompt_text = self.get_preceding_user_query(ctx);
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(prompt_text));
            }
            AIBlockAction::CopyOutput => {
                // Copy all AI output from preceding user query until the next user query
                let output_text = self.get_output_text_since_preceding_user_query(ctx);
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(output_text));
            }
            AIBlockAction::Copy => {
                // Copy the preceding user query and all AI output until the next user query
                let prompt_text = self.get_preceding_user_query(ctx);
                let output_text = self.get_output_text_since_preceding_user_query(ctx);
                let combined_text = format!("{prompt_text}\n\n{output_text}");
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(combined_text));
            }
            AIBlockAction::CopyCommand => {
                let command_text = if let Some(stored_command) = &self.last_right_clicked_command {
                    // Use the specific command that was right-clicked
                    stored_command.clone()
                } else {
                    // Fallback to collecting all commands (previous behavior)
                    let commands: Vec<String> = self
                        .requested_commands
                        .values()
                        .map(|requested_command| {
                            requested_command
                                .view
                                .as_ref(ctx)
                                .command_text()
                                .to_string()
                        })
                        .collect();

                    // Join multiple commands with a newline if there are multiple commands in a single AI block
                    // (this only happens when the agent uses parallel tool calling to execute multiple commands at once,
                    // and the user has not right clicked on a specific command)
                    commands.join("\n")
                };

                ctx.clipboard()
                    .write(ClipboardContent::plain_text(command_text));

                // Clear the stored command after copying
                self.last_right_clicked_command = None;
            }
            AIBlockAction::OpenCodeInWarp {
                #[cfg_attr(not(feature = "local_fs"), allow(unused))]
                source,
            } => {
                // Resets the interaction states of ReadSkill and ReadFiles tool call banners before opening a new code pane
                // Avoids an immediate re-hover (and stuck tooltip) while the new code pane is being created
                for handle in self.state_handles.skill_button_handles.values() {
                    if let Ok(mut state) = handle.lock() {
                        state.reset_interaction_state();
                    }
                }

                // Sends a telemetry event when a skill is opened from an 'open skill' button
                if let CodeSource::Skill {
                    reference, origin, ..
                } = source
                {
                    send_telemetry_from_ctx!(
                        SkillTelemetryEvent::Opened {
                            reference: reference.clone(),
                            name: SkillManager::as_ref(ctx)
                                .skill_by_reference(reference)
                                .map(|skill| skill.name.clone()),
                            origin: *origin,
                        },
                        ctx
                    );
                }

                #[cfg(feature = "local_fs")]
                {
                    ctx.emit(AIBlockEvent::OpenCodeInWarp {
                        source: source.clone(),
                        layout: *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                            .open_file_layout
                            .value(),
                    })
                }
            }
            AIBlockAction::ToggleTodoListExpanded(id) => {
                if let Some(state) = self.todo_list_states.get_mut(id) {
                    state.is_expanded = !state.is_expanded;
                }
            }
            AIBlockAction::ToggleCollapsibleBlockExpanded(id) => {
                if let Some(state) = self.collapsible_block_states.get_mut(id) {
                    state.toggle_expansion();
                }
            }
            AIBlockAction::ToggleCodeReviewPane => {
                ctx.emit(AIBlockEvent::ToggleCodeReviewPane {
                    entrypoint: CodeReviewPaneEntrypoint::AgentModeRunning,
                });
            }
            AIBlockAction::StoreRightClickedCommand { command } => {
                self.last_right_clicked_command = Some(command.clone());
            }
            AIBlockAction::RunAwsLoginCommand => {
                ctx.emit(AIBlockEvent::RunAwsLoginCommand);
            }
            AIBlockAction::ToggleAwsBedrockAutoLogin => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let current = *settings.aws_bedrock_auto_login.value();
                    let new_value = !current;
                    report_if_error!(settings.aws_bedrock_auto_login.set_value(new_value, ctx));
                });
            }
            AIBlockAction::ConfigureAwsLoginCommand => {
                ctx.dispatch_typed_action(&WorkspaceAction::ShowSettingsPageWithSearch {
                    search_query: "aws bedrock".to_string(),
                    section: Some(SettingsSection::WarpAgent),
                });
            }
            AIBlockAction::ToggleImportedCommentCollapsed {
                action_id,
                comment_index,
            } => {
                if let Some(group) = self.imported_comments.get_mut(action_id) {
                    if let Some(card) = group.card_mut(*comment_index) {
                        card.toggle_collapsed();
                        let is_collapsed = card.is_collapsed();
                        if let Some(state) = group.element_states.get(*comment_index) {
                            let icon = if is_collapsed {
                                Icon::ChevronRight
                            } else {
                                Icon::ChevronDown
                            };
                            state.chevron_button.update(ctx, |button, ctx| {
                                button.set_icon(Some(icon), ctx);
                            });
                        }
                    }
                }
            }
            AIBlockAction::OpenImportedCommentInCodeReview {
                action_id,
                comment_index,
            } => {
                if let Some(group) = self.imported_comments.get_mut(action_id) {
                    let repo_path = group.repo_path.clone();
                    let base_branch = group.base_branch.clone();
                    if let Some(card) = group.card_mut(*comment_index) {
                        ctx.emit(AIBlockEvent::OpenImportedCommentInCodeReview {
                            repo_path,
                            comment: Box::new(card.source().clone()),
                            base_branch,
                        });
                    }
                }
            }
            AIBlockAction::OpenAllImportedCommentsInCodeReview => {
                ctx.emit(AIBlockEvent::OpenAllImportedCommentsForConversation {
                    conversation_id: self.client_ids.conversation_id,
                });
            }
            AIBlockAction::OpenCommentInGitHub { url } => {
                ctx.open_url(url);
            }
            AIBlockAction::ViewScreenshot { action_id } => {
                // Collect all UseComputer action IDs across the entire conversation
                // so the lightbox can navigate between their screenshots.
                let conversation_id = self.client_ids.conversation_id;

                let use_computer_action_ids: Vec<AIAgentActionId> =
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .into_iter()
                        .flat_map(|c| c.use_computer_action_ids())
                        .collect();

                // Build lightbox images for each action that has a screenshot result.
                // We Arc::clone the result each iteration to release the immutable
                // borrow on ctx, allowing the mutable AssetCache update in the same
                // loop body. Arc::clone is just a refcount bump (no data copied).
                let mut screenshot_action_ids: Vec<&AIAgentActionId> = Vec::new();
                let mut images: Vec<ui_components::lightbox::LightboxImage> = Vec::new();
                for action_id in &use_computer_action_ids {
                    let Some(result) = self
                        .action_model
                        .as_ref(ctx)
                        .get_action_result(action_id)
                        .map(Arc::clone)
                    else {
                        continue;
                    };
                    let AIAgentActionResultType::UseComputer(
                        crate::ai::agent::UseComputerResult::Success(computer_use::ActionResult {
                            screenshot: Some(screenshot),
                            ..
                        }),
                    ) = &result.result
                    else {
                        continue;
                    };
                    let asset_id = format!("screenshot-{action_id}");
                    AssetCache::handle(ctx).update(ctx, |asset_cache, ctx| {
                        asset_cache.insert_raw_asset_bytes::<ImageType>(
                            asset_id.clone(),
                            &screenshot.data,
                            ctx,
                        );
                    });
                    images.push(ui_components::lightbox::LightboxImage {
                        source: ui_components::lightbox::LightboxImageSource::Resolved {
                            asset_source: warpui::assets::asset_cache::AssetSource::Raw {
                                id: asset_id,
                            },
                        },
                        description: None,
                    });
                    screenshot_action_ids.push(action_id);
                }

                if images.is_empty() {
                    return;
                }

                let initial_index = screenshot_action_ids
                    .iter()
                    .position(|id| *id == action_id)
                    .unwrap_or(0);

                ctx.dispatch_typed_action(&WorkspaceAction::OpenLightbox {
                    images,
                    initial_index,
                });
            }
            AIBlockAction::OpenSubmittedAttachmentLightbox { image_index } => {
                let decoded_images = self
                    .model
                    .inputs_to_render(ctx)
                    .iter()
                    .filter_map(|input| input.context())
                    .flat_map(|contexts| contexts.iter())
                    .filter_map(|context| match context {
                        AIAgentContext::Image(image) => Some(image),
                        _ => None,
                    })
                    .enumerate()
                    .filter_map(|(submitted_image_index, image)| {
                        let image_bytes =
                            match base64::engine::general_purpose::STANDARD.decode(&image.data) {
                                Ok(image_bytes) => image_bytes,
                                Err(error) => {
                                    log::warn!(
                                        "Failed to decode submitted image attachment for lightbox: {error}"
                                    );
                                    return None;
                                }
                            };

                        Some((submitted_image_index, image_bytes, image.file_name.clone()))
                    })
                    .collect_vec();
                let mut images = Vec::new();
                let mut initial_index = None;
                for (submitted_image_index, image_bytes, file_name) in decoded_images {
                    let asset_id = format!(
                        "submitted-attachment-lightbox-{}-{submitted_image_index}",
                        self.client_ids.client_exchange_id
                    );
                    // Raw assets are keyed by exchange/image index, so repeated opens replace the
                    // same cache entry. `AssetCache` also enforces its raw-asset size cap when
                    // inserting, so no lightbox-specific cleanup is needed here.
                    AssetCache::handle(ctx).update(ctx, |asset_cache, ctx| {
                        asset_cache.insert_raw_asset_bytes::<ImageType>(
                            asset_id.clone(),
                            &image_bytes,
                            ctx,
                        );
                    });

                    if submitted_image_index == *image_index {
                        initial_index = Some(images.len());
                    }
                    images.push(ui_components::lightbox::LightboxImage {
                        source: ui_components::lightbox::LightboxImageSource::Resolved {
                            asset_source: warpui::assets::asset_cache::AssetSource::Raw {
                                id: asset_id,
                            },
                        },
                        description: Some(file_name),
                    });
                }

                let Some(initial_index) = initial_index else {
                    return;
                };

                ctx.dispatch_typed_action(&WorkspaceAction::OpenLightbox {
                    images,
                    initial_index,
                });
            }
        }
        ctx.notify();
    }
}

impl AIBlock {
    /// Lazily create the per-action `RunAgentsCardView` so the
    /// orchestrate confirmation card can render on its first frame.
    /// Idempotent: re-running with an already-populated entry leaves
    /// it unchanged. The view drives Accept dispatch through
    /// [`BlocklistAIActionModel::execute_run_agents`] itself; only
    /// `RejectRequested` flows back here so the existing
    /// [`Self::cancel_action`] entry point handles cancellation.
    fn ensure_run_agents_card_view(
        &mut self,
        action_id: &AIAgentActionId,
        request: &RunAgentsRequest,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(existing_view) = self.run_agents_card_views.get(action_id) {
            // The view was created on an earlier streaming chunk that may
            // have carried a partial/empty request. Re-sync the edit state
            // from the latest (potentially more complete) request so the
            // card renders the correct agent count, summary, etc.
            existing_view.update(ctx, |view, ctx| {
                view.update_request(request, ctx);
            });
            return;
        }

        // Read the active orchestration config for auto-launch /
        // denied decisions from the conversation (not the singleton).
        let active_config = {
            let history = crate::BlocklistAIHistoryModel::as_ref(ctx);
            let conv = history.conversation(&self.client_ids.conversation_id);
            let result = if !request.plan_id.is_empty() {
                conv.and_then(|conv| {
                    conv.orchestration_config_for_plan(&request.plan_id)
                        .map(|(config, status)| (config.clone(), status))
                })
            } else {
                None
            };
            result
        };

        let action_id_clone = action_id.clone();
        let request_clone = request.clone();
        let action_model = self.action_model.clone();
        let run_agents_executor = self.action_model.as_ref(ctx).run_agents_executor(ctx);
        let block_model = self.model.clone();
        let view = ctx.add_typed_action_view(move |ctx_view| {
            RunAgentsCardView::new(
                action_id_clone,
                &request_clone,
                active_config,
                action_model,
                run_agents_executor,
                block_model,
                ctx_view,
            )
        });
        let action_id_for_event = action_id.clone();
        ctx.subscribe_to_view(&view, move |me, _, event, ctx| match event {
            RunAgentsCardViewEvent::RejectRequested => {
                me.cancel_action(&action_id_for_event, ctx);
            }
        });
        self.run_agents_card_views.insert(action_id.clone(), view);
    }
}
#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
