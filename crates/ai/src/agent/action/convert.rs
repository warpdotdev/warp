use std::path::PathBuf;
use std::time::Duration;

use itertools::Itertools as _;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;

use crate::agent::action::{
    AIAgentActionType, AIAgentPtyWriteMode, CommentSide, FileEdit, InsertReviewComment,
    InsertedCommentLine, InsertedCommentLocation, ReadFilesRequest, SearchCodebaseRequest,
    ShellCommandDelay, SuggestPromptRequest, UploadArtifactRequest, UseComputerRequest,
};
use crate::agent::action_result::{AnyFileContent, FileContext};
use crate::agent::convert::ToolToAIAgentActionError;
use crate::agent::FileLocations;
use crate::diff_validation::{ParsedDiff, V4AHunk};
use crate::document::AIDocumentId;

impl From<api::message::tool_call::RunShellCommand> for AIAgentActionType {
    fn from(value: api::message::tool_call::RunShellCommand) -> Self {
        AIAgentActionType::RequestCommandOutput {
            command: value.command,
            is_read_only: Some(value.is_read_only),
            rationale: None,
            uses_pager: Some(value.uses_pager),
            is_risky: Some(value.is_risky),
            wait_until_completion: value.wait_until_complete_value.is_none_or(
                |api::message::tool_call::run_shell_command::WaitUntilCompleteValue::WaitUntilComplete(
                    should_wait,
                )| should_wait,
            ),
            citations: value
                .citations
                .iter()
                .filter_map(|citation| citation.clone().try_into().ok())
                .collect(),
        }
    }
}

impl From<api::message::tool_call::WriteToLongRunningShellCommand> for AIAgentActionType {
    fn from(value: api::message::tool_call::WriteToLongRunningShellCommand) -> Self {
        AIAgentActionType::WriteToLongRunningShellCommand {
            block_id: value.command_id.into(),
            input: value.input.into(),
            mode: value.mode.map(Into::into).unwrap_or_default(),
        }
    }
}

impl From<api::message::tool_call::write_to_long_running_shell_command::Mode>
    for AIAgentPtyWriteMode
{
    fn from(value: api::message::tool_call::write_to_long_running_shell_command::Mode) -> Self {
        match value.mode {
            Some(mode) => {
                use warp_multi_agent_api::message::tool_call::write_to_long_running_shell_command::mode::Mode;
                match mode {
                    Mode::Raw(_) => AIAgentPtyWriteMode::Raw,
                    Mode::Line(_) => AIAgentPtyWriteMode::Line,
                    Mode::Block(_) => AIAgentPtyWriteMode::Block,
                }
            }
            None => AIAgentPtyWriteMode::Raw,
        }
    }
}

impl From<api::message::tool_call::SuggestNewConversation> for AIAgentActionType {
    fn from(value: api::message::tool_call::SuggestNewConversation) -> Self {
        AIAgentActionType::SuggestNewConversation {
            message_id: value.message_id,
        }
    }
}

impl From<api::message::tool_call::ApplyFileDiffs> for AIAgentActionType {
    fn from(value: api::message::tool_call::ApplyFileDiffs) -> Self {
        let diff_edits = value.diffs.into_iter().map(|file_diff| {
            FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                search: file_diff.search.none_if_default(),
                replace: file_diff.replace.none_if_default(),
                file: file_diff.file_path.none_if_default(),
            })
        });
        let v4a_updates = value.v4a_updates.into_iter().map(|v4a_update| {
            FileEdit::Edit(ParsedDiff::V4AEdit {
                file: v4a_update.file_path.none_if_default(),
                move_to: v4a_update.move_to.clone().none_if_default(),
                hunks: v4a_update
                    .hunks
                    .into_iter()
                    .map(|hunk| V4AHunk {
                        change_context: hunk.change_context,
                        pre_context: hunk.pre_context,
                        old: hunk.old,
                        new: hunk.new,
                        post_context: hunk.post_context,
                    })
                    .collect(),
            })
        });
        let file_deletes = value
            .deleted_files
            .into_iter()
            .map(|file_delete| FileEdit::Delete {
                file: file_delete.file_path.none_if_default(),
            });
        let new_file_edits = value
            .new_files
            .into_iter()
            .map(|new_file| FileEdit::Create {
                file: new_file.file_path.none_if_default(),
                content: new_file.content.none_if_default(),
            });

        AIAgentActionType::RequestFileEdits {
            file_edits: diff_edits
                .chain(v4a_updates)
                .chain(new_file_edits)
                .chain(file_deletes)
                .collect(),
            title: Some(value.summary),
        }
    }
}

