use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::block_list_element::GridType;
use crate::terminal::event::{
    BootstrappedEvent, Event, ExecutedExecutorCommandEvent, InitSshEvent, InitSubshellEvent,
    SourcedRcFileInSubshellEvent, SshLoginStatus, TerminalMode,
};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi;
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::completions::{
    ShellCompletion, ShellCompletionUpdate, ShellData as CompletionsShellData,
};
use crate::terminal::model::escape_sequences::ModeProvider;
use crate::terminal::model::index::VisibleRow;
use crate::terminal::model::iterm_image::{ITermImage, ITermImageMetadata};
use crate::terminal::shared_session::{ai_agent::encode_agent_response_event, SharedSessionStatus};
use crate::terminal::ssh::util::{InteractiveSshCommand, SshLoginState};
use crate::terminal::{block_filter::BlockFilterQuery, model::ansi::Handler};
use crate::terminal::{color, ssh, BlockPadding, ShellHost, SizeUpdate, SizeUpdateReason};
use crate::terminal::{ShellLaunchData, ShellLaunchState};
use crate::util::AsciiDebug;

pub use crate::terminal::history::HistoryEntry;

use super::ansi::{
    FinishUpdateValue, InputBufferValue, Mode, PendingHook, TmuxInstallFailedInfo,
    WarpificationUnavailableReason,
};
use super::block::{
    AgentInteractionMetadata, Block, BlockId, BlockMetadata, BlockSize, BlockState,
    BlocklistEnvVarMetadata, SerializedBlock,
};
use super::blockgrid::BlockGrid;
use super::grid::grid_handler::{
    ContainsPoint, FragmentBoundary, GridHandler, Link, PossiblePath, TermMode,
};
use super::image_map::StoredImageMetadata;
use super::index::Point;
use super::kitty::{
    create_kitty_error_reply, create_kitty_ok_reply, DeletionType, KittyAction, KittyChunk,
    KittyMessage, KittyResponse, PendingKittyMessage,
};
use super::secrets::{RespectObfuscatedSecrets, SecretAndHandle};
use super::selection::ScrollDelta;
use super::session::{BootstrapSessionType, InBandCommandOutputReceiver, SessionId};
use super::tmux::commands::TmuxCommand;
use super::{
    super::{AltScreen, BlockList},
    ansi::BootstrappedValue,
};
use super::{tmux, Secret, SecretHandle};
use crate::terminal::model::ansi::{
    ClearValue, CommandFinishedValue, ExitShellValue, InitShellValue, InitSshValue,
    InitSubshellValue, PreInteractiveSSHSessionValue, PrecmdValue, PreexecValue, SSHValue,
    SourcedRcFileForWarpValue,
};
use crate::terminal::model::grid::IndexRegion;
use crate::terminal::model::session::SessionInfo;
use crate::terminal::shell::{ShellName, ShellType};

use crate::terminal::model::secrets::ObfuscateSecrets;
use session_sharing_protocol::sharer::SessionSourceType;
use warp_core::report_error;
#[cfg(not(target_family = "wasm"))]
use warpui::util::save_as_file;

use async_channel::Sender;
use base64::Engine;
use hex::FromHexError;
use instant::Instant;
use itertools::{Either, Itertools};
use serde::Serialize;
use session_sharing_protocol::common::{
    AICommandMetadata, OrderedTerminalEventType, ParticipantId,
};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::num::ParseIntError;
use std::ops::{Range, RangeInclusive};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::SemanticSelection;
pub use warp_terminal::model::BlockIndex;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warpui::assets::asset_cache::Asset;
use warpui::image_cache::ImageType;
use warpui::r#async::executor::Background;
use warpui::AppContext;

/// Max size of the window title stack.
const TITLE_STACK_MAX_DEPTH: usize = 4096;

/// The status of a conversation transcript viewer.
/// This tracks both the loading state and the type of conversation being viewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationTranscriptViewerStatus {
    /// Loading conversation data from the server.
    Loading,
    /// Viewing a local conversation (not from ambient agent).
    ViewingLocalConversation,
    /// Viewing an ambient agent conversation with the associated task ID.
    ViewingAmbientConversation(AmbientAgentTaskId),
}

#[derive(Debug, Clone, Default)]
pub struct FindOptions {
    pub query: Option<Arc<String>>,
    pub is_case_sensitive: bool,
    pub is_regex_enabled: bool,

    /// If `Some()`, the find run only surfaces matches that are in blocks with the provided
    /// indices. If `None`, the find run surfaces matches across the entire blocklist.
    ///
    /// This is ignored when the alt screen is active.
    pub blocks_to_include_in_results: Option<Vec<BlockIndex>>,
}

impl FindOptions {
    pub fn with_is_case_sensitive(mut self, is_case_sensitive: bool) -> Self {
        self.is_case_sensitive = is_case_sensitive;
        self
    }

    pub fn with_is_regex_enabled(mut self, is_regex_enabled: bool) -> Self {
        self.is_regex_enabled = is_regex_enabled;
        self
    }

    pub fn with_query(mut self, query: Option<impl Into<Arc<String>>>) -> Self {
        self.query = query.map(Into::into);
        self
    }

    pub fn with_blocks_to_include_in_results(
        mut self,
        block_indices: Option<impl IntoIterator<Item = BlockIndex>>,
    ) -> Self {
        self.blocks_to_include_in_results =
            block_indices.map(|indices| indices.into_iter().collect());
        self
    }
}

pub enum FindOption {
    ResetWithSameQuery,
    Query(String),
    IsCaseSensitive(bool),
    IsRegexEnabled(bool),
}

/// A type that is either within the AltScreen or a specific part of the BlockList
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithinModel<T> {
    AltScreen(T),
    BlockList(WithinBlock<T>),
}

pub trait RangeInModel {
    fn range(&self) -> RangeInclusive<Point>;
}

impl<T> WithinModel<T> {
    pub fn get_inner(&self) -> &T {
        match self {
            Self::AltScreen(inner) => inner,
            Self::BlockList(block_list) => &block_list.inner,
        }
    }

    pub fn replace_inner<S>(self, inner: S) -> WithinModel<S> {
        match self {
            WithinModel::AltScreen(_) => WithinModel::AltScreen(inner),
            WithinModel::BlockList(within_block) => WithinModel::BlockList(WithinBlock::new(
                inner,
                within_block.block_index,
                within_block.grid,
            )),
        }
    }

    pub fn read_from_grid<'m, 's, U>(
        &'s self,
        model: &'m TerminalModel,
        func: impl FnOnce(&'m GridHandler, &'s T) -> anyhow::Result<U>,
    ) -> anyhow::Result<U> {
        match self {
            WithinModel::AltScreen(inner) => func(model.alt_screen().grid_handler(), inner),
            WithinModel::BlockList(within_block) => {
                let grid_handler = model.block_list().grid_handler_within_block(within_block)?;
                func(grid_handler, &within_block.inner)
            }
        }
    }

    pub fn update_grid<'m, 's, U>(
        &'s self,
        model: &'m mut TerminalModel,
        func: impl FnOnce(&'m mut GridHandler, &'s T) -> anyhow::Result<U>,
    ) -> anyhow::Result<U> {
        match self {
            WithinModel::AltScreen(inner) => func(model.alt_screen_mut().grid_handler_mut(), inner),
            WithinModel::BlockList(within_block) => {
                let grid_handler = model
                    .block_list_mut()
                    .grid_handler_mut_within_block(within_block)?;
                func(grid_handler, &within_block.inner)
            }
        }
    }
}

