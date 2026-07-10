//! Renders the AI output portion of the AI block.
//!
//! This includes text, code snippets, suggested commands, and interactive inline action UX.
use std::cell::OnceCell;
use std::cmp::Ordering;
use std::collections::HashMap;
#[allow(unused_imports)]
use std::path::{Component, Path};
use std::rc::Rc;
use std::sync::Arc;

use ai::agent::action::{
    RequestComputerUseRequest, SuggestPromptRequest, UploadArtifactRequest, UseComputerRequest,
};
use ai::agent::file_locations::group_file_contexts_for_display;
use ai::skills::{ParsedSkill, SkillReference};
use indexmap::IndexMap;
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::{button, Component as _, Options as _};
use warp_core::channel::ChannelState;
use warp_core::ui::theme::color::internal_colors;
use warp_errors::report_error;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::new_scrollable::SingleAxisConfig;
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Expanded, Fill, Flex, FormattedTextElement, Hoverable,
    MainAxisAlignment, MainAxisSize, NewScrollable, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Shrinkable, Stack, Text, Wrap,
};
use warpui::keymap::Keystroke;
use warpui::platform::{Cursor, OperatingSystem};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::radio_buttons::{RadioButtonItem, RadioButtonLayout};
use warpui::{
    Action, AppContext, Element, EntityId, ModelHandle, SingletonEntity, View, ViewHandle,
};