impl From<api::message::tool_call::ReadFiles> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadFiles) -> Self {
        AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: value.files.into_iter().map(Into::into).collect(),
        })
    }
}

impl TryFrom<api::UploadFileArtifact> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::UploadFileArtifact) -> Result<Self, Self::Error> {
        let file = value
            .file
            .filter(|file| !file.file_path.is_empty())
            .ok_or(ToolToAIAgentActionError::MissingUploadArtifactFileReference)?;

        Ok(AIAgentActionType::UploadArtifact(UploadArtifactRequest {
            file_path: file.file_path,
            description: value.description.none_if_default(),
        }))
    }
}

impl From<api::message::tool_call::SearchCodebase> for AIAgentActionType {
    fn from(value: api::message::tool_call::SearchCodebase) -> Self {
        AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
            query: value.query,
            partial_paths: if !value.path_filters.is_empty() {
                Some(value.path_filters)
            } else {
                None
            },
            codebase_path: if !value.codebase_path.is_empty() {
                Some(value.codebase_path)
            } else {
                None
            },
        })
    }
}

impl From<api::message::tool_call::Grep> for AIAgentActionType {
    fn from(value: api::message::tool_call::Grep) -> Self {
        AIAgentActionType::Grep {
            queries: value.queries,
            path: value.path,
        }
    }
}

impl From<api::message::tool_call::FileGlob> for AIAgentActionType {
    fn from(value: api::message::tool_call::FileGlob) -> Self {
        AIAgentActionType::FileGlob {
            patterns: value.patterns,
            path: if value.path.is_empty() {
                None
            } else {
                Some(value.path)
            },
        }
    }
}

impl From<api::message::tool_call::FileGlobV2> for AIAgentActionType {
    fn from(value: api::message::tool_call::FileGlobV2) -> Self {
        AIAgentActionType::FileGlobV2 {
            patterns: value.patterns,
            search_dir: if value.search_dir.is_empty() {
                None
            } else {
                Some(value.search_dir)
            },
        }
    }
}

impl From<api::message::tool_call::read_files::File> for FileLocations {
    fn from(value: api::message::tool_call::read_files::File) -> Self {
        Self {
            name: value.name,
            lines: value
                .line_ranges
                .into_iter()
                .map(|line_range| line_range.start as usize..line_range.end as usize)
                .collect(),
        }
    }
}

impl From<api::message::tool_call::ReadMcpResource> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadMcpResource) -> Self {
        let server_id = if FeatureFlag::MCPGroupedServerContext.is_enabled() {
            Uuid::parse_str(&value.server_id).ok()
        } else {
            None
        };

        AIAgentActionType::ReadMCPResource {
            server_id,
            uri: Some(value.uri),
            name: Default::default(),
        }
    }
}

impl TryFrom<api::message::tool_call::CallMcpTool> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::message::tool_call::CallMcpTool) -> Result<Self, Self::Error> {
        let Some(args) = value.args else {
            return Err(ToolToAIAgentActionError::CallMCPToolArgsError(
                String::from("missing args"),
            ));
        };

        let input = prost_to_serde_json(prost_types::Value {
            kind: Some(prost_types::value::Kind::StructValue(args)),
        })
        .map_err(ToolToAIAgentActionError::CallMCPToolArgsError)?;

        let server_id = if FeatureFlag::MCPGroupedServerContext.is_enabled() {
            Uuid::parse_str(&value.server_id).ok()
        } else {
            None
        };

        Ok(AIAgentActionType::CallMCPTool {
            server_id,
            name: value.name,
            input,
        })
    }
}

impl TryFrom<api::message::tool_call::SuggestPrompt> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::message::tool_call::SuggestPrompt) -> Result<Self, Self::Error> {
        let request = match value.display_mode {
            Some(api::message::tool_call::suggest_prompt::DisplayMode::InlineQueryBanner(
                inline_query_banner,
            )) => SuggestPromptRequest::UnitTestsSuggestion {
                title: inline_query_banner.title,
                description: inline_query_banner.description,
                query: inline_query_banner.query,
            },
            Some(api::message::tool_call::suggest_prompt::DisplayMode::PromptChip(chip)) => {
                let label = chip.label.none_if_default();
                SuggestPromptRequest::PromptSuggestion {
                    prompt: chip.prompt,
                    label,
                }
            }
            _ => {
                return Err(ToolToAIAgentActionError::SuggestPromptError(String::from(
                    "unsupported display mode",
                )));
            }
        };
        Ok(AIAgentActionType::SuggestPrompt(request))
    }
}

