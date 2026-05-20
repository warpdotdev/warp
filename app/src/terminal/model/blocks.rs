mod selection;

use crate::ai::agent::{conversation::AIConversationId, AIAgentActionId};
use crate::ai::blocklist::SerializedBlockListItem;
use crate::terminal::block_filter::BlockFilterQuery;

use crate::ai::blocklist::agent_view::{AgentViewDisplayMode, AgentViewState};
use crate::terminal::event::AfterBlockCompletedEvent;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi;
use crate::terminal::model::ansi::{
    Attr, BootstrappedValue, CharsetIndex, ClearMode, CommandFinishedValue, CursorShape,
    CursorStyle, LineClearMode, Mode, PrecmdValue, PreexecValue, Processor, StandardCharset,
    TabulationClearMode,
};
use crate::terminal::model::block::{AgentViewVisibility, Block, SerializedBlock};
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::index::{Point, VisibleRow};
use crate::terminal::model::iterm_image::ITermImage;
use crate::terminal::view::SeparatorId;
use crate::terminal::view::WithinBlockBanner;
use crate::terminal::{
    event::{
        BlockType, Event as TerminalEvent,
        Event::{AfterBlockCompleted, TerminalClear},
    },
    view::{InlineBannerId, InlineBannerItem},
};
use crate::terminal::{BlockPadding, ShellHost, SizeInfo, SizeUpdate};
use anyhow::anyhow;
use chrono::{DateTime, Local};
use instant::SystemTime;
use std::io;
use std::ops::{AddAssign, Range, RangeInclusive};
use std::sync::Arc;
use std::time::Duration;
use sum_tree::{Dimension, Item, SeekBias, SumTree};
use warp_core::features::FeatureFlag;
use warpui::color::ColorU;
use warpui::r#async::executor::Background;
use warpui::record_trace_event;

use std::collections::{HashMap, HashSet};
use warpui::{
    units::{IntoLines, IntoPixels, Lines},
    AppContext, EntityId, ViewHandle,
};

use super::block::{BlockId, BlockSize, BlockState};
use super::early_output::EarlyOutput;
use super::grid::grid_handler::{FragmentBoundary, GridHandler, PossiblePath};
use super::grid::RespectDisplayedOutput;
use super::image_map::StoredImageMetadata;
use super::kitty::{KittyAction, KittyResponse};
use super::rich_content::RichContentType;
use super::secrets::RespectObfuscatedSecrets;
use super::{ansi::InputBufferValue, block::SerializedAIMetadata};

use super::selection::ScrollDelta;
use super::terminal_model::RangeInModel;
use super::{ansi::Handler, grid::grid_handler::Link};
use crate::ai::blocklist::AIBlock;
use crate::terminal::block_list_element::GridType;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::model::terminal_model::{BlockIndex, WithinBlock};
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};

use selection::BlockListSelection;
pub use selection::SelectionRange;

#[cfg(feature = "local_fs")]
const RESTORED_BLOCK_SEPARATOR_HEIGHT: f64 = 1.5;
pub(in crate::terminal) const INLINE_BANNER_HEIGHT: f64 = 2.5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RichContentItem {
    /// TODO: Right now, most rich content is not typed. We should consider
    /// removing the `Option` and forcing all rich content to be typed.
    pub content_type: Option<RichContentType>,
    pub view_id: EntityId,
    pub last_laid_out_height: BlockHeight,
    /// The conversation ID of the active agent view when this rich content was created, if any.
    pub agent_view_conversation_id: Option<AIConversationId>,
    pub should_hide: bool,
}