use super::common::{
    render_debug_footer, render_failed_output, render_informational_footer,
    render_output_status_text, render_scrollable_collapsible_content, render_text_sections,
    DebugFooterProps, FailedOutputProps, FindContext, TextSectionsProps,
    STATUS_FOOTER_VERTICAL_PADDING, STATUS_ICON_SIZE_DELTA,
};
use super::imported_comments::render_imported_comments;
use super::todos::{render_completed_todo_items, render_todos};
use super::{
    add_highlights_to_rich_text, orchestration, render_autonomy_checkbox_setting_speedbump_footer,
    render_citation_chips, WithContentItemSpacing, CONTENT_HORIZONTAL_PADDING,
    CONTENT_ITEM_VERTICAL_MARGIN,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::comment::ReviewComment;
use crate::ai::agent::conversation::{RecordingSpanInfo, RecordingSpanStatus};
use crate::ai::agent::icons::{self, gray_stop_icon, yellow_stop_icon};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentCitation, AIAgentInput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentText, AIAgentTextSection, CancellationOutcome,
    CreateDocumentsResult, EditDocumentsResult, MessageId, ReadFilesRequest, ReadFilesResult,
    RequestCommandOutputResult, SearchCodebaseFailureReason, SearchCodebaseResult,
    StartRecordingResult, StopRecordingResult, SubagentCall, SubagentType,
    SuggestNewConversationResult, SummarizationType, TodoOperation, UploadArtifactResult,
};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::action_model::AIActionStatus;
use crate::ai::blocklist::block::model::{AIBlockModel, AIBlockModelHelper, AIBlockOutputStatus};
use crate::ai::blocklist::block::view_impl::common::{
    MaybeShimmeringText, BLOCKED_ACTION_MESSAGE_FOR_GREP_OR_FILE_GLOB,
    BLOCKED_ACTION_MESSAGE_FOR_READING_FILES, BLOCKED_ACTION_MESSAGE_FOR_SEARCHING_CODEBASE,
};
use crate::ai::blocklist::block::{
    AIBlock, AIBlockAction, AIBlockStateHandles, ActionButtons, AutonomySettingSpeedbump,
    CollapsibleElementState, CollapsibleExpansionState, EmbeddedCodeEditorView, FinishReason,
    ImportedCommentGroup, RequestedEdit, TextLocation, TodoListElementState,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::inline_action::ask_user_question_view::AskUserQuestionView;
use crate::ai::blocklist::inline_action::aws_bedrock_credentials_error::AwsBedrockCredentialsErrorView;
use crate::ai::blocklist::inline_action::create_or_edit_document::CreateOrEditDocumentAction;
use crate::ai::blocklist::inline_action::inline_action_header::{
    HeaderConfig, InteractionMode, INLINE_ACTION_HEADER_VERTICAL_PADDING,
    INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::{self, icon_size};
use crate::ai::blocklist::inline_action::requested_action::{
    render_requested_action_body_text, render_requested_action_row_for_text, RenderableAction,
};
use crate::ai::blocklist::inline_action::requested_command::RequestedCommand;
use crate::ai::blocklist::inline_action::run_agents_card_view::RunAgentsCardView;
use crate::ai::blocklist::inline_action::search_codebase::SearchCodebaseView;
use crate::ai::blocklist::inline_action::suggested_unit_tests::SuggestedUnitTestsView;
use crate::ai::blocklist::inline_action::web_fetch::WebFetchView;
use crate::ai::blocklist::inline_action::web_search::WebSearchView;
use crate::ai::blocklist::keyboard_navigable_buttons::KeyboardNavigableButtons;
use crate::ai::blocklist::secret_redaction::SecretRedactionState;
use crate::ai::blocklist::usage::rollup::compute_orchestration_rollup;
use crate::ai::blocklist::view_util::format_credits;
use crate::ai::blocklist::{AIBlockResponseRating, BlocklistAIActionModel, SuggestionChipView};
use crate::ai::paths::shell_native_absolute_path;
use crate::ai::skills::{
    icon_override_for_skill_name, render_skill_button, skill_path_from_location, SkillManager,
    SkillOpenOrigin,
};
use crate::appearance::Appearance;
use crate::code::diff_viewer::DisplayMode;
use crate::code::editor_management::CodeSource;
use crate::settings_view::SettingsSection;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::input::slash_commands::fork_button_action;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::ShellLaunchData;
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::util::link_detection::{add_link_detection_mouse_interactions, DetectedLinksState};
use crate::util::time_format::format_elapsed_seconds;
use crate::util::truncation::truncate_from_end;
use crate::view_components::action_button::ActionButton;
use crate::view_components::compactible_action_button::{
    CompactibleActionButton, RenderCompactibleActionButton, SMALL_SIZE_SWITCH_THRESHOLD,
};
use crate::workspace::WorkspaceAction;
use crate::{AIAgentTodoList, FeatureFlag};

const BLOCKED_ACTION_MESSAGE_FOR_UPLOADING_ARTIFACT: &str = "Grant access to upload this artifact?";

/// Data required to render the AI block output component.
#[derive(Copy, Clone)]
pub(crate) struct Props<'a> {
    pub(crate) model: &'a dyn AIBlockModel<View = AIBlock>,
    pub(super) state_handles: &'a AIBlockStateHandles,
    pub(super) action_buttons: &'a HashMap<AIAgentActionId, ActionButtons>,
    pub(super) view_screenshot_buttons: &'a HashMap<AIAgentActionId, ui_components::button::Button>,
    pub(super) open_recording_buttons: &'a HashMap<AIAgentActionId, ui_components::button::Button>,
    /// Whether this block's output contains recording-related actions, so
    /// rendering can skip deriving recording spans for unrelated blocks.
    pub(super) has_recording_related_actions: bool,
    pub(crate) action_model: &'a ModelHandle<BlocklistAIActionModel>,
    pub(crate) active_session: &'a ModelHandle<ActiveSession>,
    pub(super) editor_views: &'a [EmbeddedCodeEditorView],
    pub(super) current_working_directory: Option<&'a String>,
    pub(super) shell_launch_data: Option<&'a ShellLaunchData>,
    pub(crate) detected_links_state: &'a DetectedLinksState,
    pub(crate) secret_redaction_state: &'a SecretRedactionState,
    pub(super) requested_commands: &'a HashMap<AIAgentActionId, RequestedCommand>,
    pub(super) requested_mcp_tools: &'a HashMap<AIAgentActionId, RequestedCommand>,
    pub(super) requested_edits: &'a IndexMap<AIAgentActionId, RequestedEdit>,
    pub(super) unit_test_suggestions:
        &'a HashMap<AIAgentActionId, ViewHandle<SuggestedUnitTestsView>>,
    pub(super) todo_list_states: &'a HashMap<MessageId, TodoListElementState>,
    pub(super) collapsible_block_states: &'a HashMap<MessageId, CollapsibleElementState>,
    pub(crate) is_selecting_text: bool,
    pub(super) is_ai_input_enabled: bool,
    pub(crate) find_context: Option<FindContext<'a>>,
    pub(super) is_references_section_open: bool,
    pub(super) autonomy_setting_speedbump: &'a AutonomySettingSpeedbump,
    pub(super) suggested_rules: &'a Vec<ViewHandle<SuggestionChipView>>,
    pub(super) suggested_agent_mode_workflow: &'a Option<ViewHandle<SuggestionChipView>>,
    pub(super) manage_rules_button: &'a ViewHandle<ActionButton>,
    pub(super) keyboard_navigable_buttons: Option<&'a ViewHandle<KeyboardNavigableButtons>>,
    pub(super) response_rating: &'a OnceCell<AIBlockResponseRating>,
    pub(super) request_refunded_count: Option<i32>,
    pub(super) search_codebase_view: &'a HashMap<AIAgentActionId, ViewHandle<SearchCodebaseView>>,
    pub(super) web_search_views: &'a HashMap<MessageId, ViewHandle<WebSearchView>>,
    pub(super) web_fetch_views: &'a HashMap<MessageId, ViewHandle<WebFetchView>>,
    pub(super) review_changes_button: &'a ViewHandle<ActionButton>,
    pub(super) open_all_comments_button: &'a ViewHandle<ActionButton>,
    pub(super) dismiss_suggestion_button: &'a ViewHandle<ActionButton>,
    pub(super) disable_rule_suggestions_button: &'a ViewHandle<ActionButton>,
    pub(super) current_todo_list: Option<&'a AIAgentTodoList>,
    pub(super) has_accepted_edits: bool,
    pub(super) finish_reason: Option<&'a FinishReason>,
    pub(super) is_usage_footer_expanded: bool,
    pub(super) shared_session_status: &'a SharedSessionStatus,
    pub(super) terminal_view_id: EntityId,
    pub(super) is_conversation_transcript_viewer: bool,
    #[cfg(not(target_family = "wasm"))]
    pub(super) is_cloud_agent_context: bool,
    pub(super) aws_bedrock_credentials_error_view:
        Option<&'a ViewHandle<AwsBedrockCredentialsErrorView>>,
    pub(super) imported_comments: &'a HashMap<AIAgentActionId, ImportedCommentGroup>,
    /// Per-orchestrate-action card view. Each `RunAgentsCardView` owns
    /// its own edit state, button + picker handles, and in-flight
    /// spawning snapshot; AIBlock just lazily creates the view per
    /// `AIAgentActionId` and embeds it via `ChildView` when the action
    /// is rendered. Multi-card lifecycle = AIBlock lifecycle.
    pub(crate) run_agents_card_views: &'a HashMap<AIAgentActionId, ViewHandle<RunAgentsCardView>>,
    #[cfg(feature = "local_fs")]
    pub(crate) resolved_code_block_paths:
        &'a HashMap<std::path::PathBuf, Option<std::path::PathBuf>>,
    #[cfg(feature = "local_fs")]
    pub(crate) resolved_blocklist_image_sources: &'a super::common::ResolvedBlocklistImageSources,
    /// Controls how agent thinking/reasoning traces are displayed.
    pub(super) thinking_display_mode: crate::settings::ThinkingDisplayMode,
    pub(super) conversation_has_imported_comments: bool,
    pub(super) ask_user_question_view: Option<&'a ViewHandle<AskUserQuestionView>>,
    /// `true` when this block belongs to a cloud agent pane that is still in its setup phase
    /// (running environment startup commands before the first agent turn).
    pub(super) is_cloud_agent_pre_first_exchange: bool,
}

/// A `UseComputer` call whose actions are all no-ops (typically a single
/// zero-duration wait alongside screenshot params) is a screenshot-only
/// capture rather than a user-visible interaction, so it shouldn't be labeled
/// as captured in a recording.
fn should_decorate_recorded_use_computer(request: &UseComputerRequest) -> bool {
    request
        .actions
        .iter()
        .any(|action| !action.action.is_no_op())
}

pub(super) fn render(props: Props, app: &AppContext) -> Box<dyn Element> {
    let mut output_items = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    let appearance = Appearance::as_ref(app);
    let request_type = props.model.request_type(app);

    let conversation_status = props.model.conversation(app).map(|c| c.status());
    let is_conversation_in_progress = conversation_status.is_some_and(|s| s.is_in_progress());

    let status = props.model.status(app);
    let has_expanded_last_requested_command = status.output_to_render().is_some_and(|output| {
        let output = output.get();
        output.messages.last().is_some_and(|message| {
            let AIAgentOutputMessageType::Action(action) = &message.message else {
                return false;
            };

            matches!(
                &action.action,
                AIAgentActionType::RequestCommandOutput { .. }
            ) && props
                .requested_commands
                .get(&action.id)
                .is_some_and(|requested_command| {
                    requested_command.view.as_ref(app).is_header_expanded()
                })
        })
    });
    match status {
        // Ignore errors if the response is not yet complete-- it could be a deserialization
        // error that corrects itself when more output is streamed in.
        AIBlockOutputStatus::Pending => (),
        AIBlockOutputStatus::PartiallyReceived { .. }
        | AIBlockOutputStatus::Complete { .. }
        | AIBlockOutputStatus::Cancelled { .. }
        | AIBlockOutputStatus::Failed { .. } => {
            if let Some(output) = status.output_to_render() {
                let output = output.get();
                // TODO(vkodithala): Blocks with recording-related actions still
                // recompute this conversation-wide map on every render. Cache
                // spans on BlocklistAIActionModel keyed by conversation and
                // refresh on action/result mutations instead.
                let recording_spans_by_action_id = if props.has_recording_related_actions {
                    props
                        .model
                        .conversation(app)
                        .map(|conversation| {
                            conversation
                                .recording_spans_by_action_id(Some(props.action_model.as_ref(app)))
                        })
                        .unwrap_or_default()
                } else {
                    HashMap::new()
                };
                let is_complete = matches!(status, AIBlockOutputStatus::Complete { .. });
                let is_output_for_static_prompt_suggestions =
                    props.model.contains_static_prompt_suggestion_input(app);

                // We only want to render the references section, thumbs up/down ratings, and suggestions
                // when the entire response is complete to avoid intermediate states.
                let mut should_render_references_section = is_complete && request_type.is_active();
                let mut should_render_suggestions = is_complete
                    && props.model.is_latest_visible_exchange_in_root_task(app)
                    && !has_expanded_last_requested_command
                    && !is_conversation_in_progress
                    && !is_output_for_static_prompt_suggestions
                    && request_type.is_active();

                // Passive code diffs footer, after acceptance, is different from the usual footer.
                let requires_special_footer =
                    request_type.is_passive_code_diff() && props.has_accepted_edits;

                let mut should_render_footer =
                    (props.model.is_latest_visible_exchange_in_root_task(app)
                        || requires_special_footer)
                        && !has_expanded_last_requested_command
                        && !is_output_for_static_prompt_suggestions
                        && !is_conversation_in_progress
                        && request_type.is_active()
                        && !props.is_cloud_agent_pre_first_exchange
                        && !status
                            .error()
                            .map(|e| e.is_invalid_api_key())
                            .unwrap_or_default();

                let mut has_rendered_first_text_section = false;

                let mut code_section_index = 0;
                let mut text_section_index = 0;
                let mut table_section_index = 0;
                let mut image_section_index = 0;
                let mut action_index = 0;

                fn open_code_block_action(source: CodeSource) -> AIBlockAction {
                    AIBlockAction::OpenCodeInWarp { source }
                }

                fn copy_code_action(snippet: String) -> AIBlockAction {
                    AIBlockAction::CopyAIBlockCodeSnippet(snippet)
                }

                for output_message in output.messages.iter() {
                    match &output_message.message {
                        // Skip rendering text and reasoning sections if this is a passive conversation.
                        AIAgentOutputMessageType::Text(_)
                        | AIAgentOutputMessageType::Reasoning { .. }
                            if request_type.is_passive() =>
                        {
                            continue;
                        }
                        AIAgentOutputMessageType::Text(AIAgentText { sections })
                            if !are_all_text_sections_empty(sections) =>
                        {
                            let theme = appearance.theme();
                            let text_color = blended_colors::text_main(theme, theme.surface_1());

                            let text_sections = render_text_sections(
                                TextSectionsProps {
                                    model: props.model,
                                    starting_text_section_index: &mut text_section_index,
                                    starting_code_section_index: &mut code_section_index,
                                    starting_table_section_index: &mut table_section_index,
                                    starting_image_section_index: &mut image_section_index,
                                    sections,
                                    text_color,
                                    selectable: true,
                                    find_context: props.find_context,
                                    current_working_directory: props.current_working_directory,
                                    shell_launch_data: props.shell_launch_data,
                                    embedded_code_editor_views: props.editor_views,
                                    code_snippet_button_handles: &props
                                        .state_handles
                                        .normal_response_code_snippet_buttons,
                                    table_section_handles: &props
                                        .state_handles
                                        .table_section_handles,
                                    image_section_tooltip_handles: &props
                                        .state_handles
                                        .image_section_tooltip_handles,
                                    is_ai_input_enabled: props.is_ai_input_enabled,
                                    open_code_block_action_factory: Some(&open_code_block_action),
                                    copy_code_action_factory: Some(&copy_code_action),
                                    detected_links: Some(props.detected_links_state),
                                    secret_redaction_state: props.secret_redaction_state,
                                    is_selecting_text: props
                                        .state_handles
                                        .selection_handle
                                        .is_selecting(),
                                    item_spacing: CONTENT_ITEM_VERTICAL_MARGIN,
                                    #[cfg(feature = "local_fs")]
                                    resolved_code_block_paths: Some(
                                        props.resolved_code_block_paths,
                                    ),
                                    #[cfg(feature = "local_fs")]
                                    resolved_blocklist_image_sources: Some(
                                        props.resolved_blocklist_image_sources,
                                    ),
                                },
                                app,
                            );
                            output_items.add_child(
                                text_sections.with_agent_output_item_spacing(app).finish(),
                            );
                            has_rendered_first_text_section = true;
                        }
                        AIAgentOutputMessageType::Reasoning {
                            text: AIAgentText { sections },
                            finished_duration,
                        } if !are_all_text_sections_empty(sections)
                            && props.thinking_display_mode.should_render() =>
                        {
                            let header_text = if let Some(dur) = finished_duration {
                                format!("Thought for {}", format_elapsed_seconds(*dur))
                            } else {
                                "Thinking".to_string()
                            };
                            if let Some(element) = render_collapsible_block(
                                output_message,
                                header_text,
                                sections,
                                true,
                                props,
                                &mut has_rendered_first_text_section,
                                &mut text_section_index,
                                &mut code_section_index,
                                app,
                            ) {
                                output_items.add_child(element);
                            }
                        }
                        // When reasoning is present but not rendered (thinking display
                        // mode is off), we still need to advance text_section_index to
                        // stay in sync with all_text()-based link detection.
                        AIAgentOutputMessageType::Reasoning {
                            text: AIAgentText { sections },
                            ..
                        } if !are_all_text_sections_empty(sections) => {
                            text_section_index += sections.len();
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::RequestCommandOutput { .. },
                            id,
                            ..
                        }) => {
                            // Since we're rendering a requested command, it will
                            // render the citations so don't render them again.
                            should_render_references_section = false;

                            let is_action_done = props
                                .action_model
                                .as_ref(app)
                                .get_action_status(id)
                                .as_ref()
                                .is_some_and(|status| status.is_done());
                            if !is_action_done {
                                // Ratings & suggestions should not be rendered for requested command actions that are not complete.
                                should_render_footer = false;
                                should_render_suggestions = false;
                            }

                            if let Some(rendered_command) = props
                                .requested_commands
                                .get(id)
                                .map(|requested_command| requested_command.render())
                            {
                                output_items.add_child(rendered_command);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::SearchCodebase(..),
                            id,
                            ..
                        }) => {
                            // Neither ratings nor suggestions should be rendered for relevant file queries.
                            should_render_footer = false;
                            should_render_suggestions = false;
                            if let Some(rendered_message) = render_search_codebase(props, id, app) {
                                output_items.add_child(rendered_message);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::ReadFiles(ReadFilesRequest { locations: files }),
                            id,
                            ..
                        }) => {
                            if !status.is_streaming() || !files.is_empty() {
                                // get the results of the agent's read file actions so we can read the results for later use
                                let agent_action_results = props
                                    .action_model
                                    .as_ref(app)
                                    .get_action_result(id)
                                    .map(|action_result| action_result.as_ref());

                                // checks if the read file action result is completed and successful.
                                // if successful, we have FileContext with pre-computed line counts that we use to clamp displayed file ranges to the length of the file
                                let file_names = match agent_action_results {
                                    // if completed and successful, generate a user message with file info + line count
                                    Some(AIAgentActionResult {
                                        result:
                                            AIAgentActionResultType::ReadFiles(
                                                ReadFilesResult::Success {
                                                    files: file_contexts,
                                                },
                                            ),
                                        ..
                                    }) => {
                                        if file_contexts.is_empty() {
                                            // Empty file contexts — render as a failed
                                            // action so the user sees the error instead
                                            // of an empty box.
                                            let formatted_text = render_requested_action_body_text(
                                                "Failed to read files".into(),
                                                appearance.ui_font_family(),
                                                app,
                                            );
                                            let renderable_action =
                                                RenderableAction::new_with_formatted_text(
                                                    formatted_text,
                                                    app,
                                                )
                                                .with_icon(
                                                    inline_action_icons::red_x_icon(appearance)
                                                        .finish(),
                                                );
                                            output_items
                                                .add_child(renderable_action.render(app).finish());
                                            continue;
                                        }
                                        group_file_contexts_for_display(
                                            file_contexts,
                                            props.shell_launch_data,
                                            props.current_working_directory,
                                        )
                                    }
                                    // if not completed/successful, generate a user message without line count
                                    _ => files
                                        .iter()
                                        .map(|file| {
                                            file.to_user_message(
                                                props.shell_launch_data,
                                                props.current_working_directory,
                                                None,
                                            )
                                        })
                                        .collect_vec(),
                                };

                                let file_locations = files
                                    .iter()
                                    .map(|file| {
                                        let path = shell_native_absolute_path(
                                            &file.name,
                                            props.shell_launch_data,
                                            props.current_working_directory,
                                        );
                                        props
                                            .active_session
                                            .as_ref(app)
                                            .location_for_path(&path, app)
                                    })
                                    .collect::<Option<Vec<_>>>();
                                let skill = file_locations.and_then(|file_locations| {
                                    parsed_skill_for_common_locations(file_locations, app)
                                });
                                output_items.add_child(render_read_files(
                                    props,
                                    id,
                                    file_names.iter(),
                                    app,
                                    skill,
                                    action_index,
                                ));
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::RequestFileEdits { .. },
                            id,
                            ..
                        }) => {
                            let action_status =
                                props.action_model.as_ref(app).get_action_status(id);

                            let is_preprocessing = action_status
                                .clone()
                                .is_some_and(|status| status.is_preprocessing());
                            if !is_preprocessing && !status.is_streaming() {
                                if let Some(requested_edit) = props.requested_edits.get(id) {
                                    // Don't render the requested edit if the diffs are empty for passive code diffs.
                                    if request_type.is_passive_code_diff()
                                        && requested_edit.view.as_ref(app).is_pending_diffs_empty()
                                    {
                                        continue;
                                    }

                                    output_items.add_child(render_requested_edits_output_message(
                                        requested_edit,
                                        action_status,
                                        request_type.is_passive_code_diff(),
                                        app,
                                    ));
                                }
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::Grep { queries, path },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            output_items.add_child(render_file_retrieval_tool(
                                props,
                                id,
                                create_formatted_text_for_grep(props, id, queries, path, app),
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::FileGlob { patterns, path },
                            id,
                            ..
                        })
                        | AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::FileGlobV2 {
                                    patterns,
                                    search_dir: path,
                                },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            output_items.add_child(render_file_retrieval_tool(
                                props,
                                id,
                                create_formatted_text_for_file_glob(
                                    props,
                                    id,
                                    patterns,
                                    path.as_deref(),
                                    app,
                                ),
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::ReadMCPResource {
                                    server_id: _,
                                    name,
                                    uri,
                                },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            let name = uri.as_ref().unwrap_or(name);
                            output_items.add_child(render_read_mcp_resource(props, id, name, app));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::CallMCPTool { .. },
                            id,
                            ..
                        }) => {
                            // Since we're rendering an MCP tool call, it will
                            // render the citations so don't render them again.
                            should_render_references_section = false;

                            let is_action_done = props
                                .action_model
                                .as_ref(app)
                                .get_action_status(id)
                                .as_ref()
                                .is_some_and(|status| status.is_done());
                            if !is_action_done {
                                // Ratings & suggestions should not be rendered for MCP tool call actions that are not complete.
                                should_render_footer = false;
                                should_render_suggestions = false;
                            }

                            if let Some(rendered_mcp_tool) = props
                                .requested_mcp_tools
                                .get(id)
                                .map(|requested_mcp_tool| requested_mcp_tool.render())
                            {
                                output_items.add_child(rendered_mcp_tool);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::AskUserQuestion { .. },
                            id,
                            ..
                        }) if FeatureFlag::AskUserQuestion.is_enabled() => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            if let Some(rendered_ask_user_question) =
                                render_ask_user_question(id, props, app)
                            {
                                output_items.add_child(rendered_ask_user_question);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::SuggestNewConversation { .. },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            if let Some(rendered_conversation) =
                                render_suggest_new_conversation(id, props, appearance, app)
                            {
                                output_items.add_child(rendered_conversation);
                            }
                        }
                        AIAgentOutputMessageType::CommentsAddressed { comments } => {
                            should_render_suggestions = false;
                            for comment in comments {
                                output_items
                                    .add_child(render_comment_addressed_header(comment, app));
                            }
                        }
                        AIAgentOutputMessageType::TodoOperation(todo) => match todo {
                            TodoOperation::UpdateTodos { todos } if !todos.is_empty() => {
                                if let Some(conversation) = props.model.conversation(app) {
                                    if let Some(state) =
                                        props.todo_list_states.get(&output_message.id)
                                    {
                                        output_items.add_child(render_todos(
                                            &output_message.id,
                                            todos,
                                            conversation,
                                            state,
                                            app,
                                        ));
                                    }
                                }
                            }
                            TodoOperation::MarkAsCompleted { completed_todos } => {
                                if let Some(completed_text) = render_completed_todo_items(
                                    completed_todos,
                                    props.current_todo_list,
                                    app,
                                ) {
                                    output_items.add_child(completed_text);
                                }
                            }
                            _ => (),
                        },
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::SuggestPrompt(
                                    SuggestPromptRequest::UnitTestsSuggestion { .. },
                                ),
                            id,
                            ..
                        }) => {
                            if let Some(unit_test_suggestion_view) =
                                props.unit_test_suggestions.get(id)
                            {
                                if !unit_test_suggestion_view.as_ref(app).is_hidden() {
                                    output_items.add_child(render_unit_test_suggestion(
                                        unit_test_suggestion_view,
                                        app,
                                    ));
                                }
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::CreateDocuments { .. },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            if let Some(create_document) =
                                maybe_render_create_document(props, id, app)
                            {
                                output_items.add_child(create_document);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::EditDocuments { .. },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            if let Some(edit_document) = maybe_render_edit_document(props, id, app)
                            {
                                output_items.add_child(edit_document);
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::UseComputer(request),
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items.add_child(render_use_computer(
                                props,
                                id,
                                request,
                                recording_spans_by_action_id.get(id),
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::StartRecording { summary, .. },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items.add_child(render_start_recording(
                                props,
                                id,
                                summary.as_deref(),
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::StopRecording { .. },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items.add_child(render_stop_recording(props, id, app));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::ReadSkill(request),
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items.add_child(render_read_skill(
                                props,
                                id,
                                &request.skill,
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::UploadArtifact(request),
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items.add_child(render_upload_artifact(props, id, request, app));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::RequestComputerUse(request),
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            output_items
                                .add_child(render_request_computer_use(props, id, request, app));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::StartAgent {
                                    version: _,
                                    name,
                                    prompt,
                                    execution_mode,
                                    lifecycle_subscription: _,
                                },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            output_items.add_child(orchestration::render_start_agent(
                                props,
                                id,
                                name,
                                prompt,
                                execution_mode,
                                &output_message.id,
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::RunAgents(_req),
                            id,
                            ..
                        }) => {
                            // Embed the per-action `RunAgentsCardView`
                            // via `ChildView`. The view renders a
                            // "Configuring agents..." placeholder while
                            // streaming, then transitions to the full
                            // confirmation card once complete.
                            should_render_footer = false;
                            should_render_suggestions = false;
                            if let Some(card_view) = props.run_agents_card_views.get(id) {
                                output_items.add_child(ChildView::new(card_view).finish());
                            }
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action:
                                AIAgentActionType::SendMessageToAgent {
                                    addresses,
                                    subject,
                                    message,
                                },
                            id,
                            ..
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            output_items.add_child(orchestration::render_send_message(
                                props,
                                id,
                                addresses,
                                subject,
                                message,
                                &output_message.id,
                                app,
                            ));
                        }
                        AIAgentOutputMessageType::Action(AIAgentAction {
                            action: AIAgentActionType::InsertCodeReviewComments { repo_path, .. },
                            id,
                            ..
                        }) if FeatureFlag::PRCommentsV2.is_enabled() => {
                            if let Some(group) = props.imported_comments.get(id) {
                                output_items.add_child(
                                    render_imported_comments(group, app)
                                        .with_agent_output_item_spacing(app)
                                        .finish(),
                                );
                            }
                        }
                        AIAgentOutputMessageType::Summarization {
                            text,
                            finished_duration,
                            summarization_type,
                            ..
                        } if matches!(
                            summarization_type,
                            SummarizationType::ConversationSummary
                        ) && !are_all_text_sections_empty(&text.sections) =>
                        {
                            let header_text = "Conversation summarized".to_string();
                            if let Some(element) = render_collapsible_block(
                                output_message,
                                header_text,
                                &text.sections,
                                finished_duration.is_some(),
                                props,
                                &mut has_rendered_first_text_section,
                                &mut text_section_index,
                                &mut code_section_index,
                                app,
                            ) {
                                output_items.add_child(element);
                            }
                        }
                        AIAgentOutputMessageType::WebSearch(web_search_status) => {
                            if !FeatureFlag::WebSearchUI.is_enabled() {
                                continue;
                            }

                            // Render the WebSearch inline at its first position in the message stream
                            if let Some(web_search_view) =
                                props.web_search_views.get(&output_message.id)
                            {
                                output_items.add_child(ChildView::new(web_search_view).finish());
                            } else {
                                // No view yet, log warning
                                log::warn!(
                                    "[WebSearch] No view found for WebSearch message id={:?}, status={web_search_status:?}",
                                    output_message.id
                                );
                            }
                        }
                        AIAgentOutputMessageType::WebFetch(web_fetch_status) => {
                            if !FeatureFlag::WebFetchUI.is_enabled() {
                                continue;
                            }

                            // Render the WebFetch inline at its first position in the message stream
                            if let Some(web_fetch_view) =
                                props.web_fetch_views.get(&output_message.id)
                            {
                                output_items.add_child(ChildView::new(web_fetch_view).finish());
                            } else {
                                // No view yet, log warning
                                log::warn!(
                                    "[WebFetch] No view found for WebFetch message id={:?}, status={web_fetch_status:?}",
                                    output_message.id
                                );
                            }
                        }
                        AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                            output_items.add_child(
                                orchestration::render_messages_received_from_agents(
                                    messages, props, app,
                                ),
                            );
                        }
                        AIAgentOutputMessageType::DebugOutput { text } => {
                            if ChannelState::enable_debug_features() {
                                if let Some(element) = render_collapsible_debug_output(
                                    output_message,
                                    text,
                                    props,
                                    app,
                                ) {
                                    output_items.add_child(element);
                                }
                            }
                        }
                        AIAgentOutputMessageType::Subagent(SubagentCall {
                            subagent_type:
                                SubagentType::ConversationSearch {
                                    ref query,
                                    ref conversation_id,
                                    ref agent_run_id,
                                },
                            task_id: subagent_task_id,
                        }) => {
                            should_render_footer = false;
                            should_render_suggestions = false;
                            let conversation = props.model.conversation(app);
                            let is_finished = conversation
                                .and_then(|c| {
                                    c.is_subagent_task_finished(&TaskId::new(
                                        subagent_task_id.clone(),
                                    ))
                                    .ok()
                                })
                                .unwrap_or(false);
                            let is_cancelled = conversation.is_some_and(|c| {
                                let subagent_task_id = TaskId::new(subagent_task_id.clone());
                                c.get_task(&subagent_task_id).is_some_and(|task| {
                                    task.exchanges().any(|e| e.output_status.is_cancelled())
                                })
                            });
                            let icon = if is_cancelled {
                                inline_action_icons::cancelled_icon(appearance)
                            } else if is_finished {
                                inline_action_icons::green_check_icon(appearance)
                            } else {
                                icons::yellow_running_icon(appearance)
                            };

                            // Resolve which conversation is being searched. Conversation IDs use
                            // conversation history titles; agent run IDs use ambient task titles
                            // once fetched. If neither target exists, label it "this conversation".
                            let target_label = conversation_id
                                .as_ref()
                                .and_then(|target_id| {
                                    let history = BlocklistAIHistoryModel::as_ref(app);
                                    let token = ServerConversationToken::new(target_id.clone());
                                    let local_id =
                                        history.find_conversation_id_by_server_token(&token);
                                    // If the target resolves to the current conversation,
                                    // show "this conversation" instead.
                                    let is_current = local_id.is_some_and(|id| {
                                        conversation.is_some_and(|c| c.id() == id)
                                    });
                                    if is_current {
                                        return None;
                                    }
                                    let target_conversation =
                                        local_id.and_then(|id| history.conversation(&id));
                                    let title = target_conversation
                                        .and_then(|c| c.title())
                                        .map(|q| truncate_from_end(&q, 40));
                                    Some((
                                        "conversation",
                                        title.unwrap_or_else(|| target_id.clone()),
                                    ))
                                })
                                .or_else(|| {
                                    let target_id = agent_run_id.as_ref()?;
                                    let title = target_id
                                        .parse::<AmbientAgentTaskId>()
                                        .ok()
                                        .and_then(|task_id| {
                                            AgentConversationsModel::as_ref(app)
                                                .get_task_data(&task_id)
                                        })
                                        .map(|task| truncate_from_end(&task.title, 40));
                                    Some((
                                        "agent run",
                                        title.unwrap_or_else(|| truncate_from_end(target_id, 40)),
                                    ))
                                });

                            let done = is_finished || is_cancelled;
                            let verb = if done { "Searched" } else { "Searching" };

                            let mut fragments: Vec<FormattedTextFragment> =
                                vec![FormattedTextFragment::plain_text(format!("{verb} "))];
                            match &target_label {
                                Some((target_kind, name)) => {
                                    fragments.push(FormattedTextFragment::plain_text(format!(
                                        "{target_kind} "
                                    )));
                                    fragments.push(FormattedTextFragment::weighted(
                                        name.as_str(),
                                        Some(markdown_parser::weight::CustomWeight::Bold),
                                    ));
                                }
                                None => {
                                    fragments.push(FormattedTextFragment::plain_text(
                                        "this conversation",
                                    ));
                                }
                            };
                            match query {
                                Some(q) => {
                                    fragments
                                        .push(FormattedTextFragment::plain_text(format!(": {q}")));
                                }
                                None if !done => {
                                    fragments.push(FormattedTextFragment::plain_text("..."));
                                }
                                None => {}
                            };

                            let body_color = blended_colors::text_main(
                                appearance.theme(),
                                appearance.theme().background(),
                            );
                            let formatted = FormattedTextElement::new(
                                FormattedText::new(vec![FormattedTextLine::Line(fragments)]),
                                appearance.monospace_font_size(),
                                appearance.ui_font_family(),
                                appearance.ui_font_family(),
                                body_color,
                                Default::default(),
                            )
                            .set_selectable(true);

                            let mut action =
                                RenderableAction::new_with_formatted_text(formatted, app)
                                    .with_icon(icon.finish());

                            // Add a footer with the current phase status when in progress.
                            if !is_finished && !is_cancelled {
                                let phase = conversation
                                    .and_then(|c| {
                                        c.get_task(&TaskId::new(subagent_task_id.clone()))
                                    })
                                    .map(conversation_search_phase)
                                    .unwrap_or(ConversationSearchPhase::ListingMessages);
                                let phase_text = format_conversation_search_phase(&phase);
                                let theme = appearance.theme();
                                let icon_offset = icon_size(app)
                                    + crate::ai::blocklist::inline_action::inline_action_header::ICON_MARGIN;
                                let footer = Container::new(
                                    Text::new(
                                        phase_text,
                                        appearance.ui_font_family(),
                                        appearance.ui_font_size(),
                                    )
                                    .with_color(theme.sub_text_color(theme.surface_1()).into())
                                    .finish(),
                                )
                                .with_margin_left(icon_offset)
                                .finish();
                                action = action.with_footer(footer);
                            }

                            output_items.add_child(action.render(app).finish());
                        }
                        _ => (),
                    };
                    if let AIAgentOutputMessageType::Action(..) = output_message.message {
                        action_index += 1;
                    }
                }

                // Only render suggested rules and prompts if the response is complete.
                if should_render_suggestions && FeatureFlag::SuggestedRules.is_enabled() {
                    if let Some(suggestions) = render_suggested_rules_and_prompts_footer(props, app)
                    {
                        output_items.add_child(suggestions);
                    }
                }

                if should_render_references_section {
                    let exchange_id = props.model.exchange_id(app);
                    let memory_citations: Vec<AIAgentCitation> = props
                        .model
                        .conversation(app)
                        .filter(|conv| {
                            // Only show memory citations on the first exchange.
                            conv.first_exchange().map(|e| Some(e.id)) == Some(exchange_id)
                        })
                        .into_iter()
                        .flat_map(|conv| conv.fetched_memories())
                        .filter(|m| !m.memory_store_id.is_empty() && !m.memory_id.is_empty())
                        .map(|m| AIAgentCitation::AgentMemory {
                            memory_store_id: m.memory_store_id.clone(),
                            memory_id: m.memory_id.clone(),
                            content: m.content.clone(),
                        })
                        .collect();
                    let all_citations: Vec<AIAgentCitation> = output
                        .citations
                        .iter()
                        .cloned()
                        .chain(memory_citations)
                        .collect();
                    if let Some(references) = render_references_footer(&all_citations, props, app) {
                        output_items.add_child(references);
                    }
                }

                if let Some(footer) = should_render_footer
                    .then(|| render_response_footer(props, app))
                    .flatten()
                {
                    output_items.add_child(footer);
                }

                if let Some(request_refunded_count) = props.request_refunded_count {
                    match request_refunded_count.cmp(&1) {
                        Ordering::Equal | Ordering::Less => {
                            output_items.add_child(
                                render_informational_footer(
                                    app,
                                    "Sorry you had a bad experience with this interaction. We've refunded you 1 credit. We appreciate your feedback!"
                                        .to_string(),
                                )
                                .with_agent_output_item_spacing(app)
                                .finish(),
                            );
                        }
                        Ordering::Greater => {
                            output_items.add_child(
                                render_informational_footer(
                                    app,
                                    format!(
                                        "Sorry you had a bad experience with this interaction. We've refunded you {request_refunded_count} credits. We appreciate your feedback!"
                                    ),
                                )
                                .with_agent_output_item_spacing(app)
                                .finish(),
                            );
                        }
                    }
                }
            }
        }
    }

    if request_type.is_active() {
        if let AIBlockOutputStatus::Failed { error, .. } = &status {
            // While an automatic resume is still in flight, keep the failed exchange
            // quiet: skip the error banner, the "won't count towards usage" notice, and
            // the debug footer. The full failure UI is surfaced only once recovery has
            // actually failed. Dogfood builds (Local/Dev) opt out so developers still see
            // every transport failure aggressively.
            if !error.should_suppress_during_recovery() {
                output_items.add_child(
                    render_failed_output(
                        FailedOutputProps {
                            error,
                            is_ai_input_enabled: props.is_ai_input_enabled,
                            invalid_api_key_button_handle: &props
                                .state_handles
                                .invalid_api_key_button_handle,
                            subscribe_button_handle: &props.state_handles.subscribe_button_handle,
                            aws_bedrock_credentials_error_view: props
                                .aws_bedrock_credentials_error_view,
                            icon_right_margin: 16.,
                        },
                        app,
                    )
                    .with_content_item_spacing()
                    .finish(),
                );

                if props.model.is_latest_visible_exchange_in_root_task(app)
                    && !has_expanded_last_requested_command
                    && !props.model.is_restored()
                    && !error.is_invalid_api_key()
                {
                    output_items.add_child(
                        render_informational_footer(
                            app,
                            "This response won't count towards your usage.".to_string(),
                        )
                        .with_agent_output_item_spacing(app)
                        .finish(),
                    );

                    output_items.add_child(
                        render_debug_footer(
                            DebugFooterProps {
                                conversation: props.model.conversation(app),
                                model: props.model,
                                debug_copy_button_handle: props
                                    .state_handles
                                    .debug_copy_button_handle
                                    .clone(),
                                submit_issue_button_handle: props
                                    .state_handles
                                    .submit_issue_button_handle
                                    .clone(),
                                should_render_feedback_below: false,
                            },
                            |debug_id, ctx| {
                                ctx.dispatch_typed_action(AIBlockAction::CopyDebugId(debug_id))
                            },
                            |ctx| ctx.dispatch_typed_action(AIBlockAction::OpenFeedbackDocs),
                            app,
                        )
                        .with_agent_output_item_spacing(app)
                        .finish(),
                    );
                }
            }
        }
    }

    if should_render_stopped_output(props, app) {
        output_items.add_child(render_stopped_output(props, app))
    }

    output_items.finish()
}

fn should_render_stopped_output(props: Props, app: &AppContext) -> bool {
    if FeatureFlag::AgentView.is_enabled() {
        return false;
    }

    let request_type = props.model.request_type(app);
    if request_type.is_passive_code_diff() {
        return false;
    }

    let status = props.model.status(app);
    let cancellation_reason = status.cancellation_reason().cloned();
    // Reasons that keep the conversation alive (follow-ups, CLI-subagent takeover)
    // or finalize it as a success (optimistic command completion, revert) must not
    // render a stopped banner.
    if cancellation_reason.is_some_and(|reason| {
        matches!(
            reason.conversation_outcome(),
            CancellationOutcome::KeepInProgress | CancellationOutcome::Succeeded
        )
    }) {
        return false;
    }

    let has_expanded_requested_command = props
        .requested_commands
        .values()
        .any(|requested_command| requested_command.view.as_ref(app).is_header_expanded());
    // Expanded requested commands would appear after the stopped task UI, which we don't want.
    if has_expanded_requested_command {
        return false;
    }

    let is_current_exchange_empty = status
        .output_to_render()
        .is_none_or(|output| output.get().messages.is_empty());

    let is_resumed_conversation = props
        .model
        .inputs_to_render(app)
        .iter()
        .last()
        .is_some_and(|input| matches!(input, AIAgentInput::ResumeConversation { .. }));

    // When the user resumes a conversation, and cancels before any follow-up output,
    // we should avoid showing a stopped banner. Otherwise the user can stack
    // stopped banners by toggling stop and resume.
    if is_current_exchange_empty && is_resumed_conversation {
        return false;
    }

    props.finish_reason.is_some_and(|finish_reason| {
        *finish_reason == FinishReason::CancelledDuringRequestedCommandExecution
    }) || cancellation_reason.is_some()
}

// Helper function to style a requested action with standard styling when streaming and action blocked on user
fn renderable_action(
    props: Props,
    id: &AIAgentActionId,
    text: &str,
    app: &AppContext,
    footer: Option<Box<dyn Element>>,
    appearance: &Appearance,
    status: Option<&AIActionStatus>,
) -> RenderableAction {
    let mut requested_action = RenderableAction::new(text, app);
    let is_blocked_on_user = status.as_ref().is_some_and(|s| s.is_blocked());
    if is_blocked_on_user {
        requested_action =
            requested_action.with_background_color(appearance.theme().background().into_solid())
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            requested_action = requested_action.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        requested_action = requested_action
            .with_icon(action_icon(id, props.action_model, props.model, app).finish());
    }

    if let Some(footer) = footer {
        requested_action = requested_action.with_footer(footer);
    }
    requested_action
}

fn render_search_codebase(
    props: Props,
    id: &AIAgentActionId,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let status = props.action_model.as_ref(app).get_action_status(id);
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let footer = match props.autonomy_setting_speedbump {
        AutonomySettingSpeedbump::ShouldShowForCodebaseSearchFileAccess {
            action_id,
            shown,
            ..
        } if action_id == id => {
            *shown.lock() = true;
            Some(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(
                        appearance
                            .ui_builder()
                            .radio_buttons(
                                props
                                    .state_handles
                                    .codebase_search_speedbump_option_handles
                                    .clone(),
                                vec![
                                    RadioButtonItem::text(
                                        "Always allow file access for coding tasks",
                                    ),
                                    RadioButtonItem::text("Always allow file access for this repo"),
                                ],
                                props
                                    .state_handles
                                    .codebase_search_speedbump_radio_button_handle
                                    .clone(),
                                None,
                                appearance.ui_font_size(),
                                RadioButtonLayout::Row,
                            )
                            .with_style(UiComponentStyles {
                                font_color: Some(blended_colors::text_sub(
                                    theme,
                                    theme.surface_1(),
                                )),
                                font_size: Some(appearance.monospace_font_size() - 1.),
                                padding: Some(Coords::default()),
                                margin: Some(Coords {
                                    top: 4.,
                                    bottom: 4.,
                                    right: 16.,
                                    left: 0.,
                                }),
                                ..Default::default()
                            })
                            .with_button_diameter(appearance.monospace_font_size() - 1.)
                            .on_change(Rc::new(move |ctx, _, index| {
                                ctx.dispatch_typed_action(
                                    AIBlockAction::ToggleCodebaseSearchSpeedbump(index),
                                );
                            }))
                            .supports_unselected_state()
                            .build()
                            .finish(),
                    )
                    .with_child(
                        Expanded::new(
                            1.,
                            Align::new(
                                appearance
                                    .ui_builder()
                                    .link(
                                        "Manage AI Autonomy permissions".into(),
                                        None,
                                        Some(Box::new(move |ctx| {
                                            ctx.dispatch_typed_action(
                                                WorkspaceAction::ShowSettingsPageWithSearch {
                                                    search_query: "Autonomy".to_string(),
                                                    section: Some(SettingsSection::WarpAgent),
                                                },
                                            );
                                        })),
                                        props
                                            .state_handles
                                            .manage_autonomy_settings_link_handle
                                            .clone(),
                                    )
                                    .build()
                                    .finish(),
                            )
                            .right()
                            .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
        }
        _ => None,
    };

    let root_repo_path = props
        .action_model
        .as_ref(app)
        .search_codebase_executor(app)
        .as_ref(app)
        .root_repo_for_action(id);

    let requested_action = match status.as_ref() {
        Some(status) => match status {
            AIActionStatus::Preprocessing | AIActionStatus::Queued => {
                match props.search_codebase_view.get(id) {
                    Some(search_codebase_view) if FeatureFlag::SearchCodebaseUI.is_enabled() => {
                        ChildView::new(search_codebase_view).finish()
                    }
                    _ => {
                        let root_repo_path = root_repo_path?;
                        renderable_action(
                            props,
                            id,
                            format!("Search in {}", root_repo_path.to_string_lossy()).as_str(),
                            app,
                            footer,
                            appearance,
                            Some(status),
                        )
                        .render(app)
                        .finish()
                    }
                }
            }
            AIActionStatus::Blocked => {
                let root_repo_path = root_repo_path?;

                let buttons = props
                    .action_buttons
                    .get(id)
                    .expect("Button states must exist for each requested action.");

                renderable_action(
                    props,
                    id,
                    &root_repo_path.to_string_lossy(),
                    app,
                    footer,
                    appearance,
                    Some(status),
                )
                .with_header(blocked_action_header(
                    id.clone(),
                    BLOCKED_ACTION_MESSAGE_FOR_SEARCHING_CODEBASE,
                    buttons.run_button.clone(),
                    buttons.cancel_button.clone(),
                    props.action_model,
                    props.model,
                    app,
                ))
                .with_highlighted_border()
                .render(app)
                .finish()
            }
            AIActionStatus::RunningAsync => match props.search_codebase_view.get(id) {
                Some(search_codebase_view) if FeatureFlag::SearchCodebaseUI.is_enabled() => {
                    ChildView::new(search_codebase_view).finish()
                }
                _ => {
                    let root_repo_path = root_repo_path?;
                    renderable_action(
                        props,
                        id,
                        format!("Searching in {}", root_repo_path.to_string_lossy()).as_str(),
                        app,
                        footer,
                        appearance,
                        Some(status),
                    )
                    .render(app)
                    .finish()
                }
            },
            AIActionStatus::Finished(result) => match props.search_codebase_view.get(id) {
                Some(search_codebase_view) if FeatureFlag::SearchCodebaseUI.is_enabled() => {
                    ChildView::new(search_codebase_view).finish()
                }
                _ => {
                    let AIAgentActionResultType::SearchCodebase(search_codebase_result) =
                        &result.result
                    else {
                        return None;
                    };
                    match search_codebase_result {
                        SearchCodebaseResult::Success { files } => {
                            if files.is_empty() {
                                renderable_action(
                                    props,
                                    id,
                                    "No relevant files found.",
                                    app,
                                    footer,
                                    appearance,
                                    Some(status),
                                )
                                .render(app)
                                .finish()
                            } else {
                                let file_locations = files
                                    .iter()
                                    .map(|file| {
                                        props
                                            .active_session
                                            .as_ref(app)
                                            .location_for_path(&file.file_name, app)
                                    })
                                    .collect::<Option<Vec<_>>>();
                                let skill = file_locations.and_then(|file_locations| {
                                    parsed_skill_for_common_locations(file_locations, app)
                                });
                                let grouped = group_file_contexts_for_display(files, None, None);
                                return Some(render_read_files(
                                    props,
                                    id,
                                    grouped.iter(),
                                    app,
                                    skill,
                                    0,
                                ));
                            }
                        }
                        SearchCodebaseResult::Failed { reason, .. } => {
                            let root_repo_path = root_repo_path?;
                            let message = match reason {
                                SearchCodebaseFailureReason::CodebaseNotIndexed => format!(
                                    "Search in {} failed because the codebase isn't indexed",
                                    root_repo_path.to_string_lossy(),
                                ),
                                _ => {
                                    format!("Search in {} failed", root_repo_path.to_string_lossy())
                                }
                            };
                            renderable_action(
                                props,
                                id,
                                message.as_str(),
                                app,
                                footer,
                                appearance,
                                Some(status),
                            )
                            .render(app)
                            .finish()
                        }
                        SearchCodebaseResult::Cancelled => {
                            let root_repo_path = root_repo_path?;
                            renderable_action(
                                props,
                                id,
                                format!("Search in {} cancelled", root_repo_path.to_string_lossy())
                                    .as_str(),
                                app,
                                footer,
                                appearance,
                                Some(status),
                            )
                            .render(app)
                            .finish()
                        }
                    }
                }
            },
        },
        None => {
            let root_repo_path = root_repo_path?;
            renderable_action(
                props,
                id,
                format!("Search in {}", root_repo_path.to_string_lossy()).as_str(),
                app,
                footer,
                appearance,
                None,
            )
            .render(app)
            .finish()
        }
    };
    Some(requested_action)
}

pub struct LinkActionConstructors<A: Action> {
    pub construct_open_link_action: fn(std::ops::Range<usize>, TextLocation) -> A,
    pub construct_open_link_tooltip_action: fn(std::ops::Range<usize>, TextLocation) -> A,
    pub construct_changed_hover_on_link_action: fn(std::ops::Range<usize>, TextLocation, bool) -> A,
}

impl<A: Action> Clone for LinkActionConstructors<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Action> Copy for LinkActionConstructors<A> {}

impl<A: Action> LinkActionConstructors<A> {
    pub fn build_ai_block_action() -> LinkActionConstructors<AIBlockAction> {
        LinkActionConstructors {
            construct_open_link_action: |link_range, location| AIBlockAction::OpenLink {
                link_range,
                location,
            },
            construct_open_link_tooltip_action: |link_range, location| {
                AIBlockAction::OpenLinkTooltip {
                    link_range,
                    location,
                }
            },
            construct_changed_hover_on_link_action: |link_range, location, is_hovering| {
                AIBlockAction::ChangedHoverOnLink {
                    link_range,
                    location,
                    is_hovering,
                }
            },
        }
    }
}
pub struct RenderContext<'a> {
    pub shell_launch_data: Option<&'a ShellLaunchData>,
    pub current_working_directory: Option<&'a String>,
    pub detected_links_state: &'a DetectedLinksState,
    pub secret_redaction_state: &'a SecretRedactionState,
}

pub struct RenderReadFileArg<'a, A: Action> {
    render_context: RenderContext<'a>,
    find_context: Option<FindContext<'a>>,
    is_selecting_text: bool,
    link_actions: LinkActionConstructors<A>,
}

impl<'a, A: Action> RenderReadFileArg<'a, A> {
    pub fn new(
        render_context: RenderContext<'a>,
        find_context: Option<FindContext<'a>>,
        is_selecting_text: bool,
        link_actions: LinkActionConstructors<A>,
    ) -> Self {
        Self {
            render_context,
            find_context,
            is_selecting_text,
            link_actions,
        }
    }
}

impl<'a> From<Props<'a>> for RenderReadFileArg<'a, AIBlockAction> {
    fn from(val: Props<'a>) -> Self {
        Self {
            render_context: RenderContext {
                shell_launch_data: val.shell_launch_data,
                current_working_directory: val.current_working_directory,
                detected_links_state: val.detected_links_state,
                secret_redaction_state: val.secret_redaction_state,
            },
            find_context: val.find_context,
            is_selecting_text: val.is_selecting_text,
            link_actions: LinkActionConstructors::<AIBlockAction>::build_ai_block_action(),
        }
    }
}

pub fn render_read_files_text<A: Action>(
    render_read_file_args: RenderReadFileArg<A>,
    file_names: impl IntoIterator<Item = impl AsRef<str>>,
    app: &AppContext,
    appearance: &Appearance,
    action_index: usize,
) -> FormattedTextElement {
    let theme = appearance.theme();

    let file_names = file_names
        .into_iter()
        .map(|name| {
            shell_native_absolute_path(
                name.as_ref(),
                render_read_file_args.render_context.shell_launch_data,
                render_read_file_args
                    .render_context
                    .current_working_directory,
            )
        })
        .collect_vec()
        .join("\n");
    let mut formatted_files = render_requested_action_body_text(
        file_names.as_str().into(),
        appearance.ui_font_family(),
        app,
    );

    // Registering handlers for link detection
    formatted_files.register_handlers(move |mut frame, (line_index, _)| {
        let location = TextLocation::Action {
            action_index,
            line_index,
        };
        frame = add_link_detection_mouse_interactions(
            frame,
            render_read_file_args.render_context.detected_links_state,
            render_read_file_args.link_actions,
            location,
        );
        frame
    });

    // Add hover highlighting and click interactions to any detected links in the text
    formatted_files = add_highlights_to_rich_text(
        formatted_files,
        Some(render_read_file_args.render_context.detected_links_state),
        render_read_file_args.render_context.secret_redaction_state,
        render_read_file_args.find_context,
        action_index,
        file_names.lines().count(),
        theme,
        render_read_file_args.is_selecting_text,
        true,
        app,
    );
    formatted_files
}

/// Returns the display text for a `read_skill` action.
///
/// When the skill is found in the manager, formats it as a slash command
/// (e.g. `/hello-world`). When the skill is unknown, falls back to the
/// raw reference string (e.g. the path) **without** prepending an extra
/// `/`, which would otherwise produce paths like `//home/user/…`.
fn read_skill_display_text(
    skill: Option<&ParsedSkill>,
    skill_reference: &SkillReference,
) -> String {
    skill
        .map(|s| format!("/{}", s.name))
        .unwrap_or_else(|| skill_reference.display_label())
}

fn render_read_skill(
    props: Props,
    id: &AIAgentActionId,
    skill_reference: &SkillReference,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let skill = SkillManager::as_ref(app).skill_by_reference(skill_reference);

    let formatted_text = render_requested_action_body_text(
        read_skill_display_text(skill, skill_reference).into(),
        appearance.monospace_font_family(),
        app,
    );

    let mut renderable_action = RenderableAction::new_with_formatted_text(formatted_text, app);
    renderable_action =
        renderable_action.with_icon(action_icon(id, props.action_model, props.model, app).finish());

    // Renders the 'open skill' button for known, non-bundled skills.
    if let Some(skill) = skill {
        if !skill.is_bundled() {
            if let Some(button_handle) = props.state_handles.skill_button_handles.get(id).cloned() {
                let source = CodeSource::Skill {
                    reference: skill_reference.clone(),
                    location: skill.path.clone(),
                    origin: SkillOpenOrigin::ReadSkill,
                };

                let skill_icon_override = icon_override_for_skill_name(&skill.name);
                let open_button = render_skill_button(
                    "Open skill",
                    button_handle,
                    appearance,
                    skill.provider,
                    skill_icon_override,
                    move |ctx| {
                        ctx.dispatch_typed_action(AIBlockAction::OpenCodeInWarp {
                            source: source.clone(),
                        });
                    },
                );

                renderable_action = renderable_action.with_action_button(open_button);
            }
        }
    }

    renderable_action.render(app).finish()
}

/// Renders the small secondary button placed on the trailing edge of a
/// [`RenderableAction`] row (e.g. "View screenshot", "Open recording").
fn render_inline_action_secondary_button(
    appearance: &Appearance,
    button: &ui_components::button::Button,
    label: &'static str,
    on_click: ui_components::MouseEventHandler,
) -> Box<dyn Element> {
    button.render(
        appearance,
        button::Params {
            content: button::Content::Label(label.into()),
            theme: &button::themes::Secondary,
            options: button::Options {
                size: button::Size::Small,
                on_click: Some(on_click),
                ..button::Options::default(appearance)
            },
        },
    )
}

fn render_read_files(
    props: Props,
    id: &AIAgentActionId,
    file_names: impl IntoIterator<Item = impl AsRef<str>>,
    app: &AppContext,
    parsed_skill: Option<&ai::skills::ParsedSkill>,
    action_index: usize,
) -> Box<dyn Element> {
    let status = props.action_model.as_ref(app).get_action_status(id);
    let appearance = Appearance::as_ref(app);
    let formatted_files =
        render_read_files_text(props.into(), file_names, app, appearance, action_index);

    let mut renderable_action = RenderableAction::new_with_formatted_text(formatted_files, app);

    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        let buttons = props
            .action_buttons
            .get(id)
            .expect("Button states must exist for each requested action.");

        renderable_action = renderable_action
            .with_header(blocked_action_header(
                id.clone(),
                BLOCKED_ACTION_MESSAGE_FOR_READING_FILES,
                buttons.run_button.clone(),
                buttons.cancel_button.clone(),
                props.action_model,
                props.model,
                app,
            ))
            .with_highlighted_border()
            .with_background_color(appearance.theme().background().into_solid());
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            renderable_action = renderable_action.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        renderable_action = renderable_action
            .with_icon(action_icon(id, props.action_model, props.model, app).finish());
    }

    match props.autonomy_setting_speedbump {
        AutonomySettingSpeedbump::ShouldShowForFileAccess {
            checked,
            action_id,
            shown,
        } if action_id == id => {
            *shown.lock() = true;
            renderable_action =
                renderable_action.with_footer(render_autonomy_checkbox_setting_speedbump_footer(
                    "Always allow file access for coding tasks",
                    *checked,
                    AIBlockAction::ToggleAutoreadFilesSpeedbumpCheckbox,
                    props
                        .state_handles
                        .autoread_files_speedbump_checkbox_handle
                        .clone(),
                    props
                        .state_handles
                        .manage_autonomy_settings_link_handle
                        .clone(),
                    app,
                ));
        }
        _ => (),
    };

    // Renders the 'open skill' button if all files belong to the same skill directory.
    if let Some(skill) = parsed_skill {
        if let Some(button_handle) = props.state_handles.skill_button_handles.get(id).cloned() {
            let reference = SkillManager::handle(app)
                .as_ref(app)
                .reference_for_skill_path(&skill.path);
            let source = CodeSource::Skill {
                reference,
                location: skill.path.clone(),
                origin: SkillOpenOrigin::ReadFiles,
            };
            let skill_icon_override = icon_override_for_skill_name(&skill.name);
            let open_button = render_skill_button(
                &format!("/{}", skill.name),
                button_handle,
                appearance,
                skill.provider,
                skill_icon_override,
                move |ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::OpenCodeInWarp {
                        source: source.clone(),
                    });
                },
            );
            renderable_action = renderable_action.with_action_button(open_button);
        }
    }

    renderable_action.render(app).finish()
}

fn parsed_skill_for_common_locations(
    file_locations: impl IntoIterator<Item = LocalOrRemotePath>,
    app: &AppContext,
) -> Option<&ai::skills::ParsedSkill> {
    let skill_paths = file_locations
        .into_iter()
        .map(|location| skill_path_from_location(&location))
        .collect::<Option<Vec<_>>>()?;
    let first_skill_path = skill_paths.first()?;
    skill_paths
        .iter()
        .all(|skill_path| skill_path == first_skill_path)
        .then(|| SkillManager::as_ref(app).skill_by_path(first_skill_path))
        .flatten()
}

fn maybe_render_edit_document(
    props: Props,
    id: &AIAgentActionId,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let status = props.action_model.as_ref(app).get_action_status(id);

    // Document operations are always auto-executed for now
    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        todo!("Implement granular permissions for AI documents.");
    }

    let agent_action_results = props
        .action_model
        .as_ref(app)
        .get_action_result(id)
        .map(|action_result| action_result.as_ref());

    let Some(AIAgentActionResult {
        result:
            AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success { updated_documents }),
        ..
    }) = agent_action_results
    else {
        return None;
    };

    let document = updated_documents.first()?;
    let action = CreateOrEditDocumentAction::new(
        document.document_id,
        document.document_version,
        props.state_handles.ai_document_handle.clone(),
        app,
    )?;
    Some(action.render(app))
}

fn maybe_render_create_document(
    props: Props,
    id: &AIAgentActionId,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let status = props.action_model.as_ref(app).get_action_status(id);

    // Document operations are always auto-executed for now
    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        todo!("Implement granular permissions for AI documents.");
    }

    let agent_action_results = props
        .action_model
        .as_ref(app)
        .get_action_result(id)
        .map(|action_result| action_result.as_ref());

    let Some(AIAgentActionResult {
        result:
            AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                created_documents,
            }),
        ..
    }) = agent_action_results
    else {
        return None;
    };

    let document = created_documents.first()?;
    let action = CreateOrEditDocumentAction::new(
        document.document_id,
        document.document_version,
        props.state_handles.ai_document_handle.clone(),
        app,
    )?;
    Some(action.render(app))
}

fn render_stopped_output(props: Props, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    let stopped_label = props
        .model
        .conversation_id(app)
        .and_then(|conversation_id| {
            let history = BlocklistAIHistoryModel::as_ref(app);
            let conversation = history.conversation(&conversation_id)?;

            if let Some(todo_list) = conversation.active_todo_list() {
                if let Some((item, item_index)) = todo_list.in_progress_item().and_then(|item| {
                    todo_list
                        .get_item_index(&item.id)
                        .map(|index| (item, index))
                }) {
                    return Some(format!(
                        "Stopped task {}/{}: \"{}\"",
                        item_index + 1,
                        todo_list.len(),
                        item.title
                    ));
                }
            }

            conversation
                .initial_query()
                .map(|task_name| format!("Stopped task: \"{task_name}\""))
        })
        .unwrap_or_else(|| "Stopped task".to_string());

    let stop_icon = Container::new(
        ConstrainedBox::new(gray_stop_icon(appearance).finish())
            .with_width(icon_size(app) - STATUS_ICON_SIZE_DELTA)
            .with_height(icon_size(app) - STATUS_ICON_SIZE_DELTA)
            .finish(),
    )
    .with_margin_right(6.)
    .finish();

    row.add_children([
        stop_icon,
        Expanded::new(
            1.,
            render_output_status_text(
                MaybeShimmeringText::Static(stopped_label.into()),
                appearance,
                app,
            ),
        )
        .finish(),
    ]);

    // Only show resume button for the latest cancelled task in the conversation
    if props
        .model
        .is_latest_exchange_in_terminal_pane(props.terminal_view_id, app)
        && FeatureFlag::AIResumeButton.is_enabled()
    {
        let ui_builder = appearance.ui_builder().clone();

        let play_icon = Container::new(
            ConstrainedBox::new(Icon::Play.to_warpui_icon(theme.foreground()).finish())
                .with_height(appearance.ui_font_size() + 1.)
                .with_width(appearance.ui_font_size() + 1.)
                .finish(),
        )
        .with_margin_right(4.)
        .finish();

        let button_content = {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(play_icon);

            let resume_keystroke = if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-R").expect("keystroke should parse")
            } else {
                Keystroke::parse("ctrl-alt-r").expect("keystroke should parse")
            };
            let keybinding_string = resume_keystroke.displayed();
            let keybinding = Text::new_inline(
                keybinding_string,
                appearance.ui_font_family(),
                appearance.ui_font_size() - 1.,
            )
            .with_color(theme.foreground().into())
            .finish();

            row.add_child(Container::new(keybinding).with_margin_right(4.).finish());
            row.finish()
        };

        let button_styles = UiComponentStyles::default()
            .set_border_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .set_border_width(1.)
            .set_border_color(internal_colors::neutral_4(theme).into())
            .set_padding(Coords {
                top: 2.,
                bottom: 2.,
                left: 4.,
                right: 4.,
            });

        let hovered_styles = button_styles.merge(
            UiComponentStyles::default()
                .set_background(internal_colors::fg_overlay_2(theme).into()),
        );

        let active_styles = button_styles.merge(
            UiComponentStyles::default()
                .set_background(internal_colors::fg_overlay_3(theme).into()),
        );

        let resume_button = warpui::ui_components::button::Button::new(
            props.state_handles.resume_conversation_handle.clone(),
            button_styles,
            Some(hovered_styles),
            Some(active_styles),
            None,
        )
        .with_custom_label(button_content)
        .with_tooltip(move || {
            ui_builder
                .tool_tip("Resume conversation".to_string())
                .build()
                .finish()
        })
        .with_cursor(Some(Cursor::PointingHand))
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AIBlockAction::ResumeConversation);
        })
        .finish();

        row.add_child(resume_button);
    }

    Container::new(
        ConstrainedBox::new(row.finish())
            .with_height(STATUS_FOOTER_VERTICAL_PADDING * 2. + appearance.monospace_font_size())
            .finish(),
    )
    .with_padding_left(CONTENT_HORIZONTAL_PADDING + (STATUS_ICON_SIZE_DELTA / 2.))
    .with_padding_right(CONTENT_HORIZONTAL_PADDING)
    .with_margin_bottom(8.)
    .finish()
}

