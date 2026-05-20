mod interaction_mode;
mod serialized_block;

pub use interaction_mode::*;
pub use serialized_block::*;
use warp_core::features::FeatureFlag;

use super::grid::grid_handler::{GridHandler, PerformResetGridChecks};
use super::grid::{Cursor, RespectDisplayedOutput};
use super::header_grid::HeaderGrid;
use super::header_grid::PromptEndPoint;
use super::image_map::StoredImageMetadata;
use super::kitty::{KittyAction, KittyResponse};
use super::secrets::RespectObfuscatedSecrets;
use super::selection::ScrollDelta;
use super::session::{command_executor, Sessions};
pub use super::BlockId;
use super::{bootstrap::BootstrapStage, find::RegexDFAs};
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::{AgentViewDisplayMode, AgentViewState};
use crate::{
    ai::agent::redaction::redact_secrets,
    context_chips::prompt_snapshot::PromptSnapshot,
    server::{block::DisplaySetting, ids::SyncId},
    terminal::{
        block_filter::BlockFilterQuery,
        block_list_element::GridType,
        event::{
            BlockCompletedEvent, BlockLatencyData, BlockMetadataReceivedEvent, BlockType, Event,
            UserBlockCompleted,
        },
        event_listener::ChannelEventListener,
        model::{
            ansi::{self, PrecmdValue, PreexecValue, Processor},
            blockgrid::BlockGrid,
            grid::grid_handler::TermMode,
            index::{Point, VisibleRow},
            iterm_image::ITermImage,
            secrets::ObfuscateSecrets,
            session::SessionId,
            terminal_model::{BlockIndex, WithinBlock},
            GridStorage,
        },
        shell::ShellType,
        view::WithinBlockBanner,
        BlockPadding, ShellHost, SizeInfo,
    },
};

