//! Utilities to help construct [`TerminalModel`]s and its
//! constituents, like [`Block`]s and [`BlockList`]s for use
//! in unit tests.
//!
//! Note that the example code in the documentation of this module
//! is marked as no_run only  because it's currently not possible
//! to reference `#[cfg(test)]` symbols from doctests.

use std::collections::HashMap;
use std::io::sink;
use std::sync::Arc;

use pathfinder_geometry::vector::Vector2F;
use warp_core::command::ExitCode;
use warpui::r#async::executor::Background;

use super::ansi::{
    CommandFinishedValue, CompletionMetadata, Handler, PrecmdValue, PreexecValue, Processor,
    PromptMetadata,
};
use super::block::{Block, BlockId, BlockSize};
use super::blocks::BlockList;
use super::bootstrap::BootstrapStage;
use super::image_map::StoredImageMetadata;
use super::iterm_image::{ITermImage, ITermImageDimensionUnit, ITermImageMetadata};
use super::kitty::{
    KittyAction, KittyImage, KittyImageMetadata, KittyPixelDataFormat, KittyPlacementData,
    KittyTransmissionMedium, StoreAndDisplay, StoreOnly,
};
use super::terminal_model::BlockIndex;
use super::{ObfuscateSecrets, TerminalModel};
use crate::ai::blocklist::SerializedBlockListItem;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::{BlockPadding, SizeInfo};

pub fn block_size() -> BlockSize {
    BlockSize {
        size: SizeInfo::new_without_font_metrics(10, 7),
        block_padding: block_padding(),
        max_block_scroll_limit: 1000,
        warp_prompt_height_lines: 0.6,
    }
}

pub fn test_iterm_image(image_id: u32) -> ITermImage {
    ITermImage {
        metadata: ITermImageMetadata {
            id: image_id,
            desired_width: Some((1, ITermImageDimensionUnit::Cell)),
            desired_height: Some((1, ITermImageDimensionUnit::Cell)),
            preserve_aspect_ratio: false,
            name: "test.png".to_string(),
            inline: true,
            image_size: Vector2F::new(10.0, 10.0),
        },
        data: vec![0],
    }
}

fn test_kitty_image_metadata() -> KittyImageMetadata {
    KittyImageMetadata {
        pixel_data_format: KittyPixelDataFormat::Rgba32Bit,
        transmission_medium: KittyTransmissionMedium::Direct,
        image_size: Vector2F::new(10.0, 10.0),
    }
}

pub fn test_kitty_image_metadata_map(image_id: u32) -> HashMap<u32, StoredImageMetadata> {
    HashMap::from([(
        image_id,
        StoredImageMetadata::Kitty(test_kitty_image_metadata()),
    )])
}

pub fn test_kitty_store_and_display_action(image_id: u32, placement_id: u32) -> KittyAction {
    KittyAction::StoreAndDisplay(StoreAndDisplay {
        image: KittyImage {
            metadata: test_kitty_image_metadata(),
            data: vec![0],
        },
        placement_data: KittyPlacementData {
            cols: Some(1),
            rows: Some(1),
            ..Default::default()
        },
        image_id,
        placement_id,
    })
}

pub fn test_kitty_store_only_action(image_id: u32) -> KittyAction {
    KittyAction::StoreOnly(StoreOnly {
        image: KittyImage {
            metadata: test_kitty_image_metadata(),
            data: vec![0],
        },
        image_id,
    })
}

fn block_padding() -> BlockPadding {
    BlockPadding {
        padding_top: 0.2,
        command_padding_top: 0.2,
        middle: 0.5,
        bottom: 1.0,
    }
}