fn render_requested_edits_output_message(
    requested_edit: &RequestedEdit,
    action_status: Option<AIActionStatus>,
    is_passive_code_gen_block: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let border_color = if action_status
        .as_ref()
        .is_some_and(|status| status.is_blocked())
    {
        theme.accent().into_solid()
    } else {
        theme.surface_2().into()
    };

    // If the diff failed, render a generic diff failed block instead of rendering the code diff.
    // If this is a passive code gen block, don't render anything.
    if action_status
        .as_ref()
        .is_some_and(|status| status.is_failed())
        && !is_passive_code_gen_block
    {
        let title = requested_edit
            .view
            .as_ref(app)
            .title()
            .unwrap_or("Could not apply changes to file.");
        RenderableAction::new(title, app)
            .with_icon(inline_action_icons::cancelled_icon(appearance).finish())
            .render(app)
            .finish()
    } else {
        match requested_edit.view.as_ref(app).display_mode() {
            DisplayMode::FullPane => Align::new(
                Text::new_inline(
                    "This suggestion is being edited in another tab.",
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_selectable(false)
                .finish(),
            )
            .top_center()
            .finish()
            .with_agent_output_item_spacing(app)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(theme.background().into_solid())
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish(),
            DisplayMode::Embedded { .. } => {
                let mut container = Container::new(ChildView::new(&requested_edit.view).finish())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_background_color(theme.background().into_solid())
                    .with_border(Border::all(1.).with_border_fill(border_color));

                if is_passive_code_gen_block {
                    container = container.with_margin_top(CONTENT_ITEM_VERTICAL_MARGIN);
                }

                if action_status
                    .as_ref()
                    .is_some_and(|status| status.is_blocked())
                    || is_passive_code_gen_block
                {
                    container.finish().with_content_item_spacing().finish()
                } else {
                    container
                        .finish()
                        .with_agent_output_item_spacing(app)
                        .finish()
                }
            }
            DisplayMode::InlineBanner { is_expanded, .. } => {
                let mut container = Container::new(ChildView::new(&requested_edit.view).finish())
                    .with_border(Border::all(1.).with_border_fill(theme.surface_2()))
                    .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                    .with_background_color(blended_colors::fg_overlay_2(theme).into());
                if *is_expanded {
                    container =
                        container.with_padding_bottom(INLINE_ACTION_HEADER_VERTICAL_PADDING);
                }
                container.finish()
            }
        }
    }
}

fn render_unit_test_suggestion(
    suggested_prompt: &ViewHandle<SuggestedUnitTestsView>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    Container::new(ChildView::new(suggested_prompt).finish())
        .with_border(Border::all(1.).with_border_fill(theme.surface_2()))
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_background_color(blended_colors::fg_overlay_2(theme).into())
        .with_vertical_padding(CONTENT_ITEM_VERTICAL_MARGIN)
        .finish()
}

fn render_ask_user_question(
    action_id: &AIAgentActionId,
    props: Props,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let view = props.ask_user_question_view?;
    let should_render_inline = {
        let ask_user_question_view = view.as_ref(app);
        ask_user_question_view.action_id() == action_id
            && ask_user_question_view.should_render_inline(app)
    };
    should_render_inline.then(|| ChildView::new(view).finish())
}

fn render_suggest_new_conversation(
    action_id: &AIAgentActionId,
    props: Props,
    appearance: &Appearance,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let status = props
        .action_model
        .as_ref(app)
        .get_action_status(action_id)
        .unwrap_or(AIActionStatus::Finished(Arc::new(AIAgentActionResult {
            result: AIAgentActionResultType::SuggestNewConversation(
                SuggestNewConversationResult::Cancelled,
            ),
            task_id: TaskId::new("fake-id".to_owned()),
            id: action_id.clone(),
        })));

    let theme = appearance.theme();
    if let AIActionStatus::Finished(result) = status {
        let AIAgentActionResultType::SuggestNewConversation(result) = &result.result else {
            report_error!(
                "Unexpected action result type for suggest new conversation action",
                extra: { "result_type" => ?result.result }
            );
            return None;
        };
        let (label, status_icon) = match result {
            SuggestNewConversationResult::Accepted { .. } => (
                "New conversation started",
                inline_action_icons::green_check_icon(appearance).finish(),
            ),
            SuggestNewConversationResult::Rejected => (
                "Continuing current conversation",
                warpui::elements::Icon::new(
                    Icon::FlipForward.into(),
                    internal_colors::neutral_6(theme),
                )
                .finish(),
            ),
            SuggestNewConversationResult::Cancelled => (
                "New conversation suggestion cancelled",
                inline_action_icons::cancelled_icon(appearance).finish(),
            ),
        };
        return Some(
            render_requested_action_row_for_text(
                label.into(),
                appearance.ui_font_family(),
                Some(status_icon),
                None,
                false,
                false,
                app,
            )
            .with_agent_output_item_spacing(app)
            .with_background_color(blended_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        );
    }

    if props.shared_session_status.is_viewer() {
        let header_element = HeaderConfig::new("Start a new conversation", app)
            .with_icon(gray_stop_icon(appearance))
            .render(app);

        return Some(
            header_element
                .with_agent_output_item_spacing(app)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_background_color(blended_colors::neutral_2(theme))
                .finish(),
        );
    }

    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let new_conversation_header_text =
        "It seems like the topic changed. Would you like to make a new conversation?";
    let new_conversation_header_element = HeaderConfig::new(new_conversation_header_text, app)
        .with_icon(yellow_stop_icon(appearance))
        .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
        .render(app);
    content.add_child(new_conversation_header_element);

    if let Some(menu) = props.keyboard_navigable_buttons {
        let keyboard_navigable_buttons_container = Container::new(ChildView::new(menu).finish())
            .with_horizontal_margin(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_margin(16.);
        content.add_child(keyboard_navigable_buttons_container.finish());
    }

    let border_color = blended_colors::neutral_4(theme);

    Some(
        content
            .finish()
            .with_content_item_spacing()
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(theme.background().into_solid())
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish(),
    )
}

/// Creates a FormattedText object with inline code formatting for grep queries
fn create_formatted_text_for_grep(
    props: Props,
    id: &AIAgentActionId,
    queries: &[String],
    path: &str,
    app: &AppContext,
) -> FormattedTextElement {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let action_status = props.action_model.as_ref(app).get_action_status(id);
    let is_cancelled = action_status
        .as_ref()
        .is_some_and(|status| status.is_cancelled());
    let is_queued = action_status
        .as_ref()
        .is_some_and(|status| status.is_queued());

    let display_path = if path == "." {
        "the current directory".to_string()
    } else {
        shell_native_absolute_path(
            path,
            props.shell_launch_data,
            props.current_working_directory,
        )
    };

    let formatted_text = if queries.len() == 1 {
        let query = queries
            .first()
            .expect("Queries slice should have an element");
        let mut fragments = if is_cancelled || is_queued {
            vec![
                FormattedTextFragment::plain_text("Grep for "),
                FormattedTextFragment::inline_code(query),
            ]
        } else {
            vec![
                FormattedTextFragment::plain_text("Grepping for "),
                FormattedTextFragment::inline_code(query),
            ]
        };
        fragments.push(if is_cancelled {
            FormattedTextFragment::plain_text(format!(" in {display_path} cancelled"))
        } else {
            FormattedTextFragment::plain_text(format!(" in {display_path}"))
        });
        FormattedText::new([FormattedTextLine::Line(fragments)])
    } else {
        let mut lines = Vec::new();

        if is_cancelled {
            lines.push(FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(format!(
                    "Cancelled grep for the following patterns in {display_path}"
                )),
            ]));
        } else {
            lines.push(FormattedTextLine::Line(vec![if is_queued {
                FormattedTextFragment::plain_text(format!(
                    "Grep for the following patterns in {display_path}"
                ))
            } else {
                FormattedTextFragment::plain_text(format!(
                    "Grepping for the following patterns in {display_path}"
                ))
            }]));
        }

        for query in queries {
            lines.push(FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(" - "),
                FormattedTextFragment::inline_code(query),
            ]));
        }

        FormattedText::new(lines)
    };

    FormattedTextElement::new(
        formatted_text,
        appearance.monospace_font_size(),
        appearance.ui_font_family(),
        appearance.monospace_font_family(),
        theme.main_text_color(theme.background()).into(),
        Default::default(),
    )
    .with_inline_code_properties(
        Some(
            if is_queued
                || (props.model.status(app).is_streaming()
                    && !props.model.is_first_action_in_output(id, app))
            {
                blended_colors::text_disabled(theme, theme.surface_2())
            } else {
                theme.terminal_colors().normal.green.into()
            },
        ),
        None,
    )
}