use chrono::{DateTime, Duration, FixedOffset, Local};
use hex;
use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::command::ExitCode;
use warp_terminal::model::grid::Dimensions as _;
use warp_util::path::user_friendly_path;
use warpui::units::{IntoLines, Lines};
use warpui::{r#async::executor::Background, record_trace_event};

use enum_iterator::all;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::ops::Range;
use std::{
    borrow::Cow,
    collections::HashSet,
    io,
    iter::DoubleEndedIterator,
    num::NonZeroUsize,
    ops::RangeInclusive,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const LONG_RUNNING_COMMAND_DURATION_MS: u64 = 50;
pub const LONG_RUNNING_BOTTOM_PADDING_LINES: f32 = 0.2;

/// We don't consider commands that were killed via Ctrl-C (error code 130) or that were killed
/// by SIGPIPE (error code 141) to have failed. We also don't consider the exit code for any
/// commands that didn't start execution (i.e. `preexec` was never called), as the exit code is
/// only for the last point of execution.
/// Note: we should keep this in sync with the command-corrections list:
/// https://github.com/warpdotdev/command-corrections/blob/main/src/lib.rs#L109
pub(super) fn has_block_failed(exit_code: ExitCode, block_state: BlockState) -> bool {
    block_state == BlockState::DoneWithExecution && !exit_code.was_successful()
}

pub(super) const MAX_SERIALIZED_STYLIZED_OUTPUT_LINES: usize = 5000;

/// Number of max lines to store that aren't stylized. We only store 50 lines as we only need
/// non-stylized lines for command corrections and notifications whereas we need more lines for the
/// stylized output for session restoration.
const MAX_SERIALIZED_OUTPUT_LINES: usize = 50;

/// Numbers for converting from a duration to a formatted string.
const MILLIS_PER_MIN: i64 = 60000;
const MINS_PER_HOUR: i64 = 60;

/// Floating-point numbers are expressive but not always precise, and they become less precise for
/// larger numbers. Our row-coordinates in the BlockList are stored as floating-points, and the function
/// block.find() makes comparisons between floating-point sums to find, given a BlockList row coordinate,
/// the location within a specific BlockSection (e.g., OutputGrid, Prompt, BottomPadding, etc). Because of
/// precision issues, row coordinates exactly on the row boundary may on occasion be arbitrarily and incorrectly
/// lumped into the lesser row (the one above).
///
/// By adding a small decimal to the row coordinate, we offset possible downwards precision errors. The value
/// of .0001 was found from experimentation. This could technically cause a misrendering where if the user
/// drags to a row-coordinate like 5.9999 we would place this at 6. All points after the selection mouse-up
/// event have been normalized to lie at the exact row threshold, so it can't cause a misrendering at that point.
const FLOATING_POINT_ROUNDING_ADJUSTMENT: f32 = 0.0001;

/// Delay before we mark a new background output block as ready to render. When we
/// fetch input from the shell for typeahead, we then clear it so that the typeahead
/// text isn't printed twice. If a background block only contains typeahead, this
/// causes a flicker as the block is briefly shown then hidden. To prevent this,
/// we wait before rendering the block so that the typeahead stays hidden.
#[cfg(not(test))]
const BACKGROUND_OUTPUT_RENDER_DELAY_MS: u64 = 100;

/// Minimum terminal width for truncation calculations, we use this to determine
/// how many rows to take for much narrower terminals, to ensure we have enough content
/// for block summaries given to AI.
const MIN_TERMINAL_WIDTH_FOR_TRUNCATION_CALCULATIONS: usize = 150;

lazy_static! {
    /// A set of commands that perform minimal work that we use as a baseline to measure the latency of blocks.
    /// Note that while the empty command doesn't invoke pre-exec, it still does get a newline from
    /// the shell, and runs precmd.
    static ref BASELINE_COMMANDS: HashSet<&'static str> = HashSet::from(["", "pwd", "whoami", "cd"]);
}

/// Blocklist Env Var metadata associated with this block.
#[derive(Debug, Clone)]
pub struct BlocklistEnvVarMetadata {
    /// The id used to uniquely identify the block's execution
    pub block_id: String,
    /// whether or not the env var block should be hidden
    pub should_hide_block: bool,
}

/// Tracks which views (terminal and/or agent conversations) a block should be visible in.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AgentViewVisibility {
    /// Block was created in terminal mode. It should always be visible in terminal view,
    /// and may also be attached to conversations as context.
    Terminal {
        /// Conversation IDs where this block is in pending context.
        pending_conversation_ids: HashSet<AIConversationId>,
        /// Conversation IDs where this block was attached as context.
        conversation_ids: HashSet<AIConversationId>,
    },
    /// Block was created inside an agent view conversation.
    Agent {
        /// The conversation where this block originally executed (the one where users saw this command run).
        origin_conversation_id: AIConversationId,
        /// Other conversations where users currently see this block as pending context before send.
        pending_other_conversation_ids: HashSet<AIConversationId>,
        /// Other conversations where users see this block as attached context after send.
        other_conversation_ids: HashSet<AIConversationId>,
    },
}

impl AgentViewVisibility {
    /// Visibility for a block created in the top-level terminal (not in an agent view).
    pub fn new_from_terminal() -> Self {
        Self::Terminal {
            pending_conversation_ids: HashSet::new(),
            conversation_ids: HashSet::new(),
        }
    }

    /// Visibility for a block created inside an agent view conversation.
    pub fn new_from_conversation(conversation_id: AIConversationId) -> Self {
        Self::Agent {
            origin_conversation_id: conversation_id,
            pending_other_conversation_ids: HashSet::new(),
            other_conversation_ids: HashSet::new(),
        }
    }

    pub fn agent_view_conversation_id(&self) -> Option<AIConversationId> {
        match self {
            Self::Terminal { .. } => None,
            Self::Agent {
                origin_conversation_id,
                ..
            } => Some(*origin_conversation_id),
        }
    }

    /// Adds a conversation ID to the set of conversations where this block was attached as context in a request.
    fn add_attached_conversation_id(&mut self, id: AIConversationId) {
        match self {
            Self::Terminal {
                conversation_ids, ..
            } => {
                conversation_ids.insert(id);
            }
            Self::Agent {
                origin_conversation_id,
                other_conversation_ids,
                ..
            } => {
                if id == *origin_conversation_id {
                    return;
                }
                other_conversation_ids.insert(id);
            }
        }
    }

    /// Marks the block as pending context in the conversation with the given ID.
    /// It maybe removed if the user removes the block attachment before sending the request, else if it is attached it will be 'promoted'.
    fn add_pending_conversation_id(&mut self, id: AIConversationId) {
        match self {
            Self::Terminal {
                pending_conversation_ids,
                ..
            } => {
                pending_conversation_ids.insert(id);
            }
            Self::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                ..
            } => {
                if id == *origin_conversation_id {
                    return;
                }
                pending_other_conversation_ids.insert(id);
            }
        }
    }

    /// Moves the block from pending context to attached context for the given conversation ID.
    /// Returns true if the conversation was in pending and was promoted, false otherwise.
    fn promote_pending_to_attached(&mut self, id: AIConversationId) -> bool {
        match self {
            Self::Terminal {
                pending_conversation_ids,
                conversation_ids,
            } => {
                if pending_conversation_ids.remove(&id) {
                    conversation_ids.insert(id);
                    true
                } else {
                    false
                }
            }
            Self::Agent {
                pending_other_conversation_ids,
                other_conversation_ids,
                ..
            } => {
                if pending_other_conversation_ids.remove(&id) {
                    other_conversation_ids.insert(id);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Removes a pending conversation ID from the set of conversations where this block should be visible.
    /// Returns true if the conversation ID was present and removed, false if it wasn't present.
    fn remove_pending_conversation_id(&mut self, id: AIConversationId) -> bool {
        match self {
            Self::Terminal {
                pending_conversation_ids,
                ..
            } => pending_conversation_ids.remove(&id),
            Self::Agent {
                pending_other_conversation_ids,
                ..
            } => pending_other_conversation_ids.remove(&id),
        }
    }
}

pub struct Block {
    id: BlockId,
    size: SizeInfo,
    header_grid: HeaderGrid,
    rprompt_grid: BlockGrid,
    output_grid: BlockGrid,
    padding: BlockPadding,
    state: BlockState,
    precmd_state: PrecmdState,
    pwd: Option<String>,
    git_branch: Option<String>,
    git_branch_name: Option<String>,
    virtual_env: Option<String>,
    conda_env: Option<String>,
    node_version: Option<String>,
    exit_code: ExitCode,
    session_id: Option<SessionId>,
    rprompt: Option<String>,

    /// Executor used for spawning futures in the background
    #[allow(dead_code)]
    background_executor: Arc<Background>,

    event_proxy: ChannelEventListener,

    render_delay_complete: Arc<AtomicBool>,
    was_long_running: AtomicBool,
    bootstrap_stage: BootstrapStage,

    show_bootstrap_block: bool,
    show_in_band_command_blocks: bool,
    show_memory_stats: bool,

    /// The timestamp at which this block was created (i.e.: the previous block
    /// finished and this was created, waiting for the user to execute a
    /// command).
    creation_ts: DateTime<Local>,

    /// The timestamp at which this block's command was submitted to the shell.
    /// This should be set iff the block was started.
    start_ts: Option<DateTime<Local>>,

    /// The timestamp at which we finished receiving output for this block.
    /// This should be set iff the block was finished.
    completed_ts: Option<DateTime<Local>>,

    block_index: BlockIndex,

    /// This contains some information about the session the block executed in. Primarily used to
    /// determine if commands in a restored session should be included in
    /// History::session_commands. This is optional b/c just like session_id, pwd, git_branch, etc.
    /// which are determined at precmd time, it is unset at block creation. It is also to
    /// accommodate the case where determining the ShellHost fails during session restoration, e.g.
    /// if the values in sqlite are NULL or invalid.
    shell_host: Option<ShellHost>,

    /// `true` if this block is for an in-band command, executed via the `InBandCommandExecutor`.
    ///
    /// Blocks for in-band commands are hidden by default, but still created and stored in the
    /// model because they may be shown for debugging purposes.
    pub(super) is_for_in_band_command: bool,

    /// `true` if this command block corresponds to a startup command in an oz environment executed
    /// in cloud mode.
    is_oz_environment_startup_command: bool,

    /// Blocklist Env var metadata associated with this block, if any.
    env_var_metadata: Option<BlocklistEnvVarMetadata>,

    /// Represents the 'interaction mode' for a command block with respect to the agent.
    ///
    /// See doc comment on [`InteractionMode`] for detailed explanation of semantics.
    interaction_mode: InteractionMode,

    /// This represents when a banner appears in this Block above the prompt.
    pub(super) block_banner: Option<WithinBlockBanner>,

    /// If true, we should discard the next right prompt data we receive
    /// (whether it comes from a precmd hook or from a marked prompt
    /// printed by the shell).
    ignore_next_rprompt: bool,

    prompt_snapshot: Option<PromptSnapshot>,

    /// The home directory the block was executed in.
    home_dir: Option<String>,

    filter_query: Option<BlockFilterQuery>,

    /// If the command is a cloud workflow, this is set to its id. If the block was not a workflow,
    /// this is None.
    cloud_workflow_id: Option<SyncId>,

    /// If the command included an env var invocation. If not this will be None.
    cloud_env_var_collection_id: Option<SyncId>,

    /// The last time this block was painted (i.e.: visible in the window),
    /// if ever.
    ///
    /// While [`Block`] is ultimately owned by `TerminalModel`, which is shared
    /// across threads, this field will only be updated or read from the main
    /// thread, so using a [`RefCell`] is safe.
    last_painted_at: std::cell::RefCell<Option<DateTime<Local>>>,

    /// `true` if the command corresponding to this block (whether it utilized the alt screen or
    /// not) has received user input (e.g. keystroke).
    ///
    /// This may only be `true` for long running commands.
    has_received_user_input: bool,

    /// When true, don't show the block in the blocklist.
    hidden: bool,

    /// If `true`, the output grid should not be rendered.
    should_hide_output_grid: bool,

    /// If `true`, the prompt+command grid should not be rendered.
    should_hide_command_grid: bool,

    /// [`Self::linefeed`] may discard some linefeeds at the beginning of the prompt. Doing so will
    /// alter the row numbers for [`Self::goto`] and [`Self::goto_line`] when ConPTY is involved. We
    /// track the count of discarded newlines here in order to correct the row number.
    leading_linefeeds_ignored: usize,

    /// `true` if client-side telemetry for user-generated AI data is enabled.
    pub(super) is_ai_ugc_telemetry_enabled: bool,

    /// Only set on restored blocks. Indicates whether the block was local or from a remote session.
    restored_block_was_local: Option<bool>,

    /// Tracks which views (terminal and/or agent conversations) this block should be visible in.
    ///
    /// This is only used if `FeatureFlag::AgentView` is enabled.
    agent_view_visibility: AgentViewVisibility,

    /// Whether natural language detection (NLD) was overridden (i.e., the user had manually locked
    /// the input type) at the time this block's command was submitted.
    ///
    /// This is used for debugging UI shown in the block header on dogfood builds.
    nld_overridden: bool,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Block")
            .field("block_index", &self.block_index)
            .field("prompt_grid", self.header_grid.prompt_grid())
            .field("rprompt_grid", &self.rprompt_grid)
            .field(
                "prompt_and_command_grid",
                &self.header_grid.prompt_and_command_grid(),
            )
            .field("output_grid", &self.output_grid)
            .field("is_for_in_band_command", &self.is_for_in_band_command)
            .field("state", &self.state)
            .field("precmd_state", &self.precmd_state)
            .field("creation_ts", &self.creation_ts)
            .field("start_ts", &self.start_ts)
            .field("completed_ts", &self.completed_ts)
            .field("was_long_running", &self.was_long_running)
            .field("exit_code", &self.exit_code)
            .finish()
    }
}

#[derive(Debug, PartialEq)]
enum PrecmdState {
    BeforePrecmd,
    AfterPrecmd,
}

/// Helper that groups the Block time-related fields together.
pub struct BlockTime {
    pub time_started_term: DateTime<FixedOffset>,
    pub time_completed_term: DateTime<FixedOffset>,
}

impl BlockTime {
    pub fn new(
        time_started_term: DateTime<FixedOffset>,
        time_completed_term: DateTime<FixedOffset>,
    ) -> Self {
        BlockTime {
            time_started_term,
            time_completed_term,
        }
    }
}

/// Helper that groups the Block text-related fields together.
pub struct BlockCommand {
    pub command: String,
    pub output: String,
    pub stylized_command: String,
    pub stylized_output: String,
    pub stylized_prompt: String,
}

impl BlockCommand {
    pub fn new(
        command: String,
        output: String,
        stylized_command: String,
        stylized_output: String,
        stylized_prompt: String,
    ) -> Self {
        BlockCommand {
            command,
            output,
            stylized_command,
            stylized_output,
            stylized_prompt,
        }
    }
}

/// Helper that groups the prompt-related fields together.
pub struct PromptInfo {
    pub pwd: Option<String>,
    pub git_branch: Option<String>,
    pub git_branch_name: Option<String>,
    pub virtual_env: Option<String>,
    pub conda_env: Option<String>,
    pub node_version: Option<String>,
    pub ps1: Option<String>,
    pub rprompt: Option<String>,
    pub honor_ps1: bool,
    /// JSON serialization of [`PromptSnapshot`]
    pub prompt_snapshot: Option<String>,
}

impl From<&Block> for BlockType {
    fn from(block: &Block) -> Self {
        if block.is_for_in_band_command {
            return BlockType::InBandCommand;
        }
        if block.is_static() {
            return BlockType::Static;
        }

        match block.bootstrap_stage() {
            BootstrapStage::RestoreBlocks => BlockType::Restored,
            BootstrapStage::WarpInput | BootstrapStage::Bootstrapped => BlockType::BootstrapHidden,
            BootstrapStage::ScriptExecution => {
                if block.is_empty(&AgentViewState::Inactive) {
                    BlockType::BootstrapHidden
                } else {
                    let serialized_block = block.into();
                    BlockType::BootstrapVisible(Arc::new(serialized_block))
                }
            }
            BootstrapStage::PostBootstrapPrecmd => {
                let serialized_block = block.into();

                if block.is_background() {
                    BlockType::Background(Arc::new(serialized_block))
                } else {
                    let command = block.command_to_string();
                    let mut command_with_obfuscated_secrets =
                        block.command_with_secrets_obfuscated(false);

                    let (output_truncated, mut output_truncated_with_obfuscated_secrets) =
                        if block.is_ai_ugc_telemetry_enabled {
                            // If telemetry is enabled, we collect the full output but are limiting it to
                            // the first and last 2500 lines in case the block is very large.
                            (
                                block.output_grid().content_summary(2500, 2500, false),
                                block.output_grid().content_summary(2500, 2500, true),
                            )
                        } else {
                            (
                                block
                                    .output_grid()
                                    .contents_to_string(false, Some(MAX_SERIALIZED_OUTPUT_LINES)),
                                block
                                    .output_grid()
                                    .contents_to_string_force_secrets_obfuscated(
                                        false,
                                        Some(MAX_SERIALIZED_OUTPUT_LINES),
                                    ),
                            )
                        };

                    // If secret redaction is disabled, we manually scan for secrets and redact them.
                    if matches!(
                        block.prompt_and_command_grid().should_scan_for_secrets,
                        ObfuscateSecrets::No
                    ) {
                        redact_secrets(&mut command_with_obfuscated_secrets);
                    }
                    if matches!(
                        block.output_grid().should_scan_for_secrets,
                        ObfuscateSecrets::No
                    ) {
                        redact_secrets(&mut output_truncated_with_obfuscated_secrets);
                    }

                    BlockType::User(UserBlockCompleted {
                        index: block.block_index,
                        serialized_block: Arc::new(serialized_block),
                        command,
                        command_with_obfuscated_secrets,
                        output_truncated,
                        output_truncated_with_obfuscated_secrets,
                        was_part_of_agent_interaction: block.agent_interaction_metadata().is_some(),
                        started_at: block.command_start_time(),
                        num_output_lines: block.output_grid().len() as u64,
                        num_output_lines_truncated: block
                            .output_grid()
                            .grid_handler()
                            .num_lines_truncated(),
                    })
                }
            }
        }
    }
}

impl From<&mut Block> for BlockType {
    fn from(block: &mut Block) -> Self {
        Self::from(&*block)
    }
}

#[derive(Clone)]
pub struct BlockSize {
    pub block_padding: BlockPadding,
    pub size: SizeInfo,
    pub max_block_scroll_limit: usize,
    pub warp_prompt_height_lines: f32,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BlockState {
    /// The block has not started executing.
    /// This encompasses the period after the command has been sent to the pty
    /// but before preexec is called.  It includes any echoing of characters from
    /// the shell.
    BeforeExecution,

    /// A command in the grid is currently executing - we are between preexec
    /// and precmd.
    Executing,

    /// The block is done and execution occurred.
    DoneWithExecution,

    /// The block is done and no execution occurred.
    DoneWithNoExecution,

    /// This block holds background process output, and is not associated with
    /// any particular execution or command.
    Background,

    /// This block holds static content and is programmatically added to the blocklist by Warp. An
    /// example is the information subshell bootstrap "success" block.
    Static,
}

#[derive(Debug, Clone)]
pub struct BlockMetadata {
    session_id: Option<SessionId>,
    current_working_directory: Option<String>,
}

impl BlockMetadata {
    pub fn new(session_id: Option<SessionId>, current_working_directory: Option<String>) -> Self {
        BlockMetadata {
            session_id,
            current_working_directory,
        }
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }

    pub fn current_working_directory(&self) -> Option<&str> {
        self.current_working_directory.as_deref()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BlockSection {
    /// The banner at the top of the block, above the "top padding".
    BlockBanner,
    /// Padding between the top of the block and the prompt.
    PaddingTop,
    /// Padding between the prompt and the command.
    CommandPaddingTop,
    // Combined prompt/command grid.
    PromptAndCommandGrid(Lines),
    PaddingMiddle,
    OutputGrid(Lines),
    PaddingBottom,
    EndOfBlock,
    NotContained,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BlockGridPoint {
    Prompt(Point),
    Rprompt(Point),
    PromptAndCommand(Point),
    Output(Point),
}

impl From<WithinBlock<Point>> for BlockGridPoint {
    fn from(point: WithinBlock<Point>) -> Self {
        match point.grid {
            GridType::Prompt => BlockGridPoint::Prompt(*point.get()),
            GridType::Rprompt => BlockGridPoint::Rprompt(*point.get()),
            GridType::Output => BlockGridPoint::Output(*point.get()),
            GridType::PromptAndCommand => BlockGridPoint::PromptAndCommand(*point.get()),
        }
    }
}

impl From<BlockGridPoint> for GridType {
    fn from(p: BlockGridPoint) -> GridType {
        match p {
            BlockGridPoint::Output(_) => GridType::Output,
            BlockGridPoint::Prompt(_) => GridType::Prompt,
            BlockGridPoint::Rprompt(_) => GridType::Rprompt,
            BlockGridPoint::PromptAndCommand(_) => GridType::PromptAndCommand,
        }
    }
}

impl BlockGridPoint {
    pub fn grid_point(&self) -> Point {
        match self {
            BlockGridPoint::Output(point) => *point,
            BlockGridPoint::Prompt(point) => *point,
            BlockGridPoint::Rprompt(point) => *point,
            BlockGridPoint::PromptAndCommand(point) => *point,
        }
    }

    pub fn to_within_block_point(self, block_index: BlockIndex) -> WithinBlock<Point> {
        WithinBlock::new(self.grid_point(), block_index, self.into())
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GridIndex {
    PromptAndCommand(usize),
    Command(usize),
    Output(usize),
}

#[derive(Debug)]
pub struct BlockMatches {
    prompt_and_command_grid_matches: Vec<RangeInclusive<Point>>,
    // TODO(advait): Remove this, once we complete the same-line prompt migration.
    command_grid_matches: Vec<RangeInclusive<Point>>,
    output_grid_matches: Vec<RangeInclusive<Point>>,
    filtered_output_grid_matches: Option<Vec<RangeInclusive<Point>>>,
}

/// Returns the match that applies to the given point and advances the iterator if necessary.
///
/// NOTE: `matches_iter` must be sorted in descending order.
fn active_or_prev_row<I>(
    row_iter: &mut I,
    active_row: Option<usize>,
    current_row: &usize,
) -> Option<usize>
where
    I: Iterator<Item = usize>,
{
    let mut row = active_row.or_else(|| row_iter.next());
    while let Some(curr_row) = row {
        if curr_row <= *current_row {
            return row;
        } else {
            row = row_iter.next();
        }
    }

    None
}

impl BlockMatches {
    pub fn new(
        prompt_and_command_grid_matches: Vec<RangeInclusive<Point>>,
        command_grid_matches: Vec<RangeInclusive<Point>>,
        output_grid_matches: Vec<RangeInclusive<Point>>,
    ) -> Self {
        BlockMatches {
            prompt_and_command_grid_matches,
            command_grid_matches,
            output_grid_matches,
            filtered_output_grid_matches: None,
        }
    }

    pub fn calculate_filtered_output_grid_matches(
        &self,
        displayed_output_rows: impl DoubleEndedIterator<Item = usize>,
    ) -> Vec<RangeInclusive<Point>> {
        let output_grid_match_iter = self.output_grid_matches.iter();
        let mut displayed_output_rows_iter = displayed_output_rows.rev();
        let mut displayed_row = None;
        let mut filtered_output_grid_matches = Vec::new();

        // Add the output grid matches (in descending order) if they are contained in displayed rows.
        for output_grid_match in output_grid_match_iter {
            displayed_row = active_or_prev_row(
                &mut displayed_output_rows_iter,
                displayed_row,
                &output_grid_match.end().row,
            );
            let end_row_is_displayed = displayed_row
                .is_some_and(|displayed_row| output_grid_match.end().row == displayed_row);
            displayed_row = active_or_prev_row(
                &mut displayed_output_rows_iter,
                displayed_row,
                &output_grid_match.start().row,
            );
            let start_row_is_displayed = displayed_row
                .is_some_and(|displayed_row| output_grid_match.start().row == displayed_row);

            // We include a match if both the start row and end row of the match are displayed.
            // This is to be defensive against any bugs that might cause only part of a logical line to be displayed.
            // NOTE: When a match spans multiple rows, we include it when the start and end rows are displayed
            // but the middle rows are not.
            if start_row_is_displayed && end_row_is_displayed {
                filtered_output_grid_matches.push(output_grid_match.clone());
            }
        }

        filtered_output_grid_matches
    }

    /// Sets the filtered output grid matches
    pub fn set_filtered_output_grid_matches(
        &mut self,
        filtered_output_grid_matches: Vec<RangeInclusive<Point>>,
    ) {
        self.filtered_output_grid_matches = Some(filtered_output_grid_matches);
    }

    /// Resets any filter on the output grid matches.
    pub fn reset_output_grid_matches(&mut self) {
        self.filtered_output_grid_matches = None;
    }

    pub fn prompt_and_command_grid_matches(&self) -> &[RangeInclusive<Point>] {
        self.prompt_and_command_grid_matches.as_slice()
    }

    pub fn command_grid_matches(&self) -> &[RangeInclusive<Point>] {
        self.command_grid_matches.as_slice()
    }

    pub fn output_grid_matches(&self) -> &[RangeInclusive<Point>] {
        match &self.filtered_output_grid_matches {
            Some(filtered_output_grid_matches) => filtered_output_grid_matches.as_slice(),
            None => self.output_grid_matches.as_slice(),
        }
    }

    pub fn number_of_prompt_and_command_grid_matches(&self) -> usize {
        self.prompt_and_command_grid_matches.len()
    }

    pub fn number_of_command_grid_matches(&self) -> usize {
        self.command_grid_matches.len()
    }

    pub fn number_of_output_grid_matches(&self) -> usize {
        match &self.filtered_output_grid_matches {
            Some(filtered_output_grid_matches) => filtered_output_grid_matches.len(),
            None => self.output_grid_matches.len(),
        }
    }

    pub fn num_matches(&self) -> usize {
        self.number_of_prompt_and_command_grid_matches() + self.number_of_output_grid_matches()
    }

    /// Returns the index of the bottommost match in the entire block grid.
    /// Matches are ordered from the bottom of the grid to the top.
    pub fn bottommost_match(&self) -> Option<GridIndex> {
        if self.num_matches() == 0 {
            return None;
        }

        if self.output_grid_matches().is_empty() {
            return Some(GridIndex::PromptAndCommand(0));
        }
        Some(GridIndex::Output(0))
    }

    /// Returns the index of the topmost match in the entire block grid
    pub fn topmost_match(&self) -> Option<GridIndex> {
        if self.num_matches() == 0 {
            return None;
        }

        if self.prompt_and_command_grid_matches.is_empty() {
            return Some(GridIndex::Output(self.number_of_output_grid_matches() - 1));
        }
        Some(GridIndex::PromptAndCommand(
            self.number_of_prompt_and_command_grid_matches() - 1,
        ))
    }
}

/// Calculates the optimal number of rows to use for content truncation based on terminal width.
/// If terminal width >= 150 columns, use the default row counts.
/// If terminal width < 150 columns, calculate based on a target total cells.
fn calculate_optimal_row_counts(
    terminal_width: usize,
    default_top_lines: usize,
    default_bottom_lines: usize,
) -> (usize, usize) {
    if terminal_width >= MIN_TERMINAL_WIDTH_FOR_TRUNCATION_CALCULATIONS {
        return (default_top_lines, default_bottom_lines);
    }
    if terminal_width >= MIN_TERMINAL_WIDTH_FOR_TRUNCATION_CALCULATIONS {
        return (default_top_lines, default_bottom_lines);
    }

    // We calculate the # of rows proportional to the desired top/bottom lines.
    let top_target_cells = default_top_lines * MIN_TERMINAL_WIDTH_FOR_TRUNCATION_CALCULATIONS;
    let bottom_target_cells = default_bottom_lines * MIN_TERMINAL_WIDTH_FOR_TRUNCATION_CALCULATIONS;

    let top_rows = top_target_cells / terminal_width.max(1);
    let bottom_rows = bottom_target_cells / terminal_width.max(1);

    (top_rows, bottom_rows)
}

impl Block {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: BlockId,
        sizes: BlockSize,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        bootstrap_stage: BootstrapStage,
        show_warp_bootstrap_input: bool,
        show_in_band_command_blocks: bool,
        show_memory_stats: bool,
        block_index: BlockIndex,
        honor_ps1: bool,
        should_scan_for_secrets: ObfuscateSecrets,
        is_ai_ugc_telemetry_enabled: bool,
        conversation_id: Option<AIConversationId>,
    ) -> Self {
        let perform_reset_grid_checks = if cfg!(windows) && bootstrap_stage.is_done() {
            PerformResetGridChecks::Yes
        } else {
            PerformResetGridChecks::No
        };
        let header_grid = HeaderGrid::new(
            sizes.clone(),
            event_proxy.clone(),
            should_scan_for_secrets,
            honor_ps1,
            perform_reset_grid_checks,
        );
        let rprompt_grid = BlockGrid::new(
            sizes.size,
            // Even though prompt is most likely only 1-2 lines, we allow for the bigger
            // max_scroll_limit, to account for resizing the window/pane.
            sizes.max_block_scroll_limit,
            event_proxy.clone(),
            should_scan_for_secrets,
            PerformResetGridChecks::No,
        );
        let output_grid = BlockGrid::new(
            sizes.size,
            sizes.max_block_scroll_limit,
            event_proxy.clone(),
            should_scan_for_secrets,
            perform_reset_grid_checks,
        );

        Block {
            id,
            size: sizes.size,
            header_grid,
            rprompt_grid,
            output_grid,
            padding: sizes.block_padding,
            render_delay_complete: Arc::new(AtomicBool::new(false)),
            was_long_running: AtomicBool::new(false),
            state: BlockState::BeforeExecution,
            precmd_state: PrecmdState::BeforePrecmd,
            exit_code: ExitCode::from(0),
            session_id: None,
            pwd: None,
            git_branch: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            node_version: None,
            rprompt: None,
            background_executor,
            event_proxy,
            bootstrap_stage,
            show_bootstrap_block: show_warp_bootstrap_input,
            show_in_band_command_blocks,
            show_memory_stats,
            creation_ts: Local::now(),
            start_ts: None,
            completed_ts: None,
            block_index,
            shell_host: None,
            is_for_in_band_command: false,
            env_var_metadata: None,
            interaction_mode: InteractionMode::default(),
            block_banner: None,
            ignore_next_rprompt: false,
            prompt_snapshot: None,
            home_dir: None,
            filter_query: None,
            cloud_workflow_id: None,
            cloud_env_var_collection_id: None,
            last_painted_at: None.into(),
            has_received_user_input: false,
            hidden: false,
            should_hide_output_grid: false,
            should_hide_command_grid: false,
            leading_linefeeds_ignored: 0,
            is_ai_ugc_telemetry_enabled,
            restored_block_was_local: None,
            agent_view_visibility: match conversation_id {
                Some(id) => AgentViewVisibility::new_from_conversation(id),
                None => AgentViewVisibility::new_from_terminal(),
            },
            nld_overridden: false,
            is_oz_environment_startup_command: false,
        }
    }

    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn size(&self) -> SizeInfo {
        self.size
    }

    pub fn interaction_mode(&self) -> &InteractionMode {
        &self.interaction_mode
    }

    /// Replaces this block's visibility to be associated with the given conversation.
    /// Use this when a block is being created/assigned to a conversation (e.g., entering agent view).
    pub fn set_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.agent_view_visibility = AgentViewVisibility::new_from_conversation(conversation_id);
    }

    /// Resets this block's visibility to terminal mode.
    /// Use this when a block is being returned to terminal context (e.g., exiting agent view).
    pub fn clear_conversation_id(&mut self) {
        self.agent_view_visibility = AgentViewVisibility::new_from_terminal();
    }

    /// Sets this block's agent view visibility state directly.
    /// Use this when restoring a block from serialization.
    pub fn set_agent_view_visibility(&mut self, visibility: AgentViewVisibility) {
        self.agent_view_visibility = visibility;
    }

    /// Adds a conversation ID to the set of conversations where this block is attached as context.
    pub(super) fn add_attached_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.agent_view_visibility
            .add_attached_conversation_id(conversation_id);
    }

    /// Adds a conversation ID to the set of conversations where this block is pending context.
    /// It maybe removed if the user removes the block attachment before sending the request, else if it is attached it will be 'promoted'.
    pub(super) fn add_pending_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.agent_view_visibility
            .add_pending_conversation_id(conversation_id);
    }

    /// Removes a conversation ID from the set of conversations where this block should be visible.
    /// Returns true if the conversation ID was present and removed, false if it wasn't present.
    pub(super) fn remove_pending_conversation_id(
        &mut self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.agent_view_visibility
            .remove_pending_conversation_id(conversation_id)
    }

    /// Moves the block from pending context to attached context for the given conversation ID.
    pub(super) fn promote_pending_to_attached(
        &mut self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.agent_view_visibility
            .promote_pending_to_attached(conversation_id)
    }

    pub fn agent_view_visibility(&self) -> &AgentViewVisibility {
        &self.agent_view_visibility
    }

    /// Returns whether NLD was overridden (input type was manually locked) when this block's
    /// command was submitted.
    ///
    /// This is used for debugging UI shown in the block header on dogfood builds.
    pub fn nld_overridden(&self) -> bool {
        self.nld_overridden
    }

    /// Sets whether NLD was overridden at command submission time.
    pub fn set_nld_overridden(&mut self, nld_overridden: bool) {
        self.nld_overridden = nld_overridden;
    }

    pub fn set_trim_trailing_blank_rows(&mut self, trim: bool) {
        self.output_grid.set_trim_trailing_blank_rows(trim);
    }

    pub(in crate::terminal) fn enable_full_grid_clear_behavior(&mut self) {
        self.output_grid.enable_full_grid_clear_behavior();
    }

    pub fn set_restored_block_was_local(&mut self, was_local: bool) {
        debug_assert!(
            self.bootstrap_stage == BootstrapStage::RestoreBlocks,
            "set_restored_block_was_local should only be called for restored blocks"
        );
        self.restored_block_was_local = Some(was_local);
    }

    pub fn restored_block_was_local(&self) -> Option<bool> {
        self.restored_block_was_local
    }

    pub(super) fn receiving_chars_for_prompt(&self) -> Option<ansi::PromptKind> {
        self.header_grid.receiving_chars_for_prompt
    }

    /// Replaces the block's lprompt and command combined grid with the given one.
    #[cfg(test)]
    pub fn set_prompt_and_command_grid(&mut self, prompt_and_command_grid: BlockGrid) {
        self.header_grid
            .set_prompt_and_command_grid(prompt_and_command_grid);
    }

    /// Manually update the prompt end point, for test purposes.
    #[cfg(test)]
    pub(super) fn set_raw_prompt_end_point(&mut self, point: Option<PromptEndPoint>) {
        self.header_grid.set_raw_prompt_end_point(point);
    }

    /// Replaces the block's prompt grid with the given one.
    #[cfg(test)]
    pub(super) fn set_prompt_grid(&mut self, prompt_grid: BlockGrid) {
        self.header_grid.set_prompt_grid(prompt_grid);
    }

    /// Replaces the block's rprompt grid with the given one.
    #[cfg(test)]
    pub(super) fn set_rprompt_grid(&mut self, rprompt_grid: BlockGrid) {
        self.rprompt_grid = rprompt_grid;
    }

    /// Replaces the block's output grid with the given one.
    /// Useful for test functions.
    #[cfg(test)]
    pub fn set_output_grid(&mut self, output_grid: BlockGrid) {
        self.output_grid = output_grid;
    }

    #[cfg(not(feature = "integration_tests"))]
    pub(in crate::terminal) fn block_banner(&self) -> Option<&WithinBlockBanner> {
        self.block_banner.as_ref()
    }

    #[cfg(feature = "integration_tests")]
    pub fn block_banner(&self) -> Option<&WithinBlockBanner> {
        self.block_banner.as_ref()
    }

    /// Prefer using the `reset_block_index` fn on the BlockList instead.
    /// Resets the index of the block to `index`. This is useful in the case where the block list
    /// may have had a block deleted, in which case some blocks may need to have their index reset.
    pub(super) fn reset_index(&mut self, index: BlockIndex) {
        self.block_index = index
    }

    pub fn honor_ps1(&self) -> bool {
        self.header_grid.honor_ps1()
    }

    pub fn set_honor_ps1(&mut self, honor_ps1: bool) {
        self.header_grid.set_honor_ps1(honor_ps1);
    }

    pub fn start(&mut self) {
        if self.start_ts.is_none() {
            self.start_ts = Some(Local::now());
        }

        // If we are in script execution stage and the shell starts a new block,
        // this means we have a visible bootstrap block.
        if self.bootstrap_stage() == BootstrapStage::ScriptExecution {
            self.event_proxy
                .send_terminal_event(Event::VisibleBootstrapBlock);
        }

        self.header_grid.start_command_grid();
        self.wakeup_after_delay();
    }

    /// Returns the `env_var_metadata` associated with this block, if any.
    pub fn env_var_metadata(&self) -> Option<&BlocklistEnvVarMetadata> {
        self.env_var_metadata.as_ref()
    }

    pub fn set_env_var_metadata(&mut self, env_var_metadata: BlocklistEnvVarMetadata) {
        self.env_var_metadata = Some(env_var_metadata);
    }

    pub fn all_bytes_scanned_for_secrets(&self) -> bool {
        all::<GridType>()
            .filter_map(|grid_type| self.grid_of_type(grid_type))
            .fold(false, |prev, block_grid| {
                prev && block_grid.all_bytes_scanned_for_secrets()
            })
    }

    pub(super) fn set_obfuscate_secrets(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        self.for_each_block_grid(|block_grid| {
            if obfuscate_secrets.should_redact_secret() {
                block_grid.maybe_enable_secret_obfuscation(obfuscate_secrets);
            } else {
                block_grid.disable_secret_obfuscation();
            }
        });
    }

    /// Scans the entire block (not just the dirty bytes) for secrets.
    pub fn scan_full_block_for_secrets(&mut self) {
        self.for_each_block_grid(|block_grid| block_grid.scan_full_grid_for_secrets())
    }

    /// Starts this block as a background output block, with no command.
    /// Background blocks never receive precmd metadata, so where possible they
    /// inherit the last command block's session ID.
    pub fn start_background(&mut self, session_id: Option<SessionId>) {
        if self.start_ts.is_none() {
            self.start_ts = Some(Local::now());
        }
        self.session_id = session_id;
        self.header_grid.start_command_grid();
        self.header_grid.finish_command_grid();

        // TODO(CORE-2826): We disable reset grid checks for background blocks.
        self.disable_reset_grid_checks();
        self.output_grid.start();
        self.state = BlockState::Background;
        self.wakeup_after_delay();
    }

    pub(super) fn disable_reset_grid_checks(&mut self) {
        self.header_grid.disable_reset_grid_checks();
        self.output_grid.disable_reset_grid_checks();
    }

    /// Method used in tests to finish the BlockGrid containing the command WITHOUT a linefeed in the empty command
    /// case.
    #[cfg(test)]
    pub fn finish_command_grid(&mut self) {
        self.header_grid.finish_command_grid();
    }

    /// Starts this block as an "in-band" command block, which are hidden from the user unless
    /// they've explicitly chosen to show them via the 'Blocks > Show in-band command blocks'
    /// menu item.
    pub fn start_for_in_band_command(&mut self) {
        self.is_for_in_band_command = true;
        self.start();
    }

    /// This block's minimal metadata. Until the block has received its metadata (either from a
    /// precmd hook or static construction), this metadata is incomplete.
    pub fn metadata(&self) -> BlockMetadata {
        BlockMetadata::new(self.session_id, self.pwd.clone())
    }

    pub fn has_received_user_input(&self) -> bool {
        self.has_received_user_input
    }

    /// Marks `has_received_user_input` as `true`.
    ///
    /// To be called upon writing user bytes to the pty for the first time during this block's
    /// command execution.
    pub fn mark_received_user_input(&mut self) {
        self.has_received_user_input = true;
    }

    fn init_rprompt_grid(&mut self, rprompt: &str) {
        if rprompt.is_empty() {
            return;
        }
        if self.ignore_next_rprompt {
            self.ignore_next_rprompt = false;
            return;
        }

        // Decoding the bytes passed from the shell
        if let Ok(unescaped_rprompt) = hex::decode(rprompt) {
            let mut processor = Processor::new();
            self.rprompt_grid.start();
            processor.parse_bytes(&mut self.rprompt_grid, &unescaped_rprompt, &mut io::sink());
            self.rprompt_grid.finish();
        }
    }

    pub fn set_prompt_snapshot(&mut self, prompt_snapshot: PromptSnapshot) {
        self.prompt_snapshot = Some(prompt_snapshot);
    }

    pub fn prompt_snapshot(&self) -> Option<&PromptSnapshot> {
        self.prompt_snapshot.as_ref()
    }

    /// Sets the prompt and right prompt grids in this block from grids that
    /// we cached when the last user command was submitted.
    ///
    /// We additionally set some state to ignore the next prompt and right
    /// prompt we attempt to set in this block, to ensure that the initial
    /// prompt update that normally occurs doesn't overwrite these copied
    /// grids.  We only want to skip the initial prompt update to not interfere
    /// with transient prompt features (modifications to the prompt in preexec).
    pub(super) fn set_prompt_grids_from_cached_data(
        &mut self,
        prompt_grid: BlockGrid,
        rprompt_grid: BlockGrid,
    ) {
        self.header_grid.set_prompt_from_cached_data(prompt_grid);
        self.rprompt_grid = rprompt_grid;
        self.ignore_next_rprompt = true;
    }

    /// Returns whether or not we should display an rprompt to the user.
    pub fn should_display_rprompt(&self, size: &SizeInfo) -> bool {
        self.rprompt_grid.finished()
            && self.rprompt_grid.has_received_content()
            && self.is_enough_space_for_rprompt(size)
    }

    /// Returns whether or not there is enough space to render the rprompt
    /// in addition to the prompt with the given sizing info.
    fn is_enough_space_for_rprompt(&self, size: &SizeInfo) -> bool {
        // The rprompt must start AFTER the end of the last line of the lprompt (which can include
        // the first line of the command, in the combined grid case).
        let lprompt_last_row_width =
            self.header_grid.lprompt_last_line_width_cols() as f32 * size.cell_width_px.as_f32();
        let rprompt_render_offset = self.rprompt_render_offset(size);

        rprompt_render_offset.x() > lprompt_last_row_width
    }

    /// Write a new command into this block's command grid. We use this to
    /// fill out the command grid when the actual command, as echoed by the shell,
    /// was instead classified as background output.
    pub fn init_command(&mut self, command: impl AsRef<[u8]>) {
        self.header_grid.init_command(command);
    }

    /// Copy an existing grid into this block's command grid. We use this to
    /// fill out the command grid when the actual command, as echoed by the shell,
    /// was instead classified as background output.
    pub fn copy_command_grid(&mut self, command: &BlockGrid) {
        // Cloning the background command grid as-is, instead of roundtripping
        // through a string, is more efficient and preserves shell formatting.
        self.header_grid.clone_command_from_blockgrid(command);
    }

    pub fn is_empty(&self, agent_view_state: &AgentViewState) -> bool {
        // TODO(vorporeal): this should use a larger epsilon
        self.height(agent_view_state).as_f64() < f64::EPSILON
    }

    pub fn is_restored(&self) -> bool {
        matches!(self.bootstrap_stage, BootstrapStage::RestoreBlocks)
    }

    /// Whether this is a background output block. Background output blocks are
    /// created to hold output from jobs running in the background, and have no
    /// associated command.
    pub fn is_background(&self) -> bool {
        self.state == BlockState::Background
    }

    /// Returns `true` if this is a static block. See docs on [`BlockState::Static`] for more
    /// context on static blocks.
    pub fn is_static(&self) -> bool {
        self.state == BlockState::Static
    }

    /// If true, this block is hidden and has a height of 0.
    pub fn should_hide_block(&self, agent_view_state: &AgentViewState) -> bool {
        if self.hidden {
            return true;
        }
        if FeatureFlag::AgentView.is_enabled() {
            match agent_view_state {
                AgentViewState::Active {
                    display_mode: AgentViewDisplayMode::FullScreen,
                    conversation_id: active_id,
                    ..
                } => {
                    // Agent view is active - show only blocks that belong to this conversation
                    let visible_in_conversation = match &self.agent_view_visibility {
                        AgentViewVisibility::Terminal {
                            pending_conversation_ids,
                            conversation_ids,
                        } => {
                            pending_conversation_ids.contains(active_id)
                                || conversation_ids.contains(active_id)
                        }
                        AgentViewVisibility::Agent {
                            origin_conversation_id,
                            pending_other_conversation_ids,
                            other_conversation_ids,
                        } => {
                            active_id == origin_conversation_id
                                || pending_other_conversation_ids.contains(active_id)
                                || other_conversation_ids.contains(active_id)
                        }
                    };
                    if !visible_in_conversation {
                        return true;
                    }
                }
                AgentViewState::Active {
                    display_mode: AgentViewDisplayMode::Inline,
                    ..
                }
                | AgentViewState::Inactive => {
                    // Terminal view - hide blocks that were created in agent mode
                    if matches!(
                        self.agent_view_visibility,
                        AgentViewVisibility::Agent { .. }
                    ) {
                        return true;
                    }
                }
            }
        }

        let is_bootstrap_block = self.bootstrap_stage == BootstrapStage::WarpInput;
        let is_empty_bootstrap_script_execution_block = self.bootstrap_stage
            == BootstrapStage::ScriptExecution
            && self.command_should_show_as_empty_when_finished()
            && self.output_grid().should_show_as_empty_when_finished();
        let is_empty_background_block = self.state == BlockState::Background
            && self.output_grid.should_show_as_empty_when_finished();

        (is_bootstrap_block && !self.show_bootstrap_block)
            || is_empty_bootstrap_script_execution_block
            || is_empty_background_block
            || self
                .env_var_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.should_hide_block)
            || (self.is_for_in_band_command && !self.show_in_band_command_blocks)
            || self.interaction_mode.should_hide_block()
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Prevent the block from showing in the blocklist.
    pub fn hide(&mut self) {
        self.hidden = true;
    }

    pub fn is_oz_environment_startup_command(&self) -> bool {
        self.is_oz_environment_startup_command
    }

    pub(super) fn set_is_oz_environment_startup_command(&mut self, is_startup_command: bool) {
        self.is_oz_environment_startup_command = is_startup_command;
    }

    /// Reset the block so it's no longer hidden. Undoes the effects of Self::hide().
    pub fn unhide(&mut self) {
        self.hidden = false;
    }

    pub fn toggle_hidden(&mut self) -> bool {
        self.hidden = !self.hidden;
        self.hidden
    }

    pub fn should_hide_output_grid(&self) -> bool {
        self.should_hide_output_grid
    }

    pub fn set_should_hide_output_grid(&mut self, should_hide: bool) {
        self.should_hide_output_grid = should_hide;
    }

    pub fn should_hide_command_grid(&self) -> bool {
        self.should_hide_command_grid
    }

    pub fn set_should_hide_command_grid(&mut self, should_hide: bool) {
        self.should_hide_command_grid = should_hide;
    }

    /// Returns true iff this block should be used as a scrollback block in a shared session context.
    /// The active block is included when it is eligible so viewers can restore the active prompt.
    pub fn is_scrollback_block_for_shared_session(
        &self,
        agent_view_state: &AgentViewState,
    ) -> bool {
        !self.should_hide_block(agent_view_state) && !self.is_restored()
    }

    pub fn index(&self) -> BlockIndex {
        self.block_index
    }

    /// `true` if the block is rendered in the blocklist.
    pub fn is_visible(&self, agent_view_state: &AgentViewState) -> bool {
        self.height(agent_view_state) > Lines::zero()
    }

    /// Height is the source-of-truth determinant for whether or not a block is hidden (i.e. if it
    /// has a height of 0). Thus it depends on agent_view_state, which affects whether or not a
    /// given block should be hidden.
    pub fn height(&self, agent_view_state: &AgentViewState) -> Lines {
        if self.should_hide_block(agent_view_state) {
            Lines::zero()
        } else {
            self.block_banner_height()
                + if self.should_hide_command_grid {
                    Lines::zero()
                } else {
                    self.padding_top() + self.prompt_and_command_height() + self.padding_middle()
                }
                + if self.should_hide_output_grid {
                    Lines::zero()
                } else {
                    self.output_grid_displayed_height()
                        + self.footer_top_padding()
                        + self.footer_height()
                        + self.padding_bottom()
                }
        }
    }

    /// Whether we render the prompt on the same line, in the context of a finished block. Post-same
    /// line prompt, we render on the same line for PS1, but not for Warp prompt!
    pub fn render_prompt_on_same_line(&self) -> bool {
        self.honor_ps1()
    }

    /// Used for determining the height of the block with `DisplaySettings` used when sharing a block.
    pub fn full_content_height_with_display_options(
        &self,
        display_setting: &DisplaySetting,
        show_prompt: bool,
    ) -> Lines {
        let mut height = self.padding_top();
        if show_prompt && !self.render_prompt_on_same_line() {
            height += self.prompt_height() + self.command_padding_top();
        }

        let command_height = self.prompt_and_command_height();

        height += match display_setting {
            DisplaySetting::Command => command_height,
            DisplaySetting::Output => self.output_grid_full_content_height(),
            _ => command_height + self.padding_middle() + self.output_grid_full_content_height(),
        };
        height += self.padding_bottom();
        height
    }

    /// The last part of the lifecycle for the block. After this, its contents
    /// are immutable.
    pub fn finish(&mut self, exit_code: impl Into<ExitCode>) {
        // Make sure all grids are marked as finished.
        self.header_grid.finish_command(self.bootstrap_stage);
        self.rprompt_grid.finish();
        self.output_grid.finish();

        self.exit_code = exit_code.into();

        if self.completed_ts.is_none() {
            self.completed_ts = Some(Local::now());
        }

        self.state = match self.state {
            BlockState::Executing => BlockState::DoneWithExecution,
            BlockState::Background => BlockState::Background,
            BlockState::Static => BlockState::Static,
            _ => BlockState::DoneWithNoExecution,
        };
        log::info!("Block finished with new state {:?}", self.state);

        self.block_banner = None;

        let block_type: BlockType = self.into();
        self.event_proxy
            .send_terminal_event(Event::BlockCompleted(BlockCompletedEvent {
                block_type,
                block_latency_data: self.block_latency_data(),
                num_secrets_obfuscated: self.num_secrets_obfuscated(),
                block_index: self.block_index,
                block_id: self.id.clone(),
                session_id: self.session_id,
                restored_block_was_local: self.restored_block_was_local,
            }));
    }

    pub fn num_secrets_obfuscated(&self) -> usize {
        self.header_grid.num_secrets_obfuscated() + self.output_grid.num_secrets_obfuscated()
    }

    fn block_latency_data(&self) -> Option<BlockLatencyData> {
        // We only want to record block latency data for normal execution
        // outside of the bootstrap sequence.
        if self.bootstrap_stage.is_done() && !self.is_background() && !self.is_static() {
            let command = self.header_grid.command_to_string_with_max_rows(Some(1));
            BASELINE_COMMANDS.get(command.as_str()).and_then(|command| {
                self.header_grid
                    .command_start_time()
                    .map(|started_at| BlockLatencyData {
                        command,
                        started_at,
                    })
            })
        } else {
            None
        }
    }

    /// Gets optimized content summary for a single block using terminal-width-aware truncation,
    /// taking the # of desired logical top/bottom rows into account (note that this limit is applied
    /// on a per-grid basis).
    pub fn get_block_content_summary(
        &self,
        terminal_width: usize,
        number_of_top_lines_per_grid: usize,
        number_of_bottom_lines_per_grid: usize,
    ) -> (String, String) {
        let (optimized_top_lines, optimized_bottom_lines) = calculate_optimal_row_counts(
            terminal_width,
            number_of_top_lines_per_grid,
            number_of_bottom_lines_per_grid,
        );

        let mut processed_input = self.prompt_and_command_grid().content_summary(
            optimized_top_lines,
            optimized_bottom_lines,
            true,
        );

        let mut processed_output =
            self.output_grid()
                .content_summary(optimized_top_lines, optimized_bottom_lines, true);

        // If secret redaction is disabled, we manually scan for secrets and redact them.
        if matches!(
            self.prompt_and_command_grid().should_scan_for_secrets(),
            ObfuscateSecrets::No
        ) {
            redact_secrets(&mut processed_input);
        }
        if matches!(
            self.output_grid().should_scan_for_secrets(),
            ObfuscateSecrets::No
        ) {
            redact_secrets(&mut processed_output);
        }

        (processed_input, processed_output)
    }

    // Necessary for restored blocks so we use the actual timestamp when it completed
    // rather than the timestamp when the block was restored.
    pub fn override_completed_ts(&mut self, ts: DateTime<Local>) {
        self.completed_ts = Some(ts);
    }

    // Necessary for restored blocks so we use the actual timestamp when it started
    // rather than the timestamp when the block was restored.
    pub fn override_start_ts(&mut self, ts: DateTime<Local>) {
        self.start_ts = Some(ts);
    }

    pub fn ready_to_render(&self) -> bool {
        self.finished()
            || self.bootstrap_stage == BootstrapStage::ScriptExecution
            || self.render_delay_complete.load(Ordering::Relaxed)
    }

    pub fn started(&self) -> bool {
        self.header_grid.command_started()
    }

    pub fn finished(&self) -> bool {
        self.output_grid.finished()
    }

    pub fn is_receiving_prompt(&self) -> bool {
        self.header_grid.receiving_chars_for_prompt.is_some()
    }

    /// A command-grid is active in the period after we have received the precmd
    /// hook but before the command has started executing. This includes the time
    /// when the shell echoes the command bytes that Warp wrote to the PTY.
    pub fn is_command_grid_active(&self) -> bool {
        self.state == BlockState::BeforeExecution
    }

    pub fn active_grid_type(&self) -> GridType {
        match self.state {
            BlockState::BeforeExecution => GridType::PromptAndCommand,
            BlockState::Executing
            | BlockState::Background
            | BlockState::DoneWithExecution
            | BlockState::DoneWithNoExecution
            | BlockState::Static => GridType::Output,
        }
    }

    pub fn is_executing(&self) -> bool {
        self.state == BlockState::Executing
    }

    /// Whether a command is long running.
    /// We use this to determine whether to hide the input box.
    pub fn is_active_and_long_running(&self) -> bool {
        // Use the command grid start time by default (which should be earlier)
        // than the output grid start time.  If for some reason there isn't a
        // command grid start time, then fall back to the start time of the output
        // grid.
        let start_time = match self.state {
            BlockState::Background => return !self.finished(),
            BlockState::BeforeExecution => self.header_grid.command_start_time(),
            BlockState::Executing => self
                .header_grid
                .command_start_time()
                .or_else(|| self.output_grid.start_time()),
            BlockState::DoneWithNoExecution
            | BlockState::DoneWithExecution
            | BlockState::Static => return false,
        };

        if self.was_long_running.load(Ordering::Relaxed) {
            true
        } else {
            start_time.is_some_and(|start_time| {
                let was_long_running =
                    start_time.elapsed().as_millis() >= (LONG_RUNNING_COMMAND_DURATION_MS as u128);
                // If we assessed once that a command was long running, it won't become short
                // running ever again, so we can cache and don't do this computation again
                if was_long_running {
                    self.was_long_running.store(true, Ordering::Relaxed);
                }
                was_long_running
            })
        }
    }

    #[cfg(test)]
    pub fn set_was_long_running(&mut self, was_long_running: AtomicBool) {
        self.was_long_running = was_long_running;
    }

    pub fn command_with_secrets_obfuscated(&self, include_escape_sequences: bool) -> String {
        self.header_grid
            .command_with_secrets_obfuscated(include_escape_sequences)
    }

    pub fn command_with_secrets_unobfuscated(&self, include_escape_sequences: bool) -> String {
        self.header_grid
            .command_with_secrets_unobfuscated(include_escape_sequences)
    }

    pub fn command_should_show_as_empty_when_finished(&self) -> bool {
        self.header_grid
            .command_should_show_as_empty_when_finished()
    }

    pub fn command_start_time(&self) -> Option<Instant> {
        self.header_grid.command_start_time()
    }

    pub fn prompt_and_command_number_of_rows(&self) -> usize {
        self.header_grid.prompt_and_command_number_of_rows()
    }

    pub fn is_command_empty(&self) -> bool {
        self.header_grid.is_command_empty()
    }

    pub fn is_command_finished(&self) -> bool {
        self.header_grid.is_command_finished()
    }

    pub fn command_and_output_with_secret_obfuscated(
        &self,
        include_escape_sequences: bool,
    ) -> (String, String) {
        let mut command = self.command_with_secrets_obfuscated(include_escape_sequences);
        let mut output = self
            .output_grid()
            .contents_to_string_force_secrets_obfuscated(
                include_escape_sequences,
                Some(MAX_SERIALIZED_STYLIZED_OUTPUT_LINES),
            );

        // If secret redaction is disabled, we manually scan for secrets and redact them.
        if matches!(
            self.prompt_and_command_grid().should_scan_for_secrets,
            ObfuscateSecrets::No
        ) {
            redact_secrets(&mut command);
        }
        if matches!(
            self.output_grid().should_scan_for_secrets,
            ObfuscateSecrets::No
        ) {
            redact_secrets(&mut output);
        }

        (command, output)
    }

    pub fn prompt_grid(&self) -> &BlockGrid {
        self.header_grid.prompt_grid()
    }

    pub fn prompt_contents_to_string(&self, include_escape_sequences: bool) -> String {
        self.header_grid
            .prompt_contents_to_string(include_escape_sequences)
    }

    pub fn prompt_with_secrets_obfuscated(&self, include_escape_sequences: bool) -> String {
        self.header_grid
            .prompt_with_secrets_obfuscated(include_escape_sequences)
    }

    pub fn prompt_with_secrets_unobfuscated(&self, include_escape_sequences: bool) -> String {
        self.header_grid
            .prompt_with_secrets_unobfuscated(include_escape_sequences)
    }

    pub fn prompt_and_command_with_secrets_obfuscated(
        &self,
        include_escape_sequences: bool,
    ) -> String {
        self.header_grid
            .prompt_and_command_with_secrets_obfuscated(include_escape_sequences)
    }

    pub fn prompt_and_command_with_secrets_unobfuscated(
        &self,
        include_escape_sequences: bool,
    ) -> String {
        self.header_grid
            .prompt_and_command_with_secrets_unobfuscated(include_escape_sequences)
    }

    pub fn prompt_number_of_rows(&self) -> usize {
        self.header_grid.prompt_number_of_rows()
    }

    pub fn is_prompt_empty(&self) -> bool {
        self.header_grid.is_prompt_empty()
    }

    pub fn prompt_rightmost_visible_nonempty_cell(&self) -> Option<usize> {
        self.header_grid.prompt_rightmost_visible_nonempty_cell()
    }

    pub fn prompt_grid_columns(&self) -> usize {
        self.header_grid.prompt_grid_columns()
    }

    pub fn prompt_grid_cell_height(&self) -> usize {
        self.header_grid.prompt_grid_cell_height()
    }

    pub fn prompt_and_command_grid(&self) -> &BlockGrid {
        self.header_grid.prompt_and_command_grid()
    }

    pub fn rprompt_grid(&self) -> &BlockGrid {
        &self.rprompt_grid
    }

    pub fn output_grid(&self) -> &BlockGrid {
        &self.output_grid
    }

    pub fn find_prompt_and_command_grid_matches<'a>(
        &'a self,
        dfas: &'a RegexDFAs,
    ) -> Vec<RangeInclusive<Point>> {
        self.header_grid.prompt_and_command_find(dfas).collect()
    }

    pub fn find_output_grid_matches<'a>(
        &'a self,
        dfas: &'a RegexDFAs,
    ) -> Vec<RangeInclusive<Point>> {
        self.output_grid.find(dfas).collect()
    }

    pub fn prompt_grid_offset(&self) -> Lines {
        self.block_banner_height() + self.padding_top()
    }

    /// The number of lines the command grid starts from the top of the block.
    pub fn command_grid_offset(&self) -> Lines {
        self.block_banner_height()
            + self.padding_top()
            + self.prompt_height()
            + self.command_padding_top()
    }

    /// The number of lines the combined prompt/command grid starts from the top of the block.
    pub fn prompt_and_command_grid_offset(&self) -> Lines {
        if self.header_grid.honor_ps1() {
            self.block_banner_height() + self.padding_top()
        } else {
            // Grid is drawn below custom Warp prompt in finished blocks.
            self.block_banner_height()
                + self.padding_top()
                + self.prompt_height()
                + self.command_padding_top()
        }
    }

    /// The number of lines the output grid starts from the top of the block.
    pub fn output_grid_offset(&self) -> Lines {
        self.block_banner_height()
            + self.padding_top()
            + self.prompt_and_command_height()
            + self.padding_middle()
    }

    pub fn state(&self) -> BlockState {
        self.state
    }

    pub fn is_bootstrapped(&self) -> bool {
        self.bootstrap_stage.is_bootstrapped()
    }

    pub fn bootstrap_stage(&self) -> BootstrapStage {
        self.bootstrap_stage
    }

    /// Returns the ENTIRE HEIGHT of the prompt and command (no padding top or middle included).
    /// In the case of combined grid: for Warp prompt, this includes the height of both the Warp prompt
    /// AND combined grid; for PS1, this is just the combined grid (PS1 is included there).
    pub fn prompt_and_command_height(&self) -> Lines {
        if !self.ready_to_render() || self.should_hide_command_grid {
            Lines::zero()
        } else if self.header_grid.honor_ps1 {
            // No padding between prompt and command in the case of PS1 (combined grid).
            self.header_grid.prompt_and_command_height()
        } else {
            // Handle the case of Warp built-in prompt with combined grid.
            // Note that we have non-zero `command_padding_top` in this case, unlike above!
            if self.header_grid.is_command_empty() {
                Lines::zero()
            } else {
                self.prompt_height()
                    + self.command_padding_top()
                    + self.header_grid.prompt_and_command_height()
            }
        }
    }

    /// The height of the output grid as it is displayed in the block list.
    pub fn output_grid_displayed_height(&self) -> Lines {
        if self.output_grid.is_empty() || !self.ready_to_render() {
            Lines::zero()
        } else {
            self.output_grid.len_displayed().into_lines()
        }
    }

    /// The height of the output grid's full contents, irrespective of what is
    /// actually being displayed in the block list.
    pub fn output_grid_full_content_height(&self) -> Lines {
        if self.output_grid.is_empty() || !self.ready_to_render() {
            Lines::zero()
        } else {
            self.output_grid.len().into_lines()
        }
    }

    /// Returns prompt height in lines.
    pub fn prompt_height(&self) -> Lines {
        if !self.ready_to_render() {
            Lines::zero()
        } else {
            self.header_grid.prompt_height()
        }
    }

    /// Returns the offset, in pixels, at which the rprompt should be rendered
    /// relative to the prompt.
    pub fn rprompt_render_offset(&self, size: &SizeInfo) -> Vector2F {
        let rprompt_width_cells = self.rprompt_grid.grid_storage().max_cursor_point.col;
        let rprompt_width_px = rprompt_width_cells as f32 * size.cell_width_px.as_f32();
        Vector2F::new(
            (self.prompt_grid_columns().saturating_sub(1) as f32 * size.cell_width_px().as_f32())
                - rprompt_width_px,
            self.prompt_number_of_rows().saturating_sub(1) as f32 * size.cell_height_px().as_f32(),
        )
    }

    pub fn update_padding(&mut self, padding: BlockPadding) {
        self.padding = padding;
    }

    /// Whether the command grid is missing/empty, in which case we omit padding
    /// around it. Unlike `self.header_grid.command_grid_is_empty()`, this takes into account
    /// the fact that background output blocks are expected to have an empty
    /// command grid.
    fn missing_command(&self) -> bool {
        !self.is_background() && self.header_grid.is_command_empty()
    }

    pub(in crate::terminal) fn block_banner_height(&self) -> Lines {
        if !self.ready_to_render() {
            Lines::zero()
        } else {
            match &self.block_banner {
                Some(banner) => {
                    (banner.banner_height() / self.prompt_grid_cell_height() as f32).into_lines()
                }
                None => Lines::zero(),
            }
        }
    }

    pub fn padding_top(&self) -> Lines {
        if self.missing_command() || !self.ready_to_render() {
            Lines::zero()
        } else {
            match self.block_banner {
                // Truncate the padding if there is a banner, so not break the visual relationship
                // between the block and banner, but still allow it to be smaller in compact mode.
                Some(_) => self.padding.padding_top.min(0.6).into_lines(),
                None => self.padding.padding_top.into_lines(),
            }
        }
    }

    pub fn command_padding_top(&self) -> Lines {
        if self.header_grid.is_command_empty() || !self.ready_to_render() {
            Lines::zero()
        } else {
            self.padding.command_padding_top.into_lines()
        }
    }

    pub fn padding_bottom(&self) -> Lines {
        if self.missing_command() || !self.ready_to_render() {
            Lines::zero()
        } else if self.is_active_and_long_running() {
            // The terminal size (sent to PTY) is calculated based on the window size, so we need
            // to have consistency between long-running programs and the alt screen. The larger
            // padding used on blocks looks bad on the alt screen, so we use a smaller amount here.
            // This will allow full-screen programs (e.g. `git log`) to fill the window properly
            LONG_RUNNING_BOTTOM_PADDING_LINES.into_lines()
        } else {
            self.padding.bottom.into_lines()
        }
    }

    pub fn padding_middle(&self) -> Lines {
        if self.is_background() || self.output_grid.is_empty() || !self.ready_to_render() {
            Lines::zero()
        } else {
            self.padding.middle.into_lines()
        }
    }

    pub fn has_footer(&self) -> bool {
        self.show_memory_stats
    }

    pub fn footer_top_padding(&self) -> Lines {
        if self.footer_height() == Lines::zero() {
            Lines::zero()
        } else {
            self.padding_middle()
        }
    }

    pub fn footer_height(&self) -> Lines {
        if !self.has_footer() || !self.ready_to_render() {
            Lines::zero()
        } else {
            // The footer is one line of text.
            1.0.into_lines()
        }
    }

    /// Resize terminal to new dimensions.
    pub fn resize(&mut self, size: SizeInfo) {
        self.size = size;
        self.header_grid.resize(size);
        self.output_grid.resize(size);
        self.rprompt_grid.resize(size);
    }

    /// Returns the block's grid based on type. It can return None if the request is for Prompt but
    /// user hasn't enabled Prompt grid.
    pub fn grid_of_type(&self, grid_type: GridType) -> Option<&BlockGrid> {
        match grid_type {
            GridType::PromptAndCommand => Some(self.header_grid.prompt_and_command_grid()),
            GridType::Output => Some(&self.output_grid),
            GridType::Prompt if self.honor_ps1() => Some(self.header_grid.prompt_grid()),
            GridType::Rprompt if self.honor_ps1() => Some(&self.rprompt_grid),
            GridType::Prompt => None,
            GridType::Rprompt => None,
        }
    }

    /// Executes `callback` on each grid within the block.
    fn for_each_block_grid<F: Fn(&mut BlockGrid)>(&mut self, callback: F) {
        for grid_type in all::<GridType>() {
            if let Some(block_grid) = self.grid_of_type_mut(grid_type) {
                callback(block_grid)
            }
        }
    }

    /// Returns the block's grid based on type. It can return None if the request is for Prompt but
    /// user hasn't enabled Prompt grid.
    pub fn grid_of_type_mut(&mut self, grid_type: GridType) -> Option<&mut BlockGrid> {
        match grid_type {
            GridType::PromptAndCommand => Some(self.header_grid.prompt_and_command_grid_mut()),
            GridType::Output => Some(&mut self.output_grid),
            GridType::Prompt if self.honor_ps1() => Some(self.header_grid.prompt_grid_mut()),
            GridType::Rprompt if self.honor_ps1() => Some(&mut self.rprompt_grid),
            GridType::Prompt => None,
            GridType::Rprompt => None,
        }
    }

    pub(super) fn grid_handler(&self) -> &GridHandler {
        self.grid_of_type(self.active_grid_type())
            .expect("Active grid should exist")
            .grid_handler()
    }

    pub(super) fn grid_storage(&self) -> &GridStorage {
        self.grid_of_type(self.active_grid_type())
            .expect("Active grid should exist")
            .grid_storage()
    }

    pub fn grid_handler_mut(&mut self) -> &mut GridHandler {
        self.grid_of_type_mut(self.active_grid_type())
            .expect("Active grid should exist")
            .grid_handler_mut()
    }

    pub fn set_saved_cursor(&mut self, cursor: Cursor) {
        match self.active_grid_type() {
            GridType::PromptAndCommand => self
                .header_grid
                .set_saved_cursor_for_prompt_and_command(cursor),
            GridType::Output => self.output_grid.grid_storage_mut().saved_cursor = cursor,
            GridType::Prompt => self.header_grid.set_saved_cursor_for_prompt(cursor),
            GridType::Rprompt => self.rprompt_grid.grid_storage_mut().saved_cursor = cursor,
        }
    }

    /// Returns the contents of all block grids as a string.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn contents_to_string(&self) -> String {
        self.bounds_to_string(self.start_point(), self.end_point())
    }

    pub fn command_to_string(&self) -> String {
        self.header_grid.command_to_string()
    }

    /// Returns the top-level command executed in this block.
    /// Aliases are also resolved.
    ///
    /// TODO: this doesn't yet handle transitive aliases.
    pub fn top_level_command(&self, sessions: &Sessions) -> Option<String> {
        // Get the session associated to the block.
        let session = sessions.get(self.session_id?)?;
        let escape_char = session.shell_family().escape_char();

        // Parse the raw command string to get the top-level command.
        let command = warp_completer::parsers::simple::top_level_command(
            self.command_to_string(),
            escape_char,
        )?;

        // Check for aliases.
        session
            .alias_value(command.as_str())
            .map(|s| s.to_owned())
            // An alias can technically expand into an entire command (e.g. "gl" => "PAGER=0 git log").
            .and_then(|s| warp_completer::parsers::simple::top_level_command(s, escape_char))
            // If alias expansion didn't work, then just return the original top-level command.
            .or(Some(command))
    }

    pub fn output_to_string(&self) -> String {
        self.output_grid().contents_to_string(false, None)
    }

    pub fn output_to_string_force_full_grid_contents(&self) -> String {
        self.output_grid()
            .contents_to_string_force_full_grid_contents(false, None)
    }

    pub fn output_with_secrets_unobfuscated(&self) -> String {
        self.output_grid()
            .contents_to_string_with_secrets_unobfuscated(false, None)
    }

    /// Returns an iterator over all grids in the block.
    pub fn all_grids_iter(&self) -> impl Iterator<Item = &BlockGrid> {
        [
            self.prompt_grid(),
            self.rprompt_grid(),
            self.prompt_and_command_grid(),
            self.output_grid(),
        ]
        .into_iter()
    }

    /// Returns the contents of this block as a string, with the given lime limit enforced on each
    /// grid within the block.
    pub fn contents_to_string_with_line_limit(&self, line_limit: usize) -> String {
        self.bounds_to_string_with_line_limit(
            self.start_point(),
            self.end_point(),
            Some(line_limit),
        )
    }

    fn bounds_to_string_with_line_limit(
        &self,
        start: BlockGridPoint,
        end: BlockGridPoint,
        line_limit: Option<usize>,
    ) -> String {
        let (mut start_point, mut end_point) = (start.grid_point(), end.grid_point());

        // Declared here to ensure these values live long enough for future iteration through BlockGrids.
        let top_prompt_command_blockgrid: BlockGrid;
        let bottom_prompt_command_blockgrid: Option<BlockGrid>;

        let blockgrids = {
            // Note that the prompt end point is correct here (rather than command start point). The rprompt is
            // located on the same line as the last character of the lprompt (which includes any trailing newlines).
            let prompt_end_point = self.header_grid.prompt_end_point();
            let mut first_row_of_command_index = None;
            (
                top_prompt_command_blockgrid,
                bottom_prompt_command_blockgrid,
            ) = match prompt_end_point {
                Some(PromptEndPoint::PromptEnd {
                    point: prompt_end_point,
                    has_extra_trailing_newline,
                }) => {
                    // If the lprompt has a trailing newline, we purposefully go 1 extra row below the prompt end point, since that's the "true"
                    // end of the lprompt! Particularly, the rprompt is expected to be 1 row below the lprompt "printable" content in that case.
                    let prompt_end_row = prompt_end_point.row + has_extra_trailing_newline as usize;
                    if prompt_end_row + 1
                        >= self.prompt_and_command_grid().grid_handler().total_rows()
                    {
                        // We hit this case if we have a single line command (which lives on the last line of the lprompt).
                        (self.prompt_and_command_grid().clone(), None)
                    } else {
                        // We add 1 since the split() API is exclusive for the top grid and inclusive for the bottom grid (for row index given).
                        let row_to_split_on = NonZeroUsize::new(prompt_end_row + 1)
                            .expect("prompt_end_row should never equal usize::MAX");
                        let (top_grid, bottom_grid) =
                            self.prompt_and_command_grid().split(row_to_split_on);
                        // Command starts on the same row as the last character of the lprompt (including trailing newlines).
                        first_row_of_command_index = Some(prompt_end_row);
                        (top_grid, bottom_grid)
                    }
                }
                Some(PromptEndPoint::EmptyPrompt) | Some(PromptEndPoint::Stale) | None => {
                    (self.prompt_and_command_grid().clone(), None)
                }
            };

            let prompt_and_command_grid = self.header_grid.prompt_and_command_grid();
            let (start_grid_type, end_grid_type) = (GridType::from(start), GridType::from(end));

            match bottom_prompt_command_blockgrid {
                // Cases where we need to consider the top/bottom split of the prompt & command grid.
                Some(ref bottom_prompt_command_blockgrid) => {
                    // The last row of the lprompt (which also includes the first row of the command), is the last row of the "top section" of the combined grid.
                    // This is used to determine whether the selection spans across the top/bottom portions of the combined grid.
                    let first_row_of_command_index =
                        first_row_of_command_index.expect("Prompt end point should be defined.");
                    // Whether the selection expands across the rprompt (if points are in combined prompt/command grid).
                    let start_is_after_rprompt = start_point.row > first_row_of_command_index;
                    let end_is_before_rprompt = end_point.row <= first_row_of_command_index;

                    match (start_grid_type, end_grid_type) {
                        (GridType::PromptAndCommand, GridType::PromptAndCommand)
                            if start_is_after_rprompt || end_is_before_rprompt =>
                        {
                            vec![prompt_and_command_grid]
                        }
                        (GridType::PromptAndCommand, GridType::PromptAndCommand) => {
                            // Down-adjust the end point, since it needs to be relative to the bottom split of the combined
                            // prompt/command grid which was split on first_row_of_command_index above.
                            end_point.row -= first_row_of_command_index + 1;
                            vec![
                                &top_prompt_command_blockgrid,
                                &self.rprompt_grid,
                                &bottom_prompt_command_blockgrid,
                            ]
                        }
                        (GridType::PromptAndCommand, GridType::Rprompt) => {
                            vec![&top_prompt_command_blockgrid, &self.rprompt_grid]
                        }
                        (GridType::PromptAndCommand, GridType::Output)
                            if start_point.row <= first_row_of_command_index =>
                        {
                            vec![
                                &top_prompt_command_blockgrid,
                                &self.rprompt_grid,
                                &bottom_prompt_command_blockgrid,
                                &self.output_grid,
                            ]
                        }
                        (GridType::PromptAndCommand, GridType::Output) => {
                            // Down-adjust the start point, since it needs to be relative to the bottom split of the combined
                            // prompt/command grid which was split on first_row_of_command_index above.
                            start_point.row -= first_row_of_command_index + 1;
                            vec![&bottom_prompt_command_blockgrid, &self.output_grid]
                        }
                        (GridType::Rprompt, GridType::Rprompt) => {
                            vec![&self.rprompt_grid]
                        }
                        (GridType::Rprompt, GridType::PromptAndCommand) => {
                            // Down-adjust the end point, since it needs to be relative to the bottom split of the combined
                            // prompt/command grid which was split on first_row_of_command_index above.
                            end_point.row -= first_row_of_command_index + 1;
                            vec![&self.rprompt_grid, &bottom_prompt_command_blockgrid]
                        }
                        (GridType::Rprompt, GridType::Output) => {
                            vec![
                                &self.rprompt_grid,
                                &bottom_prompt_command_blockgrid,
                                &self.output_grid,
                            ]
                        }
                        (GridType::Output, GridType::Output) => {
                            vec![&self.output_grid]
                        }
                        // All other cases should be impossible.
                        _ => vec![],
                    }
                }
                // Cases where we have only 0-1 command rows, so there's no need for interleaving the rprompt contents
                // between the top/bottom portions of the combined prompt/command grid.
                None => {
                    match (start_grid_type, end_grid_type) {
                        (GridType::PromptAndCommand, GridType::PromptAndCommand) => {
                            vec![prompt_and_command_grid]
                        }
                        (GridType::PromptAndCommand, GridType::Rprompt) => {
                            vec![prompt_and_command_grid, &self.rprompt_grid]
                        }
                        (GridType::PromptAndCommand, GridType::Output) => {
                            vec![
                                prompt_and_command_grid,
                                &self.rprompt_grid,
                                &self.output_grid,
                            ]
                        }
                        // Note, we _cannot_ have the Rprompt -> PromptAndCommand case, if there is no bottom portion.
                        (GridType::Rprompt, GridType::Rprompt) => {
                            vec![&self.rprompt_grid]
                        }
                        (GridType::Rprompt, GridType::Output) => {
                            vec![&self.rprompt_grid, &self.output_grid]
                        }
                        (GridType::Output, GridType::Output) => {
                            vec![&self.output_grid]
                        }
                        // All other cases should be impossible.
                        _ => vec![],
                    }
                }
            }
        };

        // Iterate over the BlockGrids and construct the final string, given the start/end points.
        itertools::join(
            blockgrids
                .iter()
                .enumerate()
                .map(|(idx, grid)| {
                    match (idx == 0, idx == blockgrids.len() - 1) {
                        // this is the first grid, where the selection started
                        (true, false) => grid.point_to_end_as_string(start_point),
                        // this is the last grid, where the selection ended
                        (false, true) => grid.start_to_point_as_string(end_point),
                        // this is the middle grid that's selected, but the selection
                        // neither started nor ended here
                        (false, false) => {
                            grid.contents_to_string(
                                false, /* include_escape_sequences */
                                line_limit,
                            )
                        }
                        // Selection is within a single grid of a single block.
                        (true, true) => grid.grid_handler.bounds_to_string(
                            start_point,
                            end_point,
                            false, /* include_esc_sequences */
                            RespectObfuscatedSecrets::Yes,
                            false, /* force_obfuscated_secrets */
                            RespectDisplayedOutput::Yes,
                        ),
                    }
                })
                // TODO(CORE-1663): Separating the rprompt with a newline is not correct product-wise. Ideally, the
                // rprompt should be offset the right number of "empty spaces", on the same line. For this, we'd need to
                // combine the rprompt into the combined grid as well (a much larger refactor).
                .filter(|content| !content.is_empty()),
            "\n",
        )
    }

    /// Returns the contents of the block within start and end point. Spans across multiple grids.
    /// In the case of the combined grid, we have the ordering: lprompt -> first line of command -> rprompt
    /// -> rest of command -> output. Notably, we interleave the rprompt between the split portions of the combined grid.
    /// In the legacy case of separate grids, we have the ordering: lprompt -> rprompt -> command -> output.
    pub fn bounds_to_string(&self, start: BlockGridPoint, end: BlockGridPoint) -> String {
        self.bounds_to_string_with_line_limit(start, end, None /* line_limit */)
    }

    pub fn start_point(&self) -> BlockGridPoint {
        self.header_grid.prompt_start_blockgrid_point()
    }

    pub fn end_point(&self) -> BlockGridPoint {
        BlockGridPoint::Output(self.output_grid.end_point())
    }

    pub fn start_to_point_as_string(&self, point: BlockGridPoint) -> String {
        self.bounds_to_string(self.start_point(), point)
    }

    pub fn point_to_end_as_string(&self, point: BlockGridPoint) -> String {
        self.bounds_to_string(point, self.end_point())
    }

    pub fn formatted_duration_string(&self) -> Option<String> {
        self.duration()
            .or_else(|| self.elapsed_duration_whole_secs())
            .map(Self::format_duration)
    }

    /// Returns true if this block's formatted duration string is a live elapsed counter
    /// (i.e. `formatted_duration_string()` will return a value derived from `elapsed_duration()`
    /// rather than the final `duration()`).
    ///
    /// This is kept in lock-step with `elapsed_duration()` so the view layer can decide
    /// whether to wrap the duration in a periodically-repainting element.
    pub fn is_duration_live(&self) -> bool {
        self.elapsed_duration_whole_secs().is_some()
    }

    pub fn format_duration(duration: Duration) -> String {
        let hours = duration.num_hours() > 0;
        let minutes = duration.num_minutes() % MINS_PER_HOUR > 0;

        match (hours, minutes) {
            (true, true) => {
                // example: (3h 4m 12s)
                format!(
                    " ({}h {}m {:.0}s)",
                    duration.num_hours(),
                    duration.num_minutes() % MINS_PER_HOUR,
                    (duration.num_milliseconds() % MILLIS_PER_MIN) as f32 / 1000.
                )
            }
            (true, false) => {
                // example: (10h 43s)
                format!(
                    " ({}h {:.0}s)",
                    duration.num_hours(),
                    (duration.num_milliseconds() % MILLIS_PER_MIN) as f32 / 1000.
                )
            }
            (false, true) => {
                // example: (1m 8.92s)
                format!(
                    " ({}m {:.2}s)",
                    duration.num_minutes(),
                    (duration.num_milliseconds() % MILLIS_PER_MIN) as f32 / 1000.
                )
            }
            _ => {
                // example: (16.471s)
                format!(" ({}s)", duration.num_milliseconds() as f32 / 1000.)
            }
        }
    }

    #[cfg(test)]
    fn wakeup_after_delay(&self) {
        self.render_delay_complete.store(true, Ordering::Relaxed);
        self.event_proxy.send_wakeup_event();
    }

    #[cfg(not(test))]
    fn wakeup_after_delay(&self) {
        // Spawn a future on the background thread to explicitly send a `Wakeup` event. This is so
        // we can force trigger a re-render for a command that is still running but may not have
        // any output (such as the `read` command).
        let event_proxy = self.event_proxy.clone();
        let ready_to_render = self.render_delay_complete.clone();
        let delay_ms = match self.state {
            BlockState::Background => BACKGROUND_OUTPUT_RENDER_DELAY_MS,
            _ => LONG_RUNNING_COMMAND_DURATION_MS,
        };

        self.background_executor
            .spawn(async move {
                warpui::r#async::Timer::after(std::time::Duration::from_millis(delay_ms)).await;
                ready_to_render.store(true, Ordering::Relaxed);
                event_proxy.send_wakeup_event();
            })
            .detach();
    }

    /// Returns the number of lines from the top of the block given a blocksection
    pub fn block_section_offset_from_top(&self, block_section: BlockSection) -> Lines {
        match block_section {
            BlockSection::BlockBanner => self.block_banner_height(),
            BlockSection::PaddingTop => self.prompt_grid_offset(),
            BlockSection::CommandPaddingTop => self.command_grid_offset(),
            BlockSection::PromptAndCommandGrid(row) => row + self.prompt_and_command_grid_offset(),
            BlockSection::PaddingMiddle => {
                self.block_banner_height() + self.padding_top() + self.prompt_and_command_height()
            }
            BlockSection::OutputGrid(row) => row + self.output_grid_offset(),
            BlockSection::PaddingBottom => {
                self.output_grid_offset() + self.output_grid_displayed_height()
            }
            BlockSection::EndOfBlock => {
                self.output_grid_offset()
                    + self.output_grid_displayed_height()
                    + self.padding_bottom()
            }
            BlockSection::NotContained => {
                unreachable!("Attempt to calculate offset for section not within block.")
            }
        }
    }

    /// Returns the section of a block a given row is in, where a row is from the top of the block.
    pub fn find(&self, row: Lines) -> BlockSection {
        // Add a small differential to offset possible floating-point precision errors. This prevents row coordinates
        // at exact row boundaries to be mistakenly registered in the lesser row. For example, 5.0 becomes 4.999996 and
        // is then rounded to 4.0. See comment above FLOATING_POINT_ROUNDING_ADJUSTMENT for more info.
        //
        // The use cases are twofold:
        // (1) We need ordering checks that match where the two f32s are equal if they are within the margin of precision error.
        // (2) When calculating a row, we need to round up to the higher usize value if the point is at a row threshold.
        //
        // If the position lands within a grid, we compute the row within the grid using the original, unadjusted value, to
        // ensure that the returned position is accurate, clamping at zero (in case the adjustment pushed us from a region
        // of padding into a grid).

        let adjusted_row = row + FLOATING_POINT_ROUNDING_ADJUSTMENT.into_lines();

        match adjusted_row {
            x if x < self.block_banner_height() => BlockSection::BlockBanner,
            x if x < self.block_banner_height() + self.padding_top() => BlockSection::PaddingTop,
            x if x < self.block_banner_height()
                + self.padding_top()
                + self.prompt_and_command_height() =>
            {
                BlockSection::PromptAndCommandGrid(
                    (row - self.prompt_and_command_grid_offset()).max(Lines::zero()),
                )
            }
            x if x < self.output_grid_offset() => BlockSection::PaddingMiddle,
            x if x < (self.output_grid_offset() + self.output_grid_displayed_height()) => {
                BlockSection::OutputGrid((row - self.output_grid_offset()).max(Lines::zero()))
            }
            x if x < self.height(&AgentViewState::Inactive) => BlockSection::PaddingBottom,
            _ => BlockSection::NotContained,
        }
    }

    // `session_id` lives on the Block because it could change on a per-block basis.
    // Different blocks could have different machines, users, and shells.
    pub fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }

    pub fn shell_host(&self) -> Option<ShellHost> {
        self.shell_host.clone()
    }

    pub fn set_shell_host(&mut self, shell_host: ShellHost) {
        // Bash and Fish support emoji presentation selectors for bracketed paste mode correctly (moving cursor
        // appropriately), but Zsh does not, which can lead to a duplicated character bug in the command when pasting
        // certain emojis.
        let supports_emoji_presentation_selector = shell_host.shell_type != ShellType::Zsh;
        self.header_grid
            .set_supports_emoji_presentation_selector(supports_emoji_presentation_selector);
        self.rprompt_grid
            .grid_handler
            .set_supports_emoji_presentation_selector(supports_emoji_presentation_selector);
        self.output_grid
            .grid_handler
            .set_supports_emoji_presentation_selector(supports_emoji_presentation_selector);

        self.shell_host = Some(shell_host);
    }

    #[cfg(test)]
    pub fn set_session_id(&mut self, id: SessionId) {
        self.session_id = Some(id);
    }

    pub fn pwd(&self) -> Option<&String> {
        self.pwd.as_ref()
    }

    /// Returns the "user-friendly" directory (e.g. the pwd with $HOME abbreviated to '~') for the
    /// block.
    pub fn user_friendly_pwd(&self) -> Option<Cow<'_, str>> {
        let local_home_directory =
            dirs::home_dir().and_then(|home_buf| home_buf.to_str().map(|s| s.to_owned()));
        self.pwd
            .as_ref()
            .map(|pwd| user_friendly_path(pwd.as_str(), local_home_directory.as_deref()))
    }

    pub fn set_home_dir(&mut self, home_dir: Option<String>) {
        self.home_dir = home_dir;
    }

    pub fn set_cloud_env_var_state(&mut self, env_var_collection_id: Option<SyncId>) {
        self.cloud_env_var_collection_id = env_var_collection_id;
    }

    pub fn cloud_env_var_collection_state(&self) -> Option<SyncId> {
        self.cloud_env_var_collection_id
    }

    pub fn set_cloud_workflow_state(&mut self, workflow_id: Option<SyncId>) {
        self.cloud_workflow_id = workflow_id;
    }

    pub fn cloud_workflow_state(&self) -> Option<SyncId> {
        self.cloud_workflow_id
    }

    pub fn server_pwd(&self) -> Option<Cow<'_, str>> {
        self.pwd
            .as_ref()
            .map(|pwd| user_friendly_path(pwd.as_str(), self.home_dir.as_deref()))
    }

    pub fn creation_ts(&self) -> &DateTime<Local> {
        &self.creation_ts
    }

    pub fn completed_ts(&self) -> Option<&DateTime<Local>> {
        self.completed_ts.as_ref()
    }

    pub fn start_ts(&self) -> Option<&DateTime<Local>> {
        self.start_ts.as_ref()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.start_ts
            .zip(self.completed_ts)
            .and_then(|(start, end)| {
                let duration = end.signed_duration_since(start);
                (duration > Duration::zero()).then_some(duration)
            })
    }

    /// Returns the elapsed duration since the command started executing, rounded down
    /// to the nearest second.
    ///
    /// Returns `Some` only when the block is actively executing (has `start_ts`,
    /// no `completed_ts`, and `is_executing()`). Rounding to whole seconds keeps the
    /// formatted duration string stable between one-second repaint ticks so that
    /// unrelated `ctx.notify` calls (e.g. when the user interacts with the block)
    /// don't cause the counter to flicker sub-second values.
    pub fn elapsed_duration_whole_secs(&self) -> Option<Duration> {
        self.elapsed_duration_whole_secs_at(Local::now())
    }

    /// Testable helper behind `elapsed_duration()` that takes an explicit `now`.
    fn elapsed_duration_whole_secs_at(&self, now: DateTime<Local>) -> Option<Duration> {
        if self.completed_ts.is_some() || !self.is_executing() {
            return None;
        }
        self.start_ts.and_then(|start| {
            let elapsed = now.signed_duration_since(start);
            let whole_secs = elapsed.num_seconds();
            (whole_secs > 0).then(|| Duration::seconds(whole_secs))
        })
    }

    pub fn git_branch(&self) -> Option<&String> {
        self.git_branch.as_ref()
    }

    pub fn git_branch_name(&self) -> Option<&String> {
        self.git_branch_name.as_ref()
    }

    pub fn conda_env(&self) -> Option<&String> {
        self.conda_env.as_ref()
    }

    pub fn node_version(&self) -> Option<&String> {
        self.node_version.as_ref()
    }

    #[cfg(feature = "integration_tests")]
    pub fn prompt_to_string(&self) -> String {
        self.header_grid.prompt_to_string()
    }

    pub fn virtual_env_short_name(&self) -> Option<String> {
        self.virtual_env
            .as_ref()
            .map(|env_path| env_path.rsplit('/').next().unwrap_or(env_path).to_string())
    }

    /// Determines if the command for this block failed.
    pub fn has_failed(&self) -> bool {
        has_block_failed(self.exit_code, self.state)
    }

    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }

    pub fn is_done(&self) -> bool {
        matches!(
            self.state,
            BlockState::DoneWithExecution | BlockState::DoneWithNoExecution
        )
    }

    pub fn set_show_bootstrap_block(&mut self, show_bootstrap_block: bool) {
        self.show_bootstrap_block = show_bootstrap_block;
    }

    pub fn set_show_in_band_command_blocks(&mut self, show_in_band_command_blocks: bool) {
        self.show_in_band_command_blocks = show_in_band_command_blocks;
    }

    pub fn set_show_memory_stats(&mut self, show_memory_stats: bool) {
        self.show_memory_stats = show_memory_stats
    }

    pub fn is_mode_set(&self, mode: TermMode) -> bool {
        self.output_grid.grid_handler.is_mode_set(mode)
    }

    pub fn has_received_precmd(&self) -> bool {
        self.precmd_state == PrecmdState::AfterPrecmd
    }

    pub fn is_in_band_command_block(&self) -> bool {
        self.is_for_in_band_command
    }

    /// Re-filters the output of a block if there is an active filter query present
    /// and there are truncated rows.
    pub fn maybe_refilter_output(&mut self) {
        if self
            .filter_query
            .as_ref()
            .is_some_and(|filter_query| filter_query.is_active)
        {
            self.output_grid.maybe_refilter_lines();
        }
    }

    /// Apply a filter to this block's output. The logical lines containing the
    /// matches will be shown in the block's visible output, while non-matching
    /// lines will be hidden.
    pub fn filter_output(&mut self, filter_query: BlockFilterQuery) {
        if filter_query.is_active {
            match filter_query.construct_dfas() {
                Ok(dfas) => {
                    self.output_grid.filter_lines(
                        Arc::new(dfas),
                        filter_query.num_context_lines as usize,
                        filter_query.invert_filter_enabled,
                    );
                }
                Err(dfa_build_error) => {
                    log::warn!("Error constructing Block Filter DFAs: {dfa_build_error}");
                }
            }
        } else {
            self.clear_filter();
        }
        self.filter_query = Some(filter_query);
    }

    /// Clear the applied filter. Will reset the visible output so all rows in
    /// the output grid will be visible.
    pub fn clear_filter(&mut self) {
        self.filter_query = None;
        self.output_grid.clear_filter();
    }

    pub fn current_filter(&self) -> Option<&BlockFilterQuery> {
        self.filter_query.as_ref()
    }

    pub fn displayed_output_rows(&self) -> Option<impl DoubleEndedIterator<Item = usize> + '_> {
        self.output_grid.grid_handler().displayed_output_rows()
    }

    pub fn displayed_output_row_ranges(
        &self,
    ) -> Option<impl DoubleEndedIterator<Item = RangeInclusive<usize>> + '_> {
        self.output_grid
            .grid_handler()
            .displayed_output_row_ranges()
    }

    pub fn has_active_filter(&self) -> bool {
        self.filter_query.is_some()
    }

    pub fn needs_bracketed_paste(&self) -> bool {
        if self.state == BlockState::BeforeExecution {
            self.header_grid.command_needs_bracketed_paste()
        } else {
            self.output_grid.needs_bracketed_paste()
        }
    }

    /// Returns `true` if this block is a valid option to use as context for an AI model.
    pub fn can_be_ai_context(&self, agent_view_state: &AgentViewState) -> bool {
        self.is_visible(agent_view_state)
            && !self.is_in_band_command_block()
            && !self.is_agent_monitoring()
    }

    pub fn estimated_heap_usage_bytes(&self) -> usize {
        // For now, we're only factoring in heap allocations in grids, and not
        // in other fields.
        self.all_grids_iter()
            .map(|grid| grid.estimated_memory_usage_bytes())
            .sum()
    }

    pub fn estimated_memory_usage_bytes(&self) -> usize {
        // size of struct on the stack
        std::mem::size_of_val(self)
            // size of heap-allocated data
            + self.estimated_heap_usage_bytes()
    }

    pub fn grid_storage_lines(&self) -> usize {
        use warp_terminal::model::grid::Dimensions as _;

        self.all_grids_iter()
            .map(|grid| grid.grid_storage().total_rows())
            .sum()
    }

    pub fn grid_storage_bytes(&self) -> usize {
        self.all_grids_iter()
            .map(|grid| grid.grid_storage().estimated_memory_usage_bytes())
            .sum()
    }

    pub fn flat_storage_lines(&self) -> usize {
        self.all_grids_iter()
            .map(|grid| grid.flat_storage_lines())
            .sum()
    }

    pub fn flat_storage_bytes(&self) -> usize {
        self.all_grids_iter()
            .map(|grid| grid.flat_storage_bytes())
            .sum()
    }

    pub fn last_painted_at(&self) -> Option<DateTime<Local>> {
        *self.last_painted_at.borrow()
    }

    /// Updates the block's last-painted-at time.
    pub fn update_last_painted_at(&self, last_painted_at: DateTime<Local>) {
        self.last_painted_at.borrow_mut().replace(last_painted_at);
    }

    pub(super) fn set_marked_text(&mut self, marked_text: &str, selected_range: &Range<usize>) {
        if self.state != BlockState::Executing {
            log::warn!("Tried to set marked text on block when block was not executing");
            return;
        }
        self.output_grid
            .set_marked_text(marked_text, selected_range);
    }

    pub(super) fn clear_marked_text(&mut self) {
        if self.state != BlockState::Executing {
            log::warn!("Tried to clear marked text on block when block was not executing");
            return;
        }
        self.output_grid.clear_marked_text();
    }
}