impl<B> WithinModel<B>
where
    B: ContainsPoint,
{
    pub fn contains(&self, other: &WithinModel<Point>) -> bool {
        match (self, other) {
            (WithinModel::AltScreen(range), WithinModel::AltScreen(other)) => {
                range.contains(*other)
            }
            (WithinModel::BlockList(block_range), WithinModel::BlockList(other)) => {
                block_range.grid == other.grid
                    && block_range.block_index == other.block_index
                    && block_range.inner.contains(other.inner)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd)]
pub struct WithinBlock<T> {
    pub block_index: BlockIndex,
    pub grid: GridType,
    pub inner: T,
}

impl<T> WithinBlock<T> {
    pub fn new(inner: T, block_index: BlockIndex, grid: GridType) -> WithinBlock<T> {
        Self {
            inner,
            block_index,
            grid,
        }
    }

    pub fn is_in_command_content(&self) -> bool {
        self.grid == GridType::PromptAndCommand
    }

    pub fn is_output_grid(&self) -> bool {
        self.grid == GridType::Output
    }

    pub fn in_same_block_and_grid<U>(&self, other: &WithinBlock<U>) -> bool {
        self.block_index == other.block_index && self.grid == other.grid
    }

    pub fn get(&self) -> &T {
        &self.inner
    }

    pub fn replace(&mut self, value: T) {
        self.inner = value;
    }

    pub fn map<F, U>(self, f: F) -> WithinBlock<U>
    where
        F: FnOnce(T) -> U,
    {
        let new_val: U = f(self.inner);
        WithinBlock {
            inner: new_val,
            block_index: self.block_index,
            grid: self.grid,
        }
    }

    pub fn is_within_this_block(&self, index: BlockIndex) -> bool {
        self.block_index == index
    }
}

impl<T: Copy> WithinBlock<RangeInclusive<T>> {
    /// When you wrap a Range in a WithinBlock, this method gives you the ends of the range as
    /// separately wrapped values
    pub fn unfold_range(&self) -> (WithinBlock<T>, WithinBlock<T>) {
        let start_val = WithinBlock::new(*self.inner.start(), self.block_index, self.grid);
        let end_val = WithinBlock::new(*self.inner.end(), self.block_index, self.grid);
        (start_val, end_val)
    }
}

impl WithinBlock<Point> {
    // Check if one point is visually before another in the blocklist.
    pub fn is_visually_before(&self, other: &WithinBlock<Point>, inverted_blocklist: bool) -> bool {
        // If two points are within the same block OR if the blocklist is NOT inverted
        // the visual order should be the same as the points' logical order.
        if self.block_index == other.block_index || !inverted_blocklist {
            self < other
        // Else. The visual order is reverse as the points' logical order.
        } else {
            self > other
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistoryItem {
    pub command: String,
    pub shell_host: Option<ShellHost>,
}

/// Represents whether or not bytes read from the PTY should be considered in-band command output.
enum IsReceivingInBandCommandOutput {
    Yes {
        output: InBandCommandOutputReceiver,
    },

    /// PTY output should be handled normally.
    No,
}

/// Represents whether or not bytes read from the PTY should be considered completions output.
enum IsReceivingCompletionsOutput {
    /// We're currently expecting completions data to come over the PTY.
    /// The exact data we're expecting depends on the [`CompletionsShellData`] type.
    Yes { pending: CompletionsShellData },

    /// PTY output should be handled normally.
    No,
}

/// Represents whether or not bytes read from the PTY should be considered iTerm image data.
enum IsReceivingITermImageData {
    /// We're currently expecting chunks of image data to come over the PTY to form the entire image.
    Yes { pending: ITermImage },

    /// PTY output should be handled normally.
    No,
}

/// Represents whether or not bytes read from the PTY should be considered Kitty action data.
enum IsReceivingKittyActionData {
    /// We're currently expecting chunks of action data to come over the PTY to form the entire action.
    Yes { pending: PendingKittyMessage },

    /// PTY output should be handled normally.
    No,
}

/// Represents whether or not output from the PTY should be considered part of a shell hook being
/// sent over via key-value pairs.
///
/// This is currently only used for Git Bash.
enum IsReceivingHook {
    Yes { pending_hook: Box<PendingHook> },
    No,
}

/// Information needed to render a warpify "success" block upon successful subshell bootstrap.
#[derive(Debug, Clone)]
pub struct SubshellSuccessBlockInfo {
    /// The ID of the newly bootstrapped subshell session.
    ///
    /// This ID is needed in order to associate the success block with the subshell session (and
    /// include it in the subshell "context" UI, with the flag and block border).
    pub subshell_session_id: SessionId,

    /// The command which spawned the subshell.
    pub spawning_command: String,

    pub shell_type: ShellType,

    pub session_type: BootstrapSessionType,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TmuxInstallationState {
    /// This means tmux was installed by Warp in this session, successfully or unsuccessfully.
    /// It also means we had root access and used a package manager to install tmux and all
    /// dependencies.
    InstalledByWarpRootInThisSession,
    /// This means tmux was installed by Warp in this session, successfully or unsuccessfully.
    InstalledByWarpInThisSession,
    InstalledByWarpInPriorSession,
    /// This means that warp did not install it locally. It was either installed by the user
    /// or it was installed by warp in a prior session using the package manager.
    InstalledByUser,
    /// This means we never tried to install tmux in this session.
    #[default]
    NotInstalled,
}

impl FromStr for TmuxInstallationState {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "installed_by_warp_root_in_this_session" => {
                Ok(TmuxInstallationState::InstalledByWarpRootInThisSession)
            }
            "installed_by_warp_in_this_session" => {
                Ok(TmuxInstallationState::InstalledByWarpInThisSession)
            }
            "warp" | "installed_by_warp_in_prior_session" => {
                Ok(TmuxInstallationState::InstalledByWarpInPriorSession)
            }
            "user" | "installed_by_user" => Ok(TmuxInstallationState::InstalledByUser),
            "not_installed" => Ok(TmuxInstallationState::NotInstalled),
            _ => Err(anyhow::anyhow!("Invalid TmuxInstallationState")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpInitiatedTmuxControlMode {
    pub start_time: Instant,
    pub tmux_installation: Option<TmuxInstallationState>,
}

impl WarpInitiatedTmuxControlMode {
    pub fn new(tmux_installation: Option<TmuxInstallationState>) -> Self {
        Self {
            start_time: Instant::now(),
            tmux_installation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TmuxControlModeContext {
    UserInitiated,
    WarpInitiatedForSsh(WarpInitiatedTmuxControlMode),
}

impl TmuxControlModeContext {
    pub fn tmux_installation(&self) -> Option<TmuxInstallationState> {
        match self {
            TmuxControlModeContext::UserInitiated => None,
            TmuxControlModeContext::WarpInitiatedForSsh(warp_initiated) => {
                warp_initiated.tmux_installation
            }
        }
    }
}

pub struct TerminalModel {
    /// For fullscreen programs like vim.
    alt_screen: AltScreen,

    /// True if the local user has made edits in the input editor since the last submit (shell or AI).
    is_input_dirty: bool,

    /// List of blocks. All blocks are immutable except for the current block.
    /// Always non-empty (includes an invisible block).
    block_list: BlockList,
    /// Whether the blocklist has been cleared in the lifetime of this terminal model.
    pub blocklist_has_been_cleared: bool,

    alt_screen_active: bool,

    /// Stack of saved window titles. When a title is popped from this stack, the `title` for the
    /// term is set.
    title_stack: Vec<Option<String>>,

    /// Current title of the window. Used if the `custom_title` is not set.
    title: Option<String>,
    /// Custom title of the window, set manually by the user.
    custom_title: Option<String>,

    /// Default colors to render characters.
    colors: color::List,

    /// Color overrides set via escape sequence. If a color is not set here, the view determines the
    /// color based on the theme.
    override_colors: color::OverrideList,

    pub(crate) event_proxy: ChannelEventListener,

    /// The pending `SSHValue`, if any, of the active session. This is a temporary value that's
    /// stored between when an SSH connection is initiated (the `SSH` hook executed on the local
    /// machine) and when the remote shell sends the `InitShell` DCS.
    pending_legacy_ssh_session: Option<SSHValue>,

    /// This variable allows us to differentiate between warp-initiated and user-initiated invocations of
    /// control mode. Whenever we attempt to warpify an ssh session, we track the context of when warp initiated
    /// control mode, indicating that we expect the shell to enter control mode. We reset to None whenever
    /// the active block finishes. If we enter control mode and option is None, then we know it's user-initiated.
    pending_warp_initiated_control_mode: Option<WarpInitiatedTmuxControlMode>,

    tmux_control_mode_context: Option<TmuxControlModeContext>,

    /// The path of the shell binary used for the pending shell session, if any. This is
    /// temporarily stored between the spawning of the child shell process and bootstrap completion.
    /// After bootstrapping, this is set to None.
    pending_shell_launch_data: Option<ShellLaunchData>,

    /// The resolved shell launch data for the login shell.
    /// Unlike `pending_shell_launch_data`, this persists after bootstrap
    /// so that subsystems (e.g. plugin auto-install) can read the actual
    /// shell rather than the user preference.
    active_shell_launch_data: Option<ShellLaunchData>,

    /// Partially populated `SessionInfo` from the `InitShell` DCS payload.
    ///
    /// This is used to construct a final, populated `SessionInfo` after the session is
    /// bootstrapped.
    pending_session_info: Option<SessionInfo>,

    /// If true, the terminal was bootstrapping but received a ^D from the user. We cannot stop the
    /// shell from sending us bootstrapping messages, but we can ignore them. This value is always
    /// cleared at the next precmd, because it is only relevant for the block where it was set.
    ignore_bootstrapping_messages: bool,

    // This session's startup directory path. If None, the startup directory is treated as default
    // (the user's home directory).
    session_startup_path: Option<PathBuf>,

    /// Whether or not the printable bytes read from the PTY (e.g. via the `Handler::input()`
    /// method) should be considered in-band command output.
    ///
    /// This is updated when `Handler::start_in_band_command_output()` and
    /// `Handler::end_in_band_command_output()` are called.
    is_receiving_in_band_command_output: IsReceivingInBandCommandOutput,

    #[cfg(windows)]
    /// On Windows, in-band generators send reset grid OSCs when they finish, clearing out
    /// any leftover state in conpty. When we receive these, we don't want to mistakenly route
    /// them to the active grid.
    ignore_reset_grid_after_in_band_generator: bool,

    is_receiving_completions_output: IsReceivingCompletionsOutput,

    is_receiving_iterm_image_data: IsReceivingITermImageData,

    is_receiving_kitty_image_data: IsReceivingKittyActionData,

    /// Whether or not the terminal is receiving a shell hook via key-value pairs. This is
    /// currently only used in Git Bash.
    is_receiving_hook: IsReceivingHook,

    /// `Some(true)` if the model received a SourcedRcFile DCS.
    ///
    /// The SourcedRcFile DCS is used to trigger subshell bootstrapping.
    ///
    /// This is only `Some()` in between receiving the SourcedRcFile DCS and the next InitShell
    /// DCS, where it is consumed into `self.pending_session_info`.
    did_receive_rc_file_dcs: Option<bool>,
    env_var_collection_name: Option<String>,

    /// Whether or not the underlying shell process has terminated.
    handled_exit: bool,

    /// The shell type of the login shell for this session.
    shell_launch_state: ShellLaunchState,

    /// Whether or not to respect secrets that are obfuscated, respecting the Safe Mode/Secret Redaction setting.
    obfuscate_secrets: ObfuscateSecrets,

    shared_session_status: SharedSessionStatus,

    /// The source type of the shared session (if this is a shared session).
    /// If it is not a shared session, this will be `None`.
    shared_session_source_type: Option<SessionSourceType>,

    /// Whether this terminal model was created as a cloud mode dummy session
    /// (no local shell process, deferred shared-session viewer backing).
    is_dummy_cloud_mode_session: bool,

    /// If Some, this terminal is displaying a read-only conversation transcript.
    /// Tracks both the loading state and the type of conversation being viewed.
    conversation_transcript_viewer_status: Option<ConversationTranscriptViewerStatus>,

    /// A sender for terminal-state updates that must be ordered against each other.
    /// This goes through the [`TerminalModel`] because the [`TerminalModel`] is exposed as
    /// a synchronized data structure (i.e. [`FairMutex<TerminalModel>`]) and thus multiple
    /// `send`s via the [`TerminalModel`] will be synchronized.
    ///
    /// This field is only [`Some`] if this session is shared.
    /// TODO: consider combining this with `shared_session_status` because
    /// the state can technically diverge.
    ordered_terminal_events_for_shared_session_tx: Option<Sender<OrderedTerminalEventType>>,

    /// A sender for write to pty events for a shared session viewer.
    ///
    /// This field is only [`Some`] if this session is shared.
    write_to_pty_events_for_shared_session_tx: Option<Sender<Vec<u8>>>,

    /// Whether this viewer is currently receiving historical agent conversation replay.
    /// Used to suppress live-conversation-specific actions (e.g. tombstone insertion)
    /// until the replay is complete.
    is_receiving_agent_conversation_replay: bool,

    tmux_background_outputs: HashMap<u32, Vec<u8>>,

    /// When some, the TerminalModel emits the event [Event::DetectedEndOfSshLogin]. This
    /// event is emitted either as the initial check or the confirmation check.
    notify_on_end_of_ssh_login: Option<SshLogin>,

    pub image_id_to_metadata: HashMap<u32, StoredImageMetadata>,

    /// Next ID to use for images where the ID is not explicitly specified
    /// by the Kitty protocol
    pub next_kitty_image_id: u32,
}

#[derive(Clone, Debug)]
pub struct SshLogin {
    /// The block id of the ssh session we're tracking
    block_id: BlockId,
    notification_state: SshLoginNotificationState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SshLoginNotificationState {
    /// Read all pty output to see if ssh login is complete.
    Monitoring,
    /// Read all pty output but don't send another initial notification.
    SentInitialNotification,
    /// The final notification has been sent. No need to monitor anymore.
    Completed,
}

/// This struct contains metadata for a subshell, and its presence in the SessionInfo indicates
/// that a session is in a bootstrapped subshell.
#[derive(Clone, Debug)]
pub struct SubshellInitializationInfo {
    /// The command that originally created the process for this subshell
    pub spawning_command: String,

    /// `true` if the subshell bootstrap was triggered by an RC file snippet that emits the
    /// `SourcedRcFileForWarp` DCS.
    pub was_triggered_by_rc_file_snippet: bool,

    /// The subshell was triggered from an EVC invocation
    pub env_var_collection_name: Option<String>,

    /// If the subshell is from an SSH command, store the connection details.
    /// Note that these details come from parsing the ssh command, not from retrieving
    /// any actual state on the remote host.
    pub ssh_connection_info: Option<InteractiveSshCommand>,
}

/// Since a SelectedBlockRange is a range of blocks, it is possible that
/// a debug/hidden block is within this range (even though it shouldn't be selected).
/// Care must be taken to ensure that hidden blocks are not considered "selected"
/// when performing actions like "copy".
#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedBlockRange {
    pivot: BlockIndex,
    tail: BlockIndex,
}

impl SelectedBlockRange {
    pub fn start(&self) -> BlockIndex {
        min(self.pivot, self.tail)
    }

    pub fn end(&self) -> BlockIndex {
        max(self.pivot, self.tail)
    }

    /// Returns the range of the selected index, optionally sorted.
    pub fn range(
        &self,
        sort_direction: Option<BlockSortDirection>,
    ) -> impl Iterator<Item = BlockIndex> {
        let range = self.start().0..=self.end().0;
        // Note we need the heap allocation through the box because we
        // can't return .rev() and not .rev() iterators without it.
        let boxed: Box<dyn Iterator<Item = BlockIndex>> =
            if matches!(sort_direction, Some(BlockSortDirection::MostRecentFirst)) {
                Box::new(range.rev().map(BlockIndex::from))
            } else {
                Box::new(range.map(BlockIndex::from))
            };
        boxed
    }

    fn contains(&self, block_index: BlockIndex) -> bool {
        self.range(None).any(|index| index == block_index)
    }

    fn reversed(&self) -> bool {
        self.pivot > self.tail
    }

    pub fn intersection(
        &self,
        other: &RangeInclusive<BlockIndex>,
    ) -> impl Iterator<Item = BlockIndex> {
        (max(self.start().0, other.start().0)..=min(self.end().0, other.end().0))
            .map(BlockIndex::from)
    }

    pub fn pivot(&self) -> BlockIndex {
        self.pivot
    }
}

#[derive(Copy, Clone, Serialize)]
pub enum BlockSelectionCardinality {
    None,
    One,
    Many,
}

impl BlockSelectionCardinality {
    pub fn as_keymap_context_value(&self) -> &'static str {
        match self {
            BlockSelectionCardinality::None => "None",
            BlockSelectionCardinality::One => "One",
            BlockSelectionCardinality::Many => "Many",
        }
    }
}

/// Wrapper struct around a vector of ranges to provide easier API to use
/// in the context of block selections.
/// Note that although hidden blocks may be within a SelectedBlockRange, the
/// pivot/tail of a SelectedBlockRange will never be a hidden index.
#[derive(Default, Clone, Eq, PartialEq, Debug)]
pub struct SelectedBlocks {
    ranges: Vec<SelectedBlockRange>,
}

/// The direction to sort blocks in.
#[derive(Clone, Copy, Debug)]
pub enum BlockSortDirection {
    // Most recently executed blocks are sorted first.  Matches the inverted
    // block list.
    MostRecentFirst,

    // Most recently executed blocks are sorted last.  Matches the "normal"
    // block list.
    MostRecentLast,
}

impl SelectedBlocks {
    /// Selects all blocks in range [last().pivot, new_tail]
    pub fn range_select(&mut self, new_tail: BlockIndex) {
        if let Some(last_selected_range) = self.ranges.last() {
            // replacing the entire vector ensures we collapse into a single selection range
            self.ranges = vec![SelectedBlockRange {
                pivot: last_selected_range.pivot,
                tail: new_tail,
            }];
        }
    }

    /// Toggles an arbitrary block with index=block_index.
    /// next_index is the first index larger than block_index that is not hidden.
    /// similarly, prior_index is the first index smaller than block_index that is not hidden.
    pub fn toggle(
        &mut self,
        block_index: BlockIndex,
        next_index: Option<BlockIndex>,
        prior_index: Option<BlockIndex>,
    ) {
        let next_index = next_index.unwrap_or(block_index);
        let prior_index = prior_index.unwrap_or(block_index);

        let mut position_of_next_index_in_ranges: Option<usize> = None;
        let mut position_of_prior_index_in_ranges: Option<usize> = None;

        // Determine if block_index is already selected,
        // and if so, which range is it part of
        let mut range_idx: Option<BlockIndex> = None;
        for (idx, block_range) in self.ranges.iter().enumerate() {
            if block_range.contains(block_index) {
                range_idx = Some(idx.into());
            }
            if block_range.contains(next_index) {
                position_of_next_index_in_ranges = Some(idx);
            }
            if block_range.contains(prior_index) {
                position_of_prior_index_in_ranges = Some(idx);
            }
        }

        // If block_index is already selected, then toggling it should deselect it.
        // There are a few cases to consider in the deselection case:
        // 1. The range that block_index belongs to is entirely the block_index.
        //    In this case, we should completely remove the selection range.
        // 2. The block_index is either the pivot or tail of the range it belongs to.
        //    In this case, we should adjust the pivot or tail, respectively,
        //    based on the next_index and prior_index._
        // 3. The block_index is somewhere in the middle of the range it belongs to.
        //    In this case, we should split up the range at the block_index into
        //    two ranges, using the next_index and prior_index accordingly.
        if let Some(idx) = range_idx {
            let idx = idx.0;
            let block_range = self.ranges[idx];

            let (new_pivot, new_tail) = if block_range.reversed() {
                (prior_index, next_index)
            } else {
                (next_index, prior_index)
            };

            // if block_index is the pivot and the tail of the range it is part of
            // then this is case 1: block_range is simply a singleton
            // and should be completely removed since we are toggling it off
            if block_index == block_range.pivot && block_index == block_range.tail {
                self.ranges.remove(idx);
            } else if block_index == block_range.pivot {
                // if block_index is the pivot of the range it is part of (and not also the tail),
                // this is case 2 (pivot): we just need to select a new pivot
                self.ranges[idx].pivot = new_pivot;
            } else if block_index == block_range.tail {
                // if block_index is the tail of the range it is part of (and not also the pivot),
                // this is case 2 (tail): we just need to select a new tail
                self.ranges[idx].tail = new_tail;
            } else {
                // otherwise, this is case 3: if block_index is somewhere in the middle of the range it is part of,
                // then we need to break up this range into two ranges (around block_index).
                // the existing range will end at the new_tail and we'll add a new range
                // right before the existing one with the new_pivot and the tail being the old tail
                self.ranges[idx].tail = new_tail;
                self.ranges.insert(
                    idx,
                    SelectedBlockRange {
                        pivot: new_pivot,
                        tail: block_range.tail,
                    },
                );
            }
        } else {
            // Otherwise, we are in the toggling ON case.

            // We should be able to merge selections as long as at least one of
            // next_index and prior_index is already selected (and not the same selection)
            let can_merge = position_of_next_index_in_ranges != position_of_prior_index_in_ranges;

            // If block_index isn't already selected, then it's a new selection.
            // First, try to merge this new selection with existing selections.
            let new_selection = if can_merge {
                // If this new selection sits directly in between two other
                // selection ranges, merge them all into one selection range.
                if let (
                    Some(position_of_next_index_in_ranges),
                    Some(position_of_prior_index_in_ranges),
                ) = (
                    position_of_next_index_in_ranges,
                    position_of_prior_index_in_ranges,
                ) {
                    // important: remove in reverse order to prevent shifting
                    // the index of the counterpart!
                    let (prior_range_removed, next_range_removed) =
                        if position_of_next_index_in_ranges > position_of_prior_index_in_ranges {
                            let next_range = self.ranges.remove(position_of_next_index_in_ranges);
                            (
                                self.ranges.remove(position_of_prior_index_in_ranges),
                                next_range,
                            )
                        } else {
                            (
                                self.ranges.remove(position_of_prior_index_in_ranges),
                                self.ranges.remove(position_of_next_index_in_ranges),
                            )
                        };
                    Some((prior_range_removed.start(), next_range_removed.end()))
                } else if let Some(position_of_next_index_in_ranges) =
                    position_of_next_index_in_ranges
                {
                    // Otherwise, if this new selection is right before another
                    // selection range, then merge it together.
                    let next_range_removed = self.ranges.remove(position_of_next_index_in_ranges);
                    if next_range_removed.tail == next_index {
                        Some((next_range_removed.pivot, block_index))
                    } else {
                        Some((block_index, next_range_removed.tail))
                    }
                } else if let Some(position_of_prior_index_in_ranges) =
                    position_of_prior_index_in_ranges
                {
                    // Otherwise, this new selection must be right after another
                    // selection range, so merge it together as well.
                    let prior_range_removed = self.ranges.remove(position_of_prior_index_in_ranges);
                    if prior_range_removed.tail == prior_index {
                        Some((prior_range_removed.pivot, block_index))
                    } else {
                        Some((block_index, prior_range_removed.tail))
                    }
                } else {
                    // At least one of next_index, prior_index should be set if
                    // we can merge selections so this should be unreachable
                    log::warn!(
                        "Expected to merge block selections but 'position_of_next/prior_index_in_ranges' were both None."
                    );
                    None
                }
            } else {
                // If we can't merge with an existing selection, then this is disjoint.
                Some((block_index, block_index))
            };
            if let Some((new_pivot, new_tail)) = new_selection {
                self.ranges.push(SelectedBlockRange {
                    pivot: new_pivot,
                    tail: new_tail,
                });
            }
        }
    }

    /// Remove all block selections.
    pub fn reset(&mut self) {
        self.ranges = vec![];
    }

    /// Reset all block selections to unselected except block_index
    pub fn reset_to_single(&mut self, block_index: BlockIndex) {
        let singleton = SelectedBlockRange {
            pivot: block_index,
            tail: block_index,
        };
        self.ranges = vec![singleton];
    }

    /// Reset all block selections to a sequence of single block `SelectedBlockRange`s of the given
    /// `block_indices`. Does not merge the `SelectedBlockRange`s.`
    pub fn reset_to_block_indices(&mut self, block_indices: impl IntoIterator<Item = BlockIndex>) {
        self.ranges = block_indices
            .into_iter()
            .map(|block_index| SelectedBlockRange {
                pivot: block_index,
                tail: block_index,
            })
            .collect::<Vec<_>>();
    }

    /// Returns true iff block_index is selected
    pub fn is_selected(&self, block_index: BlockIndex) -> bool {
        self.ranges
            .iter()
            .any(|block_range| block_range.contains(block_index))
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    // A "singleton" is a single selected block. That is, there is only
    // one range R and R.pivot == R.tail
    pub fn is_singleton(&self) -> bool {
        match &self.ranges[..] {
            [single] => single.pivot == single.tail,
            _ => false,
        }
    }

    /// The `tail` of all selected blocks is defined as the tail of the most
    /// recent selected block range
    pub fn tail(&self) -> Option<BlockIndex> {
        self.ranges
            .last()
            .map(|most_recent_range| most_recent_range.tail)
    }

    pub fn ranges(&self) -> &[SelectedBlockRange] {
        &self.ranges[..]
    }

    /// Returns an iterator whose items are all the selected block indices. Does not guarantee any
    /// order.
    pub fn block_indices(&self) -> impl Iterator<Item = BlockIndex> + '_ {
        self.ranges
            .iter()
            .flat_map(|selected_block_range| selected_block_range.range(None))
    }

    /// Used to produce a consistent list of selected indices to take actions on
    pub fn sorted_ranges(&self, sort_direction: BlockSortDirection) -> Vec<SelectedBlockRange> {
        let mut sorted_ranges = self.ranges.clone();
        sorted_ranges.sort_by_key(|a| a.start());
        if matches!(sort_direction, BlockSortDirection::MostRecentFirst) {
            sorted_ranges.reverse();
        }
        sorted_ranges
    }

    pub fn cardinality(&self) -> BlockSelectionCardinality {
        if self.is_empty() {
            BlockSelectionCardinality::None
        } else if self.is_singleton() {
            BlockSelectionCardinality::One
        } else {
            BlockSelectionCardinality::Many
        }
    }

    pub fn to_block_ids<'a>(
        &'a self,
        block_list: &'a BlockList,
    ) -> impl Iterator<Item = &'a BlockId> + 'a {
        self.ranges.iter().flat_map(|range| {
            range
                .range(None)
                .filter_map(|idx| block_list.block_at(idx).map(|b| b.id()))
        })
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum TerminalInputState {
    /// Alt-screen on which programs like vim run is visible.
    AltScreen,
    /// Warp Input View is visible.
    InputEditor,
    /// Block-list is visible but input will go to the running command.
    LongRunningCommand,
    /// Terminal hasn't finished bootstrapping and can't receive any input.
    NotBootstrapped,
}

impl TerminalModel {
    pub fn is_input_dirty(&self) -> bool {
        self.is_input_dirty
    }

    pub fn set_is_input_dirty(&mut self, value: bool) {
        self.is_input_dirty = value;
    }
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    /// Returns a bootstrapped `TerminalModel` with no restored blocks
    /// and just one default block to avoid any side effects of being
    /// in the middle of the bootstrap sequence.
    pub fn new_for_test(
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        should_show_bootstrap_block: bool,
        restored_blocks: Option<&[SerializedBlockListItem]>,
        honor_ps1: bool,
        is_inverted: bool,
        session_startup_path: Option<PathBuf>,
    ) -> Self {
        use super::session::get_local_hostname;

        let mut terminal_model = Self::new(
            restored_blocks,
            sizes,
            colors,
            event_proxy,
            background_executor,
            should_show_bootstrap_block,
            false,
            false,
            honor_ps1,
            is_inverted,
            ObfuscateSecrets::No,
            false,
            session_startup_path,
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
        );

        // We need to set the hostname to the local hostname to ensure that we
        // treat the session as a local one, not a remote one.  (See the
        // implementation of `SessionInfo::determine_session_type()` for more
        // details.)
        let hostname = get_local_hostname().unwrap_or_else(|_| "localhost".to_string());
        terminal_model.init_shell(InitShellValue {
            session_id: 123.into(),
            shell: "zsh".to_owned(),
            hostname,
            ..Default::default()
        });
        terminal_model.bootstrapped(BootstrappedValue {
            shell: "zsh".to_string(),
            ..Default::default()
        });
        terminal_model.command_finished(Default::default());
        terminal_model.precmd(Default::default());
        terminal_model
    }

    #[allow(clippy::too_many_arguments)]
    fn new_internal(
        restored_blocks: Option<&[SerializedBlockListItem]>,
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        should_show_bootstrap_block: bool,
        should_show_in_band_command_blocks: bool,
        should_show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
        is_ai_ugc_telemetry_enabled: bool,
        session_startup_path: Option<PathBuf>,
        shell_state: ShellLaunchState,
        shared_session_status: SharedSessionStatus,
        is_dummy_cloud_mode_session: bool,
    ) -> Self {
        let alt_screen = AltScreen::new(
            sizes.size,
            0, /* max_scroll_limit */
            event_proxy.clone(),
            obfuscate_secrets,
        );
        let block_list = BlockList::new(
            restored_blocks,
            sizes,
            event_proxy.clone(),
            background_executor,
            should_show_bootstrap_block,
            should_show_in_band_command_blocks,
            should_show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
            is_ai_ugc_telemetry_enabled,
        );

        Self {
            alt_screen,
            is_input_dirty: false,
            block_list,
            blocklist_has_been_cleared: false,
            alt_screen_active: false,
            title_stack: Vec::new(),
            title: None,
            custom_title: None,
            colors,
            override_colors: color::OverrideList::empty(),
            event_proxy,
            pending_legacy_ssh_session: None,
            pending_shell_launch_data: None,
            active_shell_launch_data: None,
            pending_session_info: None,
            ignore_bootstrapping_messages: false,
            session_startup_path,
            is_receiving_in_band_command_output: IsReceivingInBandCommandOutput::No,
            #[cfg(windows)]
            ignore_reset_grid_after_in_band_generator: false,
            is_receiving_completions_output: IsReceivingCompletionsOutput::No,
            is_receiving_iterm_image_data: IsReceivingITermImageData::No,
            is_receiving_kitty_image_data: IsReceivingKittyActionData::No,
            did_receive_rc_file_dcs: None,
            handled_exit: false,
            env_var_collection_name: None,
            shell_launch_state: shell_state,
            obfuscate_secrets,
            shared_session_status,
            shared_session_source_type: None,
            is_dummy_cloud_mode_session,
            conversation_transcript_viewer_status: None,
            ordered_terminal_events_for_shared_session_tx: None,
            write_to_pty_events_for_shared_session_tx: None,
            is_receiving_agent_conversation_replay: false,
            tmux_background_outputs: HashMap::new(),
            tmux_control_mode_context: None,
            pending_warp_initiated_control_mode: None,
            notify_on_end_of_ssh_login: None,
            is_receiving_hook: IsReceivingHook::No,
            image_id_to_metadata: HashMap::new(),
            // Start mid-way through the u32 range to avoid collisions
            next_kitty_image_id: 2147483647,
        }
    }

    /// Creates a terminal model for a local terminal session.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        restored_blocks: Option<&[SerializedBlockListItem]>,
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        should_show_bootstrap_block: bool,
        should_show_in_band_command_blocks: bool,
        should_show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
        is_ai_ugc_telemetry_enabled: bool,
        session_startup_path: Option<PathBuf>,
        shell_state: ShellLaunchState,
    ) -> Self {
        Self::new_internal(
            restored_blocks,
            sizes,
            colors,
            event_proxy,
            background_executor,
            should_show_bootstrap_block,
            should_show_in_band_command_blocks,
            should_show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
            is_ai_ugc_telemetry_enabled,
            session_startup_path,
            shell_state,
            SharedSessionStatus::NotShared,
            false,
        )
    }

    /// Creates a terminal model for a cloud mode pane before it has connected to a shared session.
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_cloud_mode_shared_session_viewer(
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
    ) -> Self {
        Self::new_internal(
            None,
            sizes,
            colors,
            event_proxy,
            background_executor,
            false,
            false,
            show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
            false,
            None,
            // TODO: use the same shell type as the sharer
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
            SharedSessionStatus::ViewPending,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_for_shared_session_viewer_internal(
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
    ) -> Self {
        Self::new_internal(
            None,
            sizes,
            colors,
            event_proxy,
            background_executor,
            false,
            false,
            show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
            false,
            None,
            // TODO: use the same shell type as the sharer
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
            SharedSessionStatus::ViewPending,
            false,
        )
    }

    /// Creates a terminal model for a terminal session that is being viewed.
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_shared_session_viewer(
        sizes: BlockSize,
        colors: color::List,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
    ) -> Self {
        Self::new_for_shared_session_viewer_internal(
            sizes,
            colors,
            event_proxy,
            background_executor,
            show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
        )
    }

    pub fn set_ordered_terminal_events_for_shared_session_tx(
        &mut self,
        tx: Sender<OrderedTerminalEventType>,
    ) {
        self.ordered_terminal_events_for_shared_session_tx = Some(tx);
    }

    pub fn clear_ordered_terminal_events_for_shared_session_tx(&mut self) {
        self.ordered_terminal_events_for_shared_session_tx = None;
    }

    fn ai_metadata_to_protocol(metadata: &AgentInteractionMetadata) -> AICommandMetadata {
        AICommandMetadata {
            tool_call_id: metadata
                .requested_command_action_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            // Any command with a long-running control state is considered agent-monitored.
            is_agent_monitored: metadata.long_running_control_state().is_some(),
        }
    }

    pub fn set_write_to_pty_events_for_shared_session_tx(&mut self, tx: Sender<Vec<u8>>) {
        self.write_to_pty_events_for_shared_session_tx = Some(tx);
    }

    pub fn send_write_to_pty_events_for_shared_session(&mut self, bytes: Vec<u8>) {
        if !FeatureFlag::SharedSessionWriteToLongRunningCommands.is_enabled()
            || !self.shared_session_status().is_executor()
        {
            return;
        }

        if let Some(tx) = &self.write_to_pty_events_for_shared_session_tx {
            if let Err(e) = tx.try_send(bytes) {
                log::warn!("Failed to send write to pty events: {e}");
            }
        }
    }

    pub fn clear_write_to_pty_events_for_shared_session_tx(&mut self) {
        self.write_to_pty_events_for_shared_session_tx = None;
    }

    /// Sends an Agent ResponseEvent to viewers if this session is shared.
    /// The participant_id should be the ID of the participant who initiated the query.
    /// The forked_from_conversation_token is used for forked conversations to help viewers
    /// link the new server-assigned token to an existing conversation from historical replay.
    pub fn send_agent_response_for_shared_session(
        &mut self,
        response: &warp_multi_agent_api::ResponseEvent,
        response_initiator: Option<ParticipantId>,
        forked_from_conversation_token: Option<String>,
    ) {
        // We should always have a response initiator for shared sessions,
        // but if we don't we should still send the response event to the viewers
        // (as opposed to completely failing and skipping the send).
        if response_initiator.is_none() {
            report_error!(anyhow::anyhow!(
                "No response initiator tracked for agent response event."
            ));
        }

        if self.shared_session_status().is_sharer() {
            if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
                let encoded = encode_agent_response_event(response);
                if let Err(e) = tx.try_send(OrderedTerminalEventType::AgentResponseEvent {
                    response_initiator,
                    response_event: encoded,
                    forked_from_conversation_token,
                }) {
                    log::warn!("Failed to send OrderedTerminalEventType::AgentResponseEvent: {e}");
                }
            }
        } else {
            log::debug!("Not sharing this session; ignoring agent response event");
        }
    }

    pub fn send_agent_conversation_replay_started_for_shared_session(&mut self) {
        if self.shared_session_status().is_sharer() {
            if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
                if let Err(e) =
                    tx.try_send(OrderedTerminalEventType::AgentConversationReplayStarted)
                {
                    log::warn!(
                        "Failed to send OrderedTerminalEventType::AgentConversationReplayStarted: {e}"
                    );
                }
            }
        }
    }

    pub fn send_agent_conversation_replay_ended_for_shared_session(&mut self) {
        if self.shared_session_status().is_sharer() {
            if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
                if let Err(e) = tx.try_send(OrderedTerminalEventType::AgentConversationReplayEnded)
                {
                    log::warn!(
                        "Failed to send OrderedTerminalEventType::AgentConversationReplayEnded: {e}"
                    );
                }
            }
        }
    }

    /// Whether the session sharing server is currently replaying
    /// conversation events (for conversation reconstruction).
    pub fn is_receiving_agent_conversation_replay(&self) -> bool {
        self.is_receiving_agent_conversation_replay
    }

    pub fn set_is_receiving_agent_conversation_replay(&mut self, value: bool) {
        self.is_receiving_agent_conversation_replay = value;
    }

    pub fn set_shared_session_source_type(
        &mut self,
        set_shared_session_source_type: SessionSourceType,
    ) {
        self.shared_session_source_type = Some(set_shared_session_source_type);
    }

    pub fn shared_session_source_type(&self) -> Option<SessionSourceType> {
        self.shared_session_source_type.clone()
    }

    pub fn is_dummy_cloud_mode_session(&self) -> bool {
        self.is_dummy_cloud_mode_session
    }

    #[cfg(test)]
    pub fn set_is_dummy_cloud_mode_session(&mut self, value: bool) {
        self.is_dummy_cloud_mode_session = value;
    }

    pub fn is_shared_ambient_agent_session(&self) -> bool {
        matches!(
            self.shared_session_source_type,
            Some(SessionSourceType::AmbientAgent { .. })
        )
    }

    pub fn ambient_agent_task_id(&self) -> Option<AmbientAgentTaskId> {
        // Check if we're viewing an ambient agent conversation transcript
        if let Some(ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id)) =
            &self.conversation_transcript_viewer_status
        {
            return Some(*task_id);
        }

        // Otherwise, check if we're in a shared ambient agent session
        if let Some(SessionSourceType::AmbientAgent { task_id }) = &self.shared_session_source_type
        {
            task_id.as_deref().and_then(|s| s.parse().ok())
        } else {
            None
        }
    }

    /// Loads the provided scrollback into the model.
    // TODO: we should be doing this in the constructor of the
    // terminal model for the viewers so that we're guaranteed that
    // loading scrollback is the first thing that we do.
    pub fn load_shared_session_scrollback(&mut self, scrollback: &[SerializedBlock]) {
        debug_assert!(self.shared_session_status().is_viewer());

        self.block_list_mut()
            .load_shared_session_scrollback(scrollback);

        // The scrollback contains the prompt for the active block, and the terminal view needs to be notified to render it.
        self.event_proxy.send_wakeup_event();
    }

    pub fn append_followup_shared_session_scrollback(&mut self, scrollback: &[SerializedBlock]) {
        debug_assert!(self.shared_session_status().is_viewer());

        self.block_list_mut()
            .append_followup_shared_session_scrollback(scrollback);

        self.event_proxy.send_wakeup_event();
    }

    pub fn obfuscate_secrets(&self) -> ObfuscateSecrets {
        self.obfuscate_secrets
    }

    pub fn set_honor_ps1(&mut self, honor_ps1: bool) {
        self.block_list_mut().set_honor_ps1(honor_ps1);
    }

    pub fn update_max_grid_size(&mut self, new_size: usize) {
        self.block_list_mut().update_max_grid_size(new_size);
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn are_any_events_pending(&self) -> bool {
        self.event_proxy.are_any_events_pending()
    }

    pub fn ignore_bootstrapping_messages(&mut self) {
        self.ignore_bootstrapping_messages = true;
    }

    pub fn exit(&mut self, reason: ExitReason) {
        // If we've already responded to the shell/event loop exiting, there's
        // nothing more to do.
        if self.handled_exit {
            return;
        }

        self.handled_exit = true;
        // Forcibly exit the alt screen so that we can show the user the
        // banner informing them that the shell process exited.
        self.exit_alt_screen(true);
        // Mark the active block as finished, as there is no way it could
        // possibly receive more output from the shell.
        self.block_list.active_block_mut().finish(0);
        self.event_proxy.send_terminal_event(Event::Exit { reason });
    }

    pub fn is_read_only(&self) -> bool {
        self.handled_exit
            || self.is_conversation_transcript_viewer()
            || self.shared_session_status().is_finished_viewer()
    }

    pub fn is_conversation_transcript_viewer(&self) -> bool {
        self.conversation_transcript_viewer_status.is_some()
    }

    pub fn is_loading_conversation_transcript(&self) -> bool {
        matches!(
            self.conversation_transcript_viewer_status,
            Some(ConversationTranscriptViewerStatus::Loading)
        )
    }

    pub fn conversation_transcript_viewer_status(
        &self,
    ) -> Option<&ConversationTranscriptViewerStatus> {
        self.conversation_transcript_viewer_status.as_ref()
    }

    pub fn set_conversation_transcript_viewer_status(
        &mut self,
        status: Option<ConversationTranscriptViewerStatus>,
    ) {
        self.conversation_transcript_viewer_status = status;
    }

    pub fn colors(&self) -> color::List {
        self.colors
    }

    pub fn override_colors(&self) -> color::OverrideList {
        self.override_colors
    }

    /// Obfuscates the secret identified by the given secret handle, returning whether
    /// it was successful.
    pub fn obfuscate_secret(&mut self, secret: &WithinModel<SecretHandle>) -> anyhow::Result<()> {
        secret.update_grid(self, |grid_handler, secret| {
            grid_handler.obfuscate_secret(*secret)
        })
    }

    /// Unobfuscates the secret identified by the given secret handle, returning whether
    /// it was successful.
    pub fn unobfuscate_secret(&mut self, secret: &WithinModel<SecretHandle>) -> anyhow::Result<()> {
        secret.update_grid(self, |grid_handler, secret| {
            grid_handler.unobfuscate_secret(*secret)
        })
    }

    /// Returns a tuple of [`Secret`] and [`SecretHandle`] at the given point or `None` if none is identified or Safe Mode/Secret Redaction is disabled.
    pub fn secret_at_point(&self, point: &WithinModel<Point>) -> Option<SecretAndHandle<'_>> {
        if matches!(self.obfuscate_secrets, ObfuscateSecrets::No) {
            return None;
        }

        point
            .read_from_grid(self, |grid_handler, point| {
                Ok(grid_handler.secret_at_displayed_point(*point))
            })
            .ok()
            .flatten()
    }

    /// Returns the [`Secret`] for the given [`SecretHandle`] or `None` if none is identified.
    pub fn secret_from_handle(&self, handle: &WithinModel<SecretHandle>) -> Option<&Secret> {
        handle
            .read_from_grid(self, |grid_handler, handle| {
                Ok(grid_handler.secret_by_handle(*handle))
            })
            .ok()
            .flatten()
    }

    pub fn terminal_input_state(&self) -> TerminalInputState {
        if !self.block_list().is_bootstrapped() {
            TerminalInputState::NotBootstrapped
        } else if self.is_alt_screen_active() {
            TerminalInputState::AltScreen
        } else if self
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            TerminalInputState::LongRunningCommand
        } else {
            TerminalInputState::InputEditor
        }
    }

    pub fn is_active_block_bootstrapped(&self) -> bool {
        self.block_list.active_block().is_bootstrapped()
    }

    pub fn active_block_metadata(&self) -> BlockMetadata {
        self.block_list.active_block().metadata()
    }

    pub fn active_block_id(&self) -> &BlockId {
        self.block_list.active_block_id()
    }

    pub fn has_pending_ssh_session(&self) -> bool {
        self.pending_legacy_ssh_session.is_some()
    }

    pub fn pending_shell_type(&self) -> Option<ShellType> {
        self.pending_session_info
            .as_ref()
            .map(|session_info| session_info.shell.shell_type())
    }

    /// Returns the session ID of the pending (not yet bootstrapped) session, if any.
    pub fn pending_session_id(&self) -> Option<SessionId> {
        self.pending_session_info
            .as_ref()
            .map(|session_info| session_info.session_id)
    }

    pub fn is_pending_wsl(&self) -> bool {
        matches!(
            &self.pending_shell_launch_data,
            Some(ShellLaunchData::WSL { .. })
        )
    }

    pub fn is_pending_msys2(&self) -> bool {
        matches!(
            &self.pending_shell_launch_data,
            Some(ShellLaunchData::MSYS2 { .. })
        )
    }

    pub fn shell_launch_state(&self) -> &ShellLaunchState {
        &self.shell_launch_state
    }

    pub fn pending_subshell_session(&self) -> Option<&SubshellInitializationInfo> {
        self.pending_session_info
            .as_ref()
            .and_then(|session_info| session_info.subshell_info.as_ref())
    }

    pub fn block_list(&self) -> &BlockList {
        &self.block_list
    }

    pub fn is_block_list_empty(&self) -> bool {
        self.block_list.is_empty()
    }

    pub fn block_list_mut(&mut self) -> &mut BlockList {
        &mut self.block_list
    }

    pub fn remove_image_id_to_metadata_entry(&mut self, image_id: u32) {
        self.image_id_to_metadata.remove(&image_id);
    }

    /// Starts the active block and resets block-to-block state. For local sessions, this is called
    /// from the input editor when it sends user bytes to the pty (usually the
    /// next command to run, but also ctrl-d). Once we've written to the pty on
    /// the user's behalf, we consider the active block started.
    pub fn start_command_execution(&mut self) {
        self.block_list.start_active_block();
    }

    pub fn start_command_execution_from_env_var_collection(
        &mut self,
        env_var_metadata: BlocklistEnvVarMetadata,
    ) {
        self.start_command_execution();
        self.block_list
            .active_block_mut()
            .set_env_var_metadata(env_var_metadata);
    }

    /// Starts the execution for a command in a shared session (sharer or viewer).
    pub fn start_command_execution_for_shared_session(
        &mut self,
        participant_id: ParticipantId,
        agent_metadata: Option<AgentInteractionMetadata>,
    ) {
        self.start_command_execution();

        // If this command has AI metadata, attach it to the active block.
        if let Some(ai_metadata) = &agent_metadata {
            self.block_list
                .active_block_mut()
                .set_agent_interaction_mode(ai_metadata.clone());
        }

        // TODO (suraj): add participant ID to active block metadata.

        // If this is a sharer, send an event to indicate the start of the command execution
        // along with the identity of the participant that ran the command.
        if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
            if let Err(e) = tx.try_send(OrderedTerminalEventType::CommandExecutionStarted {
                participant_id,
                ai_metadata: agent_metadata.as_ref().map(Self::ai_metadata_to_protocol),
            }) {
                log::warn!("Failed to send OrderedTerminalEventType::CommandExecutionStarted: {e}");
            }
        }
    }

    /// Starts the command execution (per `Self::start_command_execution`) and additionally sets
    /// the given `ai_metadata` on the active block.
    pub fn start_command_execution_with_ai_metadata(
        &mut self,
        agent_metadata: AgentInteractionMetadata,
    ) {
        self.start_command_execution();
        self.block_list
            .active_block_mut()
            .set_agent_interaction_mode(agent_metadata);
    }

    // Starts active block as a background block. Used in Alacritty integration tests to
    // work with the output grid directly.
    pub fn start_active_block_as_background_block(&mut self) {
        self.block_list.active_block_mut().start_background(None);
    }

    pub fn is_receiving_in_band_command_output(&self) -> bool {
        matches!(
            self.is_receiving_in_band_command_output,
            IsReceivingInBandCommandOutput::Yes { .. }
        )
    }

    /// This session's startup path. If None, the startup path is the default path (the user's home
    /// directory).
    pub fn session_startup_path(&self) -> Option<PathBuf> {
        self.session_startup_path.clone()
    }

    /// Returns the block from which we should be retrieving prompt-related data.
    pub fn prompt_block(&self) -> Option<&Block> {
        self.block_list()
            .blocks()
            .iter()
            .rev()
            .find(|block| block.has_received_precmd())
    }

    /// Returns the grid containing the user's custom prompt.
    pub fn prompt_grid(&self) -> Option<&BlockGrid> {
        self.prompt_block().map(|block| block.prompt_grid())
    }

    /// Returns **all** selected text across the entire `TerminalView` view hierarchy.
    /// This includes selected text within regular blocks, AI blocks, inline actions, etc.
    pub fn selection_to_string(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
        app: &AppContext,
    ) -> Option<String> {
        if self.alt_screen_active {
            self.alt_screen.selection_to_string(semantic_selection)
        } else {
            self.block_list
                .selection_to_string(semantic_selection, inverted_blocklist, app)
        }
    }

    /// Returns the underlying text string for the given range in the model.
    pub fn string_at_range<T: RangeInModel>(
        &self,
        item: &WithinModel<T>,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
    ) -> String {
        match item {
            WithinModel::AltScreen(inner) => {
                let (start, end) = inner.range().into_inner();
                self.alt_screen
                    .bounds_to_string(start, end, respect_obfuscated_secrets)
            }
            WithinModel::BlockList(inner) => self
                .block_list
                .string_at_range(inner, respect_obfuscated_secrets),
        }
    }

    /// A variant of [`Self::string_at_range`] for when the text is a link that
    /// we want to open. In that case, the existence of zero-width spaces can
    /// case a double-encode of the url when we attempt to open it (see CORE-1573).
    /// Here, we pull the text at the given range, and then trim whitespace
    /// (including zero-width spaces) from the end before returning the url.
    pub fn link_at_range<T: RangeInModel>(
        &self,
        item: &WithinModel<T>,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
    ) -> String {
        let text = self.string_at_range(item, respect_obfuscated_secrets);
        text.trim_matches(['\u{200B}', ' ', '\n', '\r', '\t'])
            .to_owned()
    }

    /// Return all possible file paths containing the grid point ordered from longest to shortest.
    pub fn possible_file_paths_at_point(
        &self,
        point: WithinModel<Point>,
    ) -> impl Iterator<Item = WithinModel<PossiblePath>> {
        match point {
            WithinModel::AltScreen(inner_point) => Either::Left(
                self.alt_screen
                    .possible_file_paths_at_point(inner_point)
                    .map(WithinModel::AltScreen),
            ),
            WithinModel::BlockList(inner_point) => Either::Right(
                self.block_list
                    .possible_file_paths_at_point(inner_point)
                    .map(WithinModel::BlockList),
            ),
        }
    }

    pub fn url_at_point(&self, point: &WithinModel<Point>) -> Option<WithinModel<Link>> {
        match point {
            WithinModel::AltScreen(inner_point) => self
                .alt_screen
                .url_at_point(inner_point)
                .map(WithinModel::AltScreen),
            WithinModel::BlockList(inner_point) => self
                .block_list
                .url_at_point(inner_point)
                .map(WithinModel::BlockList),
        }
    }

    /// Get boundary of the word at the given point.
    pub fn fragment_boundary_at_point(
        &self,
        point: &WithinModel<Point>,
    ) -> WithinModel<FragmentBoundary> {
        match point {
            WithinModel::AltScreen(inner_point) => {
                WithinModel::AltScreen(self.alt_screen.fragment_boundary_at_point(inner_point))
            }
            WithinModel::BlockList(inner_point) => {
                WithinModel::BlockList(self.block_list.fragment_boundary_at_point(inner_point))
            }
        }
    }

    pub fn clear_visible_screen(&mut self) {
        self.block_list.clear_visible_screen();
    }

    pub fn update_colors(&mut self, colors: color::List) {
        self.colors = colors;
    }

    pub fn raw_grid_for_ref_tests(&self) -> &GridHandler {
        if self.alt_screen_active {
            self.alt_screen.grid_handler()
        } else {
            self.block_list.active_block().grid_handler()
        }
    }

    pub fn alt_screen(&self) -> &AltScreen {
        &self.alt_screen
    }

    pub fn alt_screen_mut(&mut self) -> &mut AltScreen {
        &mut self.alt_screen
    }

    pub fn is_alt_screen_active(&self) -> bool {
        self.alt_screen_active
    }

    pub fn set_pending_shell_launch_data(&mut self, shell_launch_data: ShellLaunchData) {
        self.active_shell_launch_data = Some(shell_launch_data.clone());
        self.pending_shell_launch_data = Some(shell_launch_data);
    }

    pub fn active_shell_launch_data(&self) -> Option<&ShellLaunchData> {
        self.active_shell_launch_data.as_ref()
    }

    pub fn set_login_shell_spawned(&mut self, shell_type: ShellType) {
        self.shell_launch_state = self
            .shell_launch_state
            .clone()
            .spawned_with_shell_type(shell_type);
        self.event_proxy
            .send_terminal_event(Event::ShellSpawned(shell_type));
        // Ensure the title is invalidated
        self.set_title(None);
    }

    pub fn get_pending_session_info(&self) -> &Option<SessionInfo> {
        &self.pending_session_info
    }

    pub fn is_term_mode_set(&self, mode: TermMode) -> bool {
        if self.alt_screen_active {
            return self.alt_screen().is_mode_set(mode);
        }
        self.block_list().active_block().is_mode_set(mode)
    }

    pub fn get_shell(&self) -> Option<AvailableShell> {
        match &self.shell_launch_state {
            ShellLaunchState::DeterminingShell {
                available_shell, ..
            } => available_shell.clone(),
            ShellLaunchState::ShellSpawned {
                available_shell, ..
            } => available_shell.clone(),
        }
    }

    pub fn shared_session_status(&self) -> &SharedSessionStatus {
        &self.shared_session_status
    }

    pub fn set_shared_session_status(&mut self, shared_session_status: SharedSessionStatus) {
        self.shared_session_status = shared_session_status;
    }

    /// Returns whether this terminal is viewing a shared session.
    pub fn is_shared_session_viewer(&self) -> bool {
        self.shared_session_status.is_viewer()
    }

    /// Resize terminal to new dimensions.
    /// The block sort direction is needed to update the state of the find dialog.
    pub fn resize(&mut self, size_update: SizeUpdate) {
        // Only resize the model on a pane size change or gap size change.  If it's just
        // the content height changing, we don't need to resize the model, and resizing
        // the model will actually clear the selection state, which we don't want to do.
        if size_update.pane_size_changed()
            || size_update.gap_height_changed()
            || size_update.is_refresh()
            || size_update.rows_or_columns_changed()
        {
            self.alt_screen.resize(&size_update);

            // Don't reflow old blocks for shared session size updates:
            // - Viewers skip reflow when the sharer's size changed
            //   (viewers can still reflow via their own pane/font resizes).
            // - Sharers skip reflow when honoring a viewer's reported size
            //   (the viewer's smaller size is transient and shouldn't reshape history).
            let update_old_blocks = match size_update.update_reason {
                SizeUpdateReason::SharerSizeChanged { .. }
                    if self.shared_session_status().is_viewer() =>
                {
                    false
                }
                SizeUpdateReason::ViewerSizeReported { .. } => false,
                _ => true,
            };
            self.block_list.resize(&size_update, update_old_blocks);
        }

        if size_update.rows_or_columns_changed() {
            let num_rows = size_update.new_size.rows();
            let num_cols = size_update.new_size.columns();
            if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
                if let Err(e) = tx.try_send(OrderedTerminalEventType::Resize {
                    window_size: session_sharing_protocol::common::WindowSize {
                        num_rows,
                        num_cols,
                    },
                }) {
                    log::warn!("Failed to send OrderedTerminalEventType::Resize: {e}");
                }
            }

            if self.tmux_control_mode_context.is_some() {
                self.emit_handler_event(HandlerEvent::RunTmuxCommand(
                    TmuxCommand::UpdateClientSize { num_rows, num_cols },
                ));
            }
        }
    }

    pub fn update_blockheight_items(
        &mut self,
        padding: BlockPadding,
        subshell_separator_height: f32,
    ) {
        self.block_list
            .update_blockheight_items(padding, subshell_separator_height);
    }

    /// Activate the alternate screen. This copies over relevant state from the
    /// block list and clears the alt screen's contents.
    ///
    /// If the alternate screen is already active, this will not re-initialize
    /// it.
    pub(crate) fn enter_alt_screen(&mut self, save_cursor_and_clear_screen: bool) {
        if self.alt_screen_active {
            log::info!("Tried to enter the alternate screen, but it was already active");
            return;
        }

        // Set alt screen cursor to the current primary screen cursor.
        let block_list_cursor = self
            .block_list
            .active_block_mut()
            .grid_storage()
            .cursor()
            .clone();
        self.alt_screen.grid_handler_mut().update_cursor(|cursor| {
            *cursor = block_list_cursor.clone();
        });

        self.alt_screen.reset_pending_lines_to_scroll();

        // Reset keyboard mode state so a new alt screen session doesn't inherit
        // stale modes from a previous one.
        self.alt_screen
            .grid_handler_mut()
            .reset_keyboard_mode_state();

        if save_cursor_and_clear_screen {
            // Drop information about the primary screen's saved cursor.
            self.block_list
                .active_block_mut()
                .set_saved_cursor(block_list_cursor);

            // Reset alt screen contents.
            let bg = self.alt_screen.grid_storage().cursor().template.bg;
            self.alt_screen
                .grid_storage_mut()
                .region_mut(..)
                .each(|cell| *cell = bg.into());
            self.alt_screen.grid_handler_mut().clear_secrets();
        }

        self.alt_screen_mut().clear_selection();
        self.alt_screen_active = true;

        self.event_proxy
            .send_terminal_event(Event::TerminalModeSwapped(TerminalMode::AltScreen));
    }

    /// Deactivate the alternate screen, switching back to the block list and
    /// copying over relevant state.
    ///
    /// If the alternate screen is not active, this has no effect. This guards
    /// against programs that set or unset the alternate screen mode multiple
    /// times, like `info`  (see WAR-5897).
    fn exit_alt_screen(&mut self, restore_cursor: bool) {
        if !self.alt_screen_active {
            log::info!("Tried to exit the alternate screen, but it was already inactive");
            return;
        }

        self.alt_screen_mut().grid_handler_mut().evict_all_images();

        self.alt_screen_active = false;

        if restore_cursor {
            self.block_list.active_block_mut().restore_cursor_position();
        }

        self.event_proxy
            .send_terminal_event(Event::TerminalModeSwapped(TerminalMode::BlockList));
    }

    #[cfg(test)]
    pub fn set_altscreen_active(&mut self) {
        self.alt_screen_active = true;
    }

    /// Sets whether any content within a grid that is "secret-like" should be obfuscated.
    pub fn set_obfuscate_secrets(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        // Secret obfuscation is forced off in shared sessions so changing
        // the setting during a shared session should be a no-op (for this session).
        if self.shared_session_status.is_sharer_or_viewer() {
            return;
        }

        self.obfuscate_secrets = obfuscate_secrets;
        self.alt_screen.set_obfuscate_secrets(obfuscate_secrets);
        self.block_list.set_obfuscate_secrets(obfuscate_secrets);
    }

    /// Disables secret obfuscation for shared session creators only.
    ///
    /// Specifically, secret obfuscation is disabled starting
    /// from the `first_scrollback_block_index` onwards.
    pub fn disable_secret_obfuscation_for_shared_sesson_creator(
        &mut self,
        first_scrollback_block_index: BlockIndex,
    ) {
        if !self.shared_session_status.is_sharer() {
            log::warn!(
                "Tried to disable secret obfuscation without being a shared session creator."
            );
            return;
        }

        let setting = ObfuscateSecrets::No;
        self.obfuscate_secrets = setting;

        // Disable obfuscation in the alt-screen.
        self.alt_screen.set_obfuscate_secrets(setting);

        // Ensure that all scrollback blocks and any subsequent blocks don't have their secrets obfuscated.
        let active_block_index = self.block_list.active_block_index();
        for block_index in
            BlockIndex::range_as_iter(first_scrollback_block_index..active_block_index)
        {
            self.block_list
                .set_obfuscate_secrets_for_block(block_index, setting);
        }
        self.block_list
            .set_obfuscate_secrets_for_subsequent_blocks(setting);
    }

    fn restored_block_commands(&self) -> Vec<HistoryEntry> {
        let mut commands = Vec::new();
        for block in self.block_list.blocks() {
            if block.is_restored()
                && !block.is_background()
                && block.state() != BlockState::DoneWithNoExecution
            {
                let entry = HistoryEntry::for_restored_block(block.command_to_string(), block);
                commands.push(entry);
            }
        }
        commands
    }

    /// Updates the filter on a block.
    pub fn update_filter_on_block(
        &mut self,
        block_index: BlockIndex,
        block_filter_query: BlockFilterQuery,
    ) {
        self.block_list_mut()
            .filter_block_output(block_index, block_filter_query);
    }

    pub fn clear_filter_on_block(&mut self, block_index: BlockIndex) {
        self.block_list_mut().clear_filter_on_block(block_index);
    }

    pub fn get_filter_on_block(&self, block_index: BlockIndex) -> Option<&BlockFilterQuery> {
        self.block_list().filter_for_block(block_index)
    }

    fn send_title_event(&mut self, title: Option<String>) {
        let title = title.unwrap_or(self.shell_launch_state().display_name().into());
        let title_event = Event::Title(title);
        self.event_proxy.send_terminal_event(title_event);
    }

    pub fn set_custom_title(&mut self, custom_title: Option<String>) {
        self.custom_title.clone_from(&custom_title);
        // If the custom title set by the user is None, we "reset" to whatever the title was set by
        // the shell / Warp itself.
        self.send_title_event(match custom_title {
            Some(_) => custom_title,
            None => self.title.clone(),
        });
    }

    pub fn custom_title(&self) -> Option<String> {
        self.custom_title.clone()
    }

    /// Returns the terminal's natural title (set by ANSI escape sequences).
    /// This does not include any custom title override.
    pub fn terminal_title(&self) -> Option<String> {
        self.title.clone()
    }

    /// Reports user input on the terminal as potential typeahead.
    pub fn push_user_input(&mut self, input: &str) {
        if !self.alt_screen_active {
            self.block_list.early_output_mut().push_user_input(input);
        }
    }

    fn emit_handler_event(&mut self, event: HandlerEvent) {
        self.event_proxy.send_handler_event(event);
    }

    pub fn set_env_var_collection_name(&mut self, value: Option<String>) {
        self.env_var_collection_name = value;
    }

    pub fn set_pending_warp_initiated_control_mode(&mut self) {
        let tmux_installation = self
            .tmux_control_mode_context
            .and_then(|context| context.tmux_installation());
        self.pending_warp_initiated_control_mode =
            Some(WarpInitiatedTmuxControlMode::new(tmux_installation));
    }

    pub fn set_pending_warp_initiated_control_mode_with_install_tmux(&mut self, with_root: bool) {
        self.pending_warp_initiated_control_mode =
            Some(WarpInitiatedTmuxControlMode::new(Some(if with_root {
                TmuxInstallationState::InstalledByWarpRootInThisSession
            } else {
                TmuxInstallationState::InstalledByWarpInThisSession
            })));
    }

    pub fn clear_pending_warp_initiated_control_mode(&mut self) {
        self.pending_warp_initiated_control_mode = None;
    }

    /// Informs the terminal model to start watching for ssh output that indicates the session
    /// has progressed past authentication/login. When login is complete, emit Event::DetectedEndOfSshLogin.
    pub fn start_notify_on_end_of_ssh_login(&mut self) {
        let id_of_ssh_block = self.active_block_id().clone();
        self.notify_on_end_of_ssh_login = Some(SshLogin {
            block_id: id_of_ssh_block,
            notification_state: SshLoginNotificationState::Monitoring,
        });
    }

    /// Stop monitoring for the end of ssh login.
    pub fn end_notify_on_ssh_login_complete(&mut self) {
        self.notify_on_end_of_ssh_login = None;
    }

    /// Emits the event [Event::DetectedEndOfSshLogin] if the last line of output in the
    /// ssh session indicates login is complete. The check_type parameter specifies whether
    /// this is the initial check or a confirmation check (i.e., a previous check has already
    /// succeeded).
    ///
    /// Overall, the heuristic waits for the line "Last login:" to appear in a line of output,
    /// indicating that login is complete. However, this isn't enough. Users might have a .hushlogin
    /// that suppresses that output line, so we also have a backup check. When we receive
    /// a line of output that is not a known SSH output, we consider that to be some mild evidence that
    /// login is complete. Though, because that output line might be a false alarm (i.e., it could be
    /// an SSH banner OR a line like "Permission denied."), we wait some amount of time and check again
    /// before indicating we're ready for warpification.
    pub fn check_for_end_of_ssh_login(&mut self, confirmation_check: bool) {
        let Some(mut ssh_login_state) = self.notify_on_end_of_ssh_login.clone() else {
            return;
        };

        // Only check for the end of ssh login if it was specifically enabled for the current active block.
        let active_block = self.block_list().active_block();
        if &ssh_login_state.block_id != active_block.id() {
            return;
        }

        // Only check for the end of ssh login if it wasn't already detected and notified.
        if ssh_login_state.notification_state == SshLoginNotificationState::Completed {
            return;
        }

        let is_initial_check = !confirmation_check;
        let block_output = active_block.output_to_string();
        match ssh::util::check_ssh_login_state(&block_output) {
            SshLoginState::LastLogin | SshLoginState::PromptDetected => {
                self.event_proxy
                    .send_terminal_event(Event::DetectedEndOfSshLogin(
                        SshLoginStatus::ReadyToWarpify,
                    ));

                ssh_login_state.notification_state = SshLoginNotificationState::Completed;
            }
            SshLoginState::NonSshOutput => {
                // If we detect non-SSH output AND we haven't already notified, send a notification.
                if is_initial_check {
                    if ssh_login_state.notification_state == SshLoginNotificationState::Monitoring {
                        self.event_proxy
                            .send_terminal_event(Event::DetectedEndOfSshLogin(
                                SshLoginStatus::RecheckBeforeWarpifying,
                            ));

                        // We want to avoid emitting redundant events for the initial check.
                        ssh_login_state.notification_state =
                            SshLoginNotificationState::SentInitialNotification;
                    }
                } else {
                    self.event_proxy
                        .send_terminal_event(Event::DetectedEndOfSshLogin(
                            SshLoginStatus::ReadyToWarpify,
                        ));

                    ssh_login_state.notification_state = SshLoginNotificationState::Completed;
                }
            }
            SshLoginState::Authenticating => {
                // False alarm case. If this is the confirmation check and it's detected that
                // we have NOT completed login, then we should start over and go back to monitoring
                // each output chunk for lines indicating login completion.
                if !is_initial_check {
                    ssh_login_state.notification_state = SshLoginNotificationState::Monitoring;
                }
            }
        }

        // Update the notification state.
        self.notify_on_end_of_ssh_login = Some(ssh_login_state);
    }

    pub fn is_ssh_block(&self) -> bool {
        self.notify_on_end_of_ssh_login.is_some()
    }

    pub fn tmux_control_mode_active(&self) -> bool {
        self.tmux_control_mode_context.is_some()
    }

    pub fn is_pending_warp_initiated_control_mode(&self) -> bool {
        self.pending_warp_initiated_control_mode.is_some()
    }

    pub fn is_warpified_ssh(&self) -> bool {
        matches!(
            self.tmux_control_mode_context,
            Some(TmuxControlModeContext::WarpInitiatedForSsh { .. })
        )
    }
}

/// Used in the ansi::Handler implementation for TerminalModel below. Performs
/// the provided method call on the active handler, either the block_list or the
/// alt_screen if it is active.
macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        if $self.alt_screen_active {
            $self.alt_screen.$method($( $arg ),*)
        } else {
            $self.block_list.$method($( $arg ),*)
        }
    }
}

impl TerminalModel {
    pub fn needs_bracketed_paste(&mut self) -> bool {
        delegate!(self.needs_bracketed_paste())
    }

    pub fn set_marked_text(&mut self, marked_text: &str, selected_range: &Range<usize>) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }
        delegate!(self.set_marked_text(marked_text, selected_range))
    }

    pub fn clear_marked_text(&mut self) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }
        delegate!(self.clear_marked_text())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CommandType {
    InBandCommand,
    User,
    Bootstrap,
}

#[derive(Clone, Debug)]
pub enum HandlerEvent {
    InitShell {
        pending_session_info: Box<SessionInfo>,
    },
    Bootstrapped(BootstrappedEvent),
    Precmd {
        session_id: Option<SessionId>,
        handled_after_inband: bool,
        env_vars: HashMap<String, String>,
    },
    Preexec,
    CommandFinished {
        command_type: CommandType,
    },
    PromptStart,
    RPromptStart,
    PromptEnd,
    SetMode {
        mode: Mode,
    },
    UnsetMode {
        mode: Mode,
    },
    StartTmuxControlMode,
    TmuxControlModeReady {
        primary_pane: u32,
        context: Option<TmuxControlModeContext>,
    },
    EndTmuxControlMode,
    RunTmuxCommand(TmuxCommand),
}

impl ansi::Handler for TerminalModel {
    fn set_title(&mut self, title: Option<String>) {
        // Don't set the tab title if the title event is for a running in-band command.
        if self.block_list().is_writing_or_executing_in_band_command() {
            return;
        }

        // Filter out null bytes from the title string.
        let filtered_title = title.map(|t| t.replace('\0', ""));

        self.title.clone_from(&filtered_title);

        // Only send this event if there was no custom title set.
        if self.custom_title.is_none() {
            self.send_title_event(filtered_title);
        }
    }

    fn set_cursor_style(&mut self, style: Option<ansi::CursorStyle>) {
        delegate!(self.set_cursor_style(style));
    }

    fn set_cursor_shape(&mut self, shape: ansi::CursorShape) {
        delegate!(self.set_cursor_shape(shape));
    }

    fn input(&mut self, c: char) {
        // TODO: we should figure out what it means to be simultaneously expecting
        // in-band command output and completions data, which is technically possible
        // with the current data structures.
        if let IsReceivingInBandCommandOutput::Yes { output } =
            &mut self.is_receiving_in_band_command_output
        {
            let is_receiving_prompt_chars = self.block_list.active_block().is_receiving_prompt();
            if !is_receiving_prompt_chars {
                output.input(c);
                return;
            }
        } else if let IsReceivingCompletionsOutput::Yes {
            pending: CompletionsShellData::Raw { output },
        } = &mut self.is_receiving_completions_output
        {
            output.push(c);
            return;
        }

        delegate!(self.input(c))
    }

    fn goto(&mut self, row: VisibleRow, column: usize) {
        if let IsReceivingInBandCommandOutput::Yes { output } =
            &mut self.is_receiving_in_band_command_output
        {
            output.goto(row.0, column);
            return;
        }
        delegate!(self.goto(row, column));
    }

    fn goto_line(&mut self, row: VisibleRow) {
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
        if let IsReceivingInBandCommandOutput::Yes { output: cursor, .. } =
            &mut self.is_receiving_in_band_command_output
        {
            cursor.carriage_return();
            return;
        }
        delegate!(self.carriage_return());
    }

    fn linefeed(&mut self) -> ScrollDelta {
        if matches!(
            self.is_receiving_completions_output,
            IsReceivingCompletionsOutput::Yes { .. }
        ) {
            return ScrollDelta::zero();
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
        if matches!(
            self.is_receiving_completions_output,
            IsReceivingCompletionsOutput::Yes { .. }
        ) {
            return;
        }

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
        self.title_stack = Vec::new();
        self.title = None;

        self.alt_screen.reset_state();
        self.block_list.reset_state();
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        delegate!(self.reverse_index())
    }

    fn terminal_attribute(&mut self, attribute: ansi::Attr) {
        self.alt_screen.terminal_attribute(attribute);
        self.block_list.terminal_attribute(attribute);
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        if let ansi::Mode::SwapScreen {
            save_cursor_and_clear_screen,
        } = mode
        {
            self.enter_alt_screen(save_cursor_and_clear_screen);
            return;
        }

        self.alt_screen.set_mode(mode);
        self.block_list.set_mode(mode);
        self.emit_handler_event(HandlerEvent::SetMode { mode });
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        match mode {
            ansi::Mode::SwapScreen {
                save_cursor_and_clear_screen,
            } => {
                self.exit_alt_screen(save_cursor_and_clear_screen);
                return;
            }
            ansi::Mode::SyncOutput => {
                // When synchronized output is turned off, we should redraw.
                self.event_proxy.send_wakeup_event();
            }
            _ => {}
        }

        self.alt_screen.unset_mode(mode);
        self.block_list.unset_mode(mode);
        self.emit_handler_event(HandlerEvent::UnsetMode { mode });
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

    fn set_color(&mut self, index: usize, color: warpui::color::ColorU) {
        self.override_colors[index] = Some(color);
    }

    fn dynamic_color_sequence<W: std::io::Write>(
        &mut self,
        writer: &mut W,
        code: u8,
        index: usize,
        terminator: &str,
    ) {
        let color = self.override_colors[index].unwrap_or(self.colors[index]);
        let response = format!(
            "\x1b]{};rgb:{1:02x}{1:02x}/{2:02x}{2:02x}/{3:02x}{3:02x}{4}",
            code, color.r, color.g, color.b, terminator
        );
        let _ = writer.write_all(response.as_bytes());
    }

    fn reset_color(&mut self, index: usize) {
        self.override_colors[index] = None;
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
        if self.title_stack.len() >= TITLE_STACK_MAX_DEPTH {
            // Remove from the bottom of the title if it exceeds the maximum depth.
            self.title_stack.remove(0);
        }

        self.title_stack.push(self.title.clone());
    }

    fn pop_title(&mut self) {
        if let Some(popped) = self.title_stack.pop() {
            self.set_title(popped);
        }
    }

    fn text_area_size_pixels<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_pixels(writer));
    }

    fn text_area_size_chars<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_chars(writer));
    }

    fn prompt_marker(&mut self, marker: ansi::PromptMarker) {
        if matches!(
            self.is_receiving_completions_output,
            IsReceivingCompletionsOutput::Yes { .. }
        ) && matches!(marker, ansi::PromptMarker::StartPrompt { .. })
        {
            self.end_completions_output();
        }

        let event = match &marker {
            ansi::PromptMarker::StartPrompt { kind } => match kind {
                ansi::PromptKind::Initial => HandlerEvent::PromptStart,
                ansi::PromptKind::Right => HandlerEvent::RPromptStart,
            },
            ansi::PromptMarker::EndPrompt => HandlerEvent::PromptEnd,
        };
        delegate!(self.prompt_marker(marker));
        self.emit_handler_event(event);
    }

    fn command_finished(&mut self, data: CommandFinishedValue) {
        // If we ssh from a doesn't-understand-bracketed-paste shell into one
        // that enables it, then get disconnected, we'll be stuck in a state
        // of bracketed paste being enabled, but the local shell doesn't know
        // how to turn it off (and will never do so).  We forcibly unset the
        // mode to avoid getting stuck in this state.
        self.unset_mode(Mode::BracketedPaste);

        // Similar to bracketed paste, above, make sure we quit out of the
        // alt screen if we're currently in it.  This prevents issues where we
        // remain in the alt screen after disconnect when we should return to
        // the blocklist (for the local shell).
        self.exit_alt_screen(true);

        let block_id = data.next_block_id.to_string();
        let is_for_in_band_command = self.block_list().active_block().is_in_band_command_block();
        let finished_block_bootstrap_stage = self.block_list().active_block().bootstrap_stage();
        delegate!(self.command_finished(data));

        if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
            if let Err(e) = tx.try_send(OrderedTerminalEventType::CommandExecutionFinished {
                next_block_id: block_id.into(),
            }) {
                log::warn!("Failed to send OrderedTerminalEventType::CommandFinished: {e}");
            }
        }

        self.emit_handler_event(HandlerEvent::CommandFinished {
            command_type: if is_for_in_band_command {
                CommandType::InBandCommand
            } else if finished_block_bootstrap_stage == BootstrapStage::PostBootstrapPrecmd {
                CommandType::User
            } else {
                CommandType::Bootstrap
            },
        });
    }

    fn precmd(&mut self, data: PrecmdValue) {
        self.ignore_bootstrapping_messages = false;
        let session_id = data.session_id;
        let mut env_vars = HashMap::new();
        if let Some(kube_config) = data.kube_config.clone() {
            env_vars.insert("KUBECONFIG".to_string(), kube_config);
        }
        let handled_after_inband = data.was_sent_after_in_band_command();
        delegate!(self.precmd(data));

        self.emit_handler_event(HandlerEvent::Precmd {
            session_id: session_id.map(|id| id.into()),
            handled_after_inband,
            env_vars,
        });
    }

    fn preexec(&mut self, data: PreexecValue) {
        delegate!(self.preexec(data));
        self.emit_handler_event(HandlerEvent::Preexec);
    }

    fn bootstrapped(&mut self, value: BootstrappedValue) {
        self.block_list.bootstrapped(value.clone());

        let pending_session_info = match self.pending_session_info.take() {
            Some(session_info) => session_info,
            None => {
                // Not being able to read the value should not cause a full-app crash. Instead,
                // bootstrapping should fail in the same way that it would if the DCS message
                // were otherwise corrupted.
                log::error!("Received bootstrap message with no pending session info.");
                return;
            }
        };

        let rcfiles_duration_seconds = match (value.rcfiles_start_time, value.rcfiles_end_time) {
            (Some(start_time), Some(end_time)) => Some((end_time - start_time).into()),
            _ => None,
        };

        let fully_populated_session_info = pending_session_info
            .merge_from_bootstrapped_value(value, self.tmux_control_mode_context.is_some());

        self.block_list
            .early_output_mut()
            .init_session(&fully_populated_session_info);

        let spawning_command = fully_populated_session_info
            .subshell_info
            .as_ref()
            .map(|subshell_info| subshell_info.spawning_command.clone())
            .unwrap_or(self.block_list.active_block().command_to_string());

        self.emit_handler_event(HandlerEvent::Bootstrapped(BootstrappedEvent {
            spawning_command,
            session_info: Box::new(fully_populated_session_info),
            restored_block_commands: self.restored_block_commands(),
            rcfiles_duration_seconds,
        }));
    }

    fn pre_interactive_ssh_session(&mut self, _value: PreInteractiveSSHSessionValue) {
        self.event_proxy
            .send_terminal_event(Event::PreInteractiveSSHSession);
    }

    fn ssh(&mut self, value: SSHValue) {
        if !self.ignore_bootstrapping_messages {
            let remote_shell = value.remote_shell.clone();
            self.pending_legacy_ssh_session = Some(value);
            self.event_proxy
                .send_terminal_event(Event::SSH(remote_shell));
        }
    }

    fn exit_shell(&mut self, data: ExitShellValue) {
        log::info!(
            "Received ExitShell hook from shell for session_id: {:?}",
            data.session_id
        );
        self.event_proxy.send_terminal_event(Event::ExitShell {
            session_id: data.session_id,
        });
    }

    fn init_shell(&mut self, data: InitShellValue) {
        if !self.ignore_bootstrapping_messages {
            let subshell_info = if data.is_subshell {
                let was_triggered_by_rc_file_snippet =
                    self.did_receive_rc_file_dcs.take().unwrap_or(false);
                let env_var_collection_name = self.env_var_collection_name.take();
                let spawning_command = self.block_list().active_block().command_to_string();

                let ssh_connection_info =
                    ssh::util::parse_interactive_ssh_command(&spawning_command);

                Some(SubshellInitializationInfo {
                    spawning_command,
                    was_triggered_by_rc_file_snippet,
                    env_var_collection_name,
                    ssh_connection_info,
                })
            } else {
                None
            };

            let shell_type = ShellType::from_name(&data.shell)
                .unwrap_or_else(|| panic!("invalid shell name: {}", data.shell));

            let pending_session_info = SessionInfo::create_pending(
                shell_type,
                data,
                subshell_info,
                self.pending_shell_launch_data.take(),
                self.pending_legacy_ssh_session.take(),
                matches!(
                    self.tmux_control_mode_context,
                    Some(TmuxControlModeContext::WarpInitiatedForSsh { .. })
                ),
                self.block_list().active_block().session_id(),
            );
            self.pending_session_info = Some(pending_session_info.clone());

            if self.block_list().is_bootstrapped() {
                self.block_list_mut().reinit_shell();
            }

            self.emit_handler_event(HandlerEvent::InitShell {
                pending_session_info: Box::new(pending_session_info),
            });
        }
    }

    fn clear(&mut self, _data: ClearValue) {
        self.clear_visible_screen();
    }

    fn input_buffer(&mut self, data: InputBufferValue) {
        delegate!(self.input_buffer(data));
    }

    fn init_subshell(&mut self, data: InitSubshellValue) {
        let is_tmux_ssh = self.pending_warp_initiated_control_mode.is_some();
        let shell_type = ShellType::from_name(data.shell.as_str());
        if let Some(shell_type) = shell_type {
            self.event_proxy
                .send_terminal_event(Event::InitSubshell(InitSubshellEvent {
                    shell_type,
                    uname: data.uname,
                }));
        } else {
            log::error!(
                "Received invalid shell name in init_subshell: {} | is_tmux_ssh: {}",
                data.shell,
                is_tmux_ssh
            );
            if is_tmux_ssh {
                self.event_proxy
                    .send_terminal_event(Event::RemoteWarpificationIsUnavailable(
                        WarpificationUnavailableReason::UnsupportedShell {
                            shell_name: data.shell,
                        },
                    ))
            }
        }
    }

    fn sourced_rc_file(&mut self, data: SourcedRcFileForWarpValue) {
        // If the blocklist is already bootstrapped, the user's RC file must be sourced in a
        // subshell.
        if self.block_list.is_bootstrapped() {
            self.did_receive_rc_file_dcs = Some(true);
            let shell_type = ShellType::from_name(data.shell.as_str());
            match shell_type {
                Some(shell_type) => {
                    self.event_proxy
                        .send_terminal_event(Event::SourcedRcFileInSubshell(
                            SourcedRcFileInSubshellEvent {
                                shell_type,
                                uname: data.uname,
                                tmux: data.tmux,
                            },
                        ))
                }
                None => {
                    log::error!(
                        "Received invalid shell name in SourcedRCFileForWarpValue: {}",
                        data.shell
                    );
                }
            }
        }
    }

    fn init_ssh(&mut self, data: InitSshValue) {
        let shell_type = ShellType::from_name(data.shell.as_str());
        match shell_type {
            Some(shell_type @ (ShellType::Bash | ShellType::Zsh | ShellType::Fish)) => self
                .event_proxy
                .send_terminal_event(Event::InitSsh(InitSshEvent {
                    shell_type,
                    uname: data.uname,
                })),
            _ => self
                .event_proxy
                .send_terminal_event(Event::RemoteWarpificationIsUnavailable(
                    WarpificationUnavailableReason::UnsupportedShell {
                        shell_name: data.shell,
                    },
                )),
        }
    }

    fn finish_update(&mut self, data: FinishUpdateValue) {
        self.event_proxy
            .send_terminal_event(Event::FinishUpdate(data));
    }

    fn remote_warpification_is_unavailable(&mut self, data: WarpificationUnavailableReason) {
        self.event_proxy
            .send_terminal_event(Event::RemoteWarpificationIsUnavailable(data));
    }

    fn notify_ssh_tmux_is_installed(&mut self, tmux_installation: TmuxInstallationState) {
        if let Some(ref mut warp_initiated_for_ssh) = self.pending_warp_initiated_control_mode {
            warp_initiated_for_ssh.tmux_installation = Some(tmux_installation);
        }
        self.event_proxy
            .send_terminal_event(Event::SshTmuxInstaller(tmux_installation));
    }

    fn tmux_install_failed(&mut self, data: TmuxInstallFailedInfo) {
        self.event_proxy
            .send_terminal_event(Event::TmuxInstallFailed {
                line: data.line,
                command: data.command,
            });
    }

    fn start_in_band_command_output(&mut self) {
        let starting_cursor_point = self
            .block_list()
            .active_block()
            .grid_handler()
            .cursor_point();
        self.is_receiving_in_band_command_output = IsReceivingInBandCommandOutput::Yes {
            output: InBandCommandOutputReceiver::new(
                starting_cursor_point,
                self.block_list().size(),
            ),
        };
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn end_in_band_command_output(&mut self, from_osc_sequence: bool) {
        match &mut self.is_receiving_in_band_command_output {
            IsReceivingInBandCommandOutput::Yes { output } => {
                match validate_and_decode_in_band_command_output_to_bytes(output.as_str()) {
                    Ok(decoded_bytes) => {
                        match ExecutedExecutorCommandEvent::parse_generator_payload(decoded_bytes) {
                            Ok(event) => {
                                log::info!(
                                    "Parsed generator output for command {}",
                                    event.command_id
                                );
                                self.event_proxy
                                    .send_terminal_event(Event::ExecutedInBandCommand(event));
                            }
                            Err(e) => {
                                log::warn!("Failed to parse generator output: {e:#}");
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to decode generator output: {e:#}");
                    }
                };
                self.is_receiving_in_band_command_output = IsReceivingInBandCommandOutput::No;
            }
            IsReceivingInBandCommandOutput::No => {
                log::warn!("Received 'end_in_band_command_output' while not expecting to read in-band command output.");
            }
        }

        #[cfg(windows)]
        if from_osc_sequence {
            self.ignore_reset_grid_after_in_band_generator = true;
        }
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        if let Some(SshLogin {
            notification_state, ..
        }) = &self.notify_on_end_of_ssh_login
        {
            if matches!(
                notification_state,
                SshLoginNotificationState::Monitoring
                    | SshLoginNotificationState::SentInitialNotification
            ) {
                self.check_for_end_of_ssh_login(false);
            }
        }

        let bytes = input.bytes();

        // Send a copy of the bytes to subscribers.
        self.event_proxy.send_pty_read_event(bytes);

        // Send a copy of the bytes for the active shared session, if applicable.
        // When processing a synchronized output frame, `on_finish_byte_processing` is called
        // both when the frame is flushed and when we initially process the raw bytes (the ordering of the two
        // depends on whether we receive the start and end markers in the same batch of bytes). We only want to send
        // the raw bytes to viewers, not the flushed frame - they'll handle the synchronized output framing themselves.
        if !input.is_synchronized_output_frame() && self.shared_session_status().is_sharer() {
            if let Some(tx) = &self.ordered_terminal_events_for_shared_session_tx {
                if let Err(e) = tx.try_send(OrderedTerminalEventType::PtyBytesRead {
                    bytes: bytes.to_owned(),
                }) {
                    log::warn!("Failed to send OrderedTerminalEventType::PtyBytesRead: {e}");
                }
            }
        }

        delegate!(self.on_finish_byte_processing(input))
    }

    fn on_reset_grid(&mut self) {
        #[cfg(windows)]
        if self.ignore_reset_grid_after_in_band_generator {
            self.ignore_reset_grid_after_in_band_generator = false;
            return;
        }
        delegate!(self.on_reset_grid());
    }

    fn tmux_control_mode_event(&mut self, event: tmux::ControlModeEvent) {
        match event {
            tmux::ControlModeEvent::BackgroundPaneOutput { pane, byte } => {
                let output = self.tmux_background_outputs.entry(pane).or_default();
                output.push(byte);
                if byte == b'\n' && output.ends_with(b"$$$\r\n") {
                    match tmux::parse_generator_output(output) {
                        Some(command_event) => {
                            self.event_proxy
                                .send_terminal_event(Event::ExecutedInBandCommand(command_event));
                        }
                        None => {
                            log::warn!(
                                "Could not parse tmux generator output: {:?}",
                                AsciiDebug(output)
                            );
                        }
                    }
                    self.tmux_background_outputs.remove(&pane);
                }
            }
            tmux::ControlModeEvent::Starting => {
                if let Some(warp_initiated_for_ssh) = self.pending_warp_initiated_control_mode {
                    self.tmux_control_mode_context = Some(
                        TmuxControlModeContext::WarpInitiatedForSsh(warp_initiated_for_ssh),
                    );
                } else {
                    self.tmux_control_mode_context = Some(TmuxControlModeContext::UserInitiated);
                }
                self.emit_handler_event(HandlerEvent::StartTmuxControlMode);

                self.emit_handler_event(HandlerEvent::RunTmuxCommand(
                    TmuxCommand::GetPrimaryWindowPane,
                ));

                let size = self.block_list.size();
                let num_rows = size.rows();
                let num_cols = size.columns();

                if self.tmux_control_mode_context != Some(TmuxControlModeContext::UserInitiated) {
                    // We don't want to intentionally disable persistence when the user runs tmux control
                    // mode on their own.
                    self.emit_handler_event(HandlerEvent::RunTmuxCommand(
                        TmuxCommand::SetDestroyUnattached,
                    ));

                    self.emit_handler_event(HandlerEvent::RunTmuxCommand(
                        TmuxCommand::SetWindowSizeToSmallest,
                    ));
                }

                self.emit_handler_event(HandlerEvent::RunTmuxCommand(
                    TmuxCommand::UpdateClientSize { num_cols, num_rows },
                ));
            }
            tmux::ControlModeEvent::Exited => {
                self.tmux_control_mode_context = None;
                self.emit_handler_event(HandlerEvent::EndTmuxControlMode);
            }
            tmux::ControlModeEvent::ControlModeReady { primary_pane, .. } => {
                self.emit_handler_event(HandlerEvent::TmuxControlModeReady {
                    primary_pane,
                    context: self.tmux_control_mode_context,
                });
                self.event_proxy
                    .send_terminal_event(Event::TmuxControlModeReady { primary_pane });
            }
        }
    }

    fn start_completions_output(&mut self, data: CompletionsShellData) {
        self.is_receiving_completions_output = IsReceivingCompletionsOutput::Yes { pending: data };
    }

    fn end_completions_output(&mut self) {
        match std::mem::replace(
            &mut self.is_receiving_completions_output,
            IsReceivingCompletionsOutput::No,
        ) {
            IsReceivingCompletionsOutput::Yes { pending } => {
                self.event_proxy
                    .send_terminal_event(Event::CompletionsFinished(pending.into()));
            }
            IsReceivingCompletionsOutput::No => {
                log::warn!("Tried to unexpectedly end completions output.")
            }
        }
    }

    fn on_completion_result_received(&mut self, completion_result: ShellCompletion) {
        match &mut self.is_receiving_completions_output {
            IsReceivingCompletionsOutput::Yes {
                pending: CompletionsShellData::IncrementallyTyped { output },
            } => {
                output.push(completion_result);
            }
            IsReceivingCompletionsOutput::Yes {
                pending: CompletionsShellData::Raw { .. },
            } => {
                log::warn!(
                    "Received typed completion result but expected to be in raw completions mode"
                );
            }
            IsReceivingCompletionsOutput::No => {
                log::warn!("Unexpectedly received completion result");
            }
        }
    }

    fn update_last_completion_result(&mut self, completion_update: ShellCompletionUpdate) {
        match &mut self.is_receiving_completions_output {
            IsReceivingCompletionsOutput::Yes {
                pending: CompletionsShellData::IncrementallyTyped { output },
            } => {
                if let Some(last_item) = output.last_mut() {
                    last_item.update(completion_update);
                } else {
                    log::warn!("Received update last completion result OSC before any completion results have been received");
                }
            }
            IsReceivingCompletionsOutput::Yes {
                pending: CompletionsShellData::Raw { .. },
            } => {
                log::warn!(
                    "Received typed completion result but expected to be in raw completions mode"
                );
            }
            IsReceivingCompletionsOutput::No => {
                log::warn!("Unexpectedly received completion result");
            }
        }
    }

    fn send_completions_prompt(&mut self) {
        self.event_proxy
            .send_terminal_event(Event::SendCompletionsPrompt);
    }

    fn start_iterm_image_receiving(&mut self, metadata: ITermImageMetadata) {
        let pending = ITermImage {
            metadata,
            ..Default::default()
        };
        self.is_receiving_iterm_image_data = IsReceivingITermImageData::Yes { pending };
    }

    fn end_iterm_image_receiving(&mut self) {
        match std::mem::replace(
            &mut self.is_receiving_iterm_image_data,
            IsReceivingITermImageData::No,
        ) {
            IsReceivingITermImageData::Yes { mut pending } => {
                // iTerm image is base64 encoded
                let Ok(decoded_bytes) =
                    base64::engine::general_purpose::STANDARD.decode(&pending.data[..])
                else {
                    return;
                };
                pending.data = decoded_bytes;

                if !pending.metadata.inline {
                    #[cfg(not(target_family = "wasm"))]
                    if let Some(cwd) = self
                        .active_block_metadata()
                        .current_working_directory()
                        .map(|cwd| cwd.to_string())
                    {
                        let mut path = PathBuf::from(cwd);
                        path.push(pending.metadata.name);
                        let _ = save_as_file(&pending.data[..], path);
                    }
                    return;
                }

                let Ok(image_type) = ImageType::try_from_bytes(&pending.data[..]) else {
                    return;
                };
                let Some(image_size) = image_type.image_size() else {
                    return;
                };

                pending.metadata.image_size = image_size.to_f32();

                self.handle_completed_iterm_image(pending);
            }
            IsReceivingITermImageData::No => {
                log::warn!("Received 'end_iterm_image_receiving' while not expecting to read iTerm image chunks.")
            }
        }
    }

    fn on_iterm_image_data_received(&mut self, image_data: &[u8]) {
        match &mut self.is_receiving_iterm_image_data {
            IsReceivingITermImageData::Yes { pending } => {
                pending.data.extend_from_slice(image_data);
            }
            IsReceivingITermImageData::No => {
                log::warn!("Unexpectedly received iTerm image chunk");
            }
        }
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        self.image_id_to_metadata.insert(
            image.metadata.id,
            StoredImageMetadata::ITerm(image.metadata.clone()),
        );

        delegate!(self.handle_completed_iterm_image(image))
    }

    fn start_receiving_hook(&mut self, hook_name: String) {
        if let Some(pending_hook) = PendingHook::create(&hook_name) {
            self.is_receiving_hook = IsReceivingHook::Yes {
                pending_hook: Box::new(pending_hook),
            };
        } else {
            log::warn!("Creating of pending {hook_name} hook failed");
        }
    }

    fn finish_receiving_hook(&mut self) -> Option<PendingHook> {
        match std::mem::replace(&mut self.is_receiving_hook, IsReceivingHook::No) {
            IsReceivingHook::Yes {
                pending_hook: pending_shell_hook,
            } => Some(*pending_shell_hook),
            IsReceivingHook::No => {
                log::warn!("Unexpectedly received an end to receiving a pending hook");
                None
            }
        }
    }

    fn update_hook(&mut self, key: String, value: String) {
        match &mut self.is_receiving_hook {
            IsReceivingHook::Yes {
                pending_hook: pending_shell_hook,
            } => {
                pending_shell_hook.update(key, value);
            }
            IsReceivingHook::No => {
                log::warn!("Tried to unexpectedly update pending hook");
            }
        }
    }

    fn end_kitty_action_receiving<W: std::io::Write>(&mut self, writer: &mut W) {
        let is_receiving_kitty_image_data = std::mem::replace(
            &mut self.is_receiving_kitty_image_data,
            IsReceivingKittyActionData::No,
        );

        let IsReceivingKittyActionData::Yes { mut pending } = is_receiving_kitty_image_data else {
            log::warn!("Received 'end_kitty_action_receiving' while not expecting to read kitty image chunks.");
            return;
        };

        let message_id = pending.control_data.image_id;
        let verbosity = pending.control_data.verbosity;

        if message_id.is_none() {
            pending.control_data.image_id = Some(self.next_kitty_image_id);
            self.next_kitty_image_id = self.next_kitty_image_id.wrapping_add(1);
            // 0 is an invalid ID for kitty images
            if self.next_kitty_image_id == 0 {
                self.next_kitty_image_id += 1;
            }
        }

        let message = match KittyMessage::try_from(pending) {
            Ok(message) => message,
            Err(err) => {
                log::warn!("{err:?}");
                if let Some(message_id) = message_id {
                    if verbosity.send_error() {
                        let _ = writer.write_all(&create_kitty_error_reply(message_id, err.into()));
                    }
                }
                return;
            }
        };

        match KittyAction::try_from(message) {
            Ok(action) => {
                match &action {
                    KittyAction::StoreOnly(action) => {
                        self.image_id_to_metadata.insert(
                            action.image_id,
                            StoredImageMetadata::Kitty(action.image.metadata.clone()),
                        );
                    }
                    KittyAction::StoreAndDisplay(action) => {
                        self.image_id_to_metadata.insert(
                            action.image_id,
                            StoredImageMetadata::Kitty(action.image.metadata.clone()),
                        );
                    }
                    KittyAction::DisplayStoredImage(_) => {}
                    KittyAction::QuerySupport(_) => {}
                    KittyAction::Delete {
                        delete_placements_only,
                        deletion_type,
                    } => match deletion_type {
                        DeletionType::DeleteAll => {
                            if !delete_placements_only {
                                self.image_id_to_metadata.clear();
                            }

                            if self.alt_screen_active {
                                self.alt_screen.grid_handler_mut().evict_all_images();
                            } else {
                                for block in self.block_list_mut().blocks_mut() {
                                    block.grid_handler_mut().evict_all_images();
                                }
                            }
                        }
                        DeletionType::DeleteById(delete_by_id) => {
                            if !delete_placements_only {
                                self.image_id_to_metadata.remove(&delete_by_id.image_id);
                            }

                            if self.alt_screen_active {
                                if let Some(placement_id) = delete_by_id.placement_id {
                                    self.alt_screen
                                        .grid_handler_mut()
                                        .evict_placement(delete_by_id.image_id, placement_id);
                                } else {
                                    self.alt_screen
                                        .grid_handler_mut()
                                        .evict_image(delete_by_id.image_id);
                                }
                            } else {
                                for block in self.block_list_mut().blocks_mut() {
                                    if let Some(placement_id) = delete_by_id.placement_id {
                                        block
                                            .grid_handler_mut()
                                            .evict_placement(delete_by_id.image_id, placement_id);
                                    } else {
                                        block.grid_handler_mut().evict_image(delete_by_id.image_id);
                                    }
                                }
                            }
                        }
                    },
                }

                match self.handle_completed_kitty_action(action.clone(), &mut HashMap::new()) {
                    Some(Ok(_)) => {
                        if let Some(message_id) = message_id {
                            if verbosity.send_ok() {
                                let _ = writer.write_all(&create_kitty_ok_reply(message_id));
                            }
                        }
                    }
                    Some(Err(err)) => {
                        log::warn!("{err:?}");
                        if let Some(message_id) = message_id {
                            if verbosity.send_error() {
                                let _ =
                                    writer.write_all(&create_kitty_error_reply(message_id, err));
                            }
                        }
                    }
                    None => {}
                };
            }
            Err(err) => {
                log::warn!("{err:?}");
                if let Some(message_id) = message_id {
                    if verbosity.send_error() {
                        let _ = writer.write_all(&create_kitty_error_reply(message_id, err));
                    }
                }
            }
        };
    }

    fn on_kitty_image_chunk_received(&mut self, chunk: KittyChunk) {
        match &mut self.is_receiving_kitty_image_data {
            IsReceivingKittyActionData::Yes { pending } => {
                pending.payload.push(chunk.payload);
            }
            IsReceivingKittyActionData::No => {
                self.is_receiving_kitty_image_data = IsReceivingKittyActionData::Yes {
                    pending: PendingKittyMessage {
                        control_data: chunk.control_data,
                        payload: vec![chunk.payload],
                    },
                };
            }
        }
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        _metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        delegate!(self.handle_completed_kitty_action(action, &mut self.image_id_to_metadata))
    }

    fn pluggable_notification(&mut self, title: Option<String>, body: String) {
        if FeatureFlag::PluggableNotifications.is_enabled() {
            self.event_proxy
                .send_terminal_event(Event::PluggableNotification { title, body });
        }
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

impl ModeProvider for TerminalModel {
    fn is_term_mode_set(&self, mode: TermMode) -> bool {
        self.is_term_mode_set(mode)
    }
}

/// Validates and decodes in-band command output sent via `warp_send_generator_output_osc_message`.
/// Upon success, returns the string content of the generator output. The OSC payload is expected
/// to conform to the following format:
///
///   <content_length>;<content>
///
/// where `content_length` is the length (number of bytes) in `content`.  If the
/// payload does not conform to this format or if expected content length does not
/// match the actual content length, returns an error.
fn validate_and_decode_in_band_command_output_to_bytes(
    raw_payload: &str,
) -> Result<Vec<u8>, InBandCommandOutputDecodingError> {
    let components = raw_payload.splitn(2, ';').collect_vec();
    if components.len() != 2 {
        return Err(InBandCommandOutputDecodingError::NoContentLengthHeader);
    }

    let expected_content_length = components[0]
        .parse::<usize>()
        .map_err(InBandCommandOutputDecodingError::ContentLengthHeaderCorrupted)?;
    let payload: &str = components[1].trim();
    let actual_content_length = payload.len();
    if actual_content_length != expected_content_length {
        return Err(InBandCommandOutputDecodingError::ContentLengthMismatch {
            actual_length: actual_content_length,
            expected_length: expected_content_length,
        });
    }

    hex::decode(payload).map_err(InBandCommandOutputDecodingError::HexDecodingFailure)
}

#[derive(thiserror::Error, Debug)]
enum InBandCommandOutputDecodingError {
    #[error("Missing content length header.")]
    NoContentLengthHeader,
    #[error("DCS content length header is corrupted: {0:?}")]
    ContentLengthHeaderCorrupted(ParseIntError),
    #[error("Content length header does not match length of received content. Actual: {actual_length}, expected: {expected_length}")]
    ContentLengthMismatch {
        actual_length: usize,
        expected_length: usize,
    },
    #[error("Failed to hex-decode the DCS payload: {0:?}")]
    HexDecodingFailure(FromHexError),
}

#[derive(Debug, Copy, Clone)]
pub enum ExitReason {
    /// The shell process exited naturally
    ShellProcessExited,
    /// PTY spawn failed
    PtySpawnFailed,
    /// PTY connection was lost/disconnected
    PtyDisconnected,
    /// Process was killed/terminated
    ProcessKilled,
    /// Shell could not be found/determined
    ShellNotFound,
}

#[cfg(test)]
#[path = "terminal_model_tests.rs"]
pub(crate) mod tests;