/// Creates a FormattedText object with inline code formatting for file glob queries
fn create_formatted_text_for_file_glob(
    props: Props,
    id: &AIAgentActionId,
    patterns: &[String],
    path: Option<&str>,
    app: &AppContext,
) -> FormattedTextElement {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let action_status = props.action_model.as_ref(app).get_action_status(id);
    let is_cancelled = action_status
        .as_ref()
        .is_some_and(|status| status.is_cancelled());
    let is_queued = action_status
        .as_ref()
        .is_some_and(|status| status.is_queued());

    let path = path
        .map(|path| {
            shell_native_absolute_path(
                path,
                props.shell_launch_data,
                props.current_working_directory,
            )
        })
        .unwrap_or_else(|| "the current directory".to_string());

    let formatted_text = if patterns.len() == 1 {
        let pattern = patterns
            .first()
            .expect("Patterns slice should have an element");

        let mut fragments = if is_cancelled || is_queued {
            vec![
                FormattedTextFragment::plain_text("Search for files that match "),
                FormattedTextFragment::inline_code(pattern),
            ]
        } else {
            vec![
                FormattedTextFragment::plain_text("Finding files that match "),
                FormattedTextFragment::inline_code(pattern),
            ]
        };
        fragments.push(if is_cancelled {
            FormattedTextFragment::plain_text(format!(" in {path} cancelled"))
        } else {
            FormattedTextFragment::plain_text(format!(" in {path}"))
        });
        FormattedText::new([FormattedTextLine::Line(fragments)])
    } else {
        let mut lines = Vec::new();

        if is_cancelled {
            lines.push(FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(format!(
                    "Cancelled search for files that match the following patterns in {path}"
                )),
            ]));
        } else {
            lines.push(FormattedTextLine::Line(vec![if is_queued {
                FormattedTextFragment::plain_text(format!(
                    "Find files that match the following patterns in {path}"
                ))
            } else {
                FormattedTextFragment::plain_text(format!(
                    "Finding files that match the following patterns in {path}"
                ))
            }]));
        }

        for pattern in patterns {
            lines.push(FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(" - "),
                FormattedTextFragment::inline_code(pattern),
            ]));
        }
        FormattedText::new(lines)
    };

    FormattedTextElement::new(
        formatted_text,
        appearance.monospace_font_size(),
        appearance.ui_font_family(),
        appearance.monospace_font_family(),
        theme.main_text_color(theme.background()).into(),
        Default::default(),
    )
    .with_inline_code_properties(
        Some(
            if is_queued
                || (props.model.status(app).is_streaming()
                    && !props.model.is_first_action_in_output(id, app))
            {
                blended_colors::text_disabled(theme, theme.surface_2())
            } else {
                theme.terminal_colors().normal.green.into()
            },
        ),
        None,
    )
}