/// Used in the ansi::Handler implementation for Block below. Performs
/// the provided method call on the active grid, the command grid if in input mode
/// or the output grid if in output mode.
macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        match $self.header_grid.receiving_chars_for_prompt {
            Some(ansi::PromptKind::Initial) => {
                $self.header_grid.$method($( $arg ),*)
            },
            Some(ansi::PromptKind::Right) => {
                if !$self.ignore_next_rprompt {
                    $self.rprompt_grid.$method($( $arg ),*)
                } else {
                    Default::default()
                }
            },
            _ => {
                if $self.state == BlockState::BeforeExecution {
                    $self.header_grid.$method($( $arg ),*)
                } else {
                    $self.output_grid.$method($( $arg ),*)
                }
            }
        }
    };
}

impl ansi::Handler for Block {
    fn set_title(&mut self, _: Option<String>) {
        log::error!("Handler method Block::set_title should never be called. This should be handled by TerminalModel.");
    }

    fn set_cursor_style(&mut self, style: Option<ansi::CursorStyle>) {
        delegate!(self.set_cursor_style(style));
    }

    fn set_cursor_shape(&mut self, shape: ansi::CursorShape) {
        delegate!(self.set_cursor_shape(shape));
    }

    fn input(&mut self, c: char) {
        delegate!(self.input(c));
    }