impl From<warp_multi_agent_api::FileContent> for FileContext {
    fn from(content: warp_multi_agent_api::FileContent) -> Self {
        let line_range = content.line_range.map(|r| r.start as usize..r.end as usize);

        FileContext::new(
            content.file_path,
            AnyFileContent::StringContent(content.content),
            line_range,
            None,
        )
    }
}

impl From<warp_multi_agent_api::AnyFileContent> for FileContext {
    fn from(content: warp_multi_agent_api::AnyFileContent) -> Self {
        match content.content {
            Some(api::any_file_content::Content::BinaryContent(binary_content)) => {
                FileContext::new(
                    binary_content.file_path,
                    AnyFileContent::BinaryContent(binary_content.data),
                    None,
                    None,
                )
            }
            Some(api::any_file_content::Content::TextContent(text_content)) => {
                let line_range = text_content
                    .line_range
                    .map(|r| r.start as usize..r.end as usize);

                FileContext::new(
                    text_content.file_path,
                    AnyFileContent::StringContent(text_content.content),
                    line_range,
                    None,
                )
            }
            None => unreachable!("AnyFileContent should always have a content"),
        }
    }
}

impl From<api::message::tool_call::ReadDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadDocuments) -> Self {
        use crate::agent::action::ReadDocumentsRequest;
        AIAgentActionType::ReadDocuments(ReadDocumentsRequest {
            document_ids: value
                .documents
                .into_iter()
                .filter_map(|doc| AIDocumentId::try_from(doc.document_id).ok())
                .collect(),
        })
    }
}

impl From<api::message::tool_call::EditDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::EditDocuments) -> Self {
        use crate::agent::action::{DocumentDiff, EditDocumentsRequest};
        AIAgentActionType::EditDocuments(EditDocumentsRequest {
            diffs: value
                .diffs
                .into_iter()
                .filter_map(|diff| {
                    AIDocumentId::try_from(diff.document_id)
                        .map(|document_id| DocumentDiff {
                            document_id,
                            search: diff.search,
                            replace: diff.replace,
                        })
                        .ok()
                })
                .collect(),
        })
    }
}

impl From<api::message::tool_call::CreateDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::CreateDocuments) -> Self {
        use crate::agent::action::{CreateDocumentsRequest, DocumentToCreate};
        AIAgentActionType::CreateDocuments(CreateDocumentsRequest {
            documents: value
                .new_documents
                .into_iter()
                .map(|doc| DocumentToCreate {
                    content: doc.content,
                    title: if doc.title.is_empty() {
                        // DO NOT SUBMIT
                        // crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE.to_string()
                        "".to_string()
                    } else {
                        doc.title
                    },
                })
                .collect(),
        })
    }
}

impl From<api::message::tool_call::ReadShellCommandOutput> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadShellCommandOutput) -> Self {
        let delay = match value.delay {
            Some(api::message::tool_call::read_shell_command_output::Delay::Duration(duration)) => {
                Some(ShellCommandDelay::Duration(Duration::from_secs(
                    duration.seconds as u64,
                )))
            }
            Some(api::message::tool_call::read_shell_command_output::Delay::OnCompletion(_)) => {
                Some(ShellCommandDelay::OnCompletion)
            }
            None => None,
        };
        AIAgentActionType::ReadShellCommandOutput {
            block_id: value.command_id.into(),
            delay,
        }
    }
}

impl From<api::message::tool_call::TransferShellCommandControlToUser> for AIAgentActionType {
    fn from(value: api::message::tool_call::TransferShellCommandControlToUser) -> Self {
        AIAgentActionType::TransferShellCommandControlToUser {
            reason: value.reason,
        }
    }
}

impl TryFrom<api::message::tool_call::UseComputer> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::message::tool_call::UseComputer) -> Result<Self, Self::Error> {
        use api::message::tool_call::use_computer;