/// Renders the Grep and File Glob tools.
fn render_file_retrieval_tool(
    props: Props,
    action_id: &AIAgentActionId,
    tool_formatted_text: FormattedTextElement,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let status = props.action_model.as_ref(app).get_action_status(action_id);

    let mut config = RenderableAction::new_with_formatted_text(tool_formatted_text, app);

    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        let buttons = props
            .action_buttons
            .get(action_id)
            .expect("Button states must exist for each requested action.");

        config = config
            .with_header(blocked_action_header(
                action_id.clone(),
                BLOCKED_ACTION_MESSAGE_FOR_GREP_OR_FILE_GLOB,
                buttons.run_button.clone(),
                buttons.cancel_button.clone(),
                props.action_model,
                props.model,
                app,
            ))
            .with_highlighted_border()
            .with_background_color(appearance.theme().background().into_solid());
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(action_id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            config = config.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        config =
            config.with_icon(action_icon(action_id, props.action_model, props.model, app).finish());
    }

    match props.autonomy_setting_speedbump {
        AutonomySettingSpeedbump::ShouldShowForFileAccess {
            action_id: show_for_action_id,
            checked,
            shown,
        } if show_for_action_id == action_id => {
            *shown.lock() = true;
            config = config.with_footer(render_autonomy_checkbox_setting_speedbump_footer(
                "Always allow file access for coding tasks",
                *checked,
                AIBlockAction::ToggleAutoreadFilesSpeedbumpCheckbox,
                props
                    .state_handles
                    .autoread_files_speedbump_checkbox_handle
                    .clone(),
                props
                    .state_handles
                    .manage_autonomy_settings_link_handle
                    .clone(),
                app,
            ));
        }
        _ => (),
    };

    config.render(app).finish()
}