/// A helper struct for creating a [`BlockList`] for use in tests.
///
/// For example, to create a [`BlockList`] that respects the user's custom
/// prompt:
///
/// ```no_run
/// # use warp::terminal::model::test_utils::TestBlockListBuilder;
/// let block_list = TestBlockListBuilder::new()
///     .with_honor_ps1(true)
///     .build();
/// ```
/// For tests that want to observe the events produced through interactions
/// with the block list, a custom [`ChannelEventListener`] can be registered.
///
/// This example restores a block, and asserts that an event was sent over
/// the channel event proxy:
///
/// ```no_run
/// # use warp::terminal::event::{BlockType, Event};
/// # use warp::terminal::event_listener::ChannelEventListener;
/// # use warp::terminal::model::block::SerializedBlock;
/// # use warp::terminal::model::test_utils::TestBlockListBuilder;
///
/// let (events_tx, events_rx) = async_channel::unbounded();
/// let channel_event_proxy = ChannelEventListener::builder_for_test()
///     .with_terminal_events_tx(events_tx)
///     .build();
///
/// let block = SerializedBlock::new_for_test("test".into(), "test".into());
///
/// let block_list = TestBlockListBuilder::new()
///     .with_channel_event_proxy(channel_event_proxy)
///     .with_restored_blocks(&[block])
///     .build();
///
/// let Ok(Event::BlockCompleted(data)) = events_rx.try_recv() else {
///     panic!("Expected a BlockCompleted event to have been generated!");
/// };
///
/// assert!(matches!(data.block_type, BlockType::Restored));
/// ```
pub struct TestBlockListBuilder<'a> {
    restored_blocks: Option<&'a [SerializedBlockListItem]>,
    honor_ps1: bool,
    block_sizes: BlockSize,
    channel_event_proxy: ChannelEventListener,
}

impl<'a> TestBlockListBuilder<'a> {
    pub fn new() -> Self {
        Self {
            restored_blocks: None,
            honor_ps1: false,
            block_sizes: block_size(),
            channel_event_proxy: ChannelEventListener::new_for_test(),
        }
    }

    pub fn with_restored_blocks(mut self, restored_blocks: &'a [SerializedBlockListItem]) -> Self {
        self.restored_blocks = Some(restored_blocks);
        self
    }

    pub fn with_honor_ps1(mut self, honor_ps1: bool) -> Self {
        self.honor_ps1 = honor_ps1;
        self
    }

    pub fn with_block_sizes(mut self, block_sizes: BlockSize) -> Self {
        self.block_sizes = block_sizes;
        self
    }

    pub fn with_channel_event_proxy(mut self, channel_event_proxy: ChannelEventListener) -> Self {
        self.channel_event_proxy = channel_event_proxy;
        self
    }

    pub fn build(self) -> BlockList {
        let mut block_list = BlockList::new(
            self.restored_blocks,
            self.block_sizes,
            self.channel_event_proxy,
            Arc::new(Background::default()),
            false, /* show_warp_bootstrap_input */
            false, /* show_warp_bootstrap_input */
            false, /* show_memory_stats */
            self.honor_ps1,
            false, /* is_inverted */
            ObfuscateSecrets::No,
            false, /* is_telemetry_enabled */
        );
        // This is usually done by the terminal manager after constructing the blocklist,
        // but we have tests assuming the separator exists.
        if self.restored_blocks.is_some() {
            block_list.append_session_restoration_separator_to_block_list(false);
        }
        block_list
    }
}

impl Default for TestBlockListBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// A helper struct for creating a [`Block`] for use in tests.
///
/// For example, to create a [`Block`] that respects the user's custom prompt:
///
/// ```no_run
/// # use warp::terminal::model::test_utils::TestBlockBuilder;
/// let block = TestBlockBuilder::new()
///     .with_honor_ps1(true)
///     .build();
/// ```
pub struct TestBlockBuilder {
    block_index: BlockIndex,
    honor_ps1: bool,
    event_proxy: ChannelEventListener,
    size: BlockSize,
    bootstrap_stage: BootstrapStage,
}

/// A helper struct for creating a [`Block`] for use in tests.
impl TestBlockBuilder {
    pub fn new() -> Self {
        Self {
            block_index: BlockIndex::zero(),
            honor_ps1: false,
            event_proxy: ChannelEventListener::new_for_test(),
            size: block_size(),
            bootstrap_stage: BootstrapStage::PostBootstrapPrecmd,
        }
    }

    pub fn with_block_index(mut self, block_index: BlockIndex) -> Self {
        self.block_index = block_index;
        self
    }

    pub fn with_honor_ps1(mut self, honor_ps1: bool) -> Self {
        self.honor_ps1 = honor_ps1;
        self
    }

    pub fn with_event_proxy(mut self, event_proxy: ChannelEventListener) -> Self {
        self.event_proxy = event_proxy;
        self
    }

    pub fn with_size_info(mut self, size: SizeInfo) -> Self {
        self.size.size = size;
        self
    }

    pub fn with_bootstrap_stage(mut self, bootstrap_stage: BootstrapStage) -> Self {
        self.bootstrap_stage = bootstrap_stage;
        self
    }