        let actions = value
            .actions
            .into_iter()
            .map(|action| {
                let target = convert_computer_use_target(action.target);
                let Some(action_type) = action.r#type else {
                    return Err(ToolToAIAgentActionError::MissingComputerUseActionType);
                };
                let action = match action_type {
                    use_computer::action::Type::MouseMove(mouse_move) => {
                        Ok(computer_use::Action::MouseMove {
                            to: coordinates_to_vec(mouse_move.to.as_ref())?,
                        })
                    }
                    use_computer::action::Type::MouseDown(mouse_down) => {
                        Ok(computer_use::Action::MouseDown {
                            button: to_computer_use_button(mouse_down.button()),
                            at: coordinates_to_vec(mouse_down.at.as_ref())?,
                        })
                    }
                    use_computer::action::Type::MouseUp(mouse_up) => {
                        Ok(computer_use::Action::MouseUp {
                            button: to_computer_use_button(mouse_up.button()),
                        })
                    }
                    use_computer::action::Type::MouseWheel(mouse_wheel) => {
                        let direction = to_scroll_direction(mouse_wheel.direction());
                        let distance = to_scroll_distance(mouse_wheel.distance)?;
                        Ok(computer_use::Action::MouseWheel {
                            at: coordinates_to_vec(mouse_wheel.at.as_ref())?,
                            direction,
                            distance,
                        })
                    }
                    use_computer::action::Type::Wait(wait) => {
                        let duration = wait.duration.unwrap_or_default();
                        if duration.seconds < 0 || duration.nanos < 0 {
                            return Err(ToolToAIAgentActionError::InvalidComputerUseWaitDuration);
                        }
                        let duration = Duration::from_secs(duration.seconds as u64)
                            + Duration::from_nanos(duration.nanos as u64);
                        Ok(computer_use::Action::Wait(duration))
                    }
                    use_computer::action::Type::TypeText(type_text) => {
                        Ok(computer_use::Action::TypeText {
                            text: type_text.text,
                        })
                    }
                    use_computer::action::Type::KeyDown(key_down) => {
                        let key = convert_key(key_down.key)?;
                        Ok(computer_use::Action::KeyDown { key })
                    }
                    use_computer::action::Type::KeyUp(key_up) => {
                        let key = convert_key(key_up.key)?;
                        Ok(computer_use::Action::KeyUp { key })
                    }
                }?;
                log_window_target_coord(&action, target);
                Ok(computer_use::TargetedAction { action, target })
            })
            .try_collect()?;
        let screenshot_params = value
            .post_actions_screenshot_params
            .map(convert_screenshot_params);
        Ok(AIAgentActionType::UseComputer(UseComputerRequest {
            action_summary: value.action_summary,
            actions,
            screenshot_params,
        }))
    }
}

impl From<api::message::tool_call::RequestComputerUse> for AIAgentActionType {
    fn from(value: api::message::tool_call::RequestComputerUse) -> Self {
        use crate::agent::action::RequestComputerUseRequest;
        AIAgentActionType::RequestComputerUse(RequestComputerUseRequest {
            task_summary: value.task_summary,
            screenshot_params: value.screenshot_params.map(convert_screenshot_params),
        })
    }
}

impl From<api::message::tool_call::StartRecording> for AIAgentActionType {
    fn from(value: api::message::tool_call::StartRecording) -> Self {
        let limits = value.limits;
        AIAgentActionType::StartRecording {
            frame_rate: value.frame_rate.max(0) as u32,
            max_duration: limits
                .as_ref()
                .and_then(|l| l.max_duration.as_ref())
                .map(|d| Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32)),
            max_size_bytes: limits
                .as_ref()
                .map(|l| l.max_size_bytes)
                .filter(|&bytes| bytes > 0)
                .map(|bytes| bytes as u64),
        }
    }
}

impl From<api::message::tool_call::StopRecording> for AIAgentActionType {
    fn from(value: api::message::tool_call::StopRecording) -> Self {
        AIAgentActionType::StopRecording {
            recording_id: value.recording_id,
        }
    }
}

impl From<api::message::tool_call::FetchConversation> for AIAgentActionType {
    fn from(value: api::message::tool_call::FetchConversation) -> Self {
        AIAgentActionType::FetchConversation {
            conversation_id: value.conversation_id,
        }
    }
}

/// Converts API ScreenshotParams to the internal computer_use type.
fn convert_screenshot_params(
    params: api::message::tool_call::ScreenshotParams,
) -> computer_use::ScreenshotParams {
    let region = params
        .region
        .and_then(|r| match (r.top_left.as_ref(), r.bottom_right.as_ref()) {
            (Some(tl), Some(br)) => Some(computer_use::ScreenshotRegion {
                top_left: computer_use::Vector2I::new(tl.x, tl.y),
                bottom_right: computer_use::Vector2I::new(br.x, br.y),
            }),
            _ => None,
        });

    computer_use::ScreenshotParams {
        max_long_edge_px: (params.max_long_edge_px > 0).then_some(params.max_long_edge_px as usize),
        max_total_px: (params.max_total_px > 0).then_some(params.max_total_px as usize),
        region,
        target: convert_computer_use_target(params.target),
    }
}