impl RichContentItem {
    pub fn new(
        content_type: Option<RichContentType>,
        view_id: EntityId,
        agent_view_conversation_id: Option<AIConversationId>,
        should_hide: bool,
    ) -> Self {
        Self {
            content_type,
            view_id,
            last_laid_out_height: BlockHeight::from(1.0),
            agent_view_conversation_id,
            should_hide,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(
        content_type: Option<RichContentType>,
        view_id: EntityId,
        agent_view_conversation_id: Option<AIConversationId>,
    ) -> Self {
        Self::new(content_type, view_id, agent_view_conversation_id, false)
    }

    pub fn should_hide_for_agent_view_state(&self, agent_view_state: &AgentViewState) -> bool {
        if !FeatureFlag::AgentView.is_enabled() {
            return false;
        }

        match agent_view_state {
            AgentViewState::Active {
                conversation_id,
                display_mode: AgentViewDisplayMode::FullScreen,
                ..
            } => Some(*conversation_id) != self.agent_view_conversation_id,
            AgentViewState::Active {
                display_mode: AgentViewDisplayMode::Inline,
                ..
            }
            | AgentViewState::Inactive => self.agent_view_conversation_id.is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockHeightItem {
    Block(BlockHeight),
    Gap(BlockHeight),
    RestoredBlockSeparator {
        /// The height of the separator in `Lines` when visible (when `is_hidden` is false).
        height_when_visible: BlockHeight,
        is_historical_conversation_restoration: bool,
        /// Whether this separator is hidden (e.g., in agent view).
        is_hidden: bool,
    },
    /// A banner that shows up between blocks in the block list.
    InlineBanner {
        /// The height of the banner in `Lines` when visible (when `is_hidden` is false).
        height_when_visible: BlockHeight,
        banner: InlineBannerItem,
        /// Whether this banner is hidden (e.g., in agent view).
        is_hidden: bool,
    },
    SubshellSeparator {
        /// The height of the separator in `Lines` when visible (when `is_hidden` is false).
        height_when_visible: BlockHeight,
        separator_id: SeparatorId,
        /// Whether this separator is hidden (e.g., in agent view).
        is_hidden: bool,
    },
    RichContent(RichContentItem),
}

impl BlockHeightItem {
    pub fn height(&self) -> BlockHeight {
        match self {
            BlockHeightItem::Block(height) => *height,
            BlockHeightItem::Gap(height) => *height,
            BlockHeightItem::RestoredBlockSeparator {
                height_when_visible: height,
                is_hidden,
                ..
            } => {
                if *is_hidden {
                    BlockHeight::zero()
                } else {
                    *height
                }
            }
            BlockHeightItem::InlineBanner {
                height_when_visible: height,
                is_hidden,
                ..
            } => {
                if *is_hidden {
                    BlockHeight::zero()
                } else {
                    *height
                }
            }
            BlockHeightItem::SubshellSeparator {
                height_when_visible,
                is_hidden,
                ..
            } => {
                if *is_hidden {
                    BlockHeight::zero()
                } else {
                    *height_when_visible
                }
            }
            BlockHeightItem::RichContent(item) => {
                if item.should_hide {
                    BlockHeight::zero()
                } else {
                    item.last_laid_out_height
                }
            }
        }
    }
}

/// A set of saved prompt data that we may choose to render instead of data
/// from the active block to avoid flicker.
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CachedPromptData {
    /// The prompt grid data.
    pub prompt_grid: BlockGrid,
    /// The right-side prompt grid data.
    pub rprompt_grid: BlockGrid,
    /// The time at which the block containing these prompts was created.
    pub block_creation_time: DateTime<Local>,
}

/// Data about a particular scroll position relative to a block.
#[derive(Clone, Copy, Debug)]
pub struct BlockScrollPosition {
    /// The block index that the top of the viewport is in.
    pub block_index: BlockIndex,
    /// The offset into the block that the top of the viewport is in.
    pub offset_from_block_top: Lines,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RemovableBlocklistItem {
    InlineBanner(InlineBannerId),
    RichContent(EntityId),
}

pub struct BlockList {
    blocks: Vec<Block>,
    block_heights: SumTree<BlockHeightItem>,
    block_id_to_block_index: HashMap<BlockId, BlockIndex>,
    size: SizeInfo,
    early_output: EarlyOutput,

    /// Set of rich content views whose heights may be out of date and should be
    /// remeasured on the next layout.
    dirty_rich_content_items: HashSet<EntityId>,

    /// The gap height to set after a clear or ctrl-l.
    /// The view needs to tell the model this because the gap height varies based
    /// on the input mode, which is a view concept, not a model concept.
    /// It can't be passed as an argument to clear, because clear comes as the
    /// response on the terminal event loop, not from the view.
    next_gap_height_in_lines: Option<Lines>,

    /// Mapping from a unique identifier (representing some non-block item) to the corresponding index in the
    /// SumTree's TotalCount dimension.
    removable_blocklist_item_positions: HashMap<RemovableBlocklistItem, TotalIndex>,

    /// The max scroll limit for each block.
    max_grid_size_limit: usize,

    /// The event proxy that proxies terminal events (such as wakeups) to the view.
    event_proxy: ChannelEventListener,

    /// The current block list selection.
    /// Do not set this value directly - use [`Self::set_selection`] and [`Self::clear_selection`] instead.
    selection: Option<BlockListSelection>,
    /// If this is Some, and if smart-select is enabled, double-clicking within this range will
    /// select this range instead of the normal smart-select logic. The purpose of this is to
    /// allow double-click selection to work on the TerminalView::highlighted_link even when it
    /// contains spaces. Smart-select never traverses across whitespace.
    smart_select_override: Option<WithinBlock<RangeInclusive<Point>>>,

    bootstrap_stage: BootstrapStage,

    padding: BlockPadding,
    warp_prompt_height_lines: f32,

    /// Executor used for spawning threads in the background.
    background_executor: Arc<Background>,

    show_warp_bootstrap_block: bool,

    show_in_band_command_blocks: bool,

    show_memory_stats: bool,

    active_gap: Option<Gap>,

    honor_ps1: bool,

    is_restored_session: bool,

    restored_session_ts: Option<DateTime<Local>>,

    latest_block_finished_time: Option<SystemTime>,

    /// The number of in-band commands that are "in-flight", where "in-flight" is defined as
    /// written to the PTY without yet a completed block.
    ///
    /// This isn't a simple boolean 'is_in_band_command_in_flight' because it is possible that
    /// with two in-band commands queued in quick succession, the second command may be written
    /// to the PTY before warp_precmd is executed after the first command. `warp_precmd` is used
    /// to emit the `CommandFinished` hook, and so it would be possible to mistakenly mark the
    /// boolean `false`.
    in_flight_in_band_command_count: usize,

    /// The most recently received populated Precmd payload.
    ///
    /// This may be used to initialize the active block if we receive an unpopulated Precmd payload,
    /// as is done after the completion of in-band commands. Since In-band commands are guaranteed
    /// not to modify the context for which information is sent in the Precmd payload, it's not
    /// necessary to recompute the precmd payload after an in-band command runs. Thus we send an
    /// unpopulated precmd payload for in-band commands to make their execution as fast as
    /// possible.
    last_populated_precmd_payload: Option<PrecmdValue>,

    /// Cached data about the prompt that was visible in the input the last time
    /// the user submitted a command.  Some prompt tools update the prompt just
    /// before the shell starts executing the command, and we want to avoid briefly
    /// displaying that "transient" prompt to reduce flicker in the UI.
    ///
    /// This is also used to persist the user-visible prompt across in-band command
    /// blocks; we don't want those blocks to produce an updated prompt in the input,
    /// so we use this data to set the prompt in subsequent blocks.
    cached_prompt_data: Option<CachedPromptData>,

    obfuscate_secrets: ObfuscateSecrets,

    /// `true` if client-side telemetry for user-generated AI data is enabled.
    is_ai_ugc_telemetry_enabled: bool,

    /// Persisted info about the scroll position before a filter is applied. This
    /// data is used return users to their original scroll position after a
    /// filter is removed.
    ///
    /// If a non-filter scroll event occurs, this data is cleared and we don't
    /// return users to their original position when the filter is removed.
    scroll_position_before_filter: Option<BlockScrollPosition>,

    /// Whether the blocklist is inverted (i.e. the input is pinned to the top). This is
    /// relevant wherever we're traversing the blocklist's sumtree (i.e. in clamp_to_grid_points)
    is_inverted: bool,

    agent_view_state: AgentViewState,

    /// The view ID of a rich content item that should always remain at the bottom
    /// of the blocklist. After any other insertion, this item is automatically
    /// removed and re-appended so it stays last.
    pinned_to_bottom: Option<EntityId>,
    is_executing_oz_environment_startup_commands: bool,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for BlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(&self.blocks).finish()
    }
}

/// Filter for finding blocks in the block list.
///
/// The default does not count any zero-height blocks. We use zero-height blocks for
/// various things we don't want to render (i.e. the active block, bootstrap blocks).
/// This can be overridden by setting `include_hidden`.
#[derive(Debug, Clone, Copy)]
pub struct BlockFilter {
    /// Include 0-height hidden blocks (such as the active block and bootstrap blocks).
    pub include_hidden: bool,
    /// Include background output blocks.
    pub include_background: bool,
}

impl BlockFilter {
    /// Tests if a block matches this filter.
    pub fn matches(self, block: &Block, agent_view_state: &AgentViewState) -> bool {
        (self.include_background || !block.is_background())
            && (self.include_hidden || !block.is_empty(agent_view_state))
    }

    /// Block filter for visible command blocks. This excludes background output
    /// blocks and hidden blocks.
    pub fn commands() -> BlockFilter {
        BlockFilter {
            include_background: false,
            include_hidden: false,
        }
    }
}

impl Default for BlockFilter {
    fn default() -> Self {
        Self {
            include_hidden: false,
            include_background: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Gap {
    /// Index into the block heights sum tree.
    index: usize,
    current_height: Lines,
    /// The height of the gap when constructed. This is needed to ensure that the Gap never exceeds
    /// it's original height.
    original_height: Lines,
}

impl Gap {
    /// Returns the height of the gap in lines
    pub fn height(&self) -> Lines {
        self.current_height
    }

    /// Returns the index of the gap in the block heights sum tree
    pub fn index(&self) -> TotalIndex {
        self.index.into()
    }
}

/// A point in the block list coordinate space. Row 0, column 0 is the top left corner of all
/// blocks.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd)]
pub struct BlockListPoint {
    // TODO(vorporeal): Use `Lines` as the type here.
    pub row: Lines,
    pub column: usize,
}

impl BlockListPoint {
    pub fn new(row: impl IntoLines, column: usize) -> Self {
        BlockListPoint {
            row: row.into_lines(),
            column,
        }
    }

    pub fn from_within_block_point(
        within_block_point: &WithinBlock<Point>,
        block_list: &BlockList,
    ) -> Self {
        let mut block_heights_cursor = block_list
            .block_heights
            .cursor::<BlockIndex, BlockHeightSummary>();
        block_heights_cursor.seek(&within_block_point.block_index, SeekBias::Right);

        let block = &block_list.blocks[within_block_point.block_index.0];
        let delta_to_top_of_block = match within_block_point.grid {
            GridType::Output => block.output_grid_offset(),
            GridType::Prompt => block.prompt_grid_offset(),
            GridType::PromptAndCommand => block.prompt_and_command_grid_offset(),
            GridType::Rprompt => block.prompt_grid_offset(),
        };

        let row = within_block_point.get().row.into_lines()
            + delta_to_top_of_block
            + block_heights_cursor.start().height;

        BlockListPoint::new(row, within_block_point.get().col)
    }
}

impl float_cmp::ApproxEq for BlockListPoint {
    type Margin = float_cmp::F64Margin;

    fn approx_eq<M: Into<Self::Margin>>(self, other: Self, margin: M) -> bool {
        // Until we actually store `Lines` inside `BlockListPoint`, we'll do a
        // conversion here in order to make sure the comparison is done with the
        // appropriate epsilon.
        self.row
            .into_lines()
            .approx_eq(other.row.into_lines(), margin)
            && self.column == other.column
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct BlockHeight(Lines);

/// Index/dimension of _all_ the items in the BlockHeights sum tree. This includes blocks, banners,
/// gaps, and anything that could be within the BlockList but isn't necessarily a block.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct TotalIndex(pub usize);

impl std::ops::Add<usize> for TotalIndex {
    type Output = TotalIndex;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct BlockHeightSummary {
    pub total_count: usize,
    pub height: Lines,
    pub block_count: usize,
}

impl BlockHeight {
    pub const fn zero() -> Self {
        Self(Lines::zero())
    }

    pub fn as_f64(self) -> f64 {
        self.0.as_f64()
    }

    pub fn into_lines(self) -> Lines {
        self.0
    }
}

/// Delegate for `BlockList` that delegates the method to either the early output
/// model (if between blocks) or the active block
macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        if $self.is_early_output() {
            EarlyOutput::handler($self).$method($( $arg ),*)
        } else {
            delegate_to_block!($self.$method($( $arg ),*))
        }
    };
}

/// Delegate for `BlockList` that delegates the method to the active
/// block and optionally updates block heights if the active block's height was changed by the
/// method.
macro_rules! delegate_to_block {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        $self.active_block_mut().$method($( $arg ),*)
    };
}

/// Represents an update to the block heights sum tree.
#[derive(Debug)]
enum BlockHeightUpdate {
    /// An item was inserted at the given index.
    Insertion(TotalIndex),

    /// An item, which was previously at the given index, was removed.
    Removal(TotalIndex),
}

struct SharedSessionScrollbackBlocks<'a> {
    completed_blocks: &'a [SerializedBlock],
    active_block: Option<&'a SerializedBlock>,
}

impl<'a> SharedSessionScrollbackBlocks<'a> {
    fn new(scrollback: &'a [SerializedBlock]) -> Self {
        match scrollback.split_last() {
            Some((active_block, completed_blocks)) if active_block.completed_ts.is_none() => Self {
                completed_blocks,
                active_block: Some(active_block),
            },
            _ => Self {
                completed_blocks: scrollback,
                active_block: None,
            },
        }
    }
}

impl BlockList {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        restored_blocks: Option<&[SerializedBlockListItem]>,
        sizes: BlockSize,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        show_warp_bootstrap_input: bool,
        show_in_band_command_blocks: bool,
        show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
        is_ai_ugc_telemetry_enabled: bool,
    ) -> Self {
        let mut block_list = Self::new_internal(
            sizes,
            event_proxy,
            background_executor,
            show_warp_bootstrap_input,
            show_in_band_command_blocks,
            show_memory_stats,
            honor_ps1,
            is_inverted,
            obfuscate_secrets,
            is_ai_ugc_telemetry_enabled,
        );
        block_list.initialize(restored_blocks);
        block_list
    }

    /// Creates a `BlockList` ready to add restored blocks.
    /// Note that at this point, there is no default block yet.
    ///
    /// The lifecycle for the block list should be:
    ///
    /// 1. Create the block list.
    /// 2. Add any restored blocks via `restore_block`. Note that this works
    /// in a different way from the `finalize_block_and_advance_list` function. `restore_block` will take
    /// the block as the input and consider that one whole block to create,
    /// feed input into, and finish whereas `finalize_block_and_advance_list` will create the _subsequent_
    /// block.
    /// 3. Create the `BootstrapStage::WarpInput` block through
    /// `create_warp_input_block`. From here on, there is always a default
    /// block which is hidden until it is started.
    /// 4. We progress through the bootstrap stages with the `finalize_block_and_advance_list` function.
    /// 5. After we hit `BootstrapStage::PostBootstrapPrecmd`, it's normal
    /// execution. `finalize_block_and_advance_list` is still the main function to advance the block list.
    /// 6. If `reinit_shell` is called, we are bootstrapping another shell
    /// session. We would revert back to step 3. The invariant of the default
    /// block always being there is unchanged.
    #[allow(clippy::too_many_arguments)]
    fn new_internal(
        sizes: BlockSize,
        event_proxy: ChannelEventListener,
        background_executor: Arc<Background>,
        show_warp_bootstrap_input: bool,
        show_in_band_command_blocks: bool,
        show_memory_stats: bool,
        honor_ps1: bool,
        is_inverted: bool,
        obfuscate_secrets: ObfuscateSecrets,
        is_ai_ugc_telemetry_enabled: bool,
    ) -> Self {
        let bootstrap_stage = BootstrapStage::RestoreBlocks;
        let block_heights = SumTree::new();
        BlockList {
            blocks: vec![],
            block_heights,
            block_id_to_block_index: HashMap::new(),
            removable_blocklist_item_positions: HashMap::new(),
            active_gap: None,
            dirty_rich_content_items: HashSet::new(),
            size: sizes.size,
            next_gap_height_in_lines: None,
            max_grid_size_limit: sizes.max_block_scroll_limit,
            event_proxy: event_proxy.clone(),
            selection: None,
            smart_select_override: None,
            bootstrap_stage,
            padding: sizes.block_padding,
            warp_prompt_height_lines: sizes.warp_prompt_height_lines,
            background_executor,
            show_warp_bootstrap_block: show_warp_bootstrap_input,
            show_in_band_command_blocks,
            show_memory_stats,
            honor_ps1,
            is_restored_session: false,
            restored_session_ts: None,
            latest_block_finished_time: None,
            early_output: EarlyOutput::new(event_proxy),
            in_flight_in_band_command_count: 0,
            last_populated_precmd_payload: None,
            cached_prompt_data: None,
            obfuscate_secrets,
            is_ai_ugc_telemetry_enabled,
            scroll_position_before_filter: None,
            is_inverted,
            agent_view_state: AgentViewState::Inactive,
            pinned_to_bottom: None,
            is_executing_oz_environment_startup_commands: false,
        }
    }

    /// Must be called before the model is used. Even if no blocks are to be restored,
    /// this is necessary in the BlockList lifecycle.
    fn initialize(&mut self, restored_blocks: Option<&[SerializedBlockListItem]>) {
        if let Some(restored_blocks) = restored_blocks {
            self.is_restored_session = true;

            let mut processor = Processor::new();

            self.restored_session_ts = restored_blocks.last().and_then(|item| match item {
                SerializedBlockListItem::Command { block } => block.completed_ts,
            });

            for block in restored_blocks {
                match block {
                    SerializedBlockListItem::Command { block } => {
                        // For session-restoration, we only want to restore blocks
                        // that were completed.
                        if block.start_ts.is_some() && block.completed_ts.is_some() {
                            self.restore_block(
                                block,
                                BootstrapStage::RestoreBlocks,
                                &mut processor,
                            );
                        } else {
                            log::warn!(
                                "Tried to restore a block that was either not started or not completed"
                            );
                        }
                    }
                }
            }
        }
        self.create_warp_input_block();
        // Note: We no longer call start() here.
        // When shell input arrives, the block will be started (see the `input` handler).
        // This ensures sessions without a shell (like cloude mode) don't permanently trigger is_active_and_long_running()
        // since the block will never be finished.
    }

    pub(super) fn load_shared_session_scrollback(&mut self, scrollback: &[SerializedBlock]) {
        let scrollback_blocks = SharedSessionScrollbackBlocks::new(scrollback);
        // If the snapshot will restore any blocks, finish the placeholder active block first.
        // For an empty snapshot, keep the existing placeholder active block instead of replacing
        // it with another hidden block.
        if !scrollback.is_empty() && !self.active_block().finished() {
            self.active_block_mut().finish(0);
        }

        // Simulate finishing bootstrapping once we get the scrollback.
        self.set_bootstrapped();
        let mut processor: Processor = Processor::new();

        for block in scrollback_blocks.completed_blocks {
            if block.start_ts.is_some() && block.completed_ts.is_some() {
                self.restore_block(block, BootstrapStage::PostBootstrapPrecmd, &mut processor);
            } else {
                log::warn!("A non-active scrollback block was either not started or not completed");
            }
        }
        if let Some(active_block) = scrollback_blocks.active_block {
            self.restore_block(
                active_block,
                BootstrapStage::PostBootstrapPrecmd,
                &mut processor,
            );
        } else {
            self.ensure_active_block_after_shared_session_scrollback();
        }
    }

    pub(super) fn append_followup_shared_session_scrollback(
        &mut self,
        scrollback: &[SerializedBlock],
    ) {
        self.set_bootstrapped();
        let mut processor = Processor::new();
        let scrollback_blocks = SharedSessionScrollbackBlocks::new(scrollback);

        for block in scrollback_blocks.completed_blocks {
            if self.block_index_for_id(&block.id).is_some() {
                continue;
            }
            if block.start_ts.is_some() && block.completed_ts.is_some() {
                self.finish_active_block_before_followup_append();
                self.restore_block(block, BootstrapStage::PostBootstrapPrecmd, &mut processor);
            } else {
                log::warn!("A non-active follow-up scrollback block was either not started or not completed");
            }
        }

        match scrollback_blocks.active_block {
            Some(active_block) if self.block_index_for_id(&active_block.id).is_none() => {
                self.finish_active_block_before_followup_append();
                self.restore_block(
                    active_block,
                    BootstrapStage::PostBootstrapPrecmd,
                    &mut processor,
                );
            }
            Some(_) | None => {
                self.ensure_active_block_after_shared_session_scrollback();
            }
        }
    }

    fn finish_active_block_before_followup_append(&mut self) {
        if !self.active_block().finished() {
            self.active_block_mut().finish(0);
            self.update_active_block_height();
        }
    }

    fn ensure_active_block_after_shared_session_scrollback(&mut self) {
        if self.active_block().finished() {
            self.create_new_block(
                BlockId::new(),
                BootstrapStage::PostBootstrapPrecmd,
                None,
                None,
            );
        }
    }

    /// This is an important function in the block list lifecycle. After this
    /// is called, there's an invariant where we always have an active block
    /// that's hidden until it's `start`ed.
    fn create_warp_input_block(&mut self) {
        self.create_new_block(
            BlockId::new(),
            BootstrapStage::WarpInput,
            Default::default(),
            None,
        );
        self.bootstrap_stage = BootstrapStage::WarpInput;
    }

    pub fn restored_session_ts(&self) -> &Option<DateTime<Local>> {
        &self.restored_session_ts
    }

    pub fn is_restored_session(&self) -> bool {
        self.is_restored_session
    }

    pub(super) fn grid_handler_within_block<T>(
        &self,
        secret: &WithinBlock<T>,
    ) -> anyhow::Result<&GridHandler> {
        let block = self
            .blocks
            .get(secret.block_index.0)
            .ok_or_else(|| anyhow!("error finding block"))?;
        let block_grid = block
            .grid_of_type(secret.grid)
            .ok_or_else(|| anyhow!("error finding block grid"))?;
        Ok(block_grid.grid_handler())
    }

    pub(super) fn grid_handler_mut_within_block<T>(
        &mut self,
        secret: &WithinBlock<T>,
    ) -> anyhow::Result<&mut GridHandler> {
        let block = self
            .blocks
            .get_mut(secret.block_index.0)
            .ok_or_else(|| anyhow!("error finding block"))?;
        let block_grid = block
            .grid_of_type_mut(secret.grid)
            .ok_or_else(|| anyhow!("error finding block grid"))?;
        Ok(block_grid.grid_handler_mut())
    }

    pub fn set_honor_ps1(&mut self, honor_ps1: bool) {
        self.honor_ps1 = honor_ps1;
        self.active_block_mut().set_honor_ps1(honor_ps1);
    }

    pub fn set_is_inverted(&mut self, is_inverted: bool) {
        self.is_inverted = is_inverted;
    }

    pub fn update_max_grid_size(&mut self, new_size: usize) {
        self.max_grid_size_limit = new_size;
    }

    pub fn active_block_index(&self) -> BlockIndex {
        (self.blocks.len() - 1).into()
    }

    pub fn active_block(&self) -> &Block {
        self.blocks.last().expect("at least one block should exist")
    }

    pub fn active_block_id(&self) -> &BlockId {
        self.active_block().id()
    }

    fn next_gap_height(&self) -> Option<Lines> {
        self.next_gap_height_in_lines
    }

    /// Returns the active gap, if there is one.
    pub fn active_gap(&self) -> Option<&Gap> {
        self.active_gap.as_ref()
    }

    /// Clears the visible screen--moving everything that's currently visible into scrollback.
    pub fn clear_visible_screen(&mut self) {
        self.finish_background_block();
        let mut new_sum_tree = SumTree::new();

        // Remove all gaps from the sum tree.
        for block in self.block_heights.cursor::<TotalIndex, ()>() {
            if !matches!(block, BlockHeightItem::Gap(_)) {
                new_sum_tree.push(*block);
            }
        }

        self.block_heights = {
            let mut cursor = new_sum_tree.cursor::<BlockIndex, ()>();
            cursor.slice(&BlockIndex(self.blocks.len()), SeekBias::Left)
        };

        // If the active block has started (i.e. is running)--then insert the gap _after_ the block.
        // If the active block has not started (e.g. the user pressed ctrl-l)--insert the gap
        // _before_ the active block so the next command the user executes is after the gap.
        let gap_height = if let Some(height) = self.next_gap_height() {
            height
        } else {
            log::error!("Expected gap height to be set before clear");
            // Since the gap was removed from the block_heights tree, clear active_gap.
            // We do not expect to be in this state, but if we are, we shouldn't
            // leave the model inconsistent.
            self.active_gap = None;
            return;
        };

        let gap = BlockHeightItem::Gap(gap_height.into());
        let agent_view_state = self.agent_view_state.clone();
        let active_block_height = self.active_block_mut().height(&agent_view_state).into();

        if self.active_block().started() {
            self.block_heights
                .push(BlockHeightItem::Block(active_block_height));
            self.block_heights.push(gap);

            self.active_gap = Some(Gap {
                index: self.block_heights.summary().total_count - 1,
                current_height: gap_height,
                original_height: gap_height,
            });
        } else {
            self.block_heights.push(gap);
            self.active_gap = Some(Gap {
                index: self.block_heights.summary().total_count - 1,
                current_height: gap_height,
                original_height: gap_height,
            });

            self.block_heights
                .push(BlockHeightItem::Block(active_block_height));
        }

        self.event_proxy.send_terminal_event(TerminalClear);
    }

    #[cfg(feature = "local_fs")]
    pub(in crate::terminal) fn append_session_restoration_separator_to_block_list(
        &mut self,
        is_historical_conversation_restoration: bool,
    ) {
        self.insert_non_block_item_before_block(
            self.active_block_index(),
            BlockHeightItem::RestoredBlockSeparator {
                height_when_visible: BlockHeight::from(RESTORED_BLOCK_SEPARATOR_HEIGHT),
                is_historical_conversation_restoration,
                is_hidden: false,
            },
        );
    }

    /// Inserts an inline banner _before_ the provided block_index.
    pub fn insert_inline_banner_before_block(
        &mut self,
        index: BlockIndex,
        banner: InlineBannerItem,
        height: Option<f64>,
    ) {
        let height = BlockHeight::from(height.unwrap_or(INLINE_BANNER_HEIGHT));
        let inserted_index = self.insert_non_block_item_before_block(
            index,
            BlockHeightItem::InlineBanner {
                banner,
                height_when_visible: height,
                is_hidden: false,
            },
        );
        self.removable_blocklist_item_positions.insert(
            RemovableBlocklistItem::InlineBanner(banner.id),
            inserted_index,
        );
    }

    /// Inserts an inline banner _after_ the provided block_index.
    pub fn insert_inline_banner_after_block(
        &mut self,
        index: BlockIndex,
        banner: InlineBannerItem,
    ) {
        // Inserting right after `index` is equivalent to inserting right before index+1.
        self.insert_inline_banner_before_block(index + BlockIndex(1), banner, None);
    }

    /// Appends an inline banner to the blocklist.
    ///
    /// If there is no long-running command, the banner is inserted after the last non-hidden block.
    /// If there is a long-running block, the banner will be inserted before that block.
    /// This is intentional to avoid situations where there is a banner
    /// "pinned" to the bottom of the blocklist while a block changes above it.
    pub fn append_inline_banner(&mut self, banner: InlineBannerItem) {
        self.insert_inline_banner_before_block(self.active_block_index(), banner, None)
    }

    pub fn append_inline_banner_with_custom_height(
        &mut self,
        banner: InlineBannerItem,
        height: f64,
    ) {
        self.insert_inline_banner_before_block(self.active_block_index(), banner, Some(height))
    }

    /// Appends an inline banner to the blocklist.
    ///
    /// If there is no long-running command, the banner is inserted after the last non-hidden block.
    /// If there is a long-running block, the banner will be inserted after that block.
    pub fn append_inline_banner_after_long_running(&mut self, banner: InlineBannerItem) {
        if self.active_block().is_active_and_long_running() {
            self.insert_inline_banner_after_block(self.active_block_index(), banner)
        } else {
            self.insert_inline_banner_before_block(self.active_block_index(), banner, None)
        }
    }

    pub fn append_subshell_separator(
        &mut self,
        separator_id: SeparatorId,
        subshell_separator_height: f32,
    ) {
        self.insert_non_block_item_before_block(
            self.active_block_index(),
            BlockHeightItem::SubshellSeparator {
                separator_id,
                height_when_visible: BlockHeight::from(subshell_separator_height),
                is_hidden: false,
            },
        );
    }

    /// Insert a rich content `View` at the end of the BlockList.
    pub fn append_rich_content(
        &mut self,
        item: RichContentItem,
        insert_below_long_running_block: bool,
    ) {
        let view_id = item.view_id;
        let insertion_index = if self.active_block().is_active_and_long_running()
            && insert_below_long_running_block
        {
            self.append_item_to_blocklist(BlockHeightItem::RichContent(item))
        } else {
            // If there's no long-running block, then the active block is a default block that is hidden
            // until it's started. This is an invariant of the blocklist (see create_warp_input_block). In this
            // case, we should add the rich content above that hidden block.
            self.insert_non_block_item_before_block(
                self.active_block_index(),
                BlockHeightItem::RichContent(item),
            )
        };
        self.removable_blocklist_item_positions.insert(
            RemovableBlocklistItem::RichContent(view_id),
            insertion_index,
        );
        self.mark_rich_content_dirty(view_id);
        self.maintain_pinned_to_bottom();
    }

    /// Insert a rich content `View` at the end of the BlockList and pin it so
    /// it is automatically kept at the bottom after any subsequent insertion.
    /// Only one item can be pinned at a time; calling this replaces any
    /// existing pin.
    pub fn append_rich_content_pinned_to_bottom(&mut self, item: RichContentItem) {
        let view_id = item.view_id;
        self.append_rich_content(item, true);
        self.pinned_to_bottom = Some(view_id);
    }

    pub(in crate::terminal) fn unpin_rich_content_from_bottom(&mut self, view_id: EntityId) {
        if self.pinned_to_bottom == Some(view_id) {
            self.pinned_to_bottom = None;
        }
    }

    /// If a rich content item is pinned to the bottom, removes it from its
    /// current position and re-appends it so it remains last in the blocklist.
    fn maintain_pinned_to_bottom(&mut self) {
        // Take the pin to prevent re-entrant calls during the re-append.
        let Some(pinned_id) = self.pinned_to_bottom.take() else {
            return;
        };

        let Some(&index) = self
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(pinned_id))
        else {
            return;
        };

        // Read the item data from the sum tree before removing it.
        let item = {
            let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
            cursor.seek(&index, SeekBias::Right);
            match cursor.item() {
                Some(BlockHeightItem::RichContent(item)) => *item,
                _ => return,
            }
        };

        // Remove from current position and re-append at the bottom.
        // pinned_to_bottom is None during this call, so append_rich_content's
        // call to maintain_pinned_to_bottom is a no-op (no recursion).
        self.remove_rich_content(pinned_id);
        self.append_rich_content(item, true);

        // Restore the pin.
        self.pinned_to_bottom = Some(pinned_id);
    }

    fn append_item_to_blocklist(&mut self, item: BlockHeightItem) -> TotalIndex {
        self.finish_background_block();

        let inserted_index = TotalIndex(self.block_heights.summary().total_count);

        self.block_heights.push(item);

        self.update_block_height_indices(
            BlockHeightUpdate::Insertion(inserted_index),
            matches!(
                item,
                BlockHeightItem::InlineBanner { .. }
                    | BlockHeightItem::RichContent(_)
                    | BlockHeightItem::SubshellSeparator { .. }
            ),
        );

        // Force a re-draw since the blocklist has changed.
        self.event_proxy.send_wakeup_event();

        inserted_index
    }

    pub fn remove_rich_content(&mut self, view_id: EntityId) {
        self.dirty_rich_content_items.remove(&view_id);
        self.remove_item_from_blocklist(RemovableBlocklistItem::RichContent(view_id));
        if self.pinned_to_bottom == Some(view_id) {
            self.pinned_to_bottom = None;
        }
    }

    pub fn update_agent_view_conversation_id_for_rich_content(
        &mut self,
        rich_content_view_id: EntityId,
        agent_view_conversation_id: Option<AIConversationId>,
    ) {
        let Some(&index) = self
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(rich_content_view_id))
        else {
            return;
        };

        let agent_view_state = &self.agent_view_state;
        self.block_heights = {
            let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
            let mut new_tree = cursor.slice(&index, SeekBias::Right);

            if let Some(BlockHeightItem::RichContent(item)) = cursor.item() {
                let should_hide = RichContentItem {
                    agent_view_conversation_id,
                    ..*item
                }
                .should_hide_for_agent_view_state(agent_view_state);
                new_tree.push(BlockHeightItem::RichContent(RichContentItem {
                    agent_view_conversation_id,
                    should_hide,
                    ..*item
                }));
                cursor.next();
            }

            new_tree.push_tree(cursor.suffix());
            new_tree
        };

        self.event_proxy.send_wakeup_event();
    }

    /// Marks the rich content item with the given view ID as needing its height
    /// to be remeasured on the next layout.
    pub fn mark_rich_content_dirty(&mut self, view_id: EntityId) {
        self.dirty_rich_content_items.insert(view_id);
    }

    /// Takes and clears the set of dirty rich content view IDs.
    pub(in crate::terminal) fn take_dirty_rich_content_items(&mut self) -> HashSet<EntityId> {
        std::mem::take(&mut self.dirty_rich_content_items)
    }

    pub fn remove_inline_banner(&mut self, banner_id: InlineBannerId) {
        self.remove_item_from_blocklist(RemovableBlocklistItem::InlineBanner(banner_id));
    }

    /// Removes a removable item from the blocklist. This supports removing inline banners and rich content blocks.
    fn remove_item_from_blocklist(&mut self, item: RemovableBlocklistItem) {
        if let Some(index) = self.removable_blocklist_item_positions.remove(&item) {
            self.block_heights = {
                let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
                let mut tree_before_banner = cursor.slice(&index, SeekBias::Right);

                if !matches!(
                    cursor.item(),
                    Some(BlockHeightItem::InlineBanner { .. } | BlockHeightItem::RichContent(_))
                ) {
                    log::warn!("no inline banner or rich content block at the index to remove");
                    return;
                }

                cursor.next();

                tree_before_banner.push_tree(cursor.suffix());
                tree_before_banner
            };

            self.update_block_height_indices(BlockHeightUpdate::Removal(index), true);

            // Force a re-draw since the blocklist has changed.
            self.event_proxy.send_wakeup_event();
        }
    }

    /// Update indices in the block heights SumTree after an item is removed or added.
    ///
    /// We index into the SumTree (using [`TotalIndex`]) for inline banners
    /// and the active gap - if an item is removed / added, all indices after it are
    /// invalidated. This shifts them down / up by one so they refer to the correct
    /// items.
    ///
    /// Banners/Rich blocks and gaps are indexed differently, so the caller must indicate if
    /// a banner/rich block or gap was removed.
    ///
    /// If the item being removed / added is the active gap, this should be called after
    /// clearing `self.active_gap`, and `is_not_active_gap` should be set to false.
    fn update_block_height_indices(
        &mut self,
        block_height_update: BlockHeightUpdate,
        is_not_active_gap: bool,
    ) {
        // Adjust the indices of any following banners in the SumTree to reflect the update.
        self.removable_blocklist_item_positions
            .values_mut()
            .for_each(|pos| {
                pos.0 = match block_height_update {
                    BlockHeightUpdate::Insertion(i) if *pos > i => pos.0 + 1,
                    BlockHeightUpdate::Removal(i) if *pos > i => pos.0 - 1,
                    _ => pos.0,
                };
            });

        // If the active gap index changed as a result of the update, update the active gap index.
        if let Some(active_gap) = self.active_gap.as_mut() {
            // Not clearing active_gap first is a usage error because we use `>=`
            // in the comparison below - if we're updating the gap, that condition
            // will always match. If the gap were at index 0, that could cause an
            // integer underflow which could panic.
            debug_assert!(
                is_not_active_gap,
                "Must clear active_gap before updating indices. Not doing so is a usage error that could cause integer underflow which could panic. See usage of 'is_not_active_gap' for more detail."
            );
            // Use `>=` instead of `>` because gaps and banners use TotalIndex slightly differently.
            // When we seek into block_heights by TotalIndex, we're comparing it to
            // BlockHeightSummary.total_count, the count of items up to the current cursor position.
            //
            // Suppose the block_heights SumTree contains these height items:
            // block -> block -> inline_banner -> gap -> block
            //
            // With inline banners, we use SeekBias::Left, and the cursor should
            // be at the arrow just _after_ the banner. In this case, that's
            // 3, and the index is the count of items up to and including the
            // banner (alternatively, banner locations are 1-indexed).
            //
            // With the active gap, however, we use SeekBias::Right, and the cursor
            // should be at the arrow just _before_ the gap. That's the exact
            // same arrow as for the banner, so in this case the banner's index
            // is the same as the active gap index. The active gap index is
            // the count of items _before_ the gap, as if the gap were 0-indexed.
            // This is why we need `>=`, since if the gap is just after the
            // banner, they have the same index.
            let active_gap_index = active_gap.index;
            active_gap.index = match block_height_update {
                BlockHeightUpdate::Insertion(i) if active_gap_index >= i.0 => active_gap_index + 1,
                BlockHeightUpdate::Removal(i) if active_gap_index >= i.0 => active_gap_index - 1,
                _ => active_gap_index,
            };
        }
    }

    /// Inserts the `item` into the blocklist at the given `index`.
    /// We only want to use this in the block list lifecycle after
    /// `create_warp_input_block`. For non-block items before then, we should
    /// insert the item directly into the sumtree.
    /// Returns the inserted index (according to the TotalCount dimension).
    fn insert_non_block_item_before_block(
        &mut self,
        index: BlockIndex,
        item: BlockHeightItem,
    ) -> TotalIndex {
        self.finish_background_block();

        let (new_tree, inserted_index) = {
            let mut cursor = self.block_heights.cursor::<BlockIndex, ()>();

            // Position the cursor so that the tree includes everything except the block at index.
            // To do so, we need to seek the cursor up to right before the index and then use a SeekBias::Right
            // to clamp it between (index-1, index).
            let mut prefix = cursor.slice(&index, SeekBias::Right);

            // The index of the inserted item will be the count before insertion.
            let insertion_index = TotalIndex(prefix.summary().total_count);

            // Add the new item.
            prefix.push(item);

            // Push back the rest of the tree.
            prefix.push_tree(cursor.suffix());

            (prefix, insertion_index)
        };

        self.block_heights = new_tree;

        // Make sure we adjust other block height indices.
        self.update_block_height_indices(
            BlockHeightUpdate::Insertion(inserted_index),
            matches!(
                item,
                BlockHeightItem::InlineBanner { .. }
                    | BlockHeightItem::RichContent(_)
                    | BlockHeightItem::SubshellSeparator { .. }
            ),
        );

        // Force a re-draw since the blocklist has changed.
        self.event_proxy.send_wakeup_event();

        inserted_index
    }

    /// If the block is hidden, show it. If it's visible, hide it. See implementation
    /// of Block::hidden for how this works.
    pub fn toggle_visibility_of_block(&mut self, block_id: &BlockId) -> Option<bool> {
        if let Some(block) = self.mut_block_from_id(block_id) {
            let is_visible = !block.toggle_hidden();

            // Force a re-draw since the blocklist has changed.
            self.event_proxy.send_wakeup_event();
            return Some(is_visible);
        }
        None
    }

    pub fn unhide_block(&mut self, block_id: &BlockId) {
        if let Some(block) = self.mut_block_from_id(block_id) {
            block.unhide();

            // Force a re-draw since the blocklist has changed.
            self.event_proxy.send_wakeup_event();
        }
    }

    pub fn is_executing_oz_environment_startup_commands(&self) -> bool {
        self.is_executing_oz_environment_startup_commands
    }

    pub fn set_is_executing_oz_environment_startup_commands(
        &mut self,
        is_executing_startup_commands: bool,
    ) {
        self.is_executing_oz_environment_startup_commands = is_executing_startup_commands;
        if is_executing_startup_commands {
            self.active_block_mut().hide();
            self.active_block_mut()
                .set_is_oz_environment_startup_command(true);
        } else {
            self.active_block_mut().unhide();
            self.active_block_mut()
                .set_is_oz_environment_startup_command(false);
        }
    }

    pub fn finish_oz_environment_startup_commands_at_block(
        &mut self,
        block_id: &BlockId,
        conversation_id: Option<AIConversationId>,
    ) {
        self.is_executing_oz_environment_startup_commands = false;
        if let Some(block_index) = self.block_index_for_id(block_id) {
            for block in self.blocks.iter_mut().skip(block_index.0) {
                if block.is_background() || block.is_static() {
                    continue;
                }
                block.unhide();
                block.set_is_oz_environment_startup_command(false);
                if let Some(conversation_id) = conversation_id {
                    block.add_attached_conversation_id(conversation_id);
                }
            }
        }
        self.update_blocks_and_sumtree(None, None, |_| {}, |_| {});
    }

    /// Resets the internal block object's index to its actual index in the block list.
    /// This does not move the block, but is necessary to be called after a move (inserting or removing blocks).
    /// Also updates the block ID to block index mapping.
    fn reset_internal_block_index(&mut self, index: BlockIndex) {
        let Some(block) = self.blocks.get_mut(index.0) else {
            return;
        };
        block.reset_index(index);
        self.block_id_to_block_index
            .insert(block.id().clone(), index);
    }

    /// Remove the background output block, if it exists.
    pub(super) fn remove_background_block(&mut self) -> Option<Block> {
        let block_index = self.background_block_mut()?.index();

        let block = self.blocks.remove(block_index.0);
        self.block_id_to_block_index.remove(block.id());
        // Shift down the index of any blocks after the removed one.
        for index in BlockIndex::range_as_iter(block_index..BlockIndex(self.blocks.len())) {
            self.reset_internal_block_index(index);
        }

        let (new_heights, removed_index) = {
            let mut cursor = self.block_heights.cursor::<BlockIndex, TotalIndex>();
            let mut tree_before_block =
                cursor.slice(&(block_index + BlockIndex(1)), SeekBias::Left);
            let removed_index = *cursor.start();
            // Skip past the block being removed.
            cursor.next();
            tree_before_block.push_tree(cursor.suffix());
            (tree_before_block, removed_index)
        };
        self.block_heights = new_heights;

        // It's unlikely that they exist, but if there are any non-block items
        // after the removed block, we must update tracking information for them.
        self.update_block_height_indices(BlockHeightUpdate::Removal(removed_index), true);

        Some(block)
    }

    fn remove_block_at_index(&mut self, block_index: BlockIndex) -> Option<Block> {
        debug_assert!(block_index != self.active_block_index());

        let block = self.blocks.remove(block_index.0);
        self.block_id_to_block_index.remove(block.id());

        // Shift down the index of any blocks after the removed one.
        for index in BlockIndex::range_as_iter(block_index..BlockIndex(self.blocks.len())) {
            self.reset_internal_block_index(index);
        }

        let (new_heights, removed_index) = {
            let mut cursor = self.block_heights.cursor::<BlockIndex, TotalIndex>();
            let mut tree_before_block =
                cursor.slice(&(block_index + BlockIndex(1)), SeekBias::Left);
            let removed_index = *cursor.start();
            // Skip past the block being removed.
            cursor.next();
            tree_before_block.push_tree(cursor.suffix());
            (tree_before_block, removed_index)
        };
        self.block_heights = new_heights;

        // It's unlikely that they exist, but if there are any non-block items
        // after the removed block, we must update tracking information for them.
        self.update_block_height_indices(BlockHeightUpdate::Removal(removed_index), true);

        Some(block)
    }

    pub fn clear_user_executed_command_blocks_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
    ) {
        let active_block_index = self.active_block_index();

        let mut indices_to_remove = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            let index: BlockIndex = i.into();
            if index == active_block_index {
                continue;
            }

            // Only clear blocks that were created inside this agent view conversation. Blocks
            // created in the top-level terminal (even if later attached as context) should not be
            // removed by a conversation-scoped clear.
            match block.agent_view_visibility() {
                AgentViewVisibility::Agent {
                    origin_conversation_id: block_conversation_id,
                    ..
                } => {
                    if block_conversation_id != &conversation_id {
                        continue;
                    }
                }
                AgentViewVisibility::Terminal { .. } => continue,
            }

            // Skip agent-requested command blocks.
            if block.requested_command_action_id().is_some() {
                continue;
            }

            // Only clear blocks that are currently visible in the agent view.
            if block.is_empty(&self.agent_view_state) {
                continue;
            }

            indices_to_remove.push(index);
        }

        if indices_to_remove.is_empty() {
            return;
        }

        self.clear_selection();
        self.clear_smart_select_override();
        self.clear_scroll_position_before_filter();

        // Remove in reverse order so indices remain valid.
        for index in indices_to_remove.into_iter().rev() {
            self.remove_block_at_index(index);
        }

        // Force a re-draw since the blocklist has changed.
        self.event_proxy.send_wakeup_event();
    }

    /// Gets the active background block, if one exists.
    pub(super) fn background_block_mut(&mut self) -> Option<&mut Block> {
        // The active background block will be the one immediately before
        // the active block, skipping over any in-band commands.
        let mut index = self.active_block_index().0.checked_sub(1)?;
        while self.blocks[index].is_for_in_band_command {
            index = index.checked_sub(1)?;
        }

        // Indexing, here and above, is safe because we start with an in-range
        // value (active_block_index), and only decrease it, stopping at 0.
        let block = &mut self.blocks[index];
        if block.is_background() && !block.finished() {
            Some(block)
        } else {
            None
        }
    }

    /// The setter for Block::block_banner needs to update the block_heights SumTree in order to
    /// keep that data structure in sync.
    pub(in crate::terminal) fn set_active_block_banner(
        &mut self,
        block_banner: Option<WithinBlockBanner>,
    ) {
        self.active_block_mut().block_banner = block_banner;
        self.update_active_block_height();
    }

    pub fn active_block_mut(&mut self) -> &mut Block {
        self.blocks
            .last_mut()
            .expect("Blocklist should not be empty")
    }

    /// Returns a mutable reference to the active block. The active block should
    /// not be directly mutated outside of tests, as doing so can put the block
    /// list in an invalid state.
    #[cfg(test)]
    pub fn active_block_for_test(&mut self) -> &mut Block {
        self.active_block_mut()
    }

    pub fn mut_block_from_id(&mut self, id: &BlockId) -> Option<&mut Block> {
        self.block_index_for_id(id)
            .and_then(|index| self.blocks.get_mut(index.0))
    }

    pub fn update_active_block_height(&mut self) {
        self.update_live_block_height(self.active_block_index());
    }

    /// Update the height of the currently-active background block in the block
    /// heights SumTree.
    pub fn update_background_block_height(&mut self) {
        if let Some(block_idx) = self.background_block_mut().map(|block| block.index()) {
            self.update_live_block_height(block_idx);
        }
    }

    pub fn agent_view_state(&self) -> &AgentViewState {
        &self.agent_view_state
    }

    /// Sets the agent view state for this blocklist.
    ///
    /// With `FeatureFlag::AgentView` enabled, if the state is active, only blocks corresponding to
    /// the active state's conversation ID are rendered. If inactive, only blocks with no conversation
    /// ID (i.e. those executed in the top-level terminal context) are rendered.
    ///
    /// Do not call this method directly. Instead, use the `AgentViewController` to enter/exit the
    /// agent view.
    pub fn set_agent_view_state(&mut self, state: AgentViewState) {
        self.agent_view_state = state;
        if !self.active_block().finished() {
            if let Some(id) = self.agent_view_state.active_conversation_id() {
                // For inline agent views, add the conversation ID to Terminal variant
                // instead of replacing with Agent variant
                if self.agent_view_state.is_inline() {
                    self.active_block_mut().add_attached_conversation_id(id);
                } else {
                    self.active_block_mut().set_conversation_id(id);
                }
            } else {
                // Only clear conversation ID for blocks that were created inside agent view.
                // Terminal blocks with conversation associations should keep them.
                if matches!(
                    self.active_block().agent_view_visibility(),
                    &AgentViewVisibility::Agent { .. }
                ) {
                    self.active_block_mut().clear_conversation_id();
                }
            }
        }

        // AI blocks render with height 0 when hidden for the current agent view state, so mark
        // them dirty to force a re-measure.
        self.mark_agent_view_rich_content_dirty();

        self.update_blocks_and_sumtree(None, None, |_| {}, |_| {});
    }

    /// Marks AI / agent-view rich content as dirty so heights get re-laid out. Call this after
    /// any change that affects which rich content is visible for the current agent view state.
    fn mark_agent_view_rich_content_dirty(&mut self) {
        for (item, index) in &self.removable_blocklist_item_positions {
            if let RemovableBlocklistItem::RichContent(view_id) = item {
                let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
                cursor.seek(index, SeekBias::Right);
                if let Some(BlockHeightItem::RichContent(rich_content)) = cursor.item() {
                    if rich_content.content_type.is_some_and(|content_type| {
                        matches!(
                            content_type,
                            RichContentType::AIBlock
                                | RichContentType::EnterAgentView
                                | RichContentType::InlineAgentViewHeader
                        )
                    }) {
                        self.dirty_rich_content_items.insert(*view_id);
                    }
                }
            }
        }
    }

    pub fn refresh_heights_for_loaded_passive_code_diff(
        &mut self,
        passive_code_diff_block_id: EntityId,
    ) {
        self.mark_rich_content_dirty(passive_code_diff_block_id);
        self.update_blocks_and_sumtree(None, None, |_| {}, |_| {});
    }

    pub fn refresh_block_heights_for_passive_code_diff(&mut self) {}

    /// Associates the given blocks with a conversation, making them visible in that conversation's agent view.
    /// Returns a Vec of (block_id, visibility) for blocks that were found.
    pub fn associate_blocks_with_conversation<'a>(
        &mut self,
        block_ids: impl Iterator<Item = &'a BlockId>,
        conversation_id: AIConversationId,
    ) -> Vec<(BlockId, AgentViewVisibility)> {
        let mut modified_blocks = Vec::new();
        for block_id in block_ids {
            if let Some(block) = self.mut_block_from_id(block_id) {
                if let AgentViewVisibility::Agent {
                    origin_conversation_id,
                    ..
                } = block.agent_view_visibility()
                {
                    if *origin_conversation_id == conversation_id {
                        continue;
                    }
                }
                block.add_pending_conversation_id(conversation_id);
                modified_blocks.push((block_id.clone(), block.agent_view_visibility().clone()));
            }
        }
        modified_blocks
    }

    /// Attaches every non-oz-startup block in the list to `conversation_id` so each block is
    /// visible while that conversation is the active one in agent view. Skips blocks flagged
    /// as `is_oz_environment_startup_command` since those are hidden by their own mechanism.
    pub fn attach_non_startup_blocks_to_conversation(&mut self, conversation_id: AIConversationId) {
        for block in &mut self.blocks {
            if block.is_oz_environment_startup_command() {
                continue;
            }
            if let AgentViewVisibility::Agent {
                origin_conversation_id,
                ..
            } = block.agent_view_visibility()
            {
                if *origin_conversation_id == conversation_id {
                    continue;
                }
            }
            block.add_attached_conversation_id(conversation_id);
        }

        self.mark_agent_view_rich_content_dirty();
        self.update_blocks_and_sumtree(None, None, |_| {}, |_| {});
    }

    /// Removes the conversation association from the given blocks, making them disappear from that conversation's agent view.
    /// Returns a Vec of (block_id, visibility) for blocks that were modified.
    pub fn remove_pending_context_assocation_for_blocks<'a>(
        &mut self,
        block_ids: impl Iterator<Item = &'a BlockId>,
        conversation_id: AIConversationId,
    ) -> Vec<(BlockId, AgentViewVisibility)> {
        let mut modified_blocks = Vec::new();
        for block_id in block_ids {
            if let Some(block) = self.mut_block_from_id(block_id) {
                if block.remove_pending_conversation_id(conversation_id) {
                    modified_blocks.push((block_id.clone(), block.agent_view_visibility().clone()));
                }
            }
        }
        modified_blocks
    }