fn render_comment_addressed_header(comment: &ReviewComment, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);

    let content = comment.content.lines().join(" ");

    let comment_title = Text::new_inline(
        comment.title(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(blended_colors::text_sub(
        appearance.theme(),
        appearance.theme().background(),
    ))
    .finish();

    let children = vec![
        Shrinkable::new(
            1.,
            Text::new_inline(
                format!("Comment addressed: \"{content}\""),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(blended_colors::text_main(
                appearance.theme(),
                appearance.theme().background(),
            ))
            .finish(),
        )
        .finish(),
        Container::new(comment_title).with_padding_left(8.).finish(),
    ];

    let text_element = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_children(children)
        .finish();

    let mut renderable_action = RenderableAction::new_with_element(text_element, app);
    renderable_action =
        renderable_action.with_icon(icons::addressed_comment_icon(appearance).finish());

    renderable_action.render(app).finish()
}

fn render_read_mcp_resource(
    props: Props,
    action_id: &AIAgentActionId,
    name: &str,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let status = props.action_model.as_ref(app).get_action_status(action_id);

    let mut renderable_action = RenderableAction::new(name, app);

    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        let buttons = props
            .action_buttons
            .get(action_id)
            .expect("Button states must exist for each requested action.");

        renderable_action = renderable_action
            .with_header(blocked_action_header(
                action_id.clone(),
                "OK if I read this MCP resource?",
                buttons.run_button.clone(),
                buttons.cancel_button.clone(),
                props.action_model,
                props.model,
                app,
            ))
            .with_highlighted_border()
            .with_background_color(appearance.theme().background().into_solid());
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(action_id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            renderable_action = renderable_action.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        renderable_action = renderable_action
            .with_icon(action_icon(action_id, props.action_model, props.model, app).finish());
    }

    renderable_action.render(app).finish()
}

fn format_upload_artifact_text(
    request: &UploadArtifactRequest,
    result: Option<&UploadArtifactResult>,
) -> String {
    let mut lines = vec![format!("Upload artifact: {}", request.file_path)];

    if let Some(description) = request.description.as_deref() {
        lines.push(format!("Description: {description}"));
    }

    match result {
        Some(UploadArtifactResult::Success {
            artifact_uid,
            filepath,
            ..
        }) => {
            lines.push(format!("Status: uploaded artifact {artifact_uid}"));
            if let Some(filepath) = filepath.as_deref() {
                lines.push(format!("Uploaded file: {filepath}"));
            }
        }
        Some(UploadArtifactResult::Error(error)) => {
            lines.push(format!("Status: upload failed: {error}"));
        }
        Some(UploadArtifactResult::Cancelled) => {}
        None => {}
    }

    lines.join("\n")
}

fn render_upload_artifact(
    props: Props,
    action_id: &AIAgentActionId,
    request: &UploadArtifactRequest,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let status = props.action_model.as_ref(app).get_action_status(action_id);
    let result = props
        .action_model
        .as_ref(app)
        .get_action_result(action_id)
        .and_then(|result| match &result.result {
            AIAgentActionResultType::UploadArtifact(upload_result) => Some(upload_result),
            _ => None,
        });

    let text = format_upload_artifact_text(request, result);
    let mut renderable_action = RenderableAction::new(&text, app);

    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        let buttons = props
            .action_buttons
            .get(action_id)
            .expect("Button states must exist for each requested action.");

        renderable_action = renderable_action
            .with_header(blocked_action_header(
                action_id.clone(),
                BLOCKED_ACTION_MESSAGE_FOR_UPLOADING_ARTIFACT,
                buttons.run_button.clone(),
                buttons.cancel_button.clone(),
                props.action_model,
                props.model,
                app,
            ))
            .with_highlighted_border()
            .with_background_color(appearance.theme().background().into_solid());
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(action_id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            renderable_action = renderable_action.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        renderable_action = renderable_action
            .with_icon(action_icon(action_id, props.action_model, props.model, app).finish());
    }

    renderable_action.render(app).finish()
}

fn recording_summary(props: Props, agent_summary: Option<&str>, app: &AppContext) -> String {
    let title = props
        .model
        .conversation(app)
        .and_then(|conversation| conversation.title());
    agent_summary
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| title.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .map(ToString::to_string)
        .unwrap_or_else(|| "Recording computer-use session".to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct RecordingCardText {
    primary: String,
    subtext: Option<String>,
}

fn start_recording_card_text(
    description: &str,
    result: Option<&StartRecordingResult>,
) -> RecordingCardText {
    match result {
        Some(StartRecordingResult::Success(_)) => RecordingCardText {
            primary: "Recording started".to_string(),
            subtext: Some(description.to_string()),
        },
        Some(StartRecordingResult::Error(error)) => RecordingCardText {
            primary: "Recording failed to start".to_string(),
            subtext: Some(error.clone()),
        },
        Some(StartRecordingResult::Cancelled) => RecordingCardText {
            primary: "Recording cancelled".to_string(),
            subtext: None,
        },
        None => RecordingCardText {
            primary: "Starting recording".to_string(),
            subtext: Some(description.to_string()),
        },
    }
}

fn stop_recording_card_text(result: Option<&StopRecordingResult>) -> RecordingCardText {
    match result {
        Some(StopRecordingResult::Success(stopped)) => {
            let duration = format_video_duration(stopped.duration);
            let subtext = if matches!(
                stopped.completion_status,
                computer_use::RecordingCompletionStatus::Completed
            ) {
                duration
            } else {
                // TODO(vkodithala): Switch to typed, user-facing termination copy once finalization emits structured reasons.
                format!("Partial recording • {duration}")
            };
            RecordingCardText {
                primary: "Recording saved".to_string(),
                subtext: Some(subtext),
            }
        }
        Some(StopRecordingResult::Error(_)) | Some(StopRecordingResult::Cancelled) => {
            RecordingCardText {
                primary: "Recording could not be saved".to_string(),
                subtext: None,
            }
        }
        None => RecordingCardText {
            primary: "Saving recording".to_string(),
            subtext: None,
        },
    }
}

fn format_video_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn recording_icon(app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let color = appearance.theme().ansi_fg_red();
    ConstrainedBox::new(Icon::CircleFilled.to_warpui_icon(color.into()).finish())
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish()
}

fn recording_card(
    text: RecordingCardText,
    action_button: Option<Box<dyn Element>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let primary = Text::new(
        text.primary,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(theme, theme.background()))
    .finish();
    let mut body = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
    body.add_child(primary);
    if let Some(subtext) = text.subtext.filter(|subtext| !subtext.trim().is_empty()) {
        body.add_child(
            Text::new(
                subtext,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(blended_colors::text_disabled(theme, theme.surface_2()))
            .finish(),
        );
    }

    let mut action =
        RenderableAction::new_with_element(body.finish(), app).with_icon(recording_icon(app));
    if let Some(action_button) = action_button {
        action = action.with_action_button(action_button);
    }
    action.render(app).finish()
}

fn render_start_recording(
    props: Props,
    action_id: &AIAgentActionId,
    agent_summary: Option<&str>,
    app: &AppContext,
) -> Box<dyn Element> {
    let result = props
        .action_model
        .as_ref(app)
        .get_action_result(action_id)
        .and_then(|result| match &result.result {
            AIAgentActionResultType::StartRecording(result) => Some(result),
            _ => None,
        });
    let text = start_recording_card_text(&recording_summary(props, agent_summary, app), result);
    recording_card(text, None, app)
}

fn render_recording_footer(status: RecordingSpanStatus, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let icon_offset =
        icon_size(app) + crate::ai::blocklist::inline_action::inline_action_header::ICON_MARGIN;
    let text = match status {
        RecordingSpanStatus::Active => "Recording active",
        RecordingSpanStatus::Captured => "Captured in recording",
    };
    Container::new(
        Text::new(
            text.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.sub_text_color(theme.surface_1()).into())
        .finish(),
    )
    .with_margin_left(icon_offset)
    .finish()
}

fn render_stop_recording(
    props: Props,
    action_id: &AIAgentActionId,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::handle(app).as_ref(app);
    let result = props
        .action_model
        .as_ref(app)
        .get_action_result(action_id)
        .and_then(|result| match &result.result {
            AIAgentActionResultType::StopRecording(result) => Some(result),
            _ => None,
        });
    let mut action_button = None;
    if let Some(StopRecordingResult::Success(stopped)) = result {
        if !stopped.artifact_uid.trim().is_empty() {
            let artifact_uid = stopped.artifact_uid.clone();
            action_button = props.open_recording_buttons.get(action_id).map(|btn| {
                render_inline_action_secondary_button(
                    appearance,
                    btn,
                    "Open recording",
                    Box::new(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AIBlockAction::OpenRecordingArtifact {
                            artifact_uid: artifact_uid.clone(),
                        });
                    }),
                )
            });
        }
    }

    recording_card(stop_recording_card_text(result), action_button, app)
}

fn render_use_computer(
    props: Props,
    action_id: &AIAgentActionId,
    request: &UseComputerRequest,
    recording_span: Option<&RecordingSpanInfo>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::handle(app).as_ref(app);

    let mut renderable_action = RenderableAction::new(&request.action_summary, app)
        .with_icon(action_icon(action_id, props.action_model, props.model, app).finish());

    if should_decorate_recorded_use_computer(request) {
        if let Some(recording_span) = recording_span {
            renderable_action =
                renderable_action.with_footer(render_recording_footer(recording_span.status, app));
        }
    }

    // Add a "View screenshot" button if the action result contains a screenshot.
    let has_screenshot = props
        .action_model
        .as_ref(app)
        .get_action_result(action_id)
        .is_some_and(|result| {
            matches!(
                &result.result,
                AIAgentActionResultType::UseComputer(
                    crate::ai::agent::UseComputerResult::Success(action_result)
                ) if action_result.screenshot.is_some()
            )
        });

    if has_screenshot {
        let action_id_clone = action_id.clone();
        let view_screenshot_button = props.view_screenshot_buttons.get(action_id).map(|btn| {
            render_inline_action_secondary_button(
                appearance,
                btn,
                "View screenshot",
                Box::new(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AIBlockAction::ViewScreenshot {
                        action_id: action_id_clone.clone(),
                    });
                }),
            )
        });

        if let Some(button_element) = view_screenshot_button {
            renderable_action = renderable_action.with_action_button(button_element);
        }
    }

    renderable_action.render(app).finish()
}

fn render_request_computer_use(
    props: Props,
    action_id: &AIAgentActionId,
    request: &RequestComputerUseRequest,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let status = props.action_model.as_ref(app).get_action_status(action_id);

    let mut renderable_action = RenderableAction::new(&request.task_summary, app);

    if status.as_ref().is_some_and(|status| status.is_blocked()) {
        let buttons = props
            .action_buttons
            .get(action_id)
            .expect("Button states must exist for each requested action.");

        renderable_action = renderable_action
            .with_header(blocked_action_header(
                action_id.clone(),
                "OK if I use computer control for this task?",
                buttons.run_button.clone(),
                buttons.cancel_button.clone(),
                props.action_model,
                props.model,
                app,
            ))
            .with_highlighted_border()
            .with_background_color(appearance.theme().background().into_solid());
    } else {
        if (props.model.status(app).is_streaming()
            && !props.model.is_first_action_in_output(action_id, app))
            || status.as_ref().is_some_and(|s| s.is_queued())
        {
            renderable_action = renderable_action.with_font_color(blended_colors::text_disabled(
                appearance.theme(),
                appearance.theme().surface_2(),
            ));
        }
        renderable_action = renderable_action
            .with_icon(action_icon(action_id, props.action_model, props.model, app).finish());
    }

    renderable_action.render(app).finish()
}

/// Renders the collapsible references footer
/// if there are any citations.
fn render_references_footer(
    citations: &[AIAgentCitation],
    props: Props,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let title_row_color = theme.nonactive_ui_text_color();

    let citations_row = render_citation_chips(
        citations,
        &props.state_handles.footer_citation_chip_handles,
        appearance.monospace_font_size() - 2.,
        8.,
        app,
    )?;

    let title = Text::new_inline(
        "References",
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(title_row_color.into())
    .with_selectable(false);
    let chevron = if props.is_references_section_open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let title_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_child(
            Container::new(title.finish())
                .with_margin_right(6.)
                .finish(),
        )
        .with_child(
            ConstrainedBox::new(chevron.to_warpui_icon(title_row_color).finish())
                .with_height(icon_size(app) - 2.)
                .with_width(icon_size(app) - 2.)
                .finish(),
        );

    let mut column = Flex::column();
    column.add_child(
        Hoverable::new(
            props
                .state_handles
                .references_section_collapsible_handle
                .clone(),
            |_| {
                Container::new(title_row.finish())
                    .with_margin_bottom(8.)
                    .finish()
            },
        )
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(AIBlockAction::ToggleReferencesSection))
        .with_cursor(Cursor::PointingHand)
        .finish(),
    );

    if props.is_references_section_open {
        column.add_child(citations_row);
    }

    Some(column.finish().with_agent_output_item_spacing(app).finish())
}

/// Renders the suggested rules footer at the bottom of the block.
fn render_suggested_rules_and_prompts_footer(
    props: Props,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    // Filter out dismissed suggestions
    let dismissed_ids = props
        .model
        .conversation(app)
        .map(|c| c.dismissed_suggestion_ids().clone())
        .unwrap_or_default();

    let suggested_rules = props
        .suggested_rules
        .iter()
        .filter(|chip| {
            let logging_id = chip.as_ref(app).logging_id();
            !dismissed_ids.contains(&logging_id)
        })
        .collect_vec();

    let suggested_prompt = props.suggested_agent_mode_workflow.as_ref().filter(|chip| {
        let logging_id = chip.as_ref(app).logging_id();
        !dismissed_ids.contains(&logging_id)
    });

    // If no visible suggestions, don't render the footer
    if suggested_rules.is_empty() && suggested_prompt.is_none() {
        return None;
    }

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let title_row_color = theme.sub_text_color(theme.background());
    let title_text = Text::new_inline(
        "Suggestions:",
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(title_row_color.into())
    .with_selectable(false)
    .finish();

    let has_suggested_rules = !suggested_rules.is_empty();

    let right_buttons = {
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if has_suggested_rules {
            row.add_child(
                Container::new(ChildView::new(props.disable_rule_suggestions_button).finish())
                    .with_margin_right(4.)
                    .finish(),
            );
        }
        row.add_child(ChildView::new(props.dismiss_suggestion_button).finish());
        row.finish()
    };

    let title = Container::new(
        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1.0, title_text).finish())
            .with_child(right_buttons)
            .finish(),
    )
    .with_margin_bottom(8.)
    .finish();

    let suggested_rules = suggested_rules
        .iter()
        .map(|rule| ChildView::new(rule).finish())
        .collect_vec();

    let suggested_agent_mode_workflows = suggested_prompt
        .iter()
        .map(|workflow| ChildView::new(workflow).finish())
        .collect_vec();
    let has_suggested_agent_mode_workflow = !suggested_agent_mode_workflows.is_empty();

    let mut prompts_row = Wrap::row()
        .with_children(suggested_rules)
        .with_children(suggested_agent_mode_workflows);

    if has_suggested_rules && !has_suggested_agent_mode_workflow {
        prompts_row.add_child(ChildView::new(props.manage_rules_button).finish());
    }

    Some(
        Flex::column()
            .with_child(title)
            .with_child(prompts_row.finish())
            .finish()
            .with_agent_output_item_spacing(app)
            .finish(),
    )
}