/// Converts an optional API `ComputerUseTarget` into the internal computer_use target. An absent
/// or `Screen` target maps to the legacy whole-screen behavior.
fn convert_computer_use_target(
    target: Option<api::message::tool_call::ComputerUseTarget>,
) -> computer_use::Target {
    use api::message::tool_call::computer_use_target::Target as ApiTarget;
    match target.and_then(|t| t.target) {
        // The proto window id is an opaque string; on macOS it is a CGWindowID, so parse it back
        // to a u32. An unparseable id is treated as no valid window target and falls back to the
        // legacy whole-screen behavior rather than panicking.
        Some(ApiTarget::Window(window)) => match window.window_id.parse::<u32>() {
            Ok(window_id) => computer_use::Target::Window {
                window_id,
                pid: window.pid,
            },
            Err(_) => computer_use::Target::Screen,
        },
        Some(ApiTarget::Screen(_)) | None => computer_use::Target::Screen,
    }
}

/// Logs the raw server-provided coordinates for a window-targeted computer-use action, so the
/// agent-driven coordinate conversion can be compared against where the click actually lands.
/// Gated on COMPUTER_USE_DEBUG and routed through `log` so it surfaces in the app's log file.
fn log_window_target_coord(action: &computer_use::Action, target: computer_use::Target) {
    if std::env::var_os("COMPUTER_USE_DEBUG").is_none() {
        return;
    }
    let computer_use::Target::Window { window_id, pid } = target else {
        return;
    };
    let coord = match action {
        computer_use::Action::MouseMove { to } => Some(("mouse_move", to.x(), to.y())),
        computer_use::Action::MouseDown { at, .. } => Some(("mouse_down", at.x(), at.y())),
        computer_use::Action::MouseWheel { at, .. } => Some(("mouse_wheel", at.x(), at.y())),
        _ => None,
    };
    if let Some((kind, x, y)) = coord {
        log::info!(
            "[computer_use] server->client {kind} window#={window_id} pid={pid} raw_coord=({x},{y})"
        );
    }
}

fn coordinates_to_vec(
    coords: Option<&api::Coordinates>,
) -> Result<computer_use::Vector2I, ToolToAIAgentActionError> {
    match coords {
        Some(coords) => Ok(computer_use::Vector2I::new(coords.x, coords.y)),
        None => Err(ToolToAIAgentActionError::MissingComputerUseCoordinates),
    }
}

fn to_computer_use_button(
    api_button: api::message::tool_call::use_computer::action::MouseButton,
) -> computer_use::MouseButton {
    use api::message::tool_call::use_computer::action::MouseButton;
    match api_button {
        MouseButton::Left => computer_use::MouseButton::Left,
        MouseButton::Right => computer_use::MouseButton::Right,
        MouseButton::Middle => computer_use::MouseButton::Middle,
        MouseButton::Back => computer_use::MouseButton::Back,
        MouseButton::Forward => computer_use::MouseButton::Forward,
    }
}

fn to_scroll_direction(
    api_direction: api::message::tool_call::use_computer::action::mouse_wheel::Direction,
) -> computer_use::ScrollDirection {
    use api::message::tool_call::use_computer::action::mouse_wheel::Direction;
    match api_direction {
        Direction::Up => computer_use::ScrollDirection::Up,
        Direction::Down => computer_use::ScrollDirection::Down,
        Direction::Left => computer_use::ScrollDirection::Left,
        Direction::Right => computer_use::ScrollDirection::Right,
    }
}

fn to_scroll_distance(
    api_distance: Option<api::message::tool_call::use_computer::action::mouse_wheel::Distance>,
) -> Result<computer_use::ScrollDistance, ToolToAIAgentActionError> {
    use api::message::tool_call::use_computer::action::mouse_wheel::Distance;
    match api_distance {
        Some(Distance::Pixels(pixels)) => Ok(computer_use::ScrollDistance::Pixels(pixels)),
        Some(Distance::Clicks(clicks)) => Ok(computer_use::ScrollDistance::Clicks(clicks)),
        None => Err(ToolToAIAgentActionError::MissingComputerUseScrollDistance),
    }
}