    /// Promotes all blocks that are pending for the given conversation to attached.
    /// Returns a Vec of (block_id, visibility) for blocks that were modified.
    pub fn promote_blocks_to_attached_from_conversation(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<(BlockId, AgentViewVisibility)> {
        let mut modified_blocks = Vec::new();
        for block in &mut self.blocks {
            if block.promote_pending_to_attached(conversation_id) {
                modified_blocks.push((block.id().clone(), block.agent_view_visibility().clone()));
            }
        }
        modified_blocks
    }

    /// Update the height of an active block in the block heights SumTree. In general,
    /// blocks are immutable once finished. Only the active block and the most
    /// recent background output block can have their heights updated.
    fn update_live_block_height(&mut self, block_index: BlockIndex) {
        debug_assert!(
            block_index == self.active_block_index()
                || self
                    .block_at(block_index)
                    .is_some_and(|block| block.is_background()),
            "Can only update height for the active block and latest background block"
        );

        // Gaps are created via ctrl-l binding or `clear`, and shrink as more blocks are executed after.

        // With eg. `clear`, a block_heights re-calculation notices a difference in the total block list height after execution
        // from the "clear" block and reduces the gap, thinking "clear" is a block executed after. This means the "clear" block,
        // above the gap, is not fully cleared from the visible screen.
        // => By calculating the delta in height only after the gap, the gap will only shrink from blocks executed _after_ the gap.
        let previous_after_gap_height = match &self.active_gap {
            Some(gap) => {
                let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
                cursor.seek(&gap.index(), SeekBias::Right);

                match cursor.item() {
                    Some(BlockHeightItem::Gap(_)) => {
                        cursor.next();
                        let tree_after_gap = cursor.suffix();
                        tree_after_gap.summary().height
                    }
                    _ => Lines::zero(),
                }
            }
            None => Lines::zero(),
        };
        let block_height = if let Some(block) = self.block_at(block_index) {
            block.height(&self.agent_view_state).into()
        } else {
            log::error!(
                "Tried to update height of block at {block_index:?}, but no such block exists"
            );
            return;
        };

        self.block_heights = {
            let mut cursor = self.block_heights.cursor::<BlockIndex, ()>();
            let next_index = block_index + BlockIndex(1);
            let mut tree_before_last_block = cursor.slice(&next_index, SeekBias::Left);
            tree_before_last_block.push(BlockHeightItem::Block(block_height));

            // Advance the cursor past the current block and take the suffix to get all the items
            // after the active block.
            cursor.next();
            let suffix = cursor.suffix();

            tree_before_last_block.push_tree(suffix);
            tree_before_last_block
        };

        let mut removed_gap_index = None;

        let updated_tree_with_gap = self.active_gap.take().and_then(|active_gap| {
            let gap_index = active_gap.index();

            // First get the subtree before the gap.
            let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
            let mut tree_before_gap = cursor.slice(&gap_index, SeekBias::Right);

            if let Some(BlockHeightItem::Gap(height)) = cursor.item() {
                // Get the tree after the gap.
                cursor.next();
                let tree_after_gap = cursor.suffix();
                let height_added = tree_after_gap.summary().height - previous_after_gap_height;

                let new_gap_height = (height.0 - height_added)
                    .max(Lines::zero())
                    .min(active_gap.original_height);

                // Only keep the gap if it has a non-zero height.
                if new_gap_height > Lines::zero() {
                    tree_before_gap.push(BlockHeightItem::Gap(BlockHeight(new_gap_height)));
                    tree_before_gap.push_tree(tree_after_gap);
                    self.active_gap = Some(Gap {
                        index: active_gap.index,
                        current_height: new_gap_height,
                        original_height: active_gap.original_height,
                    });
                } else {
                    removed_gap_index = Some(gap_index);
                    tree_before_gap.push_tree(tree_after_gap);
                }

                Some(tree_before_gap)
            } else {
                log::error!("a gap is not contained at the active gap index");
                None
            }
        });

        if let Some(new_tree) = updated_tree_with_gap {
            self.block_heights = new_tree;
        }

        if let Some(removed_index) = removed_gap_index {
            self.update_block_height_indices(BlockHeightUpdate::Removal(removed_index), false);
        }
    }

    pub fn blocks(&self) -> &Vec<Block> {
        &self.blocks
    }

    pub fn blocks_mut(&mut self) -> &mut Vec<Block> {
        &mut self.blocks
    }

    pub fn block_with_id(&self, id: &BlockId) -> Option<&Block> {
        self.block_index_for_id(id)
            .and_then(|idx| self.block_at(idx))
    }

    pub fn block_at(&self, index: BlockIndex) -> Option<&Block> {
        self.blocks().get(index.0)
    }

    pub fn block_at_mut(&mut self, index: BlockIndex) -> Option<&mut Block> {
        self.blocks.get_mut(index.0)
    }

    /// Returns None if the block ID doesn't exist.
    pub fn block_index_for_id(&self, id: &BlockId) -> Option<BlockIndex> {
        self.block_id_to_block_index.get(id).copied()
    }

    pub fn block_for_ai_action_id(&self, id: &AIAgentActionId) -> Option<&Block> {
        self.blocks.iter().find(|block| {
            block.agent_interaction_metadata().is_some_and(|metadata| {
                metadata
                    .requested_command_action_id()
                    .is_some_and(|action_id| action_id == id)
            })
        })
    }

    /// Scans the block at `block_index` for secrets.
    pub fn scan_block_for_secrets(&mut self, block_index: BlockIndex) {
        if let Some(block) = self.blocks.get_mut(block_index.0) {
            block.scan_full_block_for_secrets();
        }
    }

    pub fn block_heights(&self) -> &SumTree<BlockHeightItem> {
        &self.block_heights
    }

    pub fn is_requested_command_block_immediately_after_ai_block(
        &self,
        ai_block_id: EntityId,
        block_requested_command_action_id: &AIAgentActionId,
    ) -> bool {
        let Some(ai_block_total_idx) = self
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(ai_block_id))
        else {
            return false;
        };
        let mut cursor = self
            .block_heights
            .cursor::<TotalIndex, BlockHeightSummary>();
        cursor.seek(ai_block_total_idx, SeekBias::Right);
        cursor.next();
        let Some(BlockHeightItem::Block(..)) = cursor.item() else {
            return false;
        };
        let block_idx = cursor.start().block_count;
        self.block_at(block_idx.into()).is_some_and(|block| {
            block.agent_interaction_metadata().is_some_and(|metadata| {
                metadata
                    .requested_command_action_id()
                    .is_some_and(|action_id| action_id == block_requested_command_action_id)
            })
        })
    }