    fn goto(&mut self, row: VisibleRow, column: usize) {
        // Only apply this correction for ConPTY.
        #[cfg(windows)]
        let row = row.saturating_sub(self.leading_linefeeds_ignored);
        delegate!(self.goto(row, column));
    }

    fn goto_line(&mut self, row: VisibleRow) {
        #[cfg(windows)]
        let row = row.saturating_sub(self.leading_linefeeds_ignored);
        delegate!(self.goto_line(row));
    }

    fn goto_col(&mut self, column: usize) {
        delegate!(self.goto_col(column));
    }

    fn insert_blank(&mut self, count: usize) {
        delegate!(self.insert_blank(count));
    }

    fn move_up(&mut self, rows: usize) {
        delegate!(self.move_up(rows));
    }

    fn move_down(&mut self, rows: usize) {
        delegate!(self.move_down(rows));
    }

    fn identify_terminal<W: std::io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        delegate!(self.identify_terminal(writer, intermediate));
    }

    fn report_xtversion<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.report_xtversion(writer));
    }

    fn device_status<W: std::io::Write>(&mut self, writer: &mut W, arg: usize) {
        delegate!(self.device_status(writer, arg));
    }

    fn move_forward(&mut self, columns: usize) {
        delegate!(self.move_forward(columns));
    }

    fn move_backward(&mut self, columns: usize) {
        delegate!(self.move_backward(columns));
    }

    fn move_down_and_cr(&mut self, rows: usize) {
        delegate!(self.move_down_and_cr(rows));
    }

    fn move_up_and_cr(&mut self, rows: usize) {
        delegate!(self.move_up_and_cr(rows));
    }

    fn put_tab(&mut self, count: u16) {
        delegate!(self.put_tab(count));
    }

    fn backspace(&mut self) {
        delegate!(self.backspace());
    }

    fn carriage_return(&mut self) {
        delegate!(self.carriage_return());
    }

    fn linefeed(&mut self) -> ScrollDelta {
        // If we're processing a prompt and we receive an initial blank line,
        // ignore it.  This is sometimes used in prompts (e.g.: oh-my-zsh's
        // "re5et" theme) to separate the previous command's output from the
        // prompt, but this is not needed in Warp due to us visually separating
        // blocks.
        match self.header_grid.receiving_chars_for_prompt {
            Some(ansi::PromptKind::Initial) if !self.header_grid.prompt_has_received_content() => {
                self.leading_linefeeds_ignored += 1;
                return ScrollDelta::zero();
            }
            Some(ansi::PromptKind::Right) if !self.rprompt_grid.has_received_content() => {
                self.leading_linefeeds_ignored += 1;
                return ScrollDelta::zero();
            }
            _ => {}
        }
        delegate!(self.linefeed())
    }

    fn bell(&mut self) {
        delegate!(self.bell());
    }

    fn substitute(&mut self) {
        delegate!(self.substitute());
    }

    fn newline(&mut self) {
        delegate!(self.newline());
    }

    fn set_horizontal_tabstop(&mut self) {
        delegate!(self.set_horizontal_tabstop());
    }

    fn scroll_up(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_up(rows))
    }

    fn scroll_down(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_down(rows))
    }

    fn insert_blank_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.insert_blank_lines(rows))
    }

    fn delete_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.delete_lines(rows))
    }

    fn erase_chars(&mut self, count: usize) {
        delegate!(self.erase_chars(count));
    }

    fn delete_chars(&mut self, count: usize) {
        delegate!(self.delete_chars(count));
    }

    fn move_backward_tabs(&mut self, count: u16) {
        delegate!(self.move_backward_tabs(count));
    }

    fn move_forward_tabs(&mut self, count: u16) {
        delegate!(self.move_forward_tabs(count));
    }

    fn save_cursor_position(&mut self) {
        delegate!(self.save_cursor_position());
    }

    fn restore_cursor_position(&mut self) {
        delegate!(self.restore_cursor_position());
    }

    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        delegate!(self.clear_line(mode));
    }

    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        delegate!(self.clear_screen(mode));
    }

    fn clear_tabs(&mut self, mode: ansi::TabulationClearMode) {
        delegate!(self.clear_tabs(mode));
    }

    fn reset_state(&mut self) {
        delegate!(self.reset_state());
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        delegate!(self.reverse_index())
    }

    fn terminal_attribute(&mut self, attribute: ansi::Attr) {
        delegate!(self.terminal_attribute(attribute));
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.set_mode(mode));
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.unset_mode(mode));
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        delegate!(self.set_scrolling_region(top, bottom));
    }

    fn set_keypad_application_mode(&mut self) {
        delegate!(self.set_keypad_application_mode());
    }

    fn unset_keypad_application_mode(&mut self) {
        delegate!(self.unset_keypad_application_mode());
    }

    fn set_active_charset(&mut self, index: ansi::CharsetIndex) {
        delegate!(self.set_active_charset(index));
    }

    fn configure_charset(&mut self, index: ansi::CharsetIndex, charset: ansi::StandardCharset) {
        delegate!(self.configure_charset(index, charset));
    }

    fn set_color(&mut self, index: usize, color: ColorU) {
        delegate!(self.set_color(index, color));
    }

    fn dynamic_color_sequence<W: std::io::Write>(
        &mut self,
        writer: &mut W,
        code: u8,
        index: usize,
        terminator: &str,
    ) {
        delegate!(self.dynamic_color_sequence(writer, code, index, terminator));
    }

    fn reset_color(&mut self, index: usize) {
        delegate!(self.reset_color(index));
    }

    fn clipboard_store(&mut self, clipboard: u8, data: &[u8]) {
        delegate!(self.clipboard_store(clipboard, data));
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        delegate!(self.clipboard_load(clipboard, terminator));
    }

    fn decaln(&mut self) {
        delegate!(self.decaln());
    }

    fn push_title(&mut self) {
        log::error!("Handler method Block::push_title should never be called. This should be handled by TerminalModel.");
    }

    fn pop_title(&mut self) {
        log::error!("Handler method Block::pop_title should never be called. This should be handled by TerminalModel.");
    }

    fn prompt_marker(&mut self, marker: ansi::PromptMarker) {
        match marker {
            ansi::PromptMarker::StartPrompt { kind } => {
                match kind {
                    ansi::PromptKind::Initial => {
                        self.header_grid.prompt_marker(marker);
                        if !self.header_grid.ignore_next_prompt_preview() {
                            // Reset the right prompt when the initial prompt is drawn to
                            // match shell behavior. Note that we reset the lprompt in HeaderGrid
                            // via the function call above, rather than reset it here!
                            self.rprompt_grid.reset_state();
                        }
                    }
                    ansi::PromptKind::Right => {
                        if !self.ignore_next_rprompt {
                            log::debug!("Received start prompt marker for right prompt");
                            self.rprompt_grid.reset_state();
                            self.rprompt_grid.start();
                        }
                    }
                };
                self.header_grid.receiving_chars_for_prompt = Some(kind);
            }
            ansi::PromptMarker::EndPrompt => {
                let Some(kind) = self.header_grid.receiving_chars_for_prompt else {
                    log::debug!("Received end prompt marker without a matching start marker");
                    return;
                };
                match kind {
                    ansi::PromptKind::Initial => {
                        self.header_grid.prompt_marker(marker);
                    }
                    ansi::PromptKind::Right => {
                        if self.ignore_next_rprompt {
                            self.ignore_next_rprompt = false;
                        } else {
                            log::debug!("Received end prompt marker for right prompt");
                            self.rprompt_grid.finish();
                        }
                    }
                }
                // Reset the indicator for receiving prompt characters.
                self.header_grid.receiving_chars_for_prompt = None;
                self.event_proxy.send_terminal_event(Event::PromptUpdated);
            }
        }
    }

    fn text_area_size_pixels<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_pixels(writer));
    }

    fn text_area_size_chars<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_chars(writer));
    }

    fn precmd(&mut self, data: PrecmdValue) {
        record_trace_event!("command_execution:block:precmd");
        let is_after_in_band_command = data.was_sent_after_in_band_command();

        self.header_grid.precmd(data.clone());

        self.state = BlockState::BeforeExecution;
        self.pwd = data.pwd;
        self.git_branch.clone_from(&data.git_head);
        self.git_branch_name.clone_from(&data.git_branch);
        self.virtual_env = data.virtual_env;
        self.conda_env = data.conda_env;
        self.node_version = data.node_version;
        self.session_id = data.session_id.map(Into::into);
        self.rprompt.clone_from(&data.rprompt);

        if let Some(rprompt) = data.rprompt {
            self.init_rprompt_grid(&rprompt);
        }

        self.precmd_state = PrecmdState::AfterPrecmd;
        self.event_proxy
            .send_terminal_event(Event::BlockMetadataReceived(BlockMetadataReceivedEvent {
                block_metadata: self.metadata(),
                block_index: self.block_index,
                is_after_in_band_command,
                is_done_bootstrapping: matches!(
                    self.bootstrap_stage,
                    BootstrapStage::PostBootstrapPrecmd
                ),
            }));
    }

    fn preexec(&mut self, data: PreexecValue) {
        record_trace_event!("command_execution:block:prexec");

        // This condition is a hack to fix a bug with shells that don't support bracketed paste,
        // e.g. legacy Bash versions, 4.4 or earlier.
        // https://lists.gnu.org/archive/html/info-gnu/2016-09/msg00012.html
        // The bug happens when multi-line commands are submitted, see CORE-1698. Without bracketed
        // paste, we get multiple blocks per [`crate::terminal::input::Event::ExecuteCommand`]. We
        // generally assume the code path on ExecuteCommand is responsible for starting the active
        // block. So, `self.started()` should always be true by this point. However, this assumption
        // is violated if we get multiple blocks per ExecuteCommand event. So, as a fallback, we
        // start the block here if it hasn't happened already. Note: the displayed command duration
        // in the "block label" may be under-estimated in this case.
        if !self.started() && self.state == BlockState::BeforeExecution {
            self.start();
        }

        self.header_grid.preexec(data.clone());

        let is_for_in_band_command = command_executor::is_in_band_command(data.command.as_str());
        if self.bootstrap_stage() == BootstrapStage::PostBootstrapPrecmd {
            self.event_proxy
                .send_terminal_event(Event::AfterBlockStarted {
                    block_id: self.id.clone(),
                    command: self.command_to_string(),
                    is_for_in_band_command,
                });
        }

        self.leading_linefeeds_ignored = 0;
        self.output_grid.start();
        self.state = BlockState::Executing;
        self.is_for_in_band_command = is_for_in_band_command;

        self.wakeup_after_delay();
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        delegate!(self.on_finish_byte_processing(input));
    }

    fn on_reset_grid(&mut self) {
        delegate!(self.on_reset_grid());
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        delegate!(self.handle_completed_iterm_image(image))
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        delegate!(self.handle_completed_kitty_action(action, metadata))
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    ) {
        delegate!(self.set_keyboard_enhancement_flags(mode, apply));
    }

    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes) {
        delegate!(self.push_keyboard_enhancement_flags(mode));
    }

    fn pop_keyboard_enhancement_flags(&mut self, count: u16) {
        delegate!(self.pop_keyboard_enhancement_flags(count));
    }

    fn query_keyboard_enhancement_flags<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.query_keyboard_enhancement_flags(writer));
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