fn render_response_footer(props: Props, app: &AppContext) -> Option<Box<dyn Element>> {
    if props.model.status(app).is_streaming() {
        return None;
    }

    let appearance = Appearance::as_ref(app);
    let mut flex = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    let is_passive_code_diff = props.model.request_type(app).is_passive_code_diff();

    // Show footer for any terminal state (complete, cancelled, or failed)
    let style_override = UiComponentStyles {
        font_color: Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
        ),
        width: Some(icon_size(app) + 4.),
        height: Some(icon_size(app) + 4.),
        ..Default::default()
    };
    let style_override_with_background = UiComponentStyles {
        background: Some(blended_colors::neutral_4(appearance.theme()).into()),
        ..style_override
    };

    let ui_builder = appearance.ui_builder().clone();

    // Thumbs up/down buttons.
    // (we hide these when you're in view-only mode).
    if !is_passive_code_diff && !props.is_conversation_transcript_viewer {
        let thumbs_up_button = icon_button(
            appearance,
            Icon::ThumbsUp,
            matches!(
                props.response_rating.get(),
                Some(AIBlockResponseRating::Positive)
            ),
            props.state_handles.thumbs_up_handle.clone(),
        )
        .with_tooltip(move || {
            ui_builder
                .tool_tip("Good response".to_string())
                .build()
                .finish()
        })
        .with_style(style_override)
        .with_hovered_styles(style_override_with_background)
        .with_active_styles(style_override_with_background);

        let ui_builder = appearance.ui_builder().clone();
        let thumbs_down_button = icon_button(
            appearance,
            Icon::ThumbsDown,
            matches!(
                props.response_rating.get(),
                Some(AIBlockResponseRating::Negative)
            ),
            props.state_handles.thumbs_down_handle.clone(),
        )
        .with_tooltip(move || {
            ui_builder
                .clone()
                .tool_tip("Bad response".to_string())
                .build()
                .finish()
        })
        .with_style(style_override)
        .with_hovered_styles(style_override_with_background)
        .with_active_styles(style_override_with_background);

        let (thumbs_up_button_element, thumbs_down_button_element) =
            match props.response_rating.get() {
                Some(rating) => {
                    // Mark the button that wasn't selected as disabled.
                    // (The button that _was_ selected will not be clickable since it's marked active).
                    match rating {
                        AIBlockResponseRating::Positive => (
                            thumbs_up_button.build().with_cursor(Cursor::Arrow).finish(),
                            thumbs_down_button
                                .disabled()
                                .with_disabled_styles(Default::default())
                                .build()
                                .finish(),
                        ),
                        AIBlockResponseRating::Negative => (
                            thumbs_up_button
                                .disabled()
                                .with_disabled_styles(Default::default())
                                .build()
                                .finish(),
                            thumbs_down_button
                                .build()
                                .with_cursor(Cursor::Arrow)
                                .finish(),
                        ),
                    }
                }
                None => (
                    thumbs_up_button
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(AIBlockAction::Rated { is_positive: true })
                        })
                        .finish(),
                    thumbs_down_button
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(AIBlockAction::Rated { is_positive: false })
                        })
                        .finish(),
                ),
            };
        flex.add_child(
            Container::new(thumbs_up_button_element)
                .with_margin_right(2.)
                .finish(),
        );
        flex.add_child(
            Container::new(thumbs_down_button_element)
                .with_margin_right(2.)
                .finish(),
        );
    }

    if !props.shared_session_status.is_finished_viewer() && !FeatureFlag::AgentView.is_enabled() {
        let ui_builder = appearance.ui_builder().clone();
        let continue_button = icon_button(
            appearance,
            Icon::CornerRight,
            false,
            props.state_handles.continue_conversation_handle.clone(),
        )
        .with_tooltip(move || {
            ui_builder
                .tool_tip("Continue conversation".to_string())
                .build()
                .finish()
        })
        .with_style(style_override)
        .with_hovered_styles(style_override_with_background)
        .with_active_styles(style_override_with_background)
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(AIBlockAction::ContinueConversation))
        .finish();

        flex.add_child(continue_button);
    }

    #[cfg(not(target_family = "wasm"))]
    if !props.is_conversation_transcript_viewer {
        let fork_button_tooltip = fork_button_action(
            props.model.conversation_id(app),
            props.is_cloud_agent_context,
            app,
        )
        .tooltip;

        let ui_builder = appearance.ui_builder().clone();
        let fork_button = icon_button(
            appearance,
            Icon::ArrowSplit,
            false,
            props.state_handles.fork_conversation_handle.clone(),
        )
        .with_tooltip(move || {
            ui_builder
                .tool_tip(fork_button_tooltip.to_string())
                .build()
                .finish()
        })
        .with_style(style_override)
        .with_hovered_styles(style_override_with_background)
        .with_active_styles(style_override_with_background)
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AIBlockAction::ForkConversation);
        })
        .finish();

        flex.add_child(fork_button);
    }

    flex.add_child(render_usage_button(props, app));

    // Review changes button.
    if props.has_accepted_edits && !props.shared_session_status.is_viewer() {
        // Only show Review Changes button if we're in a git repository
        let is_in_git_repo = props
            .current_working_directory
            .as_ref()
            .map(|path| repo_metadata::is_in_repo(path, app))
            .unwrap_or(false);

        if is_in_git_repo {
            flex.add_child(
                Container::new(ChildView::new(props.review_changes_button).finish())
                    .with_margin_left(4.)
                    .finish(),
            );
        }
    }

    // "Open all review comments" bulk-import button.
    if props.conversation_has_imported_comments && !props.shared_session_status.is_viewer() {
        flex.add_child(
            Container::new(ChildView::new(props.open_all_comments_button).finish())
                .with_margin_left(4.)
                .finish(),
        );
    }

    Some(flex.finish().with_content_item_spacing().finish())
}