    /// Finds the first block out of the given indices that matches the filter.
    /// This function respects the blocklist ordering, regardless of whether it renders as inverted.
    fn first_matching_block_by_index_in_list<I>(
        &self,
        filter: BlockFilter,
        block_indices: I,
    ) -> Option<BlockIndex>
    where
        I: IntoIterator<Item = BlockIndex>,
    {
        block_indices.into_iter().find(|index| {
            self.block_at(*index)
                .is_some_and(|block| filter.matches(block, &self.agent_view_state))
        })
    }

    /// Returns the first index after from_index of a block that matches the filter.
    /// This function respects the blocklist ordering, regardless of whether it renders as inverted.
    pub fn next_matching_block_from_index(
        &self,
        filter: BlockFilter,
        from_index: BlockIndex,
    ) -> Option<BlockIndex> {
        self.first_matching_block_by_index_in_list(
            filter,
            (from_index.0 + 1..self.blocks().len()).map(BlockIndex::from),
        )
    }

    /// Returns the first index after from_index of a non-hidden block.
    pub fn next_non_hidden_block_from_index(&self, from_index: BlockIndex) -> Option<BlockIndex> {
        self.next_matching_block_from_index(BlockFilter::default(), from_index)
    }

    /// Returns the first index before from_index of a block that matches the filter.
    pub fn prev_matching_block_from_index(
        &self,
        filter: BlockFilter,
        from_index: BlockIndex,
    ) -> Option<BlockIndex> {
        self.first_matching_block_by_index_in_list(
            filter,
            (0..from_index.0).rev().map(BlockIndex::from),
        )
    }