    pub fn build(self) -> Block {
        Block::new(
            BlockId::new(),
            self.size,
            self.event_proxy,
            Arc::new(Background::default()),
            self.bootstrap_stage,
            false, /* show_warp_bootstrap_input */
            false, /* show_in_band_command_blocks */
            false, /* show_memory_stats */
            self.block_index,
            self.honor_ps1,
            ObfuscateSecrets::No,
            false, /* is_telemetry_enabled */
            None,
        )
    }
}

impl Default for TestBlockBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_test_block_with_grids(
    block_index: BlockIndex,
    prompt_and_command_grid: super::blockgrid::BlockGrid,
    rprompt_grid: super::blockgrid::BlockGrid,
    output_grid: super::blockgrid::BlockGrid,
    honor_ps1: bool,
) -> super::block::Block {
    let mut block = TestBlockBuilder::new()
        .with_block_index(block_index)
        .with_honor_ps1(honor_ps1)
        .build();
    block.set_prompt_and_command_grid(prompt_and_command_grid);
    block.set_rprompt_grid(rprompt_grid);
    block.set_output_grid(output_grid);
    block
}

impl TerminalModel {
    /// Creates a simple, default [`TerminalModel`] with an optional
    /// set of restored blocks and an optional [`ChannelEventListener`]
    /// to subscribe to terminal events.
    ///
    /// See [`TerminalModel::new_for_test`] for a more configurable
    /// test constructor.
    pub fn mock(
        restored_blocks: Option<&[SerializedBlockListItem]>,
        event_proxy: Option<ChannelEventListener>,
    ) -> TerminalModel {
        TerminalModel::new_for_test(
            block_size(),
            color::List::from(&Colors::default()),
            event_proxy.unwrap_or_else(ChannelEventListener::new_for_test),
            Arc::new(Background::default()),
            false,
            restored_blocks,
            false,
            false, /* is_inverted */
            None,
        )
    }

    /// Simulates the creation of a block as if the `input` command
    /// was run and it produced `output` bytes.
    ///
    /// This includes invoking all of the relevant hooks that
    /// would be invoked by running the command against a real PTY
    /// (e.g. pre-exec, pre-cmd).
    pub fn simulate_block<B: AsBytes>(&mut self, input: B, output: B) {
        self.simulate_long_running_block(input, output);
        self.block_list_mut()
            .active_block_mut()
            .set_was_long_running(false.into());
        self.finish_block();
    }

    /// Simulates the creation of a long-running block as if the `input` command
    /// was run and it produced `output_so_far` bytes.
    pub fn simulate_long_running_block<B: AsBytes>(&mut self, input: B, output_so_far: B) {
        self.block_list_mut().active_block_mut().start();
        self.simulate_cmd(input);
        self.process_bytes(output_so_far);
        self.block_list_mut()
            .active_block_mut()
            .set_was_long_running(true.into());
    }

    /// Simulates a command being run by writing the `input`
    /// bytes as input and subsequently calling pre-exec.
    ///
    /// We assume that `input` forms a valid UTF-8 string.
    pub fn simulate_cmd<B: AsBytes>(&mut self, input: B) {
        self.process_bytes(input.as_bytes());
        self.preexec(PreexecValue {
            command: std::str::from_utf8(input.as_bytes()).unwrap().to_owned(),
            session_id: None,
        });
    }

    /// Simulates the completion of a block.
    /// Assumes that a block was running to begin with.
    pub fn finish_block(&mut self) {
        let completion_metadata = CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        };
        self.command_finished(CommandFinishedValue {
            completion_metadata: completion_metadata.clone(),
            session_id: None,
        });
        self.precmd(PrecmdValue {
            completion_metadata,
            prompt_metadata: PromptMetadata {
                session_id: Some(0),
                ..Default::default()
            },
        });
    }

    /// Processes a set of `bytes` and applies them to the model,
    /// akin to what happens when reading bytes from a real PTY.
    pub fn process_bytes<B: AsBytes>(&mut self, bytes: B) {
        let bytes = bytes.as_bytes();
        let mut processor = Processor::new();
        processor.parse_bytes(
            self,
            bytes,
            // For unit tests, there's no shell to write back to
            // so the writes should no-op.
            &mut sink(),
        );
    }
}

/// A helper trait to make it more ergonomic
/// to use types that can be converted to a byte slice.
pub trait AsBytes {
    fn as_bytes(&self) -> &[u8];
}

impl AsBytes for &str {
    fn as_bytes(&self) -> &[u8] {
        str::as_bytes(self)
    }
}

impl AsBytes for &[u8] {
    fn as_bytes(&self) -> &[u8] {
        self
    }
}