/// Renders the usage button that, on click, will expand & collapse the usage summary footer.
fn render_usage_button(props: Props, app: &AppContext) -> Box<dyn Element> {
    let Some(conversation) = props.model.conversation(app) else {
        return Empty::new().finish();
    };

    // Optional orchestration credit rollup. When the conversation has at
    // least one locally-loaded descendant with credits spent, the pill's
    // headline number and "has any usage" suppression check both switch
    // over to the orchestration total (PRODUCT invariants 11, 11b). The
    // `(+N)` last-block annotation below stays bound to the
    // orchestrator's own credits. The rollup helper returns `None` for
    // conversations with no descendants, so callers that aren't
    // orchestrators pay only the cost of one descendant-index probe.
    let rollup =
        compute_orchestration_rollup(conversation.id(), BlocklistAIHistoryModel::as_ref(app));

    // If this conversation has no usage metadata (e.g. a forked conversation from
    // mid-way through a prior conversation where the server did not send
    // ConversationUsageMetadata), avoid rendering the usage button entirely.
    let headline_credits = rollup
        .as_ref()
        .map(|r| r.total_credits)
        .unwrap_or_else(|| conversation.credits_spent());
    let has_any_usage = headline_credits > 0.0
        || conversation.credits_spent_for_last_block().is_some()
        || !conversation.token_usage().is_empty()
        || conversation.tool_usage_metadata().total_tool_calls() > 0;
    if !has_any_usage {
        return Empty::new().finish();
    }

    let appearance = Appearance::as_ref(app);
    let ui_builder = appearance.ui_builder().clone();

    let expansion_icon = if props.is_usage_footer_expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };

    let total_credits_spent = headline_credits;
    let mut credit_usage_text = format_credits(total_credits_spent);
    if let Some(credits_spent_for_last_block) = conversation.credits_spent_for_last_block() {
        // Only show the credits spent for the last block if it is different from the total credits spent
        // and we spent a non-zero amount of credits for the last block.
        // Avoid showing the credits spent for the last block if the request failed, as we refund user
        // credits in that case (so no credits were in fact spent).
        if credits_spent_for_last_block > 0.0
            && total_credits_spent != credits_spent_for_last_block
            && props.model.status(app).error().is_none()
        {
            // If the first part of the decimal is 0, we just display the whole number.
            if credits_spent_for_last_block.fract() < 0.1 {
                credit_usage_text = format!(
                    "{credit_usage_text} (+{})",
                    credits_spent_for_last_block.trunc() as i32
                );
            } else {
                credit_usage_text =
                    format!("{credit_usage_text} (+{credits_spent_for_last_block:.1})");
            }
        }
    }

    let icon_size = icon_size(app);
    let button_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(
            Container::new(
                Text::new_inline(
                    credit_usage_text,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into(),
                )
                .with_selectable(false)
                .finish(),
            )
            .with_padding_top(2.)
            .with_margin_left(4.)
            .finish(),
        )
        .with_child(
            Container::new(
                // Expansion icon
                ConstrainedBox::new(
                    expansion_icon
                        .to_warpui_icon(
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().background()),
                        )
                        .finish(),
                )
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
            )
            .with_margin_top(1.)
            .finish(),
        );

    Hoverable::new(
        props.state_handles.usage_button_handle.clone(),
        |mouse_state| {
            let mut content = Container::new(button_row.finish());

            if mouse_state.is_hovered() || mouse_state.is_clicked() {
                let background = if mouse_state.is_clicked() {
                    appearance.theme().background()
                } else {
                    blended_colors::neutral_4(appearance.theme()).into()
                };

                content = content
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                // Show tooltip on hover or while clicked
                let mut stack = Stack::new().with_child(content.finish());
                let tooltip = ui_builder
                    .tool_tip("Show credit usage details".to_string())
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );

                stack.finish()
            } else {
                content.finish()
            }
        },
    )
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(AIBlockAction::ToggleIsUsageFooterExpanded);
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

pub fn action_icon<V: View>(
    action_id: &AIAgentActionId,
    action_model: &ModelHandle<BlocklistAIActionModel>,
    ai_block_model: &dyn AIBlockModel<View = V>,
    app: &AppContext,
) -> warpui::elements::Icon {
    let appearance = Appearance::as_ref(app);
    let status = action_model.as_ref(app).get_action_status(action_id);
    match status {
        Some(status) => match status {
            AIActionStatus::Preprocessing => icons::gray_circle_icon(appearance),
            AIActionStatus::Queued => icons::gray_stop_icon(appearance),
            AIActionStatus::Blocked => icons::yellow_stop_icon(appearance),
            AIActionStatus::RunningAsync => icons::yellow_running_icon(appearance),
            AIActionStatus::Finished(result) => {
                if matches!(
                    result.result,
                    AIAgentActionResultType::RequestCommandOutput(
                        RequestCommandOutputResult::LongRunningCommandSnapshot { .. }
                    )
                ) {
                    return icons::yellow_running_icon(appearance);
                }

                if result.result.is_successful() {
                    inline_action_icons::green_check_icon(appearance)
                } else if result.result.is_cancelled() {
                    inline_action_icons::cancelled_icon(appearance)
                } else {
                    inline_action_icons::red_x_icon(appearance)
                }
            }
        },
        None => {
            if ai_block_model.status(app).is_streaming() {
                if ai_block_model.is_first_action_in_output(action_id, app) {
                    icons::yellow_running_icon(appearance)
                } else {
                    icons::gray_circle_icon(appearance)
                }
            } else {
                inline_action_icons::cancelled_icon(appearance)
            }
        }
    }
}

pub(super) fn blocked_action_header<V: View>(
    action_id: AIAgentActionId,
    text: &str,
    accept_button: CompactibleActionButton,
    cancel_button: CompactibleActionButton,
    action_model: &ModelHandle<BlocklistAIActionModel>,
    block_model: &dyn AIBlockModel<View = V>,
    app: &AppContext,
) -> HeaderConfig {
    let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> = vec![
        Rc::new(cancel_button.clone()),
        Rc::new(accept_button.clone()),
    ];
    HeaderConfig::new(text.to_owned(), app)
        .with_icon(action_icon(&action_id, action_model, block_model, app))
        .with_interaction_mode(InteractionMode::ActionButtons {
            action_buttons,
            size_switch_threshold: SMALL_SIZE_SWITCH_THRESHOLD,
        })
}

fn render_collapsible_header(
    message_id: &MessageId,
    element_state: &CollapsibleElementState,
    header_text: String,
    text_color: ColorU,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let icon_size = icon_size(app);
    let is_expanded = matches!(
        element_state.expansion_state,
        CollapsibleExpansionState::Expanded { .. }
    );

    let chevron_icon = if is_expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let message_id_clone = message_id.clone();
    let toggle_mouse_state = element_state.expansion_toggle_mouse_state.clone();

    let expandable = Hoverable::new(toggle_mouse_state, move |_is_hovered| {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new(
                    header_text.clone(),
                    appearance.ai_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(text_color)
                .with_selectable(false)
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(chevron_icon.to_warpui_icon(text_color.into()).finish())
                        .with_width(icon_size - 2.)
                        .with_height(icon_size - 2.)
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            )
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(AIBlockAction::ToggleCollapsibleBlockExpanded(
            message_id_clone.clone(),
        ));
    });

    Container::new(Flex::row().with_child(expandable.finish()).finish())
        .with_horizontal_margin(CONTENT_HORIZONTAL_PADDING + icon_size + 16.)
        .finish()
}

pub fn are_all_text_sections_empty(text_sections: &[AIAgentTextSection]) -> bool {
    text_sections
        .iter()
        .all(|section: &AIAgentTextSection| section.is_empty())
}

/// Helper to render collapsible reasoning or summarization blocks.
#[allow(clippy::too_many_arguments)]
fn render_collapsible_block(
    output_message: &AIAgentOutputMessage,
    header_text: String,
    sections: &[AIAgentTextSection],
    show_header: bool,
    props: Props,
    has_rendered_first_text_section: &mut bool,
    text_section_index: &mut usize,
    code_section_index: &mut usize,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let state = props.collapsible_block_states.get(&output_message.id)?;

    Some(render_collapsible_text_block_section(
        &output_message.id,
        state,
        header_text,
        sections,
        show_header,
        has_rendered_first_text_section,
        text_section_index,
        code_section_index,
        props,
        app,
    ))
}

/// Renders a collapsible text block (reasoning or summarization) with header, markdown-parsed sections, and scroll-pinning.
#[allow(clippy::too_many_arguments)]
fn render_collapsible_text_block_section(
    message_id: &MessageId,
    element_state: &CollapsibleElementState,
    header_text: String,
    sections: &[AIAgentTextSection],
    show_header: bool,
    has_rendered_first_text_section: &mut bool,
    text_section_index: &mut usize,
    code_section_index: &mut usize,
    props: Props,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let text_color = blended_colors::text_disabled(theme, theme.surface_2());
    let selectable = false;
    let is_streaming = props.model.status(app).is_streaming();

    let mut container = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Render the collapsible header when the block should remain manually toggleable.
    if show_header {
        container.add_child(
            Container::new(render_collapsible_header(
                message_id,
                element_state,
                header_text,
                text_color,
                app,
            ))
            .with_margin_bottom(16.)
            .finish(),
        );
        *has_rendered_first_text_section = true;
    }

    // Render sections with markdown support
    let mut table_section_index = 0;
    let mut image_section_index = 0;
    let rendered_sections = render_text_sections(
        TextSectionsProps {
            model: props.model,
            starting_text_section_index: text_section_index,
            starting_code_section_index: code_section_index,
            starting_table_section_index: &mut table_section_index,
            starting_image_section_index: &mut image_section_index,
            sections,
            text_color,
            selectable,
            find_context: props.find_context,
            current_working_directory: props.current_working_directory,
            shell_launch_data: props.shell_launch_data,
            embedded_code_editor_views: &[],
            code_snippet_button_handles: &[],
            table_section_handles: &[],
            image_section_tooltip_handles: &[],
            is_ai_input_enabled: props.is_ai_input_enabled,
            open_code_block_action_factory: (None as Option<
                &'static dyn Fn(CodeSource) -> AIBlockAction,
            >),
            copy_code_action_factory: (None as Option<&'static dyn Fn(String) -> AIBlockAction>),
            detected_links: Some(props.detected_links_state),
            secret_redaction_state: props.secret_redaction_state,
            is_selecting_text: props.state_handles.selection_handle.is_selecting(),
            item_spacing: CONTENT_ITEM_VERTICAL_MARGIN,
            #[cfg(feature = "local_fs")]
            resolved_code_block_paths: Some(props.resolved_code_block_paths),
            #[cfg(feature = "local_fs")]
            resolved_blocklist_image_sources: Some(props.resolved_blocklist_image_sources),
        },
        app,
    );
    let rendered_sections = rendered_sections
        .with_agent_output_item_spacing(app)
        .finish();

    // Use a larger height more amenable to reading once streaming is complete.
    // When thinking_display_mode is AlwaysShow, always use the larger height so the
    // viewport doesn't jump when streaming finishes.
    let max_height = if props.thinking_display_mode.should_keep_expanded() || !is_streaming {
        360.
    } else {
        120.
    };
    let body = Container::new(rendered_sections)
        .with_margin_bottom(-16.0)
        .finish();
    if let Some(scrollable) = render_scrollable_collapsible_content(
        message_id,
        element_state,
        body,
        is_streaming,
        max_height,
    ) {
        container.add_child(Container::new(scrollable).with_margin_bottom(16.0).finish());
    }

    container.finish()
}

/// Renders collapsible debug output block.
/// Collapsed: shows "Debug output" label, right chevron, then first line preview.
/// Expanded: shows "Debug output" label with down chevron, full text below.
fn render_collapsible_debug_output(
    output_message: &AIAgentOutputMessage,
    text: &str,
    props: Props,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let state = props.collapsible_block_states.get(&output_message.id)?;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let text_color = blended_colors::text_disabled(theme, theme.surface_2());
    let icon_size = icon_size(app);

    let is_expanded = matches!(
        state.expansion_state,
        CollapsibleExpansionState::Expanded { .. }
    );

    // Get first line for collapsed header preview
    let first_line = text.lines().next().unwrap_or(text);

    let mut container = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Render the collapsible header
    let chevron_icon = if is_expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let message_id_clone = output_message.id.clone();
    let toggle_mouse_state = state.expansion_toggle_mouse_state.clone();

    // When collapsed, show first line preview after chevron
    // When expanded, show nothing after chevron (full text is below)
    let first_line_owned = first_line.to_string();

    let header = Hoverable::new(toggle_mouse_state, move |_is_hovered| {
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // "Debug output" label
        row.add_child(
            Text::new(
                "Debug output".to_string(),
                appearance.ai_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(text_color)
            .with_selectable(false)
            .finish(),
        );

        // Chevron icon
        row.add_child(
            Container::new(
                ConstrainedBox::new(chevron_icon.to_warpui_icon(text_color.into()).finish())
                    .with_width(icon_size - 2.)
                    .with_height(icon_size - 2.)
                    .finish(),
            )
            .with_horizontal_margin(4.)
            .finish(),
        );

        // First line preview (only when collapsed)
        if !is_expanded {
            row.add_child(
                Text::new(
                    first_line_owned.clone(),
                    appearance.ai_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(text_color)
                .with_selectable(false)
                .finish(),
            );
        }

        row.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(AIBlockAction::ToggleCollapsibleBlockExpanded(
            message_id_clone.clone(),
        ));
    });

    container.add_child(
        Container::new(Flex::row().with_child(header.finish()).finish())
            .with_margin_bottom(if is_expanded { 16. } else { 0. })
            .finish(),
    );

    // If expanded, show full content in scrollable area
    if is_expanded {
        let full_content = Text::new(
            text.to_string(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(text_color)
        .with_selectable(true)
        .finish();

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: state.scroll_state.clone(),
                child: full_content,
            },
            Fill::None,
            Fill::None,
            Fill::None,
        )
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        let clipped_scrollable = ConstrainedBox::new(scrollable)
            .with_max_height(200.)
            .finish();

        container.add_child(
            Container::new(clipped_scrollable)
                .with_margin_bottom(16.0)
                .finish(),
        );
    }

    Some(
        container
            .finish()
            .with_agent_output_item_spacing(app)
            .finish(),
    )
}

// --- Conversation search phase detection ---

enum ConversationSearchPhase {
    ListingMessages,
    Grepping { patterns: Vec<String> },
    ReadingMessages { count: usize },
}

fn conversation_search_phase(task: &crate::ai::agent::task::Task) -> ConversationSearchPhase {
    use crate::ai::agent::{AIAgentActionType, AIAgentOutputMessageType};

    let mut current_phase = ConversationSearchPhase::ListingMessages;

    for exchange in task.exchanges() {
        let Some(output) = exchange.output_status.output() else {
            continue;
        };
        let output = output.get();
        for message in &output.messages {
            if let AIAgentOutputMessageType::Action(action) = &message.message {
                let new_phase = match &action.action {
                    AIAgentActionType::FetchConversation { .. } => {
                        Some(ConversationSearchPhase::ListingMessages)
                    }
                    AIAgentActionType::Grep { queries, .. } if !queries.is_empty() => {
                        Some(ConversationSearchPhase::Grepping {
                            patterns: queries.clone(),
                        })
                    }
                    AIAgentActionType::ReadFiles(request) if !request.locations.is_empty() => {
                        Some(ConversationSearchPhase::ReadingMessages {
                            count: request.locations.len(),
                        })
                    }
                    AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => {
                        Some(ConversationSearchPhase::ListingMessages)
                    }
                    _ => None,
                };
                if let Some(phase) = new_phase {
                    current_phase = phase;
                }
            }
        }
    }

    current_phase
}

fn format_conversation_search_phase(phase: &ConversationSearchPhase) -> String {
    match phase {
        ConversationSearchPhase::ListingMessages => "Listing messages".to_string(),
        ConversationSearchPhase::Grepping { patterns } => {
            if patterns.is_empty() {
                return "Grepping for patterns".to_string();
            }
            let joined = truncate_from_end(&patterns.join(", "), 60);
            format!("Grepping for patterns: {joined}")
        }
        ConversationSearchPhase::ReadingMessages { count } => {
            format!("Reading {count} messages")
        }
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