    /// Returns the first index before from_index of a block that is non-hidden.
    /// This function respects the blocklist ordering, regardless of whether it renders as inverted.
    pub fn prev_non_hidden_block_from_index(&self, from_index: BlockIndex) -> Option<BlockIndex> {
        self.prev_matching_block_from_index(BlockFilter::default(), from_index)
    }

    /// Returns the first block index, of all blocks, that matches the filter.
    /// This function respects the blocklist ordering, regardless of whether it renders as inverted.
    pub fn first_matching_block_by_index(&self, filter: BlockFilter) -> Option<BlockIndex> {
        self.first_matching_block_by_index_in_list(
            filter,
            (0..self.blocks().len()).map(BlockIndex::from),
        )
    }

    /// Returns the first block index, of all blocks, that is non-hidden.
    pub fn first_non_hidden_block_by_index(&self) -> Option<BlockIndex> {
        self.first_matching_block_by_index(BlockFilter::default())
    }

    /// Returns the last block index, of all blocks, that matches the filter.
    pub fn last_matching_block_by_index(&self, filter: BlockFilter) -> Option<BlockIndex> {
        self.prev_matching_block_from_index(filter, self.blocks.len().into())
    }

    /// Returns the last block index, of all blocks, that is non-hidden.
    /// This function respects the blocklist ordering, regardless of whether it renders as inverted.
    pub fn last_non_hidden_block_by_index(&self) -> Option<BlockIndex> {
        self.last_matching_block_by_index(BlockFilter::default())
    }

    pub fn last_non_hidden_block(&self) -> Option<&Block> {
        if let Some(index) = self.last_non_hidden_block_by_index() {
            return self.block_at(index);
        }

        None
    }

    /// Return the height of the last non hidden rich content block after a block index. If there is no non hidden rich content block, return None.
    pub fn last_non_hidden_rich_content_block_after_block(
        &self,
        block_index: Option<BlockIndex>,
    ) -> Option<(BlockIndex, &RichContentItem)> {
        let mut cursor = self.block_heights().cursor::<BlockHeight, BlockIndex>();
        cursor.descend_to_last_item(self.block_heights());

        while let Some(item) = cursor.item() {
            match item {
                BlockHeightItem::RichContent(item)
                    if !item.should_hide && item.last_laid_out_height > BlockHeight::zero() =>
                {
                    return Some((*cursor.start(), item));
                }
                BlockHeightItem::Block(_)
                    if block_index
                        .map(|index| *cursor.start() <= index)
                        .unwrap_or(false) =>
                {
                    return None;
                }
                _ => {
                    cursor.prev();
                }
            }
        }

        None
    }

    pub fn size(&self) -> &SizeInfo {
        &self.size
    }

    /// Sets the next gap height in lines.  This is the size the gap will be if
    /// a terminal clear event is handled
    pub fn set_next_gap_height_in_lines(&mut self, next_gap_height_in_lines: Lines) {
        self.next_gap_height_in_lines = Some(next_gap_height_in_lines);
    }

    /// Resize terminal to new dimensions.  We pass in an optional new gap height
    /// here because resizing may result in a change in gap size.
    pub fn resize(&mut self, size_update: &SizeUpdate, update_old_blocks: bool) {
        let size = size_update.new_size;
        self.size = size;
        if size_update.rows_or_columns_changed() {
            self.clear_selection();
        }

        if let Some(CachedPromptData {
            prompt_grid,
            rprompt_grid,
            ..
        }) = self.cached_prompt_data.as_mut()
        {
            prompt_grid.resize(size);
            rprompt_grid.resize(size);
        }

        let active_block_index = self.active_block_index();
        self.update_blocks_and_sumtree(
            None,
            None,
            move |b| {
                if active_block_index == b.index() || update_old_blocks {
                    b.resize(size);
                }
            },
            move |gap: &mut Gap| {
                if let Some(height) = size_update.new_gap_height {
                    gap.current_height = height;
                }
            },
        );
    }

    /// Helper function to update each block--ensuring the BlockHeights sumtree is appropriately
    /// updated to reflect the change in height. If subshell_separator_height is Some, update
    /// BlockHeightItem::SubshellSeparator to that new value. If it is None, leave it at the
    /// current value.
    fn update_blocks_and_sumtree<F, G>(
        &mut self,
        subshell_separator_height: Option<f32>,
        rich_content_heights: Option<&HashMap<EntityId, f64>>,
        block_update_fn: F,
        gap_update_fn: G,
    ) where
        F: Fn(&mut Block),
        G: Fn(&mut Gap),
    {
        let agent_view_state = &self.agent_view_state;
        self.block_heights = {
            let mut new_sum_tree = SumTree::new();

            let mut block_heights_cursor = self
                .block_heights
                .cursor::<TotalIndex, BlockHeightSummary>();

            // Start from the beginning of the block heights.
            block_heights_cursor.seek(&TotalIndex(0), SeekBias::Left);

            while let Some(item) = block_heights_cursor.item() {
                match item {
                    BlockHeightItem::Block(_) => {
                        let block_index = block_heights_cursor.start().block_count;
                        if let Some(block) = self.blocks.get_mut(block_index) {
                            block_update_fn(block);
                            new_sum_tree.push(BlockHeightItem::Block(
                                block.height(agent_view_state).into(),
                            ));
                        } else {
                            log::error!("invalid block index in block heights");
                        }
                    }
                    BlockHeightItem::Gap(_) => {
                        gap_update_fn(self.active_gap.as_mut().expect("Active gap should exist"));
                        new_sum_tree.push(BlockHeightItem::Gap(
                            self.active_gap
                                .as_ref()
                                .expect("Active gap should exist")
                                .current_height
                                .into(),
                        ));
                    }
                    BlockHeightItem::SubshellSeparator {
                        separator_id,
                        height_when_visible,
                        ..
                    } => {
                        let height_when_visible = subshell_separator_height
                            .map(|h| h.into())
                            .unwrap_or(*height_when_visible);
                        new_sum_tree.push(BlockHeightItem::SubshellSeparator {
                            separator_id: *separator_id,
                            height_when_visible,
                            is_hidden: agent_view_state.is_fullscreen(),
                        });
                    }
                    BlockHeightItem::RichContent(RichContentItem {
                        content_type,
                        view_id,
                        agent_view_conversation_id,
                        last_laid_out_height,
                        ..
                    }) => {
                        let should_hide = RichContentItem {
                            content_type: *content_type,
                            view_id: *view_id,
                            last_laid_out_height: *last_laid_out_height,
                            agent_view_conversation_id: *agent_view_conversation_id,
                            should_hide: false,
                        }
                        .should_hide_for_agent_view_state(agent_view_state);
                        let updated_height = if let Some(updated_height) =
                            rich_content_heights.and_then(|heights| heights.get(view_id))
                        {
                            updated_height
                                .into_pixels()
                                .to_lines(self.size().cell_height_px())
                                .into()
                        } else {
                            *last_laid_out_height
                        };

                        new_sum_tree.push(BlockHeightItem::RichContent(RichContentItem {
                            content_type: *content_type,
                            view_id: *view_id,
                            last_laid_out_height: updated_height,
                            agent_view_conversation_id: *agent_view_conversation_id,
                            should_hide,
                        }));
                    }
                    BlockHeightItem::RestoredBlockSeparator {
                        height_when_visible,
                        is_historical_conversation_restoration,
                        ..
                    } => {
                        new_sum_tree.push(BlockHeightItem::RestoredBlockSeparator {
                            height_when_visible: *height_when_visible,
                            is_historical_conversation_restoration:
                                *is_historical_conversation_restoration,
                            // Don't show restored block separators in the agent view.
                            is_hidden: agent_view_state.is_fullscreen(),
                        });
                    }
                    BlockHeightItem::InlineBanner {
                        banner,
                        height_when_visible: height,
                        ..
                    } => {
                        let is_hidden = agent_view_state.is_fullscreen()
                            && !banner.banner_type.is_visible_in_agent_view();
                        new_sum_tree.push(BlockHeightItem::InlineBanner {
                            banner: *banner,
                            height_when_visible: *height,
                            is_hidden,
                        });
                    }
                }
                block_heights_cursor.next();
            }
            new_sum_tree
        };

        // We also need to update the pending background block (if one
        // exists), as it is not part of the tree.
        if let Some(pending_background_block) =
            self.early_output_mut().pending_background_block_mut()
        {
            block_update_fn(pending_background_block);
        }
    }

    pub fn update_blockheight_items(
        &mut self,
        padding: BlockPadding,
        subshell_separator_height: f32,
    ) {
        self.padding = padding;
        // Clear the selection since the height of the block list changed.
        self.clear_selection();
        self.update_blocks_and_sumtree(
            Some(subshell_separator_height),
            None,
            move |b| b.update_padding(padding),
            |_| {},
        );
    }

    pub fn set_visibility_of_block_for_ai_action(
        &mut self,
        id: &AIAgentActionId,
        is_visible: bool,
    ) {
        let id = id.clone();
        self.update_blocks_and_sumtree(
            None,
            None,
            move |block| {
                if block
                    .requested_command_action_id()
                    .is_some_and(|action_id| *action_id == id)
                {
                    block.set_should_hide(!is_visible);
                }
            },
            |_| {},
        );
    }

    pub fn toggle_visibility_of_block_for_env_var(&mut self, block_id: &str) {
        let block_id = block_id.to_owned();
        self.update_blocks_and_sumtree(
            None,
            None,
            move |block| match block.env_var_metadata() {
                Some(metadata) if metadata.block_id == block_id => {
                    let mut updated_metadata = metadata.clone();
                    updated_metadata.should_hide_block = !metadata.should_hide_block;
                    block.set_env_var_metadata(updated_metadata);
                }
                _ => (),
            },
            |_| {},
        );
    }

    pub fn set_show_bootstrap_block(&mut self, show_bootstrap_block: bool) {
        self.show_warp_bootstrap_block = show_bootstrap_block;
        self.update_blocks_and_sumtree(
            None,
            None,
            move |b| b.set_show_bootstrap_block(show_bootstrap_block),
            |_| {},
        );
    }

    pub fn update_rich_content_heights(&mut self, updated_heights: &HashMap<EntityId, f64>) {
        self.update_blocks_and_sumtree(None, Some(updated_heights), |_| {}, |_| {});
    }

    pub fn set_show_in_band_command_blocks(&mut self, should_show_in_band_command_blocks: bool) {
        self.show_in_band_command_blocks = should_show_in_band_command_blocks;
        self.update_blocks_and_sumtree(
            None,
            None,
            move |b| b.set_show_in_band_command_blocks(should_show_in_band_command_blocks),
            |_| {},
        );
    }

    pub fn set_show_memory_stats(&mut self, should_show_memory_stats: bool) {
        self.show_memory_stats = should_show_memory_stats;
        self.update_blocks_and_sumtree(
            None,
            None,
            move |b| b.set_show_memory_stats(should_show_memory_stats),
            |_| {},
        );
    }

    pub fn grid_at_location<T>(&self, location: &WithinBlock<T>) -> &BlockGrid {
        let block = &self.blocks[location.block_index.0];
        block
            .grid_of_type(location.grid)
            .expect("Grid for location should exist")
    }

    /// Finds the next visible Command or Output grid in the blocklist above a given grid
    /// in the blocklist, as specified by the block_index and grid_type parameters. Returns
    /// a point in the bottom row of the discovered grid at the specified column.
    fn seek_up_to_next_grid(
        &mut self,
        block_index: BlockIndex,
        grid_type: GridType,
        column: usize,
        inverted_blocklist: bool,
    ) -> Option<WithinBlock<Point>> {
        match grid_type {
            GridType::Prompt | GridType::Rprompt | GridType::PromptAndCommand => {
                // If starting from a Command or Prompt grid or Rprompt grid, search for the next Output grid.

                // We want the next "upwards" block.
                // Imagine we're currently in block 5. If the blocklist is inverted, we want block 6. If not, we want 4.
                let prev_block_index = if inverted_blocklist {
                    self.next_non_hidden_block_from_index(block_index)
                } else {
                    self.prev_non_hidden_block_from_index(block_index)
                }?;

                let prev_block = self.block_at(prev_block_index)?;
                let prev_output_grid = prev_block.output_grid();

                if !prev_output_grid.is_empty() {
                    let point = Point::new(prev_output_grid.len_displayed() - 1, column);
                    Some(WithinBlock::new(point, prev_block_index, GridType::Output))
                } else {
                    // If the grid is empty, search upwards starting from this grid.
                    self.seek_up_to_next_grid(
                        prev_block_index,
                        GridType::Output,
                        column,
                        inverted_blocklist,
                    )
                }
            }
            GridType::Output => {
                // If starting from an Output grid, search for the next Command or PromptAndCommand grid.
                let block = self.block_at(block_index)?;
                let command_is_empty = block.is_command_empty();

                let grid_type = GridType::PromptAndCommand;

                if !command_is_empty {
                    let blockgrid = block.prompt_and_command_grid();

                    let point = Point::new(blockgrid.len() - 1, column);
                    Some(WithinBlock::new(point, block_index, grid_type))
                } else {
                    self.seek_up_to_next_grid(block_index, grid_type, column, inverted_blocklist)
                }
            }
        }
    }