fn convert_key(
    api_key: Option<api::message::tool_call::use_computer::action::Key>,
) -> Result<computer_use::Key, ToolToAIAgentActionError> {
    use api::message::tool_call::use_computer::action::key::Data;
    let key = api_key.ok_or(ToolToAIAgentActionError::MissingComputerUseKey)?;
    match key.data {
        Some(Data::Keycode(keycode)) => Ok(computer_use::Key::Keycode(keycode)),
        Some(Data::Char(char_str)) => {
            let mut chars = char_str.chars();
            let ch = chars
                .next()
                .ok_or(ToolToAIAgentActionError::InvalidComputerUseCharKey)?;
            if chars.next().is_some() {
                return Err(ToolToAIAgentActionError::InvalidComputerUseCharKey);
            }
            Ok(computer_use::Key::Char(ch))
        }
        None => Err(ToolToAIAgentActionError::MissingComputerUseKey),
    }
}

fn prost_to_serde_json(x: prost_types::Value) -> Result<serde_json::Value, String> {
    use prost_types::value::Kind::*;
    use serde_json::Value::*;

    let Some(kind) = x.kind else {
        return Err("google.protobuf.Value kind was None".to_string());
    };

    Ok(match kind {
        NullValue(_) => Null,
        BoolValue(v) => Bool(v),
        NumberValue(n) => Number(
            serde_json::Number::from_f64(n)
                .ok_or_else(|| format!("float {n} is not valid JSON number"))?,
        ),
        StringValue(s) => String(s),
        ListValue(l) => Array(
            l.values
                .into_iter()
                .map(prost_to_serde_json)
                .collect::<Result<Vec<_>, std::string::String>>()?,
        ),
        StructValue(v) => Object(
            v.fields
                .into_iter()
                .map(|(k, v)| prost_to_serde_json(v).map(|v| (k, v)))
                .collect::<Result<serde_json::Map<_, _>, std::string::String>>()?,
        ),
    })
}

/// Helper trait to easily convert default values to `None`.
/// With `prost`, scalar types are converted to their default values instead
/// of `None` if the type is unset. This trait allows a more natural
/// conversion to better denote if a value should be `Some` (indicating it was set)
/// or `None` (indicating it was unset).
///
///
/// NOTE: Consumers should use this with caution as it only makes sense where the default
/// value of the type isn't a reasonable value and `None` makes more sense instead.
trait NoneIfDefault
where
    Self: Sized + Default,
{
    fn none_if_default(self) -> Option<Self>;
}

impl NoneIfDefault for String {
    fn none_if_default(self) -> Option<Self> {
        if self == Self::default() {
            None
        } else {
            Some(self)
        }
    }
}

impl From<api::message::tool_call::InsertReviewComments> for AIAgentActionType {
    fn from(value: api::message::tool_call::InsertReviewComments) -> Self {
        AIAgentActionType::InsertCodeReviewComments {
            repo_path: PathBuf::from(value.repo_path),
            comments: value.comments.into_iter().map(Into::into).collect(),
            base_branch: value.base_branch.none_if_default(),
        }
    }
}

impl From<api::message::tool_call::insert_review_comments::CommentSide> for CommentSide {
    fn from(value: api::message::tool_call::insert_review_comments::CommentSide) -> Self {
        match value {
            api::message::tool_call::insert_review_comments::CommentSide::New => CommentSide::Right,
            api::message::tool_call::insert_review_comments::CommentSide::Old => CommentSide::Left,
        }
    }
}

impl From<api::message::tool_call::insert_review_comments::Comment> for InsertReviewComment {
    fn from(value: api::message::tool_call::insert_review_comments::Comment) -> Self {
        let location = value.location.map(|loc| InsertedCommentLocation {
            relative_file_path: loc.file_path,
            line: loc.line.and_then(|comment_line_range| {
                let side = comment_line_range.side().into();
                let diff_hunk = comment_line_range.diff_hunk;
                comment_line_range.range.map(|r| InsertedCommentLine {
                    comment_line_range: r.start as usize..r.end as usize,
                    diff_hunk_line_range: r.start as usize..r.end as usize,
                    diff_hunk_text: diff_hunk,
                    side: Some(side),
                })
            }),
        });

        InsertReviewComment {
            comment_id: value.comment_id,
            author: value.author,
            last_modified_timestamp: value.last_modified_timestamp,
            comment_body: value.comment_body,
            parent_comment_id: if value.parent_comment_id.is_empty() {
                None
            } else {
                Some(value.parent_comment_id)
            },
            comment_location: location,
            html_url: value.html_url.none_if_default(),
        }
    }
}