    /// Finds the next visible Command or Output grid in the blocklist below a given grid,
    /// as specified by the block_index and grid_type parameters. Returns a point in the
    /// top row of the discovered grid at the specified column.
    fn seek_down_to_next_grid(
        &self,
        block_index: BlockIndex,
        grid_type: GridType,
        column: usize,
        inverted_blocklist: bool,
    ) -> Option<WithinBlock<Point>> {
        match grid_type {
            GridType::Output => {
                // If starting from an Output grid, search for the Command grid in the next block.

                // We want the next "downwards" block.
                // Imagine we're currently in block 5. If the blocklist is inverted, we want block 4. If not, we want 6.
                let next_block_index = if inverted_blocklist {
                    self.prev_non_hidden_block_from_index(block_index)
                } else {
                    self.next_non_hidden_block_from_index(block_index)
                }?;

                let next_block = self.block_at(next_block_index)?;
                let next_command_is_empty = next_block.is_command_empty();
                // NOTE: there is a semantic difference here of seeking down to the next "prompt" (in the PS1 case)
                // vs the next "command" (in the Warp prompt case), when using the combined grid, rather than
                // directly going to the next "command" in both cases.
                let grid_type = GridType::PromptAndCommand;

                if !next_command_is_empty {
                    let point = Point::new(0, column);
                    Some(WithinBlock::new(point, next_block_index, grid_type))
                } else {
                    self.seek_down_to_next_grid(
                        next_block_index,
                        grid_type,
                        column,
                        inverted_blocklist,
                    )
                }
            }
            GridType::Prompt | GridType::Rprompt => {
                // TODO(CORE-1680): This code path is currently NOT reachable for the combined grid case.
                // Notably, we hit-test any selections on the rprompt grid AS the combined grid, instead
                // of correctly identifying the difference in selections (we cannot handle partial rprompt selections
                // correctly). When we resolve the linked issue we will need to update this logic accordingly
                // (from the rprompt, we should go to the PromptEndPoint row + 1 in the combined grid to be
                // semantically correct).
                // The resulting user behavior if this is not addressed is that we'll incorrectly jump from
                // the rprompt grid to the command grid, which is NOT visible to the user, in the case of the
                // combined grid (we no longer use the command grid), so the selection expansion will be "invisible".
                None
            }
            GridType::PromptAndCommand => {
                // If starting from a Command or PromptAndCommand grid, search for the Output grid in this block.
                let block = self.block_at(block_index)?;
                let output_grid = block.output_grid();

                if !output_grid.is_empty() {
                    let a = Point::new(0, column);
                    Some(WithinBlock::new(a, block_index, GridType::Output))
                } else {
                    self.seek_down_to_next_grid(
                        block_index,
                        GridType::Output,
                        column,
                        inverted_blocklist,
                    )
                }
            }
        }
    }

    /// Returns the underlying text string for the given range in the block.
    pub fn string_at_range<T: RangeInModel>(
        &self,
        item: &WithinBlock<T>,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
    ) -> String {
        let block_grid = if item.is_in_command_content() {
            self.blocks[item.block_index.0].prompt_and_command_grid()
        } else {
            self.blocks[item.block_index.0].output_grid()
        };

        let (start, end) = item.inner.range().into_inner();
        block_grid.grid_handler.bounds_to_string(
            start,
            end,
            false,
            respect_obfuscated_secrets,
            false, /* force_obfuscated_secrets */
            RespectDisplayedOutput::Yes,
        )
    }

    pub fn fragment_boundary_at_point(
        &self,
        point: &WithinBlock<Point>,
    ) -> WithinBlock<FragmentBoundary> {
        let block_grid = if point.is_in_command_content() {
            self.blocks[point.block_index.0].prompt_and_command_grid()
        } else {
            self.blocks[point.block_index.0].output_grid()
        };

        WithinBlock {
            inner: block_grid
                .grid_handler
                .fragment_boundary_at_point(&point.inner),
            block_index: point.block_index,
            grid: point.grid,
        }
    }

    /// Return all possible file paths containing the grid point ordered from longest to shortest.
    pub fn possible_file_paths_at_point(
        &self,
        point: WithinBlock<Point>,
    ) -> impl Iterator<Item = WithinBlock<PossiblePath>> {
        let block_grid = if point.is_in_command_content() {
            self.blocks[point.block_index.0].prompt_and_command_grid()
        } else {
            self.blocks[point.block_index.0].output_grid()
        };

        block_grid
            .grid_handler
            .possible_file_paths_at_point(point.inner)
            .into_iter()
            .map(move |link| WithinBlock {
                inner: link,
                block_index: point.block_index,
                grid: point.grid,
            })
    }

    pub fn url_at_point(&self, point: &WithinBlock<Point>) -> Option<WithinBlock<Link>> {
        let block_grid = match point.grid {
            GridType::Output => self.blocks.get(point.block_index.0)?.output_grid(),
            GridType::PromptAndCommand => self
                .blocks
                .get(point.block_index.0)?
                .prompt_and_command_grid(),
            // We don't support scanning in prompt for now.
            GridType::Prompt => return None,
            GridType::Rprompt => return None,
        };

        block_grid
            .grid_handler
            .url_at_point(point.inner)
            .map(|link| WithinBlock {
                inner: link,
                block_index: point.block_index,
                grid: point.grid,
            })
    }

    pub fn is_bootstrapped(&self) -> bool {
        self.bootstrap_stage.is_bootstrapped()
    }

    pub fn is_bootstrapping_precmd_done(&self) -> bool {
        self.bootstrap_stage == BootstrapStage::PostBootstrapPrecmd
    }

    pub fn is_script_execution(&self) -> bool {
        self.bootstrap_stage == BootstrapStage::ScriptExecution
    }

    #[cfg(test)]
    pub fn create_new_block_with_local_status(
        &mut self,
        block_id: BlockId,
        bootstrap_stage: BootstrapStage,
        precmd_value: Option<PrecmdValue>,
        restored_block_was_local: bool,
    ) {
        self.create_new_block(
            block_id,
            bootstrap_stage,
            precmd_value,
            Some(restored_block_was_local),
        );
    }

    /// If a precmd_value is provided, then we delegate the precmd
    /// message to the block. In normal execution, we don't have
    /// this data here because it comes from a different hook dedicated
    /// to precmd itself. One place we provide the value is session
    /// restoration, because the semantics are to create each block with
    /// all of its data (instead of dividing it amongst terminal hooks).
    fn create_new_block(
        &mut self,
        block_id: BlockId,
        bootstrap_stage: BootstrapStage,
        precmd_value: Option<PrecmdValue>,
        restored_block_was_local: Option<bool>,
    ) {
        let honor_ps1 = self.honor_ps1;
        let mut block = Block::new(
            block_id,
            self.block_size(),
            self.event_proxy.clone(),
            self.background_executor.clone(),
            bootstrap_stage,
            self.show_warp_bootstrap_block,
            self.show_in_band_command_blocks,
            self.show_memory_stats,
            self.blocks.len().into(),
            honor_ps1,
            self.obfuscate_secrets,
            self.is_ai_ugc_telemetry_enabled,
            self.agent_view_state.active_conversation_id(),
        );
        if let Some(is_local) = restored_block_was_local {
            block.set_restored_block_was_local(is_local);
        }
        if !self.blocks.is_empty() && self.active_block().is_for_in_band_command {
            if let Some(CachedPromptData {
                prompt_grid,
                rprompt_grid,
                ..
            }) = &self.cached_prompt_data
            {
                let prompt_grid = prompt_grid.clone();
                let rprompt_grid = rprompt_grid.clone();
                log::debug!("Initializing new block using cached prompt grids");
                block.set_prompt_grids_from_cached_data(prompt_grid, rprompt_grid);
            }
        }

        if self.is_executing_oz_environment_startup_commands {
            block.set_is_oz_environment_startup_command(true);
            block.hide();
        }

        self.block_heights.push(BlockHeightItem::Block(
            block.height(&self.agent_view_state).into(),
        ));
        self.block_id_to_block_index
            .insert(block.id().clone(), block.index());
        self.blocks.push(block);

        if let Some(precmd_value) = precmd_value {
            delegate_to_block!(self.precmd(precmd_value));
        }
    }

    /// Creates a new block for background output. This block is not added to the
    /// blocklist until it has meaningful output, since the shell often prints
    /// a reset sequence between commands, and we don't want to create lots of
    /// empty blocks.
    pub(super) fn create_pending_background_block(&mut self) -> Block {
        Block::new(
            BlockId::new(),
            self.block_size(),
            self.event_proxy.clone(),
            self.background_executor.clone(),
            self.bootstrap_stage,
            self.show_warp_bootstrap_block,
            self.show_in_band_command_blocks,
            self.show_memory_stats,
            BlockIndex::zero(),
            false,
            self.obfuscate_secrets,
            self.is_ai_ugc_telemetry_enabled,
            None,
        )
    }

    /// Sets whether any content within a grid that is "secret-like" should be obfuscated.
    pub(super) fn set_obfuscate_secrets(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        self.obfuscate_secrets = obfuscate_secrets;
        self.active_block_mut()
            .set_obfuscate_secrets(obfuscate_secrets);
        for block in self.blocks.iter_mut() {
            block.set_obfuscate_secrets(obfuscate_secrets);
        }
    }

    /// Sets whether subsequent blocks (including the active block) have their grids obfuscated.
    pub(super) fn set_obfuscate_secrets_for_subsequent_blocks(
        &mut self,
        obfuscate_secrets: ObfuscateSecrets,
    ) {
        self.obfuscate_secrets = obfuscate_secrets;
        self.active_block_mut()
            .set_obfuscate_secrets(obfuscate_secrets);
    }

    /// Sets whether the grids of the specified block should be obfuscated.
    pub fn set_obfuscate_secrets_for_block(
        &mut self,
        block_index: BlockIndex,
        obfuscate_secrets: ObfuscateSecrets,
    ) {
        if let Some(block) = self.blocks.get_mut(block_index.0) {
            block.set_obfuscate_secrets(obfuscate_secrets);
        }
    }

    /// Inserts a fully serialized block into the block list.
    ///
    /// This is used to insert a block snapshot from a CLI agent conversation
    /// into an already-initialized block list.
    pub fn insert_restored_block(&mut self, block: &SerializedBlock) {
        let did_active_block_receive_precmd = self.active_block().has_received_precmd();
        let mut processor = Processor::new();
        self.restore_block(block, BootstrapStage::PostBootstrapPrecmd, &mut processor);
        // restore_block consumed the previous active block and made the restored
        // block the new active (finished) block. Create a fresh active block so
        // the terminal can continue accepting input.
        self.create_new_block(
            BlockId::new(),
            self.bootstrap_stage,
            did_active_block_receive_precmd
                .then(|| self.last_populated_precmd_payload.clone())
                .flatten(),
            None,
        );
    }

    /// Splice a background block into the blocklist. This is called once the
    /// block has meaningful output.
    pub(super) fn insert_background_block(&mut self, block: Block) {
        log::debug!("Adding background block to blocklist");
        debug_assert!(
            block.is_background(),
            "Tried to insert a non-background block"
        );

        let background_block_index = self.active_block_index();
        self.blocks.insert(background_block_index.0, block);
        self.reset_internal_block_index(background_block_index);
        self.reset_internal_block_index(background_block_index + BlockIndex::from(1));

        // Splice the new block into the block heights SumTree.  This block might
        // have a non-zero height, so in order to properly update the active gap,
        // we insert it with a height of zero and then update the sumtree with the
        // actual height.
        self.block_heights = {
            let mut cursor = self.block_heights.cursor::<BlockIndex, ()>();
            let mut tree_before_active_block =
                cursor.slice(&BlockIndex(background_block_index.0 + 1), SeekBias::Left);
            tree_before_active_block.push(BlockHeightItem::Block(BlockHeight::from(0.0)));
            tree_before_active_block.push_tree(cursor.suffix());
            tree_before_active_block
        };
        self.update_live_block_height(background_block_index);

        self.event_proxy
            .send_terminal_event(TerminalEvent::BackgroundBlockStarted);
    }

    /// Initializes a [`BlockSize`] for a new block
    fn block_size(&self) -> BlockSize {
        BlockSize {
            block_padding: self.padding,
            size: self.size,
            max_block_scroll_limit: self.max_grid_size_limit,
            warp_prompt_height_lines: self.warp_prompt_height_lines,
        }
    }

    pub fn reinit_shell(&mut self) {
        let active_block = self.active_block_mut();
        active_block.finish(0);
        self.update_active_block_height();

        self.create_new_block(
            BlockId::new(),
            BootstrapStage::WarpInput,
            None, /* precmd_value */
            None, /* restored_block_is_local */
        );
        self.bootstrap_stage = BootstrapStage::WarpInput;
    }

    /// Starts the active block and resets block-to-block state. For local sessions, this is called
    /// from the input editor when it sends user bytes to the pty (usually the
    /// next command to run, but also ctrl-d). Once we've written to the pty on
    /// the user's behalf, we consider the active block started.
    ///
    /// This should usually not be called directly. Call start_command_execution in [`super::TerminalModel`] instead so relevant terminal events are emitted.
    pub(super) fn start_active_block(&mut self) {
        // Cache the prompt in preexec. By the time preexec is called, the shell should have
        // emitted the prompt (either via precmd or in between precmd and preexec).
        let active_block = self.active_block();
        let previous_prompt_grid = active_block.prompt_grid().clone();
        let previous_rprompt_grid = active_block.rprompt_grid().clone();
        self.cached_prompt_data = Some(CachedPromptData {
            prompt_grid: previous_prompt_grid,
            rprompt_grid: previous_rprompt_grid,
            block_creation_time: *active_block.creation_ts(),
        });

        self.active_block_mut().start();
        self.early_output.reset_user_input();
    }

    /// Increments `self.in_flight_in_band_command_count` and starts the active block as usual.
    pub fn start_active_block_for_in_band_command(&mut self) {
        // Cache the prompt in preexec. By the time preexec is called, the shell should have
        // emitted the prompt (either via precmd or in between precmd and preexec).
        let active_block = self.active_block();
        let previous_prompt_grid = active_block.prompt_grid().clone();
        let previous_rprompt_grid = active_block.rprompt_grid().clone();
        self.cached_prompt_data = Some(CachedPromptData {
            prompt_grid: previous_prompt_grid,
            rprompt_grid: previous_rprompt_grid,
            block_creation_time: *active_block.creation_ts(),
        });

        self.in_flight_in_band_command_count += 1;
        self.active_block_mut().start_for_in_band_command();
    }

    /// Sets the shell host for the active block.
    pub fn set_active_shell_host(&mut self, shell_host: ShellHost) {
        self.active_block_mut().set_shell_host(shell_host)
    }

    pub fn set_bootstrapped(&mut self) {
        self.bootstrap_stage = BootstrapStage::Bootstrapped;
    }

    pub fn is_empty(&self) -> bool {
        self.block_heights().summary().height.as_f64() < f64::EPSILON
    }

    /// Returns `true` if there is any visible content item in the blocklist that
    /// passes the given predicate.
    ///
    /// This checks for:
    /// - Visible command blocks (non-background, non-hidden)
    /// - Rich content (AI blocks, etc.)
    /// - Inline banners
    ///
    /// Excludes gaps and separators as they are visual dividers, not content.
    pub fn has_visible_block_height_item_where<F>(&self, predicate: F) -> bool
    where
        F: Fn(&BlockHeightItem) -> bool,
    {
        let mut cursor = self
            .block_heights
            .cursor::<TotalIndex, BlockHeightSummary>();
        cursor.seek(&TotalIndex(0), SeekBias::Right);

        while let Some(item) = cursor.item() {
            let is_visible = match item {
                BlockHeightItem::Block(height) if *height > BlockHeight::zero() => {
                    // Check if this is a non-background block (matching BlockFilter::commands())
                    let block_index = BlockIndex::from(cursor.start().block_count);
                    self.block_at(block_index)
                        .is_some_and(|block| !block.is_background())
                }
                BlockHeightItem::RichContent(rich_content) => !rich_content.should_hide,
                BlockHeightItem::InlineBanner {
                    height_when_visible,
                    is_hidden,
                    ..
                } if *height_when_visible > BlockHeight::zero() && !is_hidden => true,
                // Exclude gaps, restored block separators, and subshell separators
                _ => false,
            };

            if is_visible && predicate(item) {
                return true;
            }

            cursor.next();
        }

        false
    }

    pub fn needs_bracketed_paste(&self) -> bool {
        self.active_block().needs_bracketed_paste()
    }

    /// Adds the provided serialized block to the blocklist.
    /// If the block's start_ts is `None`, the block will not be `start`ed.
    /// If the block's completed_ts is `None`, the block will be started but not `finish`ed.
    fn restore_block(
        &mut self,
        block: &SerializedBlock,
        bootstrap_stage: BootstrapStage,
        processor: &mut Processor,
    ) {
        let precmd_value = PrecmdValue {
            pwd: block.pwd.clone(),
            git_head: block.git_head.clone(),
            git_branch: block.git_branch_name.clone(),
            virtual_env: block.virtual_env.clone(),
            conda_env: block.conda_env.clone(),
            node_version: block.node_version.clone(),
            session_id: block.session_id.map(|id| id.as_u64()),
            ps1: block.ps1.clone(),
            honor_ps1: Some(block.honor_ps1),
            kube_config: None,
            rprompt: block.rprompt.clone(),
            ps1_is_encoded: None,
            is_after_in_band_command: false,
        };

        self.create_new_block(
            block.id.clone(),
            bootstrap_stage,
            Some(precmd_value),
            block.is_local,
        );
        if let Some(shell_host) = &block.shell_host {
            self.active_block_mut().set_shell_host(shell_host.clone());
        }

        self.active_block_mut().set_honor_ps1(block.honor_ps1);

        let start_ts = match block.start_ts {
            None => {
                // The start_ts is set iff the block was started. So if it's not set, we're done.
                self.update_active_block_height();
                return;
            }
            Some(start_ts) => start_ts,
        };

        // Start the block.
        if block.is_background {
            self.active_block_mut().start_background(None);
        } else {
            self.active_block_mut().start();
            self.active_block_mut().disable_reset_grid_checks();
        }

        if let Some(serialized_ai_metadata) = block.ai_metadata.as_ref().and_then(|ai_metadata| {
            serde_json::from_str::<Option<SerializedAIMetadata>>(ai_metadata)
                .ok()
                .flatten()
        }) {
            self.active_block_mut()
                .set_interaction_mode_from_serialized_ai_metadata(serialized_ai_metadata);
        }

        // For whatever reason, the pattern here in restore_block() is to create a block and then
        // mutate it to set each restored property that isn't set via constructor.
        //
        // Don't love this.
        if let Some(visibility) = block.agent_view_visibility.clone() {
            self.active_block_mut()
                .set_agent_view_visibility(visibility.into());
        } else {
            self.active_block_mut().clear_conversation_id();
        }

        // Set the start_ts to the saved start_ts _after_ `start`ing the block (which would have set its own start_ts).
        self.active_block_mut().override_start_ts(start_ts);

        processor.parse_bytes(self, &block.stylized_command, &mut io::sink());

        if block.did_execute {
            let command = self.active_block_mut().command_to_string();
            self.preexec(PreexecValue { command });
        }

        if block.did_execute || block.is_background {
            processor.parse_bytes(self, &block.stylized_output, &mut io::sink());
        }

        let completed_ts = match block.completed_ts {
            None => {
                // The completed_ts is set iff the block was `finish`ed. So if it's not set, we're done.
                self.update_active_block_height();
                return;
            }
            Some(completed_ts) => completed_ts,
        };

        self.active_block_mut().finish(block.exit_code);
        self.update_active_block_height();

        self.event_proxy
            .send_terminal_event(AfterBlockCompleted(AfterBlockCompletedEvent {
                command_finished_to_precmd_delay: None,
                block_type: BlockType::Restored,
                num_secrets_obfuscated: self.active_block().num_secrets_obfuscated(),
                // We don't track if a restored block was a cloud workflow execution.
                cloud_workflow_id: None,
                cloud_env_var_collection_id: None,
            }));

        // Set the completed_ts to the saved completed_ts _after_ `finish`ing the block (which would have set its own completed_ts).
        self.active_block_mut().override_completed_ts(completed_ts);

        if let Some(prompt_snapshot) = &block.prompt_snapshot {
            if let Ok(prompt_snapshot) = serde_json::from_str(prompt_snapshot) {
                log::debug!("Restored prompt: {prompt_snapshot:?}");
                self.active_block_mut().set_prompt_snapshot(prompt_snapshot);
            }
        }
    }

    /// This is the main function that marks the end of a block, and the beginning of a new block.
    /// 1. Increment stage if we should.
    /// 2. Finish the active block.
    /// 3. Update block heights.
    /// 4. Adjust selection based on changed heights.
    /// 5. Create a new block.
    fn finalize_block_and_advance_list(&mut self, data: CommandFinishedValue) {
        record_trace_event!("command_execution:blocks:finalize_block_and_advance_list");
        let next_bootstrap_stage = if !self.bootstrap_stage.is_done() {
            self.bootstrap_stage.next_stage()
        } else {
            self.bootstrap_stage
        };

        if !self.active_block().is_for_in_band_command {
            self.finish_background_block();
        }

        self.active_block_mut().finish(data.exit_code);
        self.update_active_block_height();

        self.update_selection_after_height_change();
        self.create_new_block(
            data.next_block_id,
            next_bootstrap_stage,
            None, /*precmd_value*/
            None, /* restored_block_was_local */
        );
        if self.bootstrap_stage != next_bootstrap_stage {
            log::info!(
                "Incrementing stage from {:?} to {:?}",
                self.bootstrap_stage,
                &next_bootstrap_stage
            );
            self.bootstrap_stage = next_bootstrap_stage;
        }
    }

    /// Sends the `AfterBlockCompleted` event to the view.
    /// We determine what `BlockType` to send based on the bootstrap stage
    /// of the finished block.
    ///
    /// Delay will be provided if this was a block the user created through normal
    /// execution, and will be None if not (i.e. if it's a restored block from
    /// session restoration or a bootstrapping block).
    fn send_after_block_completed_event(&self, finished_block: &Block, delay: Option<Duration>) {
        let block_type = finished_block.into();
        self.event_proxy
            .send_terminal_event(AfterBlockCompleted(AfterBlockCompletedEvent {
                command_finished_to_precmd_delay: delay,
                block_type,
                num_secrets_obfuscated: finished_block.num_secrets_obfuscated(),
                cloud_workflow_id: finished_block.cloud_workflow_state(),
                cloud_env_var_collection_id: finished_block.cloud_env_var_collection_state(),
            }));
    }

    /// Finish the active background output block, if there is one. This also
    /// notifies the view of the completed block, so that it can be saved for
    /// session restoration.
    fn finish_background_block(&mut self) {
        let num_secrets_obfuscated = self
            .background_block_mut()
            .map(|block| block.num_secrets_obfuscated());
        let agent_view_state = self.agent_view_state.clone();
        if let Some(background_block) = self.background_block_mut() {
            background_block.finish(0);
            let block_index = background_block.index();

            // It's common to have empty background blocks (because they only contained
            // typeahead), so we skip serializing them.
            if !background_block.is_empty(&agent_view_state) {
                // This is similar to send_after_block_completed_event, but we can't
                // call it because background_block mutably borrows self.
                let block_type = background_block.into();
                self.event_proxy.send_terminal_event(AfterBlockCompleted(
                    AfterBlockCompletedEvent {
                        command_finished_to_precmd_delay: None,
                        block_type,
                        num_secrets_obfuscated: num_secrets_obfuscated.unwrap_or_default(),
                        // Background blocks are not tracked as cloud workflow executions.
                        cloud_workflow_id: None,
                        cloud_env_var_collection_id: None,
                    },
                ));
            }

            // Now that the block is no longer active, its height may have changed.
            self.update_live_block_height(block_index);
        }
    }

    /// Whether the block list is in a state where it could be receiving
    /// typeahead text or background job output. This occurs when the active block
    /// has not yet started but we receive output from the shell.
    fn is_early_output(&self) -> bool {
        let active_block = self.active_block();
        self.is_bootstrapping_precmd_done()
            && !active_block.started()
            && !active_block.is_receiving_prompt()
            && active_block.state() == BlockState::BeforeExecution
    }

    pub fn early_output(&self) -> &EarlyOutput {
        &self.early_output
    }

    pub fn early_output_mut(&mut self) -> &mut EarlyOutput {
        &mut self.early_output
    }

    /// Returns `true` if an in-band command is currently being written to the PTY or actively running.
    pub fn is_writing_or_executing_in_band_command(&self) -> bool {
        self.in_flight_in_band_command_count > 0
    }

    /// Returns the cached prompt data from the last user-executed block, if any.
    pub fn cached_prompt_data_from_last_user_block(&self) -> Option<&CachedPromptData> {
        self.cached_prompt_data.as_ref()
    }

    /// Updates the sumtree with the block's new height.
    fn update_block_height_at_idx(&mut self, block_index: BlockIndex) {
        if let Some(block) = self.block_at(block_index) {
            let new_block_height = block.height(&self.agent_view_state).into();

            self.block_heights = {
                let mut cursor = self.block_heights.cursor::<BlockIndex, ()>();
                // The BlockIndex dimension acts like a count rather than an index.
                // |    block 0    |    block 1    |    block 2    | ...
                // ^ count=0       ^ count=1       ^ count=2       ^ count=3
                // To position the cursor at block N, we want to seek to the left
                // of the point where count=N + 1.
                let next_index = block_index + BlockIndex(1);
                let mut tree_before_last_block = cursor.slice(&next_index, SeekBias::Left);
                tree_before_last_block.push(BlockHeightItem::Block(new_block_height));

                cursor.next();
                let suffix = cursor.suffix();
                tree_before_last_block.push_tree(suffix);
                tree_before_last_block
            };
            // TODO: Update active gap.
        }
    }

    pub fn filtered_blocks(&self) -> HashSet<BlockIndex> {
        self.blocks
            .iter()
            .filter(|&block| block.current_filter().is_some_and(|query| query.is_active))
            .map(|block| block.index())
            .collect::<HashSet<BlockIndex>>()
    }

    /// Filters the output grid of the block at the given index. Any logical lines
    /// not matching the filter will be hidden.
    pub fn filter_block_output(&mut self, block_index: BlockIndex, filter_query: BlockFilterQuery) {
        let block_to_filter = self
            .blocks
            .get_mut(block_index.0)
            .filter(|block| !block.is_empty(&self.agent_view_state));
        if let Some(block) = block_to_filter {
            block.filter_output(filter_query);
            self.update_block_height_at_idx(block_index);
        }
        self.clear_selection();
    }

    /// Re-filters the active block's output if it is long running.
    pub fn maybe_refilter_active_block_output(&mut self) {
        if self.active_block().is_active_and_long_running() {
            self.active_block_mut().maybe_refilter_output()
        }
    }

    pub fn clear_filter_on_block(&mut self, block_index: BlockIndex) {
        let block_to_clear = self
            .blocks
            .get_mut(block_index.0)
            .filter(|block| !block.is_empty(&self.agent_view_state));
        if let Some(block) = block_to_clear {
            block.clear_filter();
            self.update_block_height_at_idx(block_index);
        }
    }

    pub fn set_scroll_position_before_filter(
        &mut self,
        block_index: BlockIndex,
        offset_from_block_top: Lines,
    ) {
        self.scroll_position_before_filter = Some(BlockScrollPosition {
            block_index,
            offset_from_block_top,
        });
    }

    pub fn clear_scroll_position_before_filter(&mut self) {
        self.scroll_position_before_filter = None;
    }

    pub fn scroll_position_before_filter(&self) -> Option<BlockScrollPosition> {
        self.scroll_position_before_filter
    }

    pub fn filter_for_block(&self, block_index: BlockIndex) -> Option<&BlockFilterQuery> {
        self.blocks
            .get(block_index.0)
            .filter(|block| !block.is_empty(&self.agent_view_state))
            .and_then(|block| block.current_filter())
    }

    pub fn num_matched_lines_in_filter_for_block(&self, block_index: BlockIndex) -> Option<usize> {
        self.blocks
            .get(block_index.0)
            .filter(|block| !block.is_empty(&self.agent_view_state))
            .and_then(|block| {
                block
                    .output_grid()
                    .grid_handler()
                    .num_matched_lines_in_filter()
            })
    }

    /// Records the fact that the given block was just painted.
    pub fn record_block_painted(&self, block_index: BlockIndex) {
        if let Some(block) = self.blocks.get(block_index.0) {
            block.update_last_painted_at(Local::now());
        }
    }

    pub(in crate::terminal) fn insert_rich_content_before_block_index(
        &mut self,
        item: RichContentItem,
        block_index: BlockIndex,
    ) {
        let view_id = item.view_id;
        let inserted_index = self
            .insert_non_block_item_before_block(block_index, BlockHeightItem::RichContent(item));
        self.removable_blocklist_item_positions
            .insert(RemovableBlocklistItem::RichContent(view_id), inserted_index);
        self.mark_rich_content_dirty(view_id);
        self.maintain_pinned_to_bottom();
    }

    /// Insert a rich content item immediately after the given removable item.
    /// Returns true if insertion succeeded.
    pub(in crate::terminal) fn insert_rich_content_after_item(
        &mut self,
        after_item: RemovableBlocklistItem,
        item: RichContentItem,
    ) -> bool {
        let Some(current_index) = self
            .removable_blocklist_item_positions
            .get(&after_item)
            .copied()
        else {
            return false;
        };

        let view_id = item.view_id;

        // Recreate block heights tree with new item inserted.
        let (new_tree, inserted_index) = {
            let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
            let mut prefix = cursor.slice(&(current_index + 1), SeekBias::Right);
            let inserted_index = TotalIndex(prefix.summary().total_count);
            prefix.push(BlockHeightItem::RichContent(item));
            prefix.push_tree(cursor.suffix());
            (prefix, inserted_index)
        };

        self.block_heights = new_tree;
        self.update_block_height_indices(BlockHeightUpdate::Insertion(inserted_index), true);

        // If there is an item at the index that we are inserting into,
        // we should shift that item forward by one.
        self.removable_blocklist_item_positions
            .values_mut()
            .for_each(|pos| {
                if *pos == inserted_index {
                    pos.0 += 1;
                }
            });

        self.removable_blocklist_item_positions
            .insert(RemovableBlocklistItem::RichContent(view_id), inserted_index);
        self.event_proxy.send_wakeup_event();

        true
    }

    pub(in crate::terminal) fn set_marked_text(
        &mut self,
        marked_text: &str,
        selected_range: &Range<usize>,
    ) {
        let active_block = self.active_block_mut();
        if !active_block.is_active_and_long_running() {
            log::warn!("Tried to set marked text on blocklist while no block was active");
            return;
        }
        active_block.set_marked_text(marked_text, selected_range);
    }

    pub(in crate::terminal) fn clear_marked_text(&mut self) {
        let active_block = self.active_block_mut();
        if !active_block.is_active_and_long_running() {
            log::warn!("Tried to clear marked text on blocklist while no block was active");
            return;
        }
        active_block.clear_marked_text();
    }

    pub fn last_non_hidden_ai_block_handle(&self, app: &AppContext) -> Option<ViewHandle<AIBlock>> {
        let rich_content_view_id = self
            .last_non_hidden_rich_content_block_after_block(None)?
            .1
            .view_id;
        let active_window_id = app.windows().active_window()?;
        app.view_with_id::<AIBlock>(active_window_id, rich_content_view_id)
    }

    pub fn has_active_ai_block(&self, app: &AppContext) -> bool {
        self.last_non_hidden_ai_block_handle(app)
            .is_some_and(|handle| !handle.as_ref(app).is_finished())
    }

    /// Returns the contents of all blocks associated with bootstrap.
    pub fn bootstrap_block_contents(&self) -> String {
        let mut contents = String::new();
        for block in self.blocks.iter() {
            match block.bootstrap_stage() {
                BootstrapStage::WarpInput | BootstrapStage::ScriptExecution => {
                    contents.push_str(&block.command_to_string());
                    contents.push('\n');
                }
                // We stop at the first block that is after the bootstrapping stage.
                BootstrapStage::Bootstrapped | BootstrapStage::PostBootstrapPrecmd => break,
                BootstrapStage::RestoreBlocks => {}
            }
        }

        contents.trim().to_string()
    }

    pub(crate) fn removable_blocklist_item_position(
        &self,
        item: &RemovableBlocklistItem,
    ) -> Option<&TotalIndex> {
        self.removable_blocklist_item_positions.get(item)
    }

    pub fn get_previous_block_height_item(
        &self,
        item: RemovableBlocklistItem,
    ) -> Option<&BlockHeightItem> {
        let current_total_index = *self.removable_blocklist_item_positions.get(&item)?;
        let mut cursor = self.block_heights.cursor::<TotalIndex, ()>();
        cursor.seek(&current_total_index, SeekBias::Left);
        cursor.prev_item();
        cursor.item()
    }
}

impl ansi::Handler for BlockList {
    fn set_title(&mut self, _: Option<String>) {
        log::error!("Handler method BlockList::set_title should never be called. This should be handled by TerminalModel.");
    }

    fn set_cursor_style(&mut self, style: Option<CursorStyle>) {
        delegate!(self.set_cursor_style(style));
    }

    fn set_cursor_shape(&mut self, shape: CursorShape) {
        delegate!(self.set_cursor_shape(shape));
    }

    fn input(&mut self, c: char) {
        let is_bootstrapped = self.is_bootstrapped();
        let active_block = self.active_block_mut();

        // We typically "start" blocks when we execute the command. Start basically
        // means mark ready to render. For bootstrapping blocks, we start them
        // when they receive input. Note this means that, for example, a bootstrap script that
        // only executes `read` isn't supported.
        if !active_block.started() && !is_bootstrapped {
            self.start_active_block();
            self.update_active_block_height();
        }
        delegate!(self.input(c));
    }

    fn goto(&mut self, row: VisibleRow, col: usize) {
        delegate!(self.goto(row, col));
    }

    fn goto_line(&mut self, row: VisibleRow) {
        delegate!(self.goto_line(row));
    }

    fn goto_col(&mut self, col: usize) {
        delegate!(self.goto_col(col));
    }

    fn insert_blank(&mut self, count: usize) {
        delegate!(self.insert_blank(count));
    }

    fn move_up(&mut self, lines: usize) {
        delegate!(self.move_up(lines));
    }

    fn move_down(&mut self, lines: usize) {
        delegate!(self.move_down(lines));
    }

    fn identify_terminal<W: io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        delegate!(self.identify_terminal(writer, intermediate));
    }

    fn report_xtversion<W: io::Write>(&mut self, writer: &mut W) {
        delegate!(self.report_xtversion(writer));
    }

    fn device_status<W: io::Write>(&mut self, writer: &mut W, arg: usize) {
        // We are circumventing potential delegation to the EarlyOutputHandler
        // because powershell emits device status requests after the prompt has
        // finished but before a command has started in order to position the
        // cursor after the prompt.
        //
        // This is using a heuristic that if the typeahead is empty, it is more
        // likely that a device status request is asking about the prompt
        // positioning than a background block.
        if self.is_early_output() && !self.early_output.typeahead().is_empty() {
            EarlyOutput::handler(self).device_status(writer, arg)
        } else {
            delegate_to_block!(self.device_status(writer, arg));
        }
    }

    fn move_forward(&mut self, columns: usize) {
        delegate!(self.move_forward(columns));
    }

    fn move_backward(&mut self, columns: usize) {
        delegate!(self.move_backward(columns));
    }

    fn move_down_and_cr(&mut self, lines: usize) {
        delegate!(self.move_down_and_cr(lines));
    }

    fn move_up_and_cr(&mut self, lines: usize) {
        delegate!(self.move_up_and_cr(lines));
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
        let num_scrollback_lines = self.active_block_mut().grid_handler().history_size();
        let lines_scrolled = delegate!(self.linefeed());

        if num_scrollback_lines == self.max_grid_size_limit {
            self.update_selection_after_grid_truncation()
        }

        lines_scrolled
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

    fn scroll_up(&mut self, lines: usize) -> ScrollDelta {
        delegate!(self.scroll_up(lines))
    }

    fn scroll_down(&mut self, lines: usize) -> ScrollDelta {
        delegate!(self.scroll_down(lines))
    }

    fn insert_blank_lines(&mut self, lines: usize) -> ScrollDelta {
        delegate!(self.insert_blank_lines(lines))
    }

    fn delete_lines(&mut self, lines: usize) -> ScrollDelta {
        delegate!(self.delete_lines(lines))
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

    fn clear_line(&mut self, mode: LineClearMode) {
        delegate!(self.clear_line(mode));
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        // TODO(alokedesai): We should handle all of the clear mode enum variants here before
        // dispatching to the delegate.
        #[allow(clippy::single_match)]
        match mode {
            ClearMode::ResetAndClear => {
                // Clear all the blocks except the current block.
                self.blocks.drain(0..self.blocks.len() - 1);
                // Make sure we actually reduce the _capacity_ of self.blocks,
                // not just its length.
                self.blocks.shrink_to_fit();
                self.block_id_to_block_index.clear();
                // Reset the active block's index to be zero now that the rest of the blocks
                // have been removed. This will also populate block_id_to_block_index for this block.
                self.reset_internal_block_index(BlockIndex::zero());

                if let Some(block) = self.blocks.last() {
                    self.block_heights = SumTree::from_item(BlockHeightItem::Block(
                        block.height(&self.agent_view_state).into(),
                    ));
                } else {
                    self.block_heights = SumTree::new();
                }
                // Clear existing selection.
                self.selection.take();

                // Remove the active gap since it was removed from the block heights sum tree.
                self.active_gap.take();
            }
            ClearMode::All => {
                // TODO(alokedesai): Investigate how we can call `clear_visible_screen` here to have
                // Warp's custom logic for "clear". It's not immediately straightforward because a
                // a running program that writes output, clears the visible screen, and then writes
                // more output should all be encapsulated within a single block, which wouldn't be
                // quite right with Warp's custom clear screen logic.
            }
            _ => {}
        }
        delegate!(self.clear_screen(mode));
    }

    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        delegate!(self.clear_tabs(mode));
    }

    fn reset_state(&mut self) {
        delegate!(self.reset_state());
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        delegate!(self.reverse_index())
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        delegate!(self.terminal_attribute(attr));
    }

    fn set_mode(&mut self, mode: Mode) {
        delegate!(self.set_mode(mode));
    }

    fn unset_mode(&mut self, mode: Mode) {
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

    fn set_active_charset(&mut self, index: CharsetIndex) {
        delegate!(self.set_active_charset(index));
    }

    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        delegate!(self.configure_charset(index, charset));
    }

    fn set_color(&mut self, index: usize, color: ColorU) {
        delegate!(self.set_color(index, color));
    }

    fn dynamic_color_sequence<W: io::Write>(
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

    fn clipboard_store(&mut self, clipboard: u8, base64: &[u8]) {
        delegate!(self.clipboard_store(clipboard, base64));
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        delegate!(self.clipboard_load(clipboard, terminator));
    }

    fn decaln(&mut self) {
        delegate!(self.decaln());
    }

    fn push_title(&mut self) {
        log::error!("Handler method BlockList::push_title should never be called. This should be handled by TerminalModel.");
    }

    fn pop_title(&mut self) {
        log::error!("Handler method BlockList::pop_title should never be called. This should be handled by TerminalModel.");
    }

    fn text_area_size_pixels<W: io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_pixels(writer));
    }

    fn text_area_size_chars<W: io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_chars(writer));
    }

    fn prompt_marker(&mut self, marker: ansi::PromptMarker) {
        // Don't handle prompt characters that are received in the middle of/after command
        // execution. A zsh subshell with p10k enabled will emit prompt markers, for example, and
        // these shouldn't be interpreted as actual characters that affect the presentation of the
        // active block. (They should be just treated as regular shell output).
        if self.active_block().state() != BlockState::BeforeExecution {
            return;
        }

        if let ansi::PromptMarker::EndPrompt = marker {
            // If we receive an end prompt marker without a matching start prompt marker, we
            // treat this as an extra re-print of the last line of the prompt, which should be
            // cleared. This specifically happens when we issue the \ei bindkey to get the input
            // buffer for typeahead which results in Bash (>4) sending the last line of the prompt
            // again (since it's re-printing the last line, in preparation for the next command).
            // Note that this only happens in background output blocks (since the re-print is after
            // we've finished the previous block).
            if self.active_block().receiving_chars_for_prompt().is_none() {
                delegate!(self.clear_line(ansi::LineClearMode::Left));
                // Resetting the cursor to (0, 0) will ensure that this background block is ignored.
                delegate!(self.goto_col(0));
                log::debug!("Received end prompt marker without a matching start marker");
                return;
            }
        }
        delegate_to_block!(self.prompt_marker(marker));
    }

    /// "Finalizes" the active block and creates the new one.
    ///
    /// This will send the `BlockCompleted` event to the view. For this
    /// hook, we want to show the user the finished block as fast as
    /// possible. Anything that could be costly should be handled in
    /// precmd / `AfterBlockCompleted` instead.
    fn command_finished(&mut self, data: CommandFinishedValue) {
        if self.active_block().is_for_in_band_command {
            self.in_flight_in_band_command_count =
                self.in_flight_in_band_command_count.saturating_sub(1);
        }
        self.finalize_block_and_advance_list(data);
        self.latest_block_finished_time = Some(instant::SystemTime::now());
    }

    /// Receives metadata for the prompt and the next command, and
    /// responsible for sending the `AfterBlockCompleted` event to
    /// the view. This is where we want to perform any costly
    /// operations relevant to the _previous_ block.
    fn precmd(&mut self, data: PrecmdValue) {
        let latest_block_finished_time = self.latest_block_finished_time.take();
        // We don't need to log this delay during the bootstrapping process, since these
        // are not blocks that the user has created. The delay here also can be very high
        // and skews the metrics.
        let block_finished_to_precmd_delay = if self.bootstrap_stage.is_done() {
            latest_block_finished_time.and_then(|instant| instant.elapsed().ok())
        } else {
            None
        };

        // Since typeahead is only generated by user input, we don't start collecting
        // it (or separating it from background output) until the session is fully bootstrapped.
        if self.is_bootstrapping_precmd_done() {
            self.early_output.precmd();
        }

        // If this is the Precmd following an in-band command, the payload is not populated. If the payload
        // is not populated, use the last populated Precmd payload to initialize the new active block.
        //
        // In-band commands are guaranteed not to modify the context for which information is sent
        // in the Precmd payload, so it's not necessary to recompute the precmd payload after an
        // in-band command runs. Thus we send an unpopulated precmd payload for in-band commands to
        // make their execution as fast as possible.
        if data.was_sent_after_in_band_command() {
            let mut precmd_value = self.last_populated_precmd_payload.clone().unwrap_or(data);
            precmd_value.is_after_in_band_command = true;
            delegate_to_block!(self.precmd(precmd_value));
        } else {
            delegate_to_block!(self.precmd(data.clone()));
            self.last_populated_precmd_payload = Some(data);
        }

        // Depending on whether or not there's a background block active, the previous
        // completed block is at blocks.len - 2 or blocks.len - 3.
        let previous_block = [2usize, 3usize]
            .into_iter()
            .flat_map(|offset| self.blocks.len().checked_sub(offset))
            .map(|idx| &self.blocks[idx])
            .find(|block| !block.is_background());
        if let Some(previous_block) = previous_block {
            self.send_after_block_completed_event(previous_block, block_finished_to_precmd_delay);
        } else {
            self.event_proxy
                .send_terminal_event(TerminalEvent::BootstrapPrecmdDone);
        }
    }

    fn preexec(&mut self, data: PreexecValue) {
        // We don't start handling early output until the session is fully bootstrapped,
        // because the distinction between typeahead and background output only
        // matters for user input.
        if self.is_bootstrapping_precmd_done() {
            EarlyOutput::preexec(self);
        }

        delegate_to_block!(self.preexec(data));
    }

    fn bootstrapped(&mut self, _data: BootstrappedValue) {
        self.bootstrap_stage = BootstrapStage::Bootstrapped;
    }

    fn input_buffer(&mut self, data: InputBufferValue) {
        EarlyOutput::handler(self).input_buffer(data);
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        delegate!(self.on_finish_byte_processing(input));

        // After processing a chunk of data from the PTY, make sure the active
        // block and background block heights are up-to-date.  We do this once
        // at the end of a chunk instead of incrementally to improve performance.
        self.update_active_block_height();
        self.update_background_block_height();
    }

    fn on_reset_grid(&mut self) {
        if self.is_bootstrapping_precmd_done() {
            delegate_to_block!(self.on_reset_grid());
        }
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        delegate_to_block!(self.handle_completed_iterm_image(image))
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        delegate_to_block!(self.handle_completed_kitty_action(action, metadata))
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

impl AddAssign<&BlockHeightSummary> for BlockHeightSummary {
    fn add_assign(&mut self, other: &Self) {
        self.height += other.height;
        self.total_count += other.total_count;
        self.block_count += other.block_count;
    }
}

impl From<usize> for TotalIndex {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl Item for BlockHeightItem {
    type Summary = BlockHeightSummary;

    fn summary(&self) -> Self::Summary {
        let block_count = match self {
            BlockHeightItem::Block(_) => 1,
            BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent { .. } => 0,
        };

        Self::Summary {
            total_count: 1,
            height: self.height().0,
            block_count,
        }
    }
}

impl<T> From<T> for BlockHeight
where
    T: IntoLines,
{
    fn from(value: T) -> Self {
        Self(value.into_lines())
    }
}

impl From<BlockHeight> for f64 {
    fn from(block_height: BlockHeight) -> f64 {
        block_height.0.as_f64()
    }
}

impl From<BlockHeight> for Lines {
    fn from(block_height: BlockHeight) -> Self {
        block_height.0
    }
}

impl<'a> Dimension<'a, BlockHeightSummary> for BlockHeight {
    fn add_summary(&mut self, summary: &'a BlockHeightSummary) {
        self.0 += summary.height
    }
}

impl<'a> Dimension<'a, BlockHeightSummary> for BlockIndex {
    fn add_summary(&mut self, summary: &'a BlockHeightSummary) {
        self.0 += summary.block_count
    }
}

impl<'a> Dimension<'a, BlockHeightSummary> for TotalIndex {
    fn add_summary(&mut self, summary: &'a BlockHeightSummary) {
        self.0 += summary.total_count
    }
}

impl<'a> Dimension<'a, BlockHeightSummary> for BlockHeightSummary {
    fn add_summary(&mut self, summary: &'a BlockHeightSummary) {
        *self += summary
    }
}

#[cfg(test)]
pub trait ToTotalIndex {
    fn to_total_index(&self, block_list: &BlockList) -> TotalIndex;
}

#[cfg(test)]
impl ToTotalIndex for BlockIndex {
    // Returns the total index corresponding to the block index in the given block list.
    fn to_total_index(&self, block_list: &BlockList) -> TotalIndex {
        let mut cursor = block_list.block_heights().cursor::<BlockIndex, ()>();
        let count_including_item = cursor.slice(self, SeekBias::Right).summary().total_count;
        TotalIndex(count_including_item)
    }
}

#[cfg(test)]
#[path = "blocks_tests.rs"]
mod tests;
#[cfg(test)]
pub use self::tests::insert_block;
