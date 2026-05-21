use core::slice;
use std::{
    any::Any,
    cell::{Cell, Ref, RefCell},
    collections::{HashMap, HashSet},
    fmt, mem,
    ops::{Add, AddAssign, Range, Sub, SubAssign},
    sync::Arc,
};

use parking_lot::Mutex;
use rangemap::RangeSet;

use float_cmp::ApproxEq;
use itertools::Itertools;
use markdown_parser::TableAlignment;
use num_traits::SaturatingSub;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use sum_tree::{SeekBias, SumTree};
use vec1::Vec1;
use vim::vim::{MotionType, VimMode};
use warp_core::{
    channel::ChannelState,
    ui::{Icon, theme::Fill as ThemeFill},
};
use warpui::{
    AppContext, Entity, EntityId, ModelContext, ModelHandle,
    assets::asset_cache::AssetSource,
    color::ColorU,
    elements::{Border, Fill, ListNumbering, Margin, MouseStateHandle, Padding, ScrollData},
    fonts::{FamilyId, Properties, Weight},
    geometry::{
        rect::RectF,
        vector::{Vector2F, vec2f},
    },
    platform::LineStyle,
    text_layout::CaretPosition,
    text_layout::{LayoutCache, Line, TextFrame},
    text_selection_utils::{
        NewlineTickParams, calculate_tick_width, create_newline_tick_rect,
        selection_crosses_newline_offset_based,
    },
    units::{IntoPixels, Pixels},
};

pub use self::location::{HitTestOptions, Location};
pub use self::offset_map::{OffsetMap, SelectableTextRun};
pub use self::positioned::Positioned;
use self::{
    location::WrapDirection,
    saved_positions::SavedPositions,
    viewport::{ScrollPositionSnapshot, SizeInfo},
};
use self::{
    positioned::PositionedCursor,
    viewport::{ViewportItem, ViewportIterator, ViewportState},
};
use crate::{
    content::{
        edit::{EditDelta, LaidOutRenderDelta, ParsedUrl, TemporaryBlock, layout_temporary_blocks},
        hidden_lines_model::HiddenLinesModel,
        markdown::MarkdownStyle,
        text::{BlockHeaderSize, BufferBlockStyle, CodeBlockType, FormattedTable},
        version::BufferVersion,
    },
    editor::EmbeddedItemModel,
    render::model::debug::Describe,
};
use string_offset::{CharOffset, impl_offset};
use warpui::elements::ListIndentLevel;

use super::{
    BLOCK_FOOTER_HEIGHT,
    element::{RenderableBlock, broken_embedding::RenderableBrokenEmbedding},
    layout::{TextLayout, line_height},
};

use super::element::{CursorData, RenderContext};

pub mod bounds;
pub(crate) mod debug;
mod location;
mod offset_map;
mod positioned;
pub mod saved_positions;
pub mod table_offset_map;
pub mod viewport;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_utils;

/// Margin for comparing pixel or line values. This is fairly wide because
/// scrolling can introduce a large amount of floating-point rounding error.
pub const UNIT_MARGIN: (f32, i32) = (0.01, 2);
const AUTO_SCROLL_MARGIN: f32 = 12.;

/// The minimum height of a paragraph, not including padding or margins.
pub const PARAGRAPH_MIN_HEIGHT: Pixels = Pixels::new(24.);
const TABLE_SCROLL_REVEAL_MARGIN: Pixels = Pixels::new(8.);

pub const EMBEDDED_ITEM_FIRST_LINE_HEIGHT: f32 = 24.;

pub const TEXT_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(4.).with_right(16.),
    padding: Padding::uniform(0.),
};

pub const COMMAND_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(0.)
        .with_top(8.)
        .with_left(4.)
        .with_bottom(8.)
        .with_right(16.),
    padding: Padding::uniform(8.)
        .with_left(16.)
        .with_top(16.)
        // Reserve space for the buttons.
        .with_bottom(BLOCK_FOOTER_HEIGHT),
};

pub const BROKEN_LINK_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(0.)
        .with_top(8.)
        .with_left(4.)
        .with_bottom(8.)
        .with_right(16.),
    padding: Padding::uniform(0.)
        .with_left(12.)
        .with_top(18.)
        .with_bottom(18.)
        .with_right(8.),
};

pub const HEADER_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(4.)
        .with_top(12.)
        .with_bottom(12.)
        .with_right(16.),
    padding: Padding::uniform(0.),
};

pub const UNORDERED_LIST_MARGIN: Margin = Margin::uniform(4.).with_right(16.);
pub const UNIT_UNORDERED_LIST_PADDING: f32 = 20.;

pub const ORDERED_LIST_MARGIN: Margin = Margin::uniform(4.).with_right(16.);
pub const UNIT_ORDERED_LIST_PADDING: f32 = 20.;

pub const TASK_LIST_MARGIN: Margin = Margin::uniform(4.).with_right(16.);
pub const UNIT_TASK_LIST_PADDING: f32 = 20.;

pub const DEFAULT_BLOCK_SPACINGS: BlockSpacings = BlockSpacings {
    text: TEXT_SPACING,
    header: HEADER_SPACING,
    code_block: COMMAND_SPACING,
    task_list: IndentableBlockSpacing {
        margin: TASK_LIST_MARGIN,
        unit_padding: UNIT_TASK_LIST_PADDING,
    },
    ordered_list: IndentableBlockSpacing {
        margin: ORDERED_LIST_MARGIN,
        unit_padding: UNIT_ORDERED_LIST_PADDING,
    },
    unordered_list: IndentableBlockSpacing {
        margin: UNORDERED_LIST_MARGIN,
        unit_padding: UNIT_UNORDERED_LIST_PADDING,
    },
};

const MIN_HIDDEN_BLOCK_WIDTH: Pixels = Pixels::new(20.);
const HIDDEN_BLOCK_HEIGHT: Pixels = Pixels::new(20.);
pub const CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES: usize = 25;

/// Thickness of underline decorations in pixels.
const UNDERLINE_THICKNESS: f32 = 2.;
/// Length of dashes in dashed underline decorations in pixels.
const DASHED_UNDERLINE_DASH_LENGTH: f32 = 4.;
/// Length of gaps in dashed underline decorations in pixels.
const DASHED_UNDERLINE_GAP_LENGTH: f32 = 4.;

/// In the future, we should also support MinimumWidth(f32) setting so the content will
/// be laid out with a minimum width that could be larger than the viewport.
#[derive(Default)]
pub enum WidthSetting {
    #[default]
    FitViewport,
    InfiniteWidth,
}

/// Block types that support hit-testing on the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTestBlockType {
    Code,
    MermaidDiagram,
    Embedding,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderLayoutOptions {
    pub render_mermaid_diagrams: bool,
    pub mermaid_render_offsets: HashSet<CharOffset>,
}

#[derive(Debug)]
pub enum StyleUpdateAction {
    Relayout,
    Repaint,
    None,
}

#[derive(Default)]
struct RenderBufferVersion {
    last_rendered_version: Option<BufferVersion>,
    next_render_version: Option<BufferVersion>,
}

impl RenderBufferVersion {
    fn start_layout(&mut self) -> Option<BufferVersion> {
        // When layout starts, we eagerly set the last rendered version to the current version we are rendering.
        self.last_rendered_version = self.next_render_version;
        self.last_rendered_version
    }
}

/// Render time decoration that should be applied on a specific range of text / lines.
///
/// Use decorations for transient styles that do not affect the buffer or require text layout.
/// * Persistent styles should be applied as inline markers that are saved as part of the buffer.
/// * Transient styles that affect layout, like syntax highlighting, can't be applied when
///   rendering, so they need to be modeled in the buffer.
#[derive(Default, Debug)]
pub struct RenderDecoration {
    text: Vec<Decoration>,
    line: Vec<LineDecoration>,
}

impl RenderDecoration {
    pub(super) fn text(&self) -> &[Decoration] {
        &self.text
    }

    pub fn line_decoration_ranges(&self) -> &[LineDecoration] {
        &self.line
    }
}

/// Wrapper around a reference to the underlying render state sumtree.
/// This is so we could define an interface that returns objects with the same lifetime as the inner
/// reference with interior mutability.
pub struct RenderContentTreeRef<'a>(Ref<'a, SumTree<BlockItem>>);

impl<'a> RenderContentTreeRef<'a> {
    pub fn block_items(&self) -> impl Iterator<Item = &BlockItem> {
        let mut cursor = self.0.cursor::<(), ()>();
        cursor.descend_to_first_item(&self.0, |_| true);
        std::iter::from_fn(move || {
            let item = cursor.item()?;
            cursor.next();
            Some(item)
        })
    }
    /// Iterator over items visible in the current viewport.
    ///
    /// This returns both the `ViewportItem` and the backing `BlockItem`, for identifying what kind
    /// of item it is. It does not directly return `RenderableBlock`s, as they may depend on
    /// higher-level state.
    pub fn viewport_items(
        &self,
        viewport_height: Pixels,
        viewport_width: Pixels,
        scroll_top: Pixels,
    ) -> impl Iterator<Item = (ViewportItem, &BlockItem)> {
        ViewportIterator::new(&self.0, scroll_top, viewport_height, viewport_width)
    }

    /// Describe only the content of the rendering model.
    #[cfg(test)]
    pub fn describe_content(&self) -> impl fmt::Display + '_ {
        self.0.describe()
    }

    pub fn block_at_height(&self, height: f64) -> Option<Positioned<'_, BlockItem>> {
        let height = Height(OrderedFloat(height));

        let mut cursor = self.0.cursor::<Height, LayoutSummary>();
        // For height, we don't need to seek to exactly the starting height of the block.
        cursor.seek(&height, SeekBias::Right);
        cursor.positioned_item()
    }

    /// Returns the 0-based index of the temporary block at the given content-
    /// space height within its consecutive run of temporary blocks.
    ///
    /// Uses two O(log n) sumtree cursor seeks: one by height to locate the
    /// target block, and one by character offset to find the start of the
    /// temporary-block run. The index is the difference in cumulative item
    /// counts between the two positions.
    ///
    /// Returns `None` if the block at that height is not a `TemporaryBlock`.
    pub fn temp_block_hunk_index_at_height(&self, height: f64) -> Option<usize> {
        let height = Height(OrderedFloat(height));

        // Seek by height to find the target temporary block.
        let mut height_cursor = self.0.cursor::<Height, LayoutSummary>();
        height_cursor.seek(&height, SeekBias::Right);

        // Verify we landed on a temporary block.
        if !matches!(height_cursor.item()?, BlockItem::TemporaryBlock { .. }) {
            return None;
        }

        let target_item_count = height_cursor.start().item_count;
        let boundary_offset = height_cursor.start().content_length;

        // Seek by CharOffset to the same content-length boundary. With Left
        // bias this lands on the last non-temporary block before the run
        // (temporary blocks have content_length == 0, so they don't advance
        // the CharOffset dimension).
        let mut offset_cursor = self.0.cursor::<CharOffset, LayoutSummary>();
        offset_cursor.seek(&boundary_offset, SeekBias::Left);

        // If the offset cursor itself landed on a temporary block, the run
        // starts at the very beginning of the tree (no preceding regular
        // block). Use start().item_count as the boundary.
        let boundary_item_count =
            if matches!(offset_cursor.item(), Some(BlockItem::TemporaryBlock { .. })) {
                offset_cursor.start().item_count
            } else {
                offset_cursor.end().item_count
            };

        Some(target_item_count - boundary_item_count)
    }

    pub fn block_at_offset(&self, offset: CharOffset) -> Option<Positioned<'_, BlockItem>> {
        let mut cursor = self.0.cursor::<CharOffset, LayoutSummary>();
        if cursor.seek(&offset, SeekBias::Right) {
            cursor.positioned_item()
        } else {
            // If we can't seek exactly to the starting CharOffset of the block, the render model
            // has probably changed since this item was created. To be safe, fail the lookup.
            log::trace!("ViewportItem invalidated: no block starting at {offset}");
            None
        }
    }

    pub fn is_entire_range_of_type(
        &self,
        range: &Range<CharOffset>,
        mut matches_type: impl FnMut(&BlockItem) -> bool,
    ) -> bool {
        if range.start >= range.end {
            return false;
        }

        let Some(block) = self.block_at_offset(range.start) else {
            return false;
        };

        block.start_char_offset == range.start
            && block.end_char_offset() == range.end
            && matches_type(block.item)
    }

    pub fn mermaid_block_ranges(&self) -> Vec<Range<CharOffset>> {
        let mut cursor = self.0.cursor::<(), LayoutSummary>();
        cursor.descend_to_first_item(&self.0, |_| true);

        let mut ranges = Vec::new();
        while let Some(item) = cursor.item() {
            if matches!(item, BlockItem::MermaidDiagram { .. }) {
                let start = cursor.start().content_length;
                let end = start + item.content_length();
                ranges.push(start..end);
            }
            cursor.next();
        }

        ranges
    }

    /// Returns the cumulative Y offset (in content-space pixels) at the given line.
    ///
    /// When `line >= total_lines`, returns the total content height.
    pub fn y_offset_at_line(&self, line: LineCount) -> Pixels {
        let summary = self.0.summary();
        if line >= summary.lines {
            return (summary.height as f32).into_pixels();
        }
        let mut cursor = self.0.cursor::<LineCount, LayoutSummary>();
        cursor.seek_clamped(&line, SeekBias::Right);
        (cursor.start().height as f32).into_pixels()
    }
}

/// Model for rendering rich text.
pub struct RenderState {
    /// Content is wrapped in a RefCell so we could mutate it when we are laying out the editor element.
    /// We know this is safe because there is a one-to-one relationship between element and model.
    content: RefCell<SumTree<BlockItem>>,

    selections: RefCell<RenderedSelectionSet>,
    decorations: RenderDecoration,
    hidden_lines: Option<ModelHandle<HiddenLinesModel>>,

    /// Position IDs saved during paint.
    saved_positions: SavedPositions,

    /// State of the current viewport, which determines which items are visible.
    viewport: ViewportState,

    styles: RichTextStyles,

    /// A terminal trailing newline is added after a styled block (for example, a code block)
    /// so that the user can insert unstyled text after it.
    /// This extra newline doesn't make sense in read-only rich text, so it can be disabled.
    show_final_trailing_newline_when_non_empty: bool,
    has_final_trailing_newline: Cell<bool>,

    width_setting: WidthSetting,

    /// Channel for propagating updates from [`super::element::RichTextElement`] such as the
    /// viewport size.
    element_tx: async_channel::Sender<ElementUpdate>,
    /// Channel for laying out edits. Any model updates that require text layout are routed through
    /// this channel, along with updates that must be ordered with respect to text layout (like
    /// cursor movement).
    layout_tx: async_channel::Sender<LayoutAction>,

    /// A count of outstanding layouts.
    #[cfg(any(test, feature = "test-util"))]
    outstanding_layouts: Arc<std::sync::atomic::AtomicUsize>,

    /// The render-related content version the model is managing.
    buffer_version: RefCell<RenderBufferVersion>,

    /// Whether we are performing a lazy layout.
    lazy_layout: bool,

    pending_edits: Mutex<Vec<PendingLayout>>,
    pending_selection_change: Mutex<Option<PendingSelectionUpdate>>,
    layout_options: RenderLayoutOptions,

    /// Optional path to the document being rendered, used for resolving relative paths
    /// (e.g. relative image paths in markdown).
    document_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderEvent {
    /// The viewport size changed, and so the editor model must re-layout its contents accordingly.
    NeedsResize,
    /// New edits have been applied to the editor model.
    LayoutUpdated,
    /// Pending edits were flushed during lazy layout.
    PendingEditsFlushed,
    ViewportUpdated(Option<BufferVersion>),
}

/// Styles for rendering rich text. This fills a similar role to `UiComponentStyles`,
/// but is specialized for text.
#[derive(Clone, PartialEq, Debug)]
pub struct RichTextStyles {
    /// The text styles to use for regular body text.
    pub base_text: ParagraphStyles,
    /// The text styles to use for code.
    pub code_text: ParagraphStyles,
    /// The background fill to use for code blocks.
    pub code_background: Fill,
    /// The background fill to use for embeddings.
    pub embedding_background: Fill,
    /// The text styles to use for embeddings.
    pub embedding_text: ParagraphStyles,
    /// The border to use for code blocks.
    pub code_border: Border,
    /// The color to use for placeholder text.
    pub placeholder_color: ColorU,
    /// The fill to use for text selections.
    pub selection_fill: Fill,
    /// The fill to use for cursors.
    pub cursor_fill: Fill,
    /// Styling for inline code blocks.
    pub inline_code_style: InlineCodeStyle,
    /// Styling for inline checkbox.
    pub check_box_style: CheckBoxStyle,
    /// Styling for horizontal rules.
    pub horizontal_rule_style: HorizontalRuleStyle,
    /// Path to the broken link icon svg.
    pub broken_link_style: BrokenLinkStyle,
    /// Spacing configuration for blocks.
    pub block_spacings: BlockSpacings,
    /// Minimum height a paragraph will take. This currently
    /// is only applied for some blocks like trailing cursor, text
    /// and headers.
    pub minimum_paragraph_height: Option<Pixels>,
    /// Whether to show placeholder text on empty blocks.
    pub show_placeholder_text_on_empty_block: bool,
    /// Width of the cursor
    pub cursor_width: f32,
    /// Whether to highlight detected URLs.
    pub highlight_urls: bool,
    /// Styling for tables.
    pub table_style: TableStyle,
}

#[derive(Clone, PartialEq, Debug)]
pub struct IndentableBlockSpacing {
    margin: Margin,
    unit_padding: f32,
}

impl IndentableBlockSpacing {
    pub fn to_spacing(&self, indent_level: ListIndentLevel) -> BlockSpacing {
        BlockSpacing {
            margin: self.margin,
            padding: Padding::uniform(0.)
                .with_left((indent_level.as_usize() as f32 + 1.) * self.unit_padding),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct BlockSpacings {
    pub text: BlockSpacing,
    pub header: BlockSpacing,
    pub code_block: BlockSpacing,
    pub task_list: IndentableBlockSpacing,
    pub ordered_list: IndentableBlockSpacing,
    pub unordered_list: IndentableBlockSpacing,
}

impl Default for BlockSpacings {
    fn default() -> Self {
        DEFAULT_BLOCK_SPACINGS
    }
}

impl BlockSpacings {
    pub fn from_block_style(&self, block_type: &BufferBlockStyle) -> BlockSpacing {
        match block_type {
            BufferBlockStyle::Header { .. } => self.header,
            BufferBlockStyle::OrderedList { indent_level, .. } => {
                self.ordered_list.to_spacing(*indent_level)
            }
            BufferBlockStyle::UnorderedList { indent_level } => {
                self.unordered_list.to_spacing(*indent_level)
            }
            BufferBlockStyle::TaskList { indent_level, .. } => {
                self.task_list.to_spacing(*indent_level)
            }
            BufferBlockStyle::PlainText | BufferBlockStyle::Table { .. } => self.text,
            BufferBlockStyle::CodeBlock { .. } => self.code_block,
        }
    }
}

/// Grouping of font-related styles for rendering a specific category of text
/// (such as code or headings). In most word processors, these are referred to
/// as paragraph styles, so we keep that naming here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParagraphStyles {
    pub font_family: FamilyId,
    pub font_size: f32,
    pub font_weight: Weight,
    pub line_height_ratio: f32,
    pub text_color: ColorU,
    pub baseline_ratio: f32,
    /// Fixed-width tab stop size in spaces (intended only for fully monospace paragraphs).
    pub fixed_width_tab_size: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckBoxStyle {
    pub border_width: f32,
    pub border_color: ColorU,
    pub icon_path: &'static str,
    pub background: ColorU,
    pub hover_background: ColorU,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrokenLinkStyle {
    pub icon_path: &'static str,
    pub icon_color: ColorU,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalRuleStyle {
    pub rule_height: f32,
    pub color: ColorU,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableStyle {
    pub border_color: ColorU,
    pub header_background: ColorU,
    pub cell_background: ColorU,
    pub alternate_row_background: Option<ColorU>,
    pub text_color: ColorU,
    pub header_text_color: ColorU,
    pub scrollbar_nonactive_thumb_color: ColorU,
    pub scrollbar_active_thumb_color: ColorU,
    pub font_family: FamilyId,
    pub font_size: f32,
    pub cell_padding: f32,
    pub outer_border: bool,
    pub column_dividers: bool,
    pub row_dividers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineCodeStyle {
    pub font_family: FamilyId,
    pub background: ColorU,
    pub font_color: ColorU,
}

impl InlineCodeStyle {
    pub fn requires_relayout(&self, new_styles: &InlineCodeStyle) -> bool {
        self.font_family != new_styles.font_family
    }
}

impl TableStyle {
    pub fn requires_relayout(&self, new_styles: &TableStyle) -> bool {
        self.font_family != new_styles.font_family
            || self.font_size != new_styles.font_size
            || self.cell_padding != new_styles.cell_padding
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutSummary {
    content_length: CharOffset,
    height: f64,
    width: Pixels,
    lines: LineCount,
    item_count: usize,
}

/// Rich text height, in pixels. This wrapper makes the dimension clear (height,
/// not width), and implements SumTree requirements.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Height(OrderedFloat<f64>);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Width(OrderedFloat<Pixels>);

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct LineCount(usize);

impl_offset!(LineCount);

impl LineCount {
    pub fn as_u32(&self) -> u32 {
        self.0 as u32
    }
}

/// A character offset within a [`TextFrame`]. These offsets count characters in the Rust string
/// passed to [`warpui::text_layout::LayoutCache::layout_text()`].
///
/// Frame offsets often, but not always, correspond to glyph indices and caret positions. However,
/// they do not line up 1:1 if a glyph or grapheme contains multiple characters
///
/// They often, but not always, line up with [`CharOffset`]s in the buffer. This is not the case
/// for placeholder text (which occupies 1 character in the buffer, but several in the text frame).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameOffset(usize);

impl_offset!(FrameOffset);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenderLineLocation {
    /// Referring to a temporary block in the render state.
    Temporary {
        at_line: LineCount,
        /// Number of blocks from the at line.
        index_from_at_line: usize,
    },
    /// Referring to a line that exists in the current buffer.
    Current(LineCount),
}

impl RenderLineLocation {
    pub fn line_count(&self) -> LineCount {
        match self {
            RenderLineLocation::Temporary { at_line, .. } => *at_line,
            RenderLineLocation::Current(line_count) => *line_count,
        }
    }
}

/// A point within the editor. Unlike character and hard-wrap offsets/points, this accounts for
/// soft-wrapping, the layout of different block types, and proportional fonts.
///
/// Because soft-wrapping depends on the fonts and viewport size, `SoftWrapPoint` is not stable
/// across resizes or style changes.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SoftWrapPoint {
    /// A soft-wrapped line index within the laid-out document.
    row: u32,
    /// The point's x-offset in pixels. We use pixels here, rather than an integer count, to
    /// support visual navigation. When navigating up or down, we want to move to the point
    /// visually above or below the starting point. That point has the same pixel x-offset,
    /// but, due to variable padding and character widths, not necessarily the same character
    /// offset.
    ///
    /// For example, imagine the given text laid out with a non-monospace font, where the
    /// middle line is a list item:
    ///
    /// ```text
    /// aaaaaaa
    /// * mmmmmm
    /// iiiiiii
    /// ```
    ///
    /// If the cursor is at the start of the middle line, the characters above and below it are
    /// in the **middle** of their respective lines. Furthermore, the `a` and `i` glyphs are
    /// different widths, so the character offsets on all 3 lines are different. However, they
    /// have the same pixel x-offset.
    column: Pixels,
}

impl SoftWrapPoint {
    pub fn new(row: u32, column: Pixels) -> Self {
        Self { row, column }
    }

    /// Move to the previous row with the same column.
    pub fn previous_row(mut self) -> Option<Self> {
        self.row = self.row.checked_sub(1)?;
        Some(self)
    }

    /// Move to the next row with the same column. This is bound by the max row count.
    pub fn next_row(mut self, max_row: LineCount) -> Option<Self> {
        let next_row = self.row + 1;
        if next_row > max_row.0 as u32 {
            None
        } else {
            self.row = next_row;
            Some(self)
        }
    }

    pub fn row(&self) -> u32 {
        self.row
    }

    pub fn column(&self) -> Pixels {
        self.column
    }
}

/// A block of rich text, like a paragraph, runnable command, or list item.
#[derive(Debug, Clone)]
pub enum BlockItem {
    Paragraph(Paragraph),
    TextBlock {
        paragraph_block: ParagraphBlock,
    },
    TemporaryBlock {
        paragraph_block: ParagraphBlock,
        text_decoration: Vec<Decoration>,
        decoration: Option<ThemeFill>,
    },
    RunnableCodeBlock {
        paragraph_block: ParagraphBlock,
        code_block_type: CodeBlockType,
        /// For Mermaid code blocks that are currently rendered in code-block view because the
        /// Mermaid source is not yet available as a rendered diagram, this carries the asset
        /// source that the view layer can watch. Once the asset transitions to a successful
        /// load, the layout will re-run and emit [`BlockItem::MermaidDiagram`] instead.
        pending_mermaid_asset: Option<AssetSource>,
    },
    MermaidDiagram {
        content_length: CharOffset,
        asset_source: AssetSource,
        config: ImageBlockConfig,
    },
    TaskList {
        indent_level: ListIndentLevel,
        complete: bool,
        paragraph: Paragraph,
        mouse_state: MouseStateHandle,
    },
    UnorderedList {
        indent_level: ListIndentLevel,
        paragraph: Paragraph,
    },
    OrderedList {
        indent_level: ListIndentLevel,
        number: Option<usize>,
        paragraph: Paragraph,
    },
    Header {
        header_size: BlockHeaderSize,
        paragraph: Paragraph,
    },
    Embedded(Arc<dyn LaidOutEmbeddedItem>),
    HorizontalRule(HorizontalRuleConfig),
    Image {
        alt_text: String,
        source: String,
        asset_source: AssetSource,
        config: ImageBlockConfig,
    },
    Table(Box<LaidOutTable>),
    TrailingNewLine(Cursor),
    Hidden(HiddenBlockConfig),
}

pub struct EmbeddedItemHTMLRepresentation<'a> {
    pub element_name: &'a str,
    pub content: String,
    pub attributes: HashMap<&'a str, &'a str>,
}

pub struct EmbeddedItemRichFormat<'a> {
    pub html: EmbeddedItemHTMLRepresentation<'a>,
    pub plain_text: String,
}

pub trait EmbeddedItem: std::fmt::Debug + Send + Sync {
    // Layout the embedded item with the current text layout context.
    fn layout(&self, text_layout: &TextLayout, app: &AppContext) -> Box<dyn LaidOutEmbeddedItem>;
    fn hashed_id(&self) -> &str;
    /// Serializes this item as YAML.
    fn to_mapping(&self, style: MarkdownStyle) -> Mapping;
    // Returns the rich format of the embedded item used for copy & pasting.
    fn to_rich_format(&self, app: &AppContext) -> EmbeddedItemRichFormat<'_>;
}

pub trait LaidOutEmbeddedItem: std::fmt::Debug + Send + Sync {
    fn height(&self) -> Pixels;
    fn size(&self) -> Vector2F;
    fn first_line_bound(&self) -> Vector2F;
    fn element(
        &self,
        state: &RenderState,
        viewport_item: ViewportItem,
        model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock>;
    fn spacing(&self) -> BlockSpacing;
    /// Returns this object as a ref to the Any type.  Needed for typecasts.
    fn as_any(&self) -> &dyn Any;
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct BlockSpacing {
    pub margin: Margin,
    pub padding: Padding,
}

impl BlockSpacing {
    // Total additional offset on the x-axis with padding and margin combined.
    pub fn x_axis_offset(&self) -> Pixels {
        (self.margin.left() + self.margin.right() + self.padding.left() + self.padding.right())
            .into_pixels()
    }

    // Total additional offset on the y-axis with padding and margin combined.
    pub fn y_axis_offset(&self) -> Pixels {
        (self.margin.top() + self.margin.bottom() + self.padding.top() + self.padding.bottom())
            .into_pixels()
    }

    pub fn top_offset(&self) -> Pixels {
        (self.margin.top() + self.padding.top()).into_pixels()
    }

    pub fn left_offset(&self) -> Pixels {
        (self.margin.left() + self.padding.left()).into_pixels()
    }

    fn without_y_axis_offsets(mut self) -> Self {
        self.margin = Margin::default()
            .with_left(self.margin.left())
            .with_right(self.margin.right());
        self.padding = Padding::default()
            .with_left(self.padding.left())
            .with_right(self.padding.right());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ParagraphBlock {
    paragraphs: Vec1<Paragraph>,
}

impl ParagraphBlock {
    pub fn new(paragraphs: Vec1<Paragraph>) -> Self {
        Self { paragraphs }
    }

    pub fn spacing(&self) -> BlockSpacing {
        // In the future, we should support two separate level of spacing in a
        // ParagraphBlock: 1) the internal spacing between paragraphs 2) the overall
        // spacing of the block.
        self.paragraphs.first().spacing()
    }

    pub fn first_line_height(&self) -> f32 {
        self.paragraphs.first().first_line_height()
    }

    pub fn paragraphs(&self) -> &[Paragraph] {
        &self.paragraphs
    }

    fn content_length(&self) -> CharOffset {
        self.paragraphs
            .iter()
            .fold(CharOffset::zero(), |sum, paragraph| {
                sum + paragraph.content_length
            })
    }

    pub fn width(&self) -> Pixels {
        self.paragraphs
            .iter()
            .map(|paragraph| paragraph.width().as_f32())
            .max_by(|a, b| a.partial_cmp(b).expect("Tried to compare a NaN"))
            .unwrap_or(0.)
            .into_pixels()
    }

    pub fn height(&self) -> Pixels {
        self.paragraphs
            .iter()
            .map(|paragraph| paragraph.height.as_f32())
            .sum::<f32>()
            .into_pixels()
    }

    /// The size of this paragraph block's content, as currently laid out.
    pub fn content_size(&self) -> Vector2F {
        let width = self
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.frame.max_width())
            .reduce(f32::max)
            .unwrap_or(0.);
        vec2f(width, self.height().as_f32())
    }

    fn lines(&self) -> LineCount {
        self.paragraphs
            .iter()
            .fold(LineCount(0), |sum, paragraph| sum + paragraph.lines())
    }

    /// Returns `true` if this paragraph block is effectively empty.
    fn is_empty(&self) -> bool {
        // If there are multiple empty paragraphs, consider the paragraph non-empty, since that
        // implies the user added at least one line. If there are no paragraphs, or a single empty
        // paragraph (more likely, given our layout logic), consider the whole block empty.
        match self.paragraphs.iter().at_most_one() {
            Ok(None) => true,
            Ok(Some(paragraph)) => paragraph.is_empty(),
            Err(_) => false,
        }
    }
}

#[derive(Clone)]
pub struct Paragraph {
    /// Laid-out text content of this paragraph.
    frame: Arc<TextFrame>,
    /// Mapping between [`TextFrame`] characters and content characters.
    offsets: OffsetMap,
    /// Cached height of this paragraph's text frame.
    height: Pixels,
    width: Pixels,
    /// Content length of this paragraph, in `char`s.
    content_length: CharOffset,
    detected_url: Vec<ParsedUrl>,
    spacing: BlockSpacing,
    minimum_height: Option<Pixels>,
}

impl Paragraph {
    pub fn new(
        frame: Arc<TextFrame>,
        offsets: OffsetMap,
        content_length: CharOffset,
        active_url: Vec<ParsedUrl>,
        spacing: BlockSpacing,
        minimum_height: Option<Pixels>,
    ) -> Self {
        let height = frame
            .lines()
            .iter()
            .fold(0f32, |acc, line| acc + line_height(line))
            .into_pixels();

        let width = frame.max_width().into_pixels();
        Self {
            frame,
            offsets,
            height,
            width,
            content_length,
            detected_url: active_url,
            spacing,
            minimum_height,
        }
    }

    pub fn first_line_height(&self) -> f32 {
        self.frame
            .lines()
            .first()
            .map(line_height)
            .unwrap_or(self.height.as_f32())
    }

    pub fn spacing(&self) -> BlockSpacing {
        self.spacing
    }

    pub(super) fn frame(&self) -> &TextFrame {
        &self.frame
    }

    /// Whether or not this paragraph is effectively empty.
    pub(super) fn is_empty(&self) -> bool {
        let lines = self.frame.lines();
        lines.is_empty() || lines.iter().all(|line| line.runs.is_empty())
    }

    /// The height of this paragraph.
    pub fn height(&self) -> Pixels {
        self.height
    }

    pub fn width(&self) -> Pixels {
        self.width
    }

    fn lines(&self) -> LineCount {
        LineCount(self.frame.lines().len())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HorizontalRuleConfig {
    pub line_height: Pixels,
    pub width: Pixels,
    pub spacing: BlockSpacing,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageBlockConfig {
    pub width: Pixels,
    pub height: Pixels,
    pub spacing: BlockSpacing,
}

#[derive(Debug, Clone, Copy)]
pub struct TableBlockConfig {
    pub width: Pixels,
    pub spacing: BlockSpacing,
    pub style: TableStyle,
}

/// Layout information for a single table cell, including line-level details
/// for proper text selection rendering in cells with wrapped text.
#[derive(Debug, Clone, Default)]
pub struct CellLayout {
    pub line_heights: Vec<f32>,
    pub line_y_offsets: Vec<f32>,
    pub line_char_ranges: Vec<Range<CharOffset>>,
    pub line_widths: Vec<f32>,
    pub line_caret_positions: Vec<Vec<CaretPosition>>,
}

impl CellLayout {
    pub fn from_text_frame(frame: &TextFrame) -> Self {
        let lines = frame.lines();
        let mut line_heights = Vec::with_capacity(lines.len());
        let mut line_y_offsets = Vec::with_capacity(lines.len());
        let mut line_char_ranges = Vec::with_capacity(lines.len());
        let mut line_widths = Vec::with_capacity(lines.len());
        let mut line_caret_positions = Vec::with_capacity(lines.len());
        let mut y_offset = 0.0;

        for line in lines.iter() {
            let height = line.font_size * line.line_height_ratio;
            line_heights.push(height);
            line_y_offsets.push(y_offset);
            line_widths.push(line.width);
            line_caret_positions.push(line.caret_positions.clone());
            y_offset += height;

            let char_start = line
                .caret_positions
                .first()
                .map(|cp| cp.start_offset)
                .unwrap_or(0);
            let char_end = line
                .caret_positions
                .last()
                .map(|cp| cp.last_offset + 1)
                .unwrap_or(char_start);
            line_char_ranges.push(CharOffset::from(char_start)..CharOffset::from(char_end));
        }

        Self {
            line_heights,
            line_y_offsets,
            line_char_ranges,
            line_widths,
            line_caret_positions,
        }
    }

    pub fn line_at_char_offset(&self, char_offset: CharOffset) -> Option<usize> {
        for (i, range) in self.line_char_ranges.iter().enumerate() {
            if char_offset < range.end {
                return Some(i);
            }
        }
        if !self.line_char_ranges.is_empty() {
            Some(self.line_char_ranges.len() - 1)
        } else {
            None
        }
    }

    pub fn x_for_char_in_line(&self, line_idx: usize, char_offset: usize) -> f32 {
        let Some(carets) = self.line_caret_positions.get(line_idx) else {
            return 0.0;
        };
        let width = self.line_widths.get(line_idx).copied().unwrap_or(0.0);
        for caret in carets {
            if caret.contains_index(char_offset) {
                return caret.position_in_line;
            }
        }
        if carets
            .first()
            .is_some_and(|caret| char_offset < caret.start_offset)
        {
            0.0
        } else {
            width
        }
    }

    pub fn line_at_y_offset(&self, y: f32) -> usize {
        for i in 0..self.line_y_offsets.len() {
            let line_top = self.line_y_offsets[i];
            let line_bottom = line_top + self.line_heights.get(i).copied().unwrap_or(0.0);
            if y >= line_top && y < line_bottom {
                return i;
            }
        }
        self.line_y_offsets.len().saturating_sub(1)
    }

    /// Returns the nearest character offset for a horizontal hit-test within a line.
    ///
    /// The explicit caret list does not include the insertion point at the visual end of the
    /// line, so we compare against both the stored caret positions and the implicit line-end
    /// caret at `line_width`. That keeps table hit-testing from snapping to the last glyph when a
    /// click near the right edge is visually closer to the position after it.
    pub fn char_at_x_in_line(&self, line_idx: usize, x: f32) -> CharOffset {
        let Some(range) = self.line_char_ranges.get(line_idx) else {
            return CharOffset::zero();
        };
        if range.start >= range.end {
            return range.start;
        }

        let Some(carets) = self.line_caret_positions.get(line_idx) else {
            return range.start;
        };
        if carets.is_empty() || x <= 0.0 {
            return range.start;
        }

        let line_width = self.line_widths.get(line_idx).copied().unwrap_or(0.0);
        if x >= line_width {
            return range.end;
        }
        let mut closest = range.start;
        let mut closest_distance = f32::INFINITY;

        for caret in carets {
            let distance = (caret.position_in_line - x).abs();
            if distance <= closest_distance {
                closest = CharOffset::from(caret.start_offset).clamp(range.start, range.end);
                closest_distance = distance;
            }
        }

        if (line_width - x).abs() <= closest_distance {
            range.end
        } else {
            closest
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaidOutTable {
    pub table: FormattedTable,
    pub config: TableBlockConfig,
    pub row_heights: Vec<Pixels>,
    pub column_widths: Vec<Pixels>,
    pub total_height: Pixels,
    pub offset_map: table_offset_map::TableOffsetMap,
    pub content_length: CharOffset,
    pub cell_offset_maps: Vec<Vec<table_offset_map::TableCellOffsetMap>>,
    pub row_y_offsets: Vec<f32>,
    pub col_x_offsets: Vec<f32>,
    pub cell_text_frames: Vec<Vec<Arc<TextFrame>>>,
    pub cell_layouts: Vec<Vec<CellLayout>>,
    pub cell_links: Vec<Vec<Vec<ParsedUrl>>>,
    pub scroll_left: Cell<Pixels>,
    pub(crate) scrollbar_interaction_state: TableScrollbarInteractionState,
    /// When `false`, the surrounding container already owns horizontal scrolling, so this table
    /// should render at full intrinsic width without introducing its own clip, scrollbar, or
    /// scroll event handling.
    pub horizontal_scroll_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TableScrollbarDragState {
    pub start_position_x: Pixels,
    pub start_scroll_left: Pixels,
    pub scroll_data: ScrollData,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TableScrollbarInteractionState {
    drag_state: Cell<Option<TableScrollbarDragState>>,
    hovered: Cell<bool>,
}

impl LaidOutTable {
    pub fn height(&self) -> Pixels {
        self.total_height
    }

    pub fn width(&self) -> Pixels {
        self.config.width
    }

    pub fn spacing(&self) -> BlockSpacing {
        self.config.spacing
    }

    pub fn content_length(&self) -> CharOffset {
        self.content_length
    }

    pub fn viewport_width(&self, viewport_width: Pixels) -> Pixels {
        if !self.horizontal_scroll_allowed {
            return self.width();
        }
        viewport_width.min(self.width())
    }

    pub fn max_scroll_left(&self, viewport_width: Pixels) -> Pixels {
        if !self.horizontal_scroll_allowed {
            return Pixels::zero();
        }
        (self.width() - self.viewport_width(viewport_width)).max(Pixels::zero())
    }

    pub fn scroll_left(&self) -> Pixels {
        if !self.horizontal_scroll_allowed {
            return Pixels::zero();
        }
        self.scroll_left.get()
    }

    pub fn set_scroll_left(&self, scroll_left: Pixels, viewport_width: Pixels) -> bool {
        if !self.horizontal_scroll_allowed {
            return false;
        }
        let clamped = scroll_left
            .max(Pixels::zero())
            .min(self.max_scroll_left(viewport_width));
        if clamped.approx_eq(self.scroll_left.get(), UNIT_MARGIN) {
            false
        } else {
            self.scroll_left.set(clamped);
            true
        }
    }

    pub fn scroll_horizontally(&self, delta: Pixels, viewport_width: Pixels) -> bool {
        if !self.horizontal_scroll_allowed {
            return false;
        }
        self.set_scroll_left(self.scroll_left.get() - delta, viewport_width)
    }

    pub(crate) fn start_scrollbar_drag(&self, start_position_x: Pixels, scroll_data: ScrollData) {
        self.scrollbar_interaction_state
            .drag_state
            .set(Some(TableScrollbarDragState {
                start_position_x,
                start_scroll_left: self.scroll_left(),
                scroll_data,
            }));
    }

    pub(crate) fn end_scrollbar_drag(&self) -> bool {
        self.scrollbar_interaction_state.drag_state.take().is_some()
    }

    pub(crate) fn scrollbar_drag_state(&self) -> Option<TableScrollbarDragState> {
        self.scrollbar_interaction_state.drag_state.get()
    }

    pub(crate) fn scrollbar_hovered(&self) -> bool {
        self.scrollbar_interaction_state.hovered.get()
    }

    pub(crate) fn set_scrollbar_hovered(&self, hovered: bool) -> bool {
        self.scrollbar_interaction_state.hovered.replace(hovered) != hovered
    }

    pub(crate) fn clear_scrollbar_interaction_state(&self) {
        self.scrollbar_interaction_state.drag_state.set(None);
        self.scrollbar_interaction_state.hovered.set(false);
    }

    pub fn reveal_offset(&self, offset: CharOffset, viewport_width: Pixels) -> bool {
        if !self.horizontal_scroll_allowed {
            return false;
        }
        let Some(bounds) = self.relative_character_bounds(offset) else {
            return false;
        };
        let viewport_width = self.viewport_width(viewport_width);
        let mut new_scroll_left = self.scroll_left.get();
        let visible_start = self.scroll_left.get();
        let visible_end = visible_start + viewport_width;
        let character_start = Pixels::new(bounds.origin_x());
        let character_end = Pixels::new(bounds.origin_x() + bounds.width());

        if character_start - TABLE_SCROLL_REVEAL_MARGIN < visible_start {
            new_scroll_left = (character_start - TABLE_SCROLL_REVEAL_MARGIN).max(Pixels::zero());
        } else if character_end + TABLE_SCROLL_REVEAL_MARGIN > visible_end {
            new_scroll_left =
                (character_end + TABLE_SCROLL_REVEAL_MARGIN - viewport_width).max(Pixels::zero());
        }

        self.set_scroll_left(new_scroll_left, viewport_width)
    }

    pub fn character_bounds(&self, offset: CharOffset, table_origin: Vector2F) -> Option<RectF> {
        let bounds = self.relative_character_bounds(offset)?;
        Some(RectF::new(
            table_origin + bounds.origin() - vec2f(self.scroll_left.get().as_f32(), 0.0),
            bounds.size(),
        ))
    }

    pub fn lines(&self) -> LineCount {
        LineCount(1 + self.table.rows.len())
    }

    /// Maps an x/y coordinate within the table content bounds to the nearest
    /// character offset in the table's flattened content stream.
    pub fn coordinate_to_offset(&self, x: f32, y: f32) -> CharOffset {
        let row = self.row_at_y(y);
        let col = self.col_at_x(x);

        let Some(cell_range) = self.offset_map.cell_range(row, col) else {
            return CharOffset::zero();
        };
        let Some(cell_offset_map) = self.cell_offset_maps.get(row).and_then(|r| r.get(col)) else {
            return cell_range.start;
        };
        let cell_start = cell_range.start;
        if cell_offset_map.rendered_length() == CharOffset::zero() {
            return cell_start
                + cell_offset_map
                    .rendered_to_source(CharOffset::zero())
                    .as_usize();
        }

        let row_y_start = self.row_y_offsets.get(row).copied().unwrap_or(0.0);
        let col_start_x = self.col_x_offsets.get(col).copied().unwrap_or(0.0);
        let col_width = self
            .column_widths
            .get(col)
            .map(|w| w.as_f32())
            .unwrap_or(0.0);

        let cell_content_start_x = col_start_x + self.config.style.cell_padding;
        let cell_content_start_y = row_y_start + self.config.style.cell_padding;
        let cell_content_width = (col_width - self.config.style.cell_padding * 2.0).max(0.0);

        let x_in_cell = (x - cell_content_start_x).max(0.0);
        let y_in_cell = (y - cell_content_start_y).max(0.0);

        if let Some(cell_layout) = self.cell_layouts.get(row).and_then(|r| r.get(col)) {
            let line_idx = cell_layout.line_at_y_offset(y_in_cell);
            let rendered_offset = cell_layout.char_at_x_in_line(line_idx, x_in_cell);
            return cell_start
                + cell_offset_map
                    .rendered_to_source(rendered_offset)
                    .as_usize();
        }
        if x_in_cell <= 0.0 {
            return cell_start
                + cell_offset_map
                    .rendered_to_source(CharOffset::zero())
                    .as_usize();
        }
        if x_in_cell >= cell_content_width {
            return cell_start
                + cell_offset_map
                    .rendered_to_source(cell_offset_map.rendered_length())
                    .as_usize();
        }
        cell_start
            + cell_offset_map
                .rendered_to_source(CharOffset::zero())
                .as_usize()
    }

    /// Returns the row index containing the provided y coordinate.
    fn row_at_y(&self, y: f32) -> usize {
        let num_rows = self.row_y_offsets.len().saturating_sub(1);
        let idx = self.row_y_offsets.partition_point(|&offset| offset <= y);
        idx.saturating_sub(1).min(num_rows.saturating_sub(1))
    }

    /// Returns the column index containing the provided x coordinate.
    fn col_at_x(&self, x: f32) -> usize {
        let num_cols = self.col_x_offsets.len().saturating_sub(1);
        let idx = self.col_x_offsets.partition_point(|&offset| offset <= x);
        idx.saturating_sub(1).min(num_cols.saturating_sub(1))
    }

    /// Returns the hyperlink URL at the given character offset within the table,
    /// if the offset falls within a linked fragment.
    pub fn link_at_offset(&self, offset: CharOffset) -> Option<String> {
        let cell_at = self.offset_map.cell_at_offset(offset)?;
        let target = self
            .cell_offset_maps
            .get(cell_at.row)?
            .get(cell_at.col)?
            .source_to_rendered(cell_at.offset_in_cell)
            .as_usize();
        self.cell_links
            .get(cell_at.row)?
            .get(cell_at.col)?
            .iter()
            .find(|link| link.url_range().contains(&target))
            .map(ParsedUrl::link)
    }

    fn relative_character_bounds(&self, offset: CharOffset) -> Option<RectF> {
        let cell = self.offset_map.cell_at_offset(offset)?;
        let rendered_offset = self
            .cell_offset_maps
            .get(cell.row)?
            .get(cell.col)?
            .source_to_rendered(cell.offset_in_cell);
        let cell_layout = self.cell_layouts.get(cell.row)?.get(cell.col)?;
        let line_idx = cell_layout
            .line_at_char_offset(rendered_offset)
            .unwrap_or(0);
        let line_y = cell_layout
            .line_y_offsets
            .get(line_idx)
            .copied()
            .unwrap_or(0.0);
        let line_height = cell_layout
            .line_heights
            .get(line_idx)
            .copied()
            .unwrap_or(20.0);
        let start_x = cell_layout.x_for_char_in_line(line_idx, rendered_offset.as_usize());
        let end_x = cell_layout.x_for_char_in_line(line_idx, rendered_offset.as_usize() + 1);
        Some(RectF::new(
            self.cell_content_origin(cell.row, cell.col) + vec2f(start_x, line_y),
            vec2f((end_x - start_x).max(1.0), line_height),
        ))
    }

    pub(crate) fn cell_content_origin(&self, row: usize, col: usize) -> Vector2F {
        let col_start_x = self.col_x_offsets.get(col).copied().unwrap_or(0.0);
        let row_start_y = self.row_y_offsets.get(row).copied().unwrap_or(0.0);
        vec2f(
            col_start_x + self.config.style.cell_padding + self.cell_alignment_x_offset(row, col),
            row_start_y + self.config.style.cell_padding,
        )
    }

    fn cell_alignment_x_offset(&self, row: usize, col: usize) -> f32 {
        let cell_content_width = self
            .column_widths
            .get(col)
            .map(|width| width.as_f32())
            .unwrap_or(0.0)
            - self.config.style.cell_padding * 2.0;
        let cell_content_width = cell_content_width.max(0.0);
        let frame_width = self
            .cell_text_frames
            .get(row)
            .and_then(|row_frames| row_frames.get(col))
            .map(|frame| frame.max_width())
            .unwrap_or(0.0);

        match self.table.alignments.get(col).copied().unwrap_or_default() {
            TableAlignment::Left => 0.0,
            TableAlignment::Center => (cell_content_width - frame_width).max(0.0) / 2.0,
            TableAlignment::Right => (cell_content_width - frame_width).max(0.0),
        }
    }
}

impl HorizontalRuleConfig {
    pub fn line_size(&self) -> Vector2F {
        vec2f(self.width.as_f32(), self.line_height.as_f32())
    }
}

/// A block's position within a code editor.
#[derive(Debug, Clone, Copy)]
pub enum BlockLocation {
    Start,
    Middle,
    End,
}

impl Add for BlockLocation {
    type Output = Self;

    /// Combine two BlockLocations. This operation is non-commutative; the left-hand side should
    /// come before the right-hand side. Precedence order is `Start` > `End` > `Middle`
    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Start, _) => Self::Start,
            (_, Self::End) => Self::End,
            (Self::Middle, Self::Middle) => Self::Middle,
            _ => {
                // Out-of-order block locations should not be added together.
                if ChannelState::enable_debug_features() {
                    log::error!(
                        "Tried to combine block location {self:?} with later location {rhs:?}"
                    );
                }
                Self::Middle
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionType {
    /// Expand the visible section down.
    /// Unhide the top of the hidden section.
    ExpandDown,
    /// Expand the visible section up.
    /// Unhide the bottom of the hidden section.
    ExpandUp,
    /// Unhide the entire hidden section.
    Both,
}

impl ExpansionType {
    pub fn icon(&self) -> Icon {
        match self {
            Self::ExpandDown => Icon::ExpandDown,
            Self::ExpandUp => Icon::ExpandUp,
            Self::Both => Icon::ExpandUpAndDown,
        }
    }
}

/// Get the gutter buttons that will be displayed for a hidden section
/// based on the hidden section's location and the number of lines it covers.
/// Hidden sections at the start and end of a file have one directional button.
/// Large hidden sections in the middle of a file get two buttons to
/// expand in either direction.
/// Small hidden sections in the middle of a file get a single button to
/// expand the entire hidden section at once.
pub fn gutter_expansion_button_types(
    block_location: &BlockLocation,
    hidden_range_line_count: usize,
) -> Vec<ExpansionType> {
    match block_location {
        BlockLocation::Start => vec![ExpansionType::ExpandUp],
        BlockLocation::End => vec![ExpansionType::ExpandDown],
        BlockLocation::Middle => {
            if hidden_range_line_count >= CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES {
                vec![ExpansionType::ExpandDown, ExpansionType::ExpandUp]
            } else {
                vec![ExpansionType::Both]
            }
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct HiddenBlockConfig {
    line_count: LineCount,
    content_length: CharOffset,
    // The location of the block is set when the hidden section is first laid out,
    // and updated in RenderState.dedupe_hidden_ranges.
    block_location: BlockLocation,
}

impl HiddenBlockConfig {
    pub fn new(
        line_count: LineCount,
        content_length: CharOffset,
        block_location: BlockLocation,
    ) -> Self {
        Self {
            line_count,
            content_length,
            block_location,
        }
    }

    pub fn height(&self) -> Pixels {
        let base_height = HIDDEN_BLOCK_HEIGHT.as_f32();
        (self.display_line_count() as f32 * base_height).into_pixels()
    }

    fn display_line_count(&self) -> usize {
        self.gutter_button_types().len()
    }

    fn gutter_button_types(&self) -> Vec<ExpansionType> {
        gutter_expansion_button_types(&self.block_location, self.line_count.as_usize())
    }

    pub fn line_count(&self) -> LineCount {
        self.line_count
    }

    pub fn content_length(&self) -> CharOffset {
        self.content_length
    }
}

impl AddAssign for HiddenBlockConfig {
    fn add_assign(&mut self, other: Self) {
        self.line_count += other.line_count;
        self.content_length += other.content_length;
        self.block_location = self.block_location + other.block_location;
    }
}

/// A placeholder element for a cursor that is at the end of the buffer and on a newline.
/// In this case, we won't have a TextFrame because the new paragraph is empty. But we should
/// still render the cursor.
#[derive(Debug, Clone)]
pub struct Cursor {
    height: Pixels,
    width: Pixels,
    minimum_height: Option<Pixels>,
    spacing: BlockSpacing,
}

impl Cursor {
    pub fn new(
        height: Pixels,
        width: Pixels,
        spacing: BlockSpacing,
        minimum_height: Option<Pixels>,
    ) -> Self {
        Self {
            height,
            width,
            spacing,
            minimum_height,
        }
    }

    pub fn spacing(&self) -> BlockSpacing {
        self.spacing
    }

    /// The size of this cursor, in pixels.
    pub fn size(&self) -> Vector2F {
        vec2f(self.width.as_f32(), self.height.as_f32())
    }
}

impl RenderState {
    /// Create a new `RenderState` model.
    /// The initial content will be a single **trailing newline**.
    pub fn new(
        styles: RichTextStyles,
        lazy_layout: bool,
        hidden_lines: Option<ModelHandle<HiddenLinesModel>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (element_tx, element_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(element_rx, Self::apply_element_update, |_, _| {});

        let (layout_tx, layout_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(layout_rx, Self::handle_layout_action, |_, _| {});

        Self::new_internal(
            ctx.model_id(),
            element_tx,
            layout_tx,
            styles,
            lazy_layout,
            Pixels::zero(),
            Pixels::zero(),
            hidden_lines,
        )
    }

    /// Create a new `RenderState` with the given configuration.
    /// The initial content will be a single **trailing newline**.
    #[cfg(test)]
    pub fn new_for_test(
        styles: RichTextStyles,
        viewport_width: Pixels,
        viewport_height: Pixels,
    ) -> Self {
        let (element_tx, _) = async_channel::unbounded();
        let (layout_tx, _) = async_channel::unbounded();
        Self::new_internal(
            EntityId::new(),
            element_tx,
            layout_tx,
            styles,
            false,
            viewport_width,
            viewport_height,
            None,
        )
    }

    /// Create a new `RenderState` with the given configuration.
    /// The initial content will be a single **trailing newline**.
    #[allow(clippy::too_many_arguments)]
    fn new_internal(
        entity_id: EntityId,
        element_tx: async_channel::Sender<ElementUpdate>,
        layout_tx: async_channel::Sender<LayoutAction>,
        styles: RichTextStyles,
        lazy_layout: bool,
        viewport_width: Pixels,
        viewport_height: Pixels,
        hidden_lines: Option<ModelHandle<HiddenLinesModel>>,
    ) -> Self {
        let content = SumTree::from_item(Self::final_trailing_newline_cursor(&styles));
        Self {
            styles,
            show_final_trailing_newline_when_non_empty: true,
            has_final_trailing_newline: Cell::new(true),
            viewport: ViewportState::new(viewport_width, viewport_height),
            selections: Default::default(),
            decorations: Default::default(),
            content: RefCell::new(content),
            element_tx,
            layout_tx,
            width_setting: Default::default(),
            saved_positions: SavedPositions::new(entity_id),
            buffer_version: RefCell::new(Default::default()),
            lazy_layout,
            pending_edits: Mutex::new(Vec::new()),
            #[cfg(any(test, feature = "test-util"))]
            outstanding_layouts: Default::default(),
            pending_selection_change: Mutex::new(None),
            layout_options: Default::default(),
            document_path: None,
            hidden_lines,
        }
    }

    fn final_trailing_newline_cursor(styles: &RichTextStyles) -> BlockItem {
        BlockItem::TrailingNewLine(Cursor::new(
            styles.base_line_height(),
            styles.cursor_width.into_pixels(),
            styles
                .block_spacings
                .from_block_style(&BufferBlockStyle::PlainText),
            styles.minimum_paragraph_height,
        ))
    }

    fn should_show_final_trailing_newline(&self, tree_is_empty: bool) -> bool {
        self.show_final_trailing_newline_when_non_empty || tree_is_empty
    }

    fn tree_ends_with_trailing_newline(tree: &SumTree<BlockItem>) -> bool {
        let mut cursor = tree.cursor::<(), ()>();
        cursor.descend_to_last_item(tree);
        cursor.item().is_some_and(|item| item.is_trailing_newline())
    }

    fn remove_final_trailing_newline_if_present(&mut self) {
        if !self.has_final_trailing_newline.get() {
            return;
        }

        let content = self.content.get_mut();
        let new_tree = {
            let mut cursor = content.cursor::<CharOffset, ()>();
            cursor.descend_to_last_item(content);

            if !cursor.item().is_some_and(BlockItem::is_trailing_newline)
                || cursor.prev_item().is_none()
            {
                return;
            }

            let last_item_start = *cursor.seek_position();
            let mut slice_cursor = content.cursor::<CharOffset, ()>();
            slice_cursor.slice(&last_item_start, SeekBias::Left)
        };

        *content = new_tree;
        self.has_final_trailing_newline.set(false);
    }

    fn add_final_trailing_newline_if_missing(&mut self) {
        if self.has_final_trailing_newline.get() {
            return;
        }

        self.content
            .get_mut()
            .push(Self::final_trailing_newline_cursor(&self.styles));
        self.has_final_trailing_newline.set(true);
    }

    /// Returns reference to the underlying content tree.
    pub fn content(&self) -> RenderContentTreeRef<'_> {
        RenderContentTreeRef(self.content.borrow())
    }

    pub fn with_width_setting(mut self, setting: WidthSetting) -> Self {
        self.width_setting = setting;
        self
    }

    /// Whether the surrounding container for this render state already provides horizontal
    /// scrolling over its full content area. Blocks that would otherwise introduce a nested
    /// horizontal scroll (for example, wide Markdown tables) should render at full intrinsic
    /// width in that case.
    pub fn container_scrolls_horizontally(&self) -> bool {
        matches!(self.width_setting, WidthSetting::InfiniteWidth)
    }

    pub fn layout_options(&self) -> RenderLayoutOptions {
        self.layout_options.clone()
    }

    pub fn set_render_mermaid_diagrams(&mut self, render_mermaid_diagrams: bool) -> bool {
        if self.layout_options.render_mermaid_diagrams == render_mermaid_diagrams {
            return false;
        }

        self.layout_options.render_mermaid_diagrams = render_mermaid_diagrams;
        true
    }

    pub fn set_mermaid_render_offsets(&mut self, offsets: HashSet<CharOffset>) -> bool {
        if self.layout_options.mermaid_render_offsets == offsets {
            return false;
        }
        self.layout_options.mermaid_render_offsets = offsets;
        true
    }

    pub fn set_show_final_trailing_newline_when_non_empty(&mut self, show: bool) {
        if self.show_final_trailing_newline_when_non_empty == show {
            return;
        }

        self.show_final_trailing_newline_when_non_empty = show;

        if show {
            self.add_final_trailing_newline_if_missing();
        } else {
            self.remove_final_trailing_newline_if_present();
        }

        self.update_content_sizing();
    }

    pub fn max_line(&self) -> LineCount {
        self.content.borrow().summary().lines
    }

    /// The complete height of all laid-out content.
    pub fn height(&self) -> Pixels {
        (self.content.borrow().summary().height as f32).into_pixels()
    }

    pub fn width(&self) -> Pixels {
        self.content.borrow().summary().width
    }

    pub fn next_render_buffer_version(&self) -> Option<BufferVersion> {
        self.buffer_version.borrow().next_render_version
    }

    /// The number of blocks in the render model.
    #[cfg(any(test, feature = "test-util"))]
    pub fn blocks(&self) -> usize {
        self.content().block_items().count()
    }

    pub fn markdown_table_count(&self) -> usize {
        self.content()
            .block_items()
            .filter(|item| matches!(item, BlockItem::Table(_)))
            .count()
    }

    pub fn is_entire_range_of_type(
        &self,
        range: &Range<CharOffset>,
        matches_type: impl FnMut(&BlockItem) -> bool,
    ) -> bool {
        self.content().is_entire_range_of_type(range, matches_type)
    }

    /// The max offset in the laid out content. Note that we minus one in the end
    /// here because have a dummy placeholder offset for the end of the buffer.
    pub fn max_offset(&self) -> CharOffset {
        self.content
            .borrow()
            .summary()
            .content_length
            .saturating_sub(&CharOffset::from(1))
    }

    /// Returns the vertical viewport offset of the location.
    pub fn vertical_offset_at_render_location(
        &self,
        location: RenderLineLocation,
    ) -> Option<Pixels> {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<LineCount, LayoutSummary>();

        Self::move_cursor_to_location(&mut cursor, location);

        Some(cursor.positioned_item()?.start_y_offset - self.viewport().scroll_top())
    }

    fn move_cursor_to_location<'a>(
        cursor: &mut sum_tree::Cursor<'a, BlockItem, LineCount, LayoutSummary>,
        location: RenderLineLocation,
    ) {
        match location {
            RenderLineLocation::Current(_) => {
                cursor.seek_clamped(&location.line_count(), SeekBias::Right)
            }
            RenderLineLocation::Temporary {
                index_from_at_line, ..
            } => {
                cursor.seek_clamped(&location.line_count(), SeekBias::Left);
                if location.line_count() > LineCount(0) {
                    cursor.next();
                }
                // Temporary blocks are 0 indexed.
                for _ in 0..index_from_at_line {
                    cursor.next();
                }
            }
        }
    }

    /// Given a line range and the viewport max width, returns the viewport item and block for that range.
    pub fn blocks_in_line_range(
        &self,
        line_range: Range<RenderLineLocation>,
        max_width: Pixels,
    ) -> Vec<(ViewportItem, BlockItem)> {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<LineCount, LayoutSummary>();
        Self::move_cursor_to_location(&mut cursor, line_range.start);

        let mut blocks = Vec::new();
        let mut previous_line = line_range.start.line_count();
        let mut index_within_line = if let RenderLineLocation::Temporary {
            index_from_at_line,
            ..
        } = line_range.start
        {
            index_from_at_line
        } else {
            0
        };
        loop {
            let Some(item) = cursor.positioned_item() else {
                break;
            };
            if item.start_line != previous_line {
                index_within_line = 0;
            } else {
                index_within_line += 1;
            }

            previous_line = item.start_line;

            let spacing = item.item.spacing();
            let content_width = max_width - spacing.x_axis_offset();
            let viewport_item = ViewportItem {
                viewport_offset: Pixels::zero(),
                content_offset: item.start_y_offset,
                content_size: vec2f(content_width.as_f32(), item.item.content_height().as_f32()),
                spacing,
                block_offset: item.start_char_offset,
            };
            blocks.push((viewport_item, item.item.clone()));

            // For iterating on current line ranges, once we detect that the current item matches / or exceeds the end line,
            // we can break out of the loop. For temporary line ranges, we should check if 1) the current item hasn't past the at line
            // temporary block is anchored to 2) we haven't exceeded the index.
            match line_range.end {
                RenderLineLocation::Current(end_line) => {
                    if item.end_line() >= end_line {
                        break;
                    }
                }
                RenderLineLocation::Temporary {
                    index_from_at_line: index,
                    at_line,
                } => {
                    if item.start_line > at_line
                        || (item.start_line == at_line && index_within_line >= index)
                    {
                        break;
                    }
                }
            }
            cursor.next();
        }

        blocks
    }

    /// Baseline styles applied to rich text when rendering.
    pub fn styles(&self) -> &RichTextStyles {
        &self.styles
    }

    /// Get the document path for resolving relative paths (e.g. images).
    pub fn document_path(&self) -> Option<&std::path::Path> {
        self.document_path.as_deref()
    }

    /// Set the document path for resolving relative paths (e.g. images).
    pub fn set_document_path(&mut self, path: Option<std::path::PathBuf>) {
        self.document_path = path;
    }

    /// Update the styles used to render text. Because the render model does not directly reference
    /// the content model, the caller is responsible for updating the layout with the new styles.
    ///
    /// This returns whether a new layout is needed.
    pub fn update_styles(&mut self, new_styles: RichTextStyles) -> StyleUpdateAction {
        let styles_changed = new_styles != self.styles;
        if styles_changed {
            let action = if self.styles.requires_relayout(&new_styles) {
                StyleUpdateAction::Relayout
            } else {
                StyleUpdateAction::Repaint
            };

            self.styles = new_styles;
            return action;
        }
        StyleUpdateAction::None
    }

    /// Handle to obtain saved position IDs within the rendered text.
    pub fn saved_positions(&self) -> &SavedPositions {
        &self.saved_positions
    }

    /// Returns the current viewport state.
    pub fn viewport(&self) -> &ViewportState {
        &self.viewport
    }

    /// Return the character offset ranges of items in the viewport. Note that the start / end offset
    /// could be out of the viewport if the item is only partially visible.
    pub fn viewport_charoffset_range(&self) -> RangeSet<CharOffset> {
        let mut range_set = RangeSet::new();
        let content = self.content.borrow();
        let mut cursor = content.cursor::<Height, LayoutSummary>();
        cursor.seek_clamped(&self.viewport.scroll_top().into(), SeekBias::Left);

        let viewport_end_height = self.viewport.scroll_top() + self.viewport().height();

        // Track the current range being built
        let mut current_range_start: Option<CharOffset> = None;

        // Iterate through all items in the viewport
        loop {
            let item = cursor.positioned_item();
            if let Some(positioned_item) = item {
                // Stop if we've gone past the viewport
                if positioned_item.start_y_offset > viewport_end_height {
                    break;
                }

                let item_start = positioned_item.start_char_offset;

                if positioned_item.item.is_hidden() {
                    // This is a hidden item and we're not including hidden items
                    if let Some(range_start) = current_range_start.take() {
                        // Close the current range before this hidden item
                        if range_start < item_start {
                            range_set.insert(range_start..item_start);
                        }
                    }
                } else {
                    // If we don't have a current range, start one
                    if current_range_start.is_none() {
                        current_range_start = Some(item_start);
                    }
                }

                // Move to the next item
                cursor.next();
            } else {
                // No more items
                break;
            }
        }

        // If we ended with an open range, close it
        if let Some(range_start) = current_range_start
            && range_start < self.max_offset()
        {
            range_set.insert(range_start..self.max_offset());
        }

        range_set
    }

    /// Stores the viewport size that was calculated during layout back into
    /// the model.
    ///
    /// See [`ViewportState::set_size`].
    pub fn set_viewport_size(&mut self, size_info: SizeInfo, ctx: &mut ModelContext<Self>) {
        let height_changed = size_info
            .viewport_size
            .y()
            .approx_ne(self.viewport.height().as_f32(), UNIT_MARGIN);

        self.viewport
            .set_size(size_info.viewport_size, self.width(), self.height());

        // TODO(CLD-85): re-layout according to the high-level design (async, debounced, avoid
        // where possible).
        // In order to do this, we need to:
        // - Extract debouncing logic from the main app crate
        // - Support text layout outside the Element lifecycle
        if size_info.needs_layout {
            ctx.emit(RenderEvent::NeedsResize);
        }

        // Autoscroll when viewport height changes on mobile (e.g., keyboard appears)
        if cfg!(target_family = "wasm") && height_changed {
            self.request_autoscroll();
        }
    }

    /// Scroll the viewport by the given number of lines. Even with precise
    /// trackpad scrolling, all scroll events are reported in lines.
    pub fn scroll(&mut self, delta: Pixels, ctx: &mut ModelContext<Self>) {
        if self.viewport.scroll(delta, self.height()) {
            ctx.notify();
        }
    }

    pub fn scroll_horizontal(&mut self, delta: Pixels, ctx: &mut ModelContext<Self>) {
        if self.viewport.scroll_horizontally(delta, self.width()) {
            ctx.notify();
        }
    }

    /// Scroll to a normalized position, as returned by [`Self::snapshot_scroll_position`].
    ///
    /// This will be serialized with respect to layout changes.
    pub fn scroll_to(&mut self, position: ScrollPositionSnapshot) {
        self.submit_layout_action(LayoutAction::ScrollTo(position))
    }

    /// Snapshot the current scroll position.
    pub fn snapshot_scroll_position(&self) -> ScrollPositionSnapshot {
        ScrollPositionSnapshot::from_scroll_top(self)
    }

    pub fn scroll_data_horizontal(&self) -> ScrollData {
        let mut visible_px = self.viewport.width();
        let total_size = self.width();
        if visible_px.approx_eq(total_size, UNIT_MARGIN) {
            // This is a hack copied from the BlockListElement. Due to floating-point
            // errors, total_size and visible_px may be slightly different,
            // even if they should be the same. In that case, set them to be
            // equal so that a useless scroll bar isn't shown.
            visible_px = total_size;
        }

        ScrollData {
            scroll_start: self.viewport.scroll_left(),
            visible_px,
            total_size,
        }
    }

    pub fn set_decorations_after_layout(
        &mut self,
        mut decoration_update: UpdateDecorationAfterLayout,
    ) {
        decoration_update.sort();
        self.submit_layout_action(LayoutAction::DecorationChanged(decoration_update));
    }

    pub fn set_text_decorations(
        &mut self,
        decorations: impl Into<Vec<Decoration>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.decorations.text = decorations.into();
        // Generally, callers will provide already-sorted decorations, in which case the current
        // Rust sorting algorithm aims for linear time.
        self.decorations
            .text
            .sort_unstable_by_key(|decoration| decoration.end);
        ctx.notify();
    }

    /// The current render-time decorations, sorted by end offset.
    pub fn decorations(&self) -> &RenderDecoration {
        &self.decorations
    }

    /// Update the render model's selection. To avoid flicker, this will not be rendered until the
    /// next round of layout completes.
    pub fn update_selection(
        &mut self,
        new_selection: RenderedSelectionSet,
        buffer_version: BufferVersion,
    ) {
        self.submit_layout_action(LayoutAction::SelectionChanged {
            selections: new_selection,
            buffer_version,
        });
    }

    /// The current rendered selection state. This should track the content-level
    /// selection.
    pub fn selections<'a>(&'a self) -> Ref<'a, RenderedSelectionSet> {
        self.selections.borrow()
    }

    /// Check if the given offset is within any of the current selections.
    pub fn offset_in_active_selection(&self, offset: CharOffset) -> bool {
        self.selections()
            .iter()
            .any(|selection| offset > selection.start() && selection.end() + 1 > offset)
    }

    /// Check if the given offset any of the current selection heads.
    pub fn is_selection_head(&self, offset: CharOffset) -> bool {
        self.selections()
            .iter()
            .any(|selection| selection.head == offset)
    }

    /// Request an autoscroll using the given mode after text layout completes.
    pub fn request_autoscroll_to(&mut self, mode: AutoScrollMode) {
        self.submit_layout_action(LayoutAction::Autoscroll { mode });
    }

    /// Request an autoscroll to position the selection head within the viewport
    /// after text layout completes.
    pub fn request_autoscroll(&mut self) {
        self.submit_layout_action(LayoutAction::Autoscroll {
            mode: AutoScrollMode::ScrollToActiveSelections {
                vertical_only: false,
            },
        });
    }

    /// Request an autoscroll to position the selection head within the viewport
    /// after text layout completes, but only vertically (no horizontal autoscroll).
    pub fn request_vertical_autoscroll(&mut self) {
        self.submit_layout_action(LayoutAction::Autoscroll {
            mode: AutoScrollMode::ScrollToActiveSelections {
                vertical_only: true,
            },
        });
    }

    /// Request to autoscroll to a scroll top of exact character offset with a pixel delta.
    pub fn request_autoscroll_to_exact_vertical(
        &mut self,
        character_offset: CharOffset,
        pixel_delta: Pixels,
    ) {
        self.submit_layout_action(LayoutAction::Autoscroll {
            mode: AutoScrollMode::ScrollToExactVertical {
                character_offset,
                pixel_delta,
            },
        });
    }

    /// Submit a layout update to be processed asynchronously. This avoids mutable-borrow issues.
    pub(crate) fn submit_element_update(&self, update: ElementUpdate) {
        if let Err(err) = self.element_tx.try_send(update) {
            // We know the RenderState model still exists at this point, so it _should_ still be
            // processing updates.
            log::debug!("Error submitting layout update: {err}");
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn layout_complete(&self) -> impl std::future::Future<Output = ()> + use<> {
        let outstanding_layouts = self.outstanding_layouts.clone();
        async move {
            while outstanding_layouts.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                futures_lite::future::yield_now().await;
            }
        }
    }

    fn handle_layout_action(&mut self, action: LayoutAction, ctx: &mut ModelContext<Self>) {
        match action {
            LayoutAction::SelectionChanged {
                selections,
                buffer_version,
            } => {
                if selections != *self.selections() {
                    // If the version for the selection is newer than the last version we've rendered, then mark it as pending and apply it on our next layout
                    // Otherwise immediately apply it.
                    let active_version = self.buffer_version.borrow().last_rendered_version;
                    if active_version.is_some_and(|v| v < buffer_version) {
                        *self.pending_selection_change.lock() = Some(PendingSelectionUpdate {
                            selection: selections,
                            buffer_version,
                        });
                    } else {
                        *self.selections.borrow_mut() = selections;
                        ctx.notify();
                    }
                }
            }
            LayoutAction::DecorationChanged(decoration) => match decoration {
                UpdateDecorationAfterLayout::Line(decoration) => {
                    if decoration != self.decorations.line {
                        self.decorations.line = decoration;
                        ctx.notify();
                    }
                }
                UpdateDecorationAfterLayout::LineAndText { line, text } => {
                    let mut changed = false;

                    if line != self.decorations.line {
                        self.decorations.line = line;
                        changed = true;
                    }

                    if text != self.decorations.text {
                        self.decorations.text = text;
                        changed = true;
                    }

                    if changed {
                        ctx.notify();
                    }
                }
            },
            LayoutAction::Autoscroll { mode } => {
                self.autoscroll(mode, ctx);
            }
            LayoutAction::ScrollTo(position) => {
                if self
                    .viewport
                    .scroll_to(position.to_scroll_top(self), self.height())
                {
                    ctx.notify();
                }
            }
            LayoutAction::LayoutTemporaryBlock(blocks) => {
                // If we are performing layout lazily, push the temporary blocks to the pending edits queue which is flushed
                // at editor element layout time
                if self.lazy_layout {
                    self.pending_edits
                        .lock()
                        .push(PendingLayout::TemporaryBlocks(blocks));
                } else {
                    self.layout_temporary_blocks(blocks, ctx);
                    self.update_content_sizing();
                }

                ctx.emit(RenderEvent::LayoutUpdated);
                ctx.notify();
            }
            LayoutAction::BufferEdit {
                delta,
                buffer_version,
            } => {
                // Materialize the hidden ranges based on the version.
                let hidden_ranges = self
                    .hidden_lines
                    .as_ref()
                    .map(|hl| hl.as_ref(ctx).hidden_ranges_at_version(buffer_version));

                // If we are performing layout lazily, push the delta to the pending edits queue which is flushed
                // at editor element layout time.
                if self.lazy_layout {
                    self.pending_edits.lock().push(PendingLayout::Edit {
                        delta,
                        hidden_ranges,
                    });
                } else {
                    // If there were pending edits, we need to re-render so that RenderableBlocks
                    // are properly laid out with the new set of BlockItems.
                    self.layout_edit_delta(delta, hidden_ranges, ctx);
                    self.update_content_sizing();

                    self.flush_pending_selection_update(Some(buffer_version));
                }
                self.buffer_version.borrow_mut().next_render_version = Some(buffer_version);
                ctx.emit(RenderEvent::LayoutUpdated);
                ctx.notify();
            }
        }

        #[cfg(any(test, feature = "test-util"))]
        self.outstanding_layouts
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Given the content version that we've just laid out and rendered, flush any pending selection for that content version
    fn flush_pending_selection_update(&self, incoming_buffer_version: Option<BufferVersion>) {
        let mut pending_selection = self.pending_selection_change.lock();
        if let Some(selection) = &*pending_selection
            && incoming_buffer_version
                .map(|v| v >= selection.buffer_version)
                .unwrap_or(true)
        {
            *self.selections.borrow_mut() = selection.selection.clone();
            pending_selection.take();
        }
    }

    fn layout_temporary_blocks(&self, blocks: Vec<TemporaryBlock>, app: &AppContext) {
        let layout_cache = LayoutCache::new();
        let layout_context = self.layout_context(&layout_cache, app);
        let laid_out_blocks = layout_temporary_blocks(blocks, &layout_context);
        self.reset_temporary_block(laid_out_blocks);
    }

    pub fn layout_edit_delta(
        &self,
        delta: EditDelta,
        hidden_ranges: Option<RangeSet<CharOffset>>,
        app: &AppContext,
    ) {
        let layout_cache = LayoutCache::new();
        let layout_context = self.layout_context(&layout_cache, app);
        let laid_out_edit = delta.layout_delta(
            &layout_context,
            self.document_path.as_deref(),
            &self.layout_options,
            hidden_ranges.clone(),
            app,
        );
        self.layout_pending_edit(laid_out_edit, hidden_ranges);
    }

    /// Construct a throwaway layout cache. We only lay out modified text, so in effect,
    /// the entire RenderState is a cache.
    fn layout_context<'a>(
        &'a self,
        layout_cache: &'a LayoutCache,
        ctx: &'a AppContext,
    ) -> TextLayout<'a> {
        TextLayout::new(
            layout_cache,
            ctx.font_cache().text_layout_system(),
            &self.styles,
            match self.width_setting {
                WidthSetting::FitViewport => self.viewport.width().as_f32(),
                WidthSetting::InfiniteWidth => f32::MAX,
            },
        )
        .with_container_scrolls_horizontally(self.container_scrolls_horizontally())
    }

    /// If we are performing layout lazily, call this to flush the pending edits and selection changes
    /// at layout time in the UI framework element cycle.
    pub fn try_layout_pending_edits(&self, app: &AppContext) -> bool {
        let mut pending_edits = self.pending_edits.lock();
        let last_rendered_version = self.buffer_version.borrow_mut().start_layout();
        let pending_edits_flushed = self.lazy_layout && !pending_edits.is_empty();

        if pending_edits_flushed {
            for edit in mem::take(&mut *pending_edits) {
                match edit {
                    PendingLayout::Edit {
                        delta,
                        hidden_ranges,
                    } => {
                        self.layout_edit_delta(delta, hidden_ranges, app);
                    }
                    PendingLayout::TemporaryBlocks(blocks) => {
                        self.layout_temporary_blocks(blocks, app);
                    }
                };
            }
        }

        // Flush the pending selection changes.
        self.flush_pending_selection_update(last_rendered_version);
        pending_edits_flushed
    }

    /// Updates the model with the results of laying out its element.
    fn apply_element_update(&mut self, update: ElementUpdate, ctx: &mut ModelContext<Self>) {
        // Clear up the remaining pending edits state after layout is completed. Also make sure we have the updated
        // content sizing.
        if self.lazy_layout {
            self.update_content_sizing();
        }
        if update.pending_edits_flushed {
            ctx.emit(RenderEvent::PendingEditsFlushed);
        }

        if let Some(viewport_size) = update.viewport_size {
            self.set_viewport_size(viewport_size, ctx);
            if self.element_tx.is_empty() {
                // Don't emit this event when the channel has more current updates
                // to process. This is to avoid emitting events when the viewport info is stale.
                ctx.emit(RenderEvent::ViewportUpdated(update.buffer_version));
            }
        }
    }

    /// Submit a layout action.
    fn submit_layout_action(&mut self, action: LayoutAction) {
        #[cfg(any(test, feature = "test-util"))]
        self.outstanding_layouts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let result = self.layout_tx.try_send(action);
        debug_assert!(result.is_ok(), "Could not submit layout action: {result:?}");
    }

    /// Push a pending edit to the queue.
    pub fn add_pending_edit(&mut self, pending_edit: EditDelta, buffer_version: BufferVersion) {
        self.submit_layout_action(LayoutAction::BufferEdit {
            delta: pending_edit,
            buffer_version,
        });
    }

    pub fn add_temporary_blocks(&mut self, temporary_blocks: Vec<TemporaryBlock>) {
        self.submit_layout_action(LayoutAction::LayoutTemporaryBlock(temporary_blocks));
    }

    /// Replace all temporary blocks in the BlockItem cache with a new set of temporary
    fn reset_temporary_block(&self, mut blocks: HashMap<LineCount, Vec<BlockItem>>) {
        let mut new_tree = SumTree::new();
        {
            let content = self.content.borrow();
            let mut cursor = content.cursor::<LineCount, CharOffset>();

            if let Some(items) = blocks.remove(&LineCount::zero()) {
                for item in items {
                    new_tree.push(item);
                }
            }

            cursor.descend_to_first_item(&content, |_| true);
            while let Some(item) = cursor.item() {
                if !matches!(item, BlockItem::TemporaryBlock { .. }) {
                    new_tree.push(item.clone());
                }

                if let Some(items) = blocks.remove(&cursor.end_seek_position()) {
                    for item in items {
                        new_tree.push(item);
                    }
                }

                cursor.next();
            }
        }
        self.has_final_trailing_newline
            .set(Self::tree_ends_with_trailing_newline(&new_tree));
        *self.content.borrow_mut() = new_tree;
    }

    /// Update the render state with laid out new edits.
    fn layout_pending_edit(
        &self,
        pending_edit: LaidOutRenderDelta,
        hidden_ranges: Option<RangeSet<CharOffset>>,
    ) {
        log::trace!(
            "Applying {}-line pending edit to {}..{}",
            pending_edit.laid_out_line.len(),
            pending_edit.old_offset.start,
            pending_edit.old_offset.end
        );

        log::trace!(
            "Incoming block replacements {:?}",
            &pending_edit.laid_out_line
        );

        let hidden_range_clone = hidden_ranges.clone();
        let mut new_tree = SumTree::new();
        {
            let content = self.content.borrow();

            log::trace!("Initial blocks:\n{}", content.describe());
            let mut cursor = content.cursor::<CharOffset, CharOffset>();

            // TODO(CLD-558): Ideally, we'd use the content-level offset as is.
            let effective_start = pending_edit
                .old_offset
                .start
                .saturating_sub(&CharOffset::from(1));

            // Push blocks that are not affected by the change. We don't need to recompute them.
            new_tree.push_tree(cursor.slice(&effective_start, SeekBias::Right));
            log::trace!(
                "Keeping blocks up to {} ({}):\n{}",
                pending_edit.old_offset.start,
                effective_start,
                new_tree.describe()
            );

            for item in pending_edit.laid_out_line {
                let offset = new_tree.extent::<CharOffset>() + 1;
                // If the item should be hidden (but it's not labelled as hidden), don't push it to the sumtree.
                if !matches!(item, BlockItem::Hidden(_))
                    && hidden_range_clone
                        .as_ref()
                        .map(|hr| hr.contains(&offset))
                        .unwrap_or(false)
                {
                    continue;
                }
                new_tree.push(item);
            }

            // TODO(CLD-558): Ideally, we'd use the content-level offset as is.
            let effective_end = pending_edit
                .old_offset
                .end
                .saturating_sub(&CharOffset::from(1));

            let sub_tree_start = *cursor.start();
            let sub_tree = cursor.slice(&effective_end, SeekBias::Left);
            let mut sub_tree_cursor = sub_tree.cursor::<CharOffset, CharOffset>();
            sub_tree_cursor.descend_to_first_item(&sub_tree, |_| true);

            while let Some(item) = sub_tree_cursor.item() {
                // Do not remove the temporary blocks within the replaced range.
                if matches!(item, BlockItem::TemporaryBlock { .. }) {
                    new_tree.push(item.clone());
                }

                // If there is a hidden range that overlaps the current replacement range, push it to the sumtree for now.
                // This will be handled in the deduping logic below.
                let offset_end = sub_tree_start + sub_tree_cursor.end();
                let offset_start = sub_tree_start + *sub_tree_cursor.start();
                if (offset_end > effective_end || offset_start < effective_start)
                    && matches!(item, BlockItem::Hidden(_))
                {
                    new_tree.push(item.clone());
                }
                sub_tree_cursor.next()
            }

            // We are replacing the last element of the buffer. We should add a trailing
            // newline if there is one from the pending edit. Note we are replacing the last element
            // iff: 1) pending edit's replacement end is not zero (we are moving cursor past the next item) 2)
            // the end of cursor is past max offset of the sumtree.
            if effective_end != CharOffset::zero() && cursor.end() >= self.max_offset() {
                log::debug!("Adding trailing newline");
                if let Some(cursor) = pending_edit.trailing_newline
                    && self.should_show_final_trailing_newline(new_tree.is_empty())
                {
                    new_tree.push(BlockItem::TrailingNewLine(cursor));
                }
            } else {
                // Generally, we seek left and skip past the last block in the invalidated range.
                // However, if we're inserting before the first block in the buffer, we want to
                // keep it - since CharOffsets are non-negative and the end offset of the range is
                // exclusive, we can't otherwise represent "before the first block".
                if effective_end > CharOffset::zero() {
                    if let Some(item) = cursor.item() {
                        // Do not remove the temporary blocks within the replaced range.
                        if matches!(item, BlockItem::TemporaryBlock { .. })
                            || (cursor.end() > effective_end
                                && matches!(item, BlockItem::Hidden(_)))
                        {
                            new_tree.push(item.clone());
                        }
                    }
                    cursor.next();
                } else if !new_tree.is_empty()
                    && cursor.item().is_some_and(|item| item.is_trailing_newline())
                    && (pending_edit.trailing_newline.is_none()
                        || !self.show_final_trailing_newline_when_non_empty)
                {
                    // Remove the trailing newline when the tree is non-empty and the
                    // render configuration suppresses it, or the pending edit omits one.
                    cursor.next();
                }

                let suffix = cursor.suffix();
                log::trace!(
                    "Keeping blocks after {} ({}):\n{}",
                    pending_edit.old_offset.end,
                    effective_end,
                    suffix.describe()
                );
                new_tree.push_tree(suffix);
            }
        }

        // Hidden range updates could be incremental. This means we might end with duplicate adjacent hidden blocks.
        // This step ensures these are merged into one.
        if let Some(hidden_ranges) = hidden_ranges {
            new_tree = Self::dedupe_hidden_ranges(new_tree, hidden_ranges);
        }
        log::trace!("Resulting blocks:\n{}", new_tree.describe());
        self.has_final_trailing_newline
            .set(Self::tree_ends_with_trailing_newline(&new_tree));
        let mut content_mut = self.content.borrow_mut();
        *content_mut = new_tree;
    }

    /// Dedupe adjacent hidden ranges into one.
    fn dedupe_hidden_ranges(
        tree: SumTree<BlockItem>,
        hidden_ranges: RangeSet<CharOffset>,
    ) -> SumTree<BlockItem> {
        log::trace!("Initial tree:\n{}", tree.describe());
        let mut new_tree = SumTree::new();
        let mut cursor = tree.cursor::<CharOffset, CharOffset>();

        let max_char_offset = tree.extent::<CharOffset>() + 1;

        // Note that it's deliberate we minus one from start and keep the end as is.
        // This expands our search to capture hidden ranges that are inserted prior / after the canonical range.
        let ranges = hidden_ranges
            .into_iter()
            .sorted_by(|a, b| Ord::cmp(&a.start, &b.start))
            .map(|range| range.start.saturating_sub(&CharOffset::from(1))..range.end)
            .collect_vec();

        for range in ranges {
            let hidden_range_size = range.end - range.start - 1;
            log::trace!("==== Processing range: {:?} ====", &range);
            new_tree.push_tree(cursor.slice(&range.start, SeekBias::Left));
            log::trace!("After pushing prefix tree:\n {}", new_tree.describe());
            let sub_tree = cursor.slice(&range.end, SeekBias::Right);

            log::trace!("Processing sub_tree:\n {}", sub_tree.describe());
            let mut hidden_config: Option<HiddenBlockConfig> = None;
            let mut staging = Vec::new();

            // We want to always preserve 1) the non-hidden items 2) in the same order as they are inserted.
            for item in sub_tree.cursor::<CharOffset, CharOffset>() {
                if let BlockItem::Hidden(config) = item {
                    if let Some(prev) = &mut hidden_config {
                        *prev += *config;
                    } else {
                        hidden_config = Some(*config)
                    }
                } else if hidden_config.is_none() {
                    new_tree.push(item.clone());
                } else {
                    staging.push(item.clone())
                }
            }

            if let Some(mut config) = hidden_config {
                // Always resize the hidden range to its expected size.
                config.content_length = hidden_range_size;
                config.block_location = if range.start <= CharOffset::from(1) {
                    BlockLocation::Start
                } else if range.end == max_char_offset {
                    BlockLocation::End
                } else {
                    BlockLocation::Middle
                };
                new_tree.push(BlockItem::Hidden(config));
            }

            new_tree.extend(staging);
            log::trace!("==== Finished processing range ====");
        }

        let suffix = cursor.suffix();
        new_tree.push_tree(suffix);

        log::trace!("Resulting tree:\n{}", new_tree.describe());
        new_tree
    }

    fn update_content_sizing(&mut self) {
        self.viewport.update_content_height(self.height());
        self.viewport.update_content_width(self.width());
    }

    /// Perform an autoscroll action based on the mode.
    pub fn autoscroll(&mut self, mode: AutoScrollMode, ctx: &mut ModelContext<Self>) {
        let table_scroll_changed = self.reveal_autoscroll_offsets_in_tables(&mode);
        let ((in_line_selection_start, in_line_selection_end), vertical_autoscroll_only) =
            match mode {
                AutoScrollMode::ScrollOffsetsIntoViewport(offsets) => {
                    let (override_start, _) = self.character_width_height_range(offsets.start);
                    let (_, override_end) = self.character_width_height_range(offsets.end);
                    ((override_start, override_end), false)
                }
                AutoScrollMode::ScrollToExactVertical {
                    character_offset,
                    pixel_delta,
                } => {
                    let (start, _) = self.character_width_height_range(character_offset);
                    if self
                        .viewport
                        .scroll_to(start.y().into_pixels() + pixel_delta, self.height())
                        || table_scroll_changed
                    {
                        ctx.notify();
                    }
                    return;
                }
                AutoScrollMode::ScrollToActiveSelections { vertical_only } => {
                    let cursor_positions = self.selections().selection_map(|selection| {
                        self.character_width_height_range(selection.head)
                    });
                    (
                        Self::multiselect_autoscroll_bounding_box(
                            cursor_positions,
                            self.viewport.height(),
                            self.viewport.scroll_top(),
                        ),
                        vertical_only,
                    )
                }
                AutoScrollMode::PositionOffsetInViewportCenter(offset) => {
                    let (char_start, char_end) = self.character_width_height_range(offset);

                    // Calculate half the viewport dimensions
                    let half_viewport_width = self.viewport.width().as_f32() / 2.0;
                    let half_viewport_height = self.viewport.height().as_f32() / 2.0;

                    // Compute the bounding box centered on the character position
                    // Start = character position - half viewport
                    // End = character position + half viewport
                    let in_line_start = vec2f(
                        (char_start.x() - half_viewport_width).max(0.0),
                        (char_start.y() - half_viewport_height).max(0.0),
                    );
                    let in_line_end = vec2f(
                        (char_end.x() + half_viewport_width).min(self.width().as_f32()),
                        (char_end.y() + half_viewport_height).min(self.height().as_f32()),
                    );

                    ((in_line_start, in_line_end), false)
                }
            };

        // We should not autoscroll if the width is fitting viewport.
        let should_autoscroll_horizontally =
            !matches!(self.width_setting, WidthSetting::FitViewport) && !vertical_autoscroll_only;
        if self.viewport.autoscroll(
            in_line_selection_start,
            in_line_selection_end,
            self.height(),
            self.width(),
            should_autoscroll_horizontally,
        ) || table_scroll_changed
        {
            ctx.notify();
        }
    }

    fn reveal_autoscroll_offsets_in_tables(&self, mode: &AutoScrollMode) -> bool {
        match mode {
            AutoScrollMode::ScrollOffsetsIntoViewport(offsets) => {
                let mut changed = self.reveal_offset_in_table(offsets.start);
                if offsets.end > offsets.start {
                    changed |= self
                        .reveal_offset_in_table(offsets.end.saturating_sub(&CharOffset::from(1)));
                }
                changed
            }
            AutoScrollMode::ScrollToExactVertical {
                character_offset, ..
            }
            | AutoScrollMode::PositionOffsetInViewportCenter(character_offset) => {
                self.reveal_offset_in_table(*character_offset)
            }
            AutoScrollMode::ScrollToActiveSelections { .. } => {
                self.selections().iter().fold(false, |changed, selection| {
                    self.reveal_offset_in_table(selection.head) || changed
                })
            }
        }
    }

    fn reveal_offset_in_table(&self, offset: CharOffset) -> bool {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<CharOffset, LayoutSummary>();
        cursor.seek(&offset, SeekBias::Right);

        let Some(block) = cursor.positioned_item() else {
            return false;
        };
        let BlockItem::Table(laid_out_table) = block.item else {
            return false;
        };
        if offset < block.start_char_offset || offset >= block.end_char_offset() {
            return false;
        }

        let viewport_width =
            (self.viewport.width() - block.item.spacing().x_axis_offset()).max(Pixels::zero());
        laid_out_table.reveal_offset(offset - block.start_char_offset, viewport_width)
    }

    /// Given the coordinates of all selections, determine what is the bounding box that we want to autoscroll to.
    ///
    /// Cases:
    /// - There are selections on the screen: Autoscroll to the bounding box of the selections currently on screen.
    ///     - Note: Do not try to get all selections on screen.  This is jarring to the user.
    /// - All selections are scrolled off the bottom of the screen.
    ///     - Find all selections from top to bottom that would fit into one viewport, and autoscroll to that.
    /// - All selections are scrolled off the top of the screen.
    ///    - Find all selections from bottom to top that would fit into one viewport, and autoscroll to that.
    /// - There are selections above and below the viewport, but none on it.
    ///   - Find all selections below the viewport from top to bottom that would fit into one viewport, and autoscroll to that.
    ///   - Choosing to scroll down is arbitrary.
    fn multiselect_autoscroll_bounding_box(
        heads: Vec1<(Vector2F, Vector2F)>,
        view_height: Pixels,
        scroll_top: Pixels,
    ) -> (Vector2F, Vector2F) {
        // Find selections above, below, and in the viewport.
        let mut above = Vec::new();
        let mut inside = Vec::new();
        let mut below = Vec::new();

        for head in heads {
            if head.0.y().into_pixels() < scroll_top {
                above.push(head);
            } else if head.1.y().into_pixels() > scroll_top + view_height {
                below.push(head);
            } else {
                inside.push(head);
            };
        }

        let heads = if !inside.is_empty() {
            // If there are any cursors showing on the screen, set those cursors as the bounding box.
            //  Which shouldn't scroll the screen vertically.
            inside.sort_by(|(first, _), (second, _)| {
                first
                    .y()
                    .partial_cmp(&second.y())
                    .expect("Cursor positions should be well behaved floats.")
            });
            inside
        } else if !below.is_empty() {
            // Either there are only selections below the viewport, or there are selections above and below the viewport.
            // Sort from top to bottom.
            below.sort_by(|(first, _), (second, _)| {
                first
                    .y()
                    .partial_cmp(&second.y())
                    .expect("Cursor positions should be well behaved floats.")
            });
            below
        } else if !above.is_empty() {
            // There are only selections above the viewport.
            // Note that above must not be empty
            // Sort from bottom to top.
            above.sort_by(|(first, _), (second, _)| {
                first
                    .y()
                    .partial_cmp(&second.y())
                    .expect("Cursor positions should be well behaved floats.")
            });
            above.reverse();
            above
        } else {
            // We started with a Vec1, so we should never get here.
            panic!("There should be at least one selection.");
        };

        let (first, rest) = heads.split_first().expect("Vec1 will have at least one.");
        let mut min = first.0;
        let mut max = first.1;

        if min.y() > max.y() || (min.y() == max.y() && min.x() > max.x()) {
            mem::swap(&mut min, &mut max)
        }

        for (start_head, end_head) in rest {
            // This is all we can fit into the viewport.
            if end_head.y() - min.y() > view_height.as_f32()
                || max.y() - start_head.y() > view_height.as_f32()
            {
                break;
            }
            // If we have found a new min or max, set it.
            if min.y() > start_head.y() {
                min.set_y(start_head.y());
            }
            if max.x() < end_head.x() {
                max.set_x(end_head.x());
            }
            if max.y() < end_head.y() {
                max.set_y(end_head.y());
            }
            if min.x() > start_head.x() {
                min.set_x(start_head.x());
            }
        }
        (min, max)
    }

    fn character_width_height_range(&self, offset: CharOffset) -> (Vector2F, Vector2F) {
        let content = self.content.borrow();
        match self.character_bounds(offset) {
            Some(bound) => (bound.origin(), bound.lower_right()),
            None => {
                let mut height_cursor = content.cursor::<CharOffset, LayoutSummary>();
                height_cursor.seek(&offset, SeekBias::Right);

                // If we are at the very end of the content tree, treat the last item's bound as (0, 0) on the last line.
                (
                    vec2f(0., height_cursor.start().height as f32),
                    vec2f(0., height_cursor.end().height as f32),
                )
            }
        }
    }

    pub fn offset_to_softwrap_point(&self, offset: CharOffset) -> SoftWrapPoint {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<CharOffset, LayoutSummary>();

        cursor.seek(&offset, SeekBias::Right);
        match cursor.positioned_item() {
            Some(item) => item.offset_to_softwrap_point(offset),
            None => SoftWrapPoint::new(self.max_line().as_u32(), Pixels::zero()),
        }
    }

    pub fn softwrap_point_to_offset(&self, point: SoftWrapPoint) -> CharOffset {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<LineCount, LayoutSummary>();

        let line = LineCount(point.row() as usize);
        cursor.seek(&line, SeekBias::Right);
        match cursor.positioned_item() {
            Some(item) => item.softwrap_point_to_offset(point),
            None => self.max_offset(),
        }
    }

    /// Converts a line number to the character offset range (start, end) for that line.
    /// Line numbers are 1-indexed (LineCount).
    /// Returns the start offset of the line and the end offset (exclusive).
    pub fn line_number_to_offset_range(&self, line_number: LineCount) -> (CharOffset, CharOffset) {
        // Convert LineCount (1-indexed) to SoftWrapPoint row (0-indexed)
        let line_row = line_number.as_u32().saturating_sub(1);

        let start_offset =
            self.softwrap_point_to_offset(SoftWrapPoint::new(line_row, Pixels::zero()));
        let end_offset =
            self.softwrap_point_to_offset(SoftWrapPoint::new(line_row + 1, Pixels::zero()));

        (start_offset, end_offset)
    }

    /// The bounding box of the character at `offset`.
    fn character_bounds(&self, offset: CharOffset) -> Option<RectF> {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<CharOffset, LayoutSummary>();
        cursor.seek(&offset, SeekBias::Right);
        cursor
            .positioned_item()
            .and_then(|item| item.character_bounds(offset))
    }

    pub fn character_vertical_bounds(&self, offset: CharOffset) -> Option<(Pixels, Pixels)> {
        self.character_bounds(offset).map(|bounds| {
            (
                Pixels::new(bounds.origin_y()),
                Pixels::new(bounds.origin_y() + bounds.height()),
            )
        })
    }

    /// Returns the bounding box of the character at `offset` in viewport-relative coordinates.
    /// This can be used to position UI elements relative to text without waiting for the paint phase.
    /// Returns None if the offset is out of bounds or not laid out yet.
    pub fn character_bounds_in_viewport(&self, offset: CharOffset) -> Option<RectF> {
        let bounds = self.character_bounds(offset)?;

        // Convert from content coordinates to viewport coordinates
        let scroll_top = self.viewport.scroll_top().as_f32();
        let scroll_left = self.viewport.scroll_left().as_f32();

        let viewport_origin = vec2f(
            bounds.origin_x() - scroll_left,
            bounds.origin_y() - scroll_top,
        );

        Some(RectF::new(viewport_origin, bounds.size()))
    }

    /// Saves the text selection bounding box into the position cache.
    pub(super) fn record_text_selection(&self, ctx: &mut RenderContext) {
        // Todo (kc CLD-1018): Save all positions, and not just one.
        let selection = self.selections().first().clone();

        let Some(start) = self.character_bounds(selection.start()) else {
            return;
        };
        let Some(end) = self.character_bounds(selection.end()) else {
            return;
        };
        let origin = start.origin().min(end.origin());
        let lower_right = start.lower_right().max(end.lower_right());

        // Bound the origin of the text selection cached position by the viewport (CLD-1220).
        let mut screen_origin = ctx.content_to_screen(origin);
        let mut screen_lower_right = ctx.content_to_screen(lower_right);

        screen_origin.set_y(screen_origin.y().max(ctx.visible_bound().origin_y()));
        screen_lower_right.set_y(screen_lower_right.y().max(ctx.visible_bound().origin_y()));

        let bounding_box = RectF::from_points(screen_origin, screen_lower_right);

        if ctx.is_visible(bounding_box) {
            ctx.paint.position_cache.cache_position_for_one_frame(
                self.saved_positions.text_selection_id(),
                bounding_box,
            );
        }
    }

    /// Initializes the ordered list numbering state at the start of the viewport.
    pub(super) fn viewport_list_numbering(&self) -> ListNumbering {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<Height, ()>();
        cursor.seek_clamped(&self.viewport.scroll_top().into(), SeekBias::Left);

        // If the viewport starts with an ordered list item, we need to know its initial numbering.
        // This only depends on the ordered list items immediately above the viewport. To find them,
        // we need a linear scan, but it's bounded by the size of the ordered list above the
        // viewport, which will generally be small. If the top of the viewport isn't an ordered
        // list, we can skip this altogether.
        if !matches!(cursor.item(), Some(BlockItem::OrderedList { .. })) {
            return ListNumbering::new();
        }

        // ListNumbering can only advance forwards, so we first seek back to the start of the list
        // at the viewport location.
        let mut list_length = 0;
        while let Some(BlockItem::OrderedList { .. }) = cursor.prev_item() {
            cursor.prev();
            list_length += 1;
        }

        let mut numbering = ListNumbering::new();

        for _ in 0..list_length {
            match cursor.item() {
                Some(BlockItem::OrderedList {
                    indent_level,
                    number,
                    ..
                }) => {
                    numbering.advance(indent_level.as_usize(), *number);
                }
                other => {
                    if cfg!(debug_assertions) {
                        panic!("Should have an OrderedList item, got {other:?}");
                    }
                    // In production, silently skip over unexpected items.
                }
            }
            cursor.next();
        }

        numbering
    }

    /// Log the current render state for debugging.
    pub fn log_state(&self) {
        log::info!("RENDER STATE:\n{}", self.describe());
    }

    #[cfg(test)]
    pub fn set_content(&mut self, mut content: SumTree<BlockItem>) {
        if self.should_show_final_trailing_newline(content.is_empty()) {
            content.push(Self::final_trailing_newline_cursor(&self.styles));
        }
        self.has_final_trailing_newline
            .set(Self::tree_ends_with_trailing_newline(&content));
        self.content = content.into();
    }

    /// Scroll to the start of a given block, possibly adjusted.
    #[cfg(test)]
    fn scroll_near_block(&mut self, offset: CharOffset, adjustment: impl IntoPixels) {
        let content = self.content.borrow();
        let mut cursor = content.cursor::<CharOffset, Height>();
        cursor.seek(&offset, SeekBias::Right);
        self.viewport
            .set_scroll_top(cursor.start().into_pixels() + adjustment.into_pixels());
    }

    /// Line number of the first line in the block.
    pub fn start_line_index(&self, block: &dyn RenderableBlock) -> Option<LineCount> {
        let content = self.content();
        let offset = block.viewport_item().block_offset();
        Some(content.block_at_offset(offset)?.start_line)
    }

    /// The line height of the first line. Different from `first_line_bounds`, this does not
    /// return the viewport origin.
    pub fn first_line_height(&self, block: &dyn RenderableBlock) -> Option<f32> {
        let content = self.content();
        let block = content.block_at_height(block.viewport_item().height())?;
        Some(block.item.first_line_height())
    }

    /// The bounding box of the first line of this block, based on its viewport location.
    pub fn first_line_bounds(
        &self,
        block: &dyn RenderableBlock,
        ctx: &RenderContext,
    ) -> Option<RectF> {
        let content = self.content();
        let offset = block.viewport_item().block_offset();
        let block = content.block_at_offset(offset)?;
        Some(ctx.content_rect_to_screen(block.first_line_bounds()?))
    }

    pub fn line_range(&self, block: &dyn RenderableBlock) -> Option<Range<LineCount>> {
        let start = self.start_line_index(block)?;
        let content = self.content();
        let offset = block.viewport_item().block_offset();
        Some(start..start + content.block_at_offset(offset)?.item.lines())
    }
}

impl Entity for RenderState {
    type Event = RenderEvent;
}

/// A bundle of information computed by the element during layout, which is used to update the
/// render model.
///
/// This is how [`RenderState`] knows the viewport size.
pub(crate) struct ElementUpdate {
    pub viewport_size: Option<SizeInfo>,
    pub buffer_version: Option<BufferVersion>,
    pub pending_edits_flushed: bool,
}

/// The mode of a requested autoscroll action.
#[derive(Debug, Clone)]
pub enum AutoScrollMode {
    /// Scroll the range of offset into the viewport.
    ScrollOffsetsIntoViewport(Range<CharOffset>),
    /// Set scroll top to the exact vertical position of a character offset
    /// with an optional pixel delta.
    ScrollToExactVertical {
        character_offset: CharOffset,
        pixel_delta: Pixels,
    },
    /// Scroll to the active selections / cursors.
    ScrollToActiveSelections { vertical_only: bool },
    /// Scroll to position the given character offset at the center of the viewport.
    /// The bounding box is computed as the character position ± half the viewport dimensions,
    /// clamped to the content bounds.
    PositionOffsetInViewportCenter(CharOffset),
}

#[derive(Clone, Debug)]
enum PendingLayout {
    Edit {
        delta: EditDelta,
        hidden_ranges: Option<RangeSet<CharOffset>>,
    },
    TemporaryBlocks(Vec<TemporaryBlock>),
}

struct PendingSelectionUpdate {
    selection: RenderedSelectionSet,
    buffer_version: BufferVersion,
}

/// A change to the rendering state that's processed by the background layout task.
#[derive(Debug, Clone)]
enum LayoutAction {
    /// The selection state has changed. We dispatch this through the layout pipeline so that
    /// the on-screen cursor position only changes _after_ any relevant text is laid out.
    SelectionChanged {
        selections: RenderedSelectionSet,
        buffer_version: BufferVersion,
    },
    DecorationChanged(UpdateDecorationAfterLayout),
    /// The buffer was edited.
    BufferEdit {
        delta: EditDelta,
        buffer_version: BufferVersion,
    },
    LayoutTemporaryBlock(Vec<TemporaryBlock>),
    /// Autoscroll, to the specified range if `Some` or to the cursor location if `None`.
    Autoscroll {
        mode: AutoScrollMode,
    },
    /// Scroll to a snapshotted scroll position.
    ScrollTo(ScrollPositionSnapshot),
}

impl From<Pixels> for Height {
    fn from(value: Pixels) -> Self {
        Self(OrderedFloat(value.as_f32().into()))
    }
}

impl From<f32> for Height {
    fn from(value: f32) -> Self {
        value.into_pixels().into()
    }
}

impl From<Height> for Pixels {
    fn from(value: Height) -> Self {
        (value.0.0 as f32).into_pixels()
    }
}

impl IntoPixels for Height {
    fn into_pixels(self) -> Pixels {
        (self.0.0 as f32).into_pixels()
    }
}

impl RichTextStyles {
    /// The base line height for plain text. For consistency, this is the line
    /// height used for scrolling - otherwise, the user would scroll faster past
    /// items with a larger font size.
    pub fn base_line_height(&self) -> Pixels {
        self.base_text.line_height()
    }

    /// Selects the paragraph styles that apply to a block style.
    pub fn paragraph_styles(&self, block_style: &BufferBlockStyle) -> ParagraphStyles {
        match block_style {
            BufferBlockStyle::PlainText
            | BufferBlockStyle::UnorderedList { .. }
            | BufferBlockStyle::OrderedList { .. }
            | BufferBlockStyle::TaskList { .. } => self.base_text,
            BufferBlockStyle::Table { .. } => {
                let mut style = self.base_text;
                style.font_family = self.table_style.font_family;
                style.font_size = self.table_style.font_size;
                style
            }
            BufferBlockStyle::CodeBlock { .. } => self.code_text,
            BufferBlockStyle::Header { header_size } => {
                let mut base_text_style = self.base_text;
                base_text_style.font_size *= header_size.font_size_multiplication_ratio();
                base_text_style.font_weight = Weight::from_custom_weight(header_size.font_weight());
                base_text_style
            }
        }
    }

    pub fn requires_relayout(&self, new_styles: &RichTextStyles) -> bool {
        if self == new_styles {
            return false;
        }

        self.base_text.requires_relayout(&new_styles.base_text)
            || self.code_text.requires_relayout(&new_styles.code_text)
            || self
                .embedding_text
                .requires_relayout(&new_styles.embedding_text)
            || self
                .inline_code_style
                .requires_relayout(&new_styles.inline_code_style)
            || self.table_style.requires_relayout(&new_styles.table_style)
            || self.minimum_paragraph_height != new_styles.minimum_paragraph_height
    }
}

impl ParagraphStyles {
    pub fn line_style(&self) -> LineStyle {
        LineStyle {
            font_size: self.font_size,
            line_height_ratio: self.line_height_ratio,
            baseline_ratio: self.baseline_ratio,
            fixed_width_tab_size: self.fixed_width_tab_size,
        }
    }

    pub fn line_height(&self) -> Pixels {
        (self.font_size * self.line_height_ratio).into_pixels()
    }

    /// Default font properties for paragraphs of this style. They may be overridden by inline
    /// styles.
    pub fn properties(&self) -> Properties {
        Properties::default().weight(self.font_weight)
    }

    fn requires_relayout(&self, new_styles: &ParagraphStyles) -> bool {
        self.font_size != new_styles.font_size
            || self.line_height_ratio != new_styles.line_height_ratio
            || self.baseline_ratio != new_styles.baseline_ratio
            || self.font_weight != new_styles.font_weight
            || self.font_family != new_styles.font_family
            || self.fixed_width_tab_size != new_styles.fixed_width_tab_size
    }
}

impl AddAssign<&LayoutSummary> for LayoutSummary {
    fn add_assign(&mut self, rhs: &LayoutSummary) {
        self.height += rhs.height;
        self.content_length += rhs.content_length;
        self.width = self.width.max(rhs.width);
        self.lines += rhs.lines;
        self.item_count += rhs.item_count;
    }
}

impl BlockItem {
    pub fn paragraph(
        frame: Arc<TextFrame>,
        offsets: OffsetMap,
        content_length: CharOffset,
        spacing: BlockSpacing,
        minimum_height: Option<Pixels>,
    ) -> BlockItem {
        BlockItem::Paragraph(Paragraph::new(
            frame,
            offsets,
            content_length,
            vec![],
            spacing,
            minimum_height,
        ))
    }

    pub fn first_line_height(&self) -> f32 {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. } => paragraph.first_line_height(),
            BlockItem::TextBlock { paragraph_block } => paragraph_block.first_line_height(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            }
            | BlockItem::TemporaryBlock {
                paragraph_block, ..
            } => paragraph_block.first_line_height(),
            BlockItem::MermaidDiagram { config, .. } => config.height.as_f32(),
            BlockItem::TrailingNewLine(cursor) => cursor.height.as_f32(),
            BlockItem::HorizontalRule(config) => config.line_height.as_f32(),
            BlockItem::Image { config, .. } => config.height.as_f32(),
            BlockItem::Table(laid_out_table) => laid_out_table.height().as_f32(),
            BlockItem::Embedded(embedded_item) => embedded_item.height().as_f32(),
            BlockItem::Hidden(config) => config.height().as_f32(),
        }
    }

    pub fn spacing(&self) -> BlockSpacing {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. } => paragraph.spacing(),
            BlockItem::TextBlock { paragraph_block } => paragraph_block.spacing(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            }
            | BlockItem::TemporaryBlock {
                paragraph_block, ..
            } => paragraph_block.spacing(),
            BlockItem::MermaidDiagram { config, .. } => config.spacing,
            BlockItem::TrailingNewLine(cursor) => cursor.spacing(),
            BlockItem::HorizontalRule(config) => config.spacing,
            BlockItem::Image { config, .. } => config.spacing,
            BlockItem::Table(laid_out_table) => laid_out_table.spacing(),
            BlockItem::Embedded(embedded_item) => embedded_item.spacing(),
            BlockItem::Hidden { .. } => BlockSpacing::default(),
        }
    }

    /// The height of this item's content, without any padding or margins.
    pub fn content_height(&self) -> Pixels {
        match self {
            BlockItem::Paragraph(paragraph) | BlockItem::Header { paragraph, .. } => {
                let mut height = paragraph.height;
                if let Some(minimum_height) = paragraph.minimum_height {
                    height = height.max(minimum_height);
                }
                height
            }
            BlockItem::TextBlock { paragraph_block } => paragraph_block.height(),
            BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. } => paragraph.height(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            }
            | BlockItem::TemporaryBlock {
                paragraph_block, ..
            } => paragraph_block.height(),
            BlockItem::MermaidDiagram { config, .. } => config.height,
            BlockItem::TrailingNewLine(cursor) => {
                let mut height = cursor.height;
                if let Some(minimum_height) = cursor.minimum_height {
                    height = height.max(minimum_height);
                }
                height
            }
            BlockItem::Embedded(embedded_item) => embedded_item.height(),
            BlockItem::HorizontalRule(rule) => rule.line_height,
            BlockItem::Image { config, .. } => config.height,
            BlockItem::Table(laid_out_table) => laid_out_table.height(),
            BlockItem::Hidden(config) => config.height(),
        }
    }

    pub fn content_width(&self) -> Pixels {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. } => paragraph.width(),
            BlockItem::TextBlock { paragraph_block } => paragraph_block.width(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            }
            | BlockItem::TemporaryBlock {
                paragraph_block, ..
            } => paragraph_block.width(),
            BlockItem::MermaidDiagram { config, .. } => config.width,
            BlockItem::TrailingNewLine(cursor) => cursor.width,
            BlockItem::Embedded(object) => object.size().x().into_pixels(),
            BlockItem::HorizontalRule(rule) => rule.width,
            BlockItem::Image { config, .. } => config.width,
            BlockItem::Table(laid_out_table) => laid_out_table.width(),
            BlockItem::Hidden(_) => MIN_HIDDEN_BLOCK_WIDTH,
        }
    }

    /// The total height of this item, including content, padding, and margins.
    pub fn height(&self) -> Pixels {
        self.content_height() + self.spacing().y_axis_offset()
    }

    pub fn width(&self) -> Pixels {
        self.content_width() + self.spacing().x_axis_offset()
    }

    pub fn content_length(&self) -> CharOffset {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. } => paragraph.content_length,
            BlockItem::TextBlock { paragraph_block } => paragraph_block.content_length(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => paragraph_block.content_length(),
            BlockItem::TemporaryBlock { .. } => CharOffset::zero(),
            BlockItem::MermaidDiagram { content_length, .. } => *content_length,
            BlockItem::TrailingNewLine(_)
            | BlockItem::Embedded(_)
            | BlockItem::HorizontalRule(_)
            | BlockItem::Image { .. } => CharOffset::from(1),
            BlockItem::Table(laid_out_table) => laid_out_table.content_length(),
            BlockItem::Hidden(config) => config.content_length(),
        }
    }

    pub fn lines(&self) -> LineCount {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. } => paragraph.lines(),
            BlockItem::TextBlock { paragraph_block } => paragraph_block.lines(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => paragraph_block.lines(),
            BlockItem::TemporaryBlock { .. } => LineCount(0),
            BlockItem::MermaidDiagram { .. } => LineCount(1),
            BlockItem::TrailingNewLine(_)
            | BlockItem::Embedded(_)
            | BlockItem::HorizontalRule(_)
            | BlockItem::Image { .. } => LineCount(1),
            BlockItem::Table(laid_out_table) => laid_out_table.lines(),
            BlockItem::Hidden(config) => config.line_count(),
        }
    }

    /// Returns `true` if this item is effectively empty. A newline-only block would be considered
    /// empty, despite having a content length of 1.
    pub fn is_empty(&self) -> bool {
        match self {
            BlockItem::Paragraph(paragraph)
            | BlockItem::Header { paragraph, .. }
            | BlockItem::UnorderedList { paragraph, .. }
            | BlockItem::OrderedList { paragraph, .. }
            | BlockItem::TaskList { paragraph, .. } => paragraph.is_empty(),
            BlockItem::TextBlock { paragraph_block } => paragraph_block.is_empty(),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => paragraph_block.is_empty(),
            BlockItem::MermaidDiagram { .. } => false,
            // Embeds, images, tables, and horizontal rules are never empty.
            BlockItem::Embedded(_)
            | BlockItem::HorizontalRule(_)
            | BlockItem::Image { .. }
            | BlockItem::Table(_)
            | BlockItem::TemporaryBlock { .. } => false,
            // The trailing newline placeholder and hidden blocks are always considered empty.
            BlockItem::TrailingNewLine(_) | BlockItem::Hidden { .. } => true,
        }
    }

    pub fn is_hidden(&self) -> bool {
        matches!(self, BlockItem::Hidden { .. })
    }

    pub fn is_trailing_newline(&self) -> bool {
        matches!(self, BlockItem::TrailingNewLine(_))
    }
}

impl Positioned<'_, BlockItem> {
    fn softwrap_point_to_offset(&self, point: SoftWrapPoint) -> CharOffset {
        match self.item {
            BlockItem::Paragraph(inner) => self.paragraph(inner).softwrap_point_to_offset(point),
            BlockItem::TextBlock { paragraph_block } => {
                let text_block = self.text_block(paragraph_block);

                let mut paragraphs = text_block.paragraphs();
                paragraphs
                    .find(|paragraph| paragraph.end_line().as_u32() > point.row())
                    .map_or(self.end_char_offset(), |paragraph| {
                        paragraph.softwrap_point_to_offset(point)
                    })
            }
            BlockItem::UnorderedList { paragraph, .. } => self
                .unordered_list(paragraph)
                .softwrap_point_to_offset(point),
            BlockItem::OrderedList { paragraph, .. } => {
                self.ordered_list(paragraph).softwrap_point_to_offset(point)
            }
            BlockItem::TaskList { paragraph, .. } => {
                self.task_list(paragraph).softwrap_point_to_offset(point)
            }
            BlockItem::Header {
                paragraph: inner, ..
            } => self.header(inner).softwrap_point_to_offset(point),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => {
                let code_block = self.code_block(paragraph_block);

                let mut paragraphs = code_block.paragraphs();
                paragraphs
                    .find(|paragraph| paragraph.end_line().as_u32() > point.row())
                    .map_or(self.end_char_offset(), |paragraph| {
                        paragraph.softwrap_point_to_offset(point)
                    })
            }
            BlockItem::MermaidDiagram { .. } => self.start_char_offset,
            BlockItem::TrailingNewLine(_)
            | BlockItem::Embedded(_)
            | BlockItem::HorizontalRule(_)
            | BlockItem::Image { .. }
            | BlockItem::TemporaryBlock { .. }
            | BlockItem::Hidden { .. } => self.start_char_offset,
            BlockItem::Table(laid_out_table) => {
                let row_in_table = point.row().saturating_sub(self.start_line.as_u32()) as usize;
                if let Some(cell_range) = laid_out_table.offset_map.cell_range(row_in_table, 0) {
                    let visible_cell_start = laid_out_table
                        .cell_offset_maps
                        .get(row_in_table)
                        .and_then(|row| row.first())
                        .map(|cell| cell.rendered_to_source(CharOffset::zero()))
                        .unwrap_or(CharOffset::zero());
                    self.start_char_offset + cell_range.start + visible_cell_start.as_usize()
                } else {
                    self.start_char_offset
                }
            }
        }
    }

    fn offset_to_softwrap_point(&self, offset: CharOffset) -> SoftWrapPoint {
        match self.item {
            BlockItem::Paragraph(inner) => self.paragraph(inner).offset_to_softwrap_point(offset),
            BlockItem::TextBlock { paragraph_block } => {
                let text_block = self.text_block(paragraph_block);

                let mut paragraphs = text_block.paragraphs();
                paragraphs
                    .find(|paragraph| paragraph.end_char_offset() > offset)
                    .map_or(
                        SoftWrapPoint::new(self.end_line().as_u32(), Pixels::zero()),
                        |paragraph| paragraph.offset_to_softwrap_point(offset),
                    )
            }
            BlockItem::UnorderedList { paragraph, .. } => self
                .unordered_list(paragraph)
                .offset_to_softwrap_point(offset),
            BlockItem::OrderedList { paragraph, .. } => self
                .ordered_list(paragraph)
                .offset_to_softwrap_point(offset),
            BlockItem::Header { paragraph, .. } => {
                self.header(paragraph).offset_to_softwrap_point(offset)
            }
            BlockItem::TaskList { paragraph, .. } => {
                self.task_list(paragraph).offset_to_softwrap_point(offset)
            }
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => {
                let code_block = self.code_block(paragraph_block);

                let mut paragraphs = code_block.paragraphs();
                paragraphs
                    .find(|paragraph| paragraph.end_char_offset() > offset)
                    .map_or(
                        SoftWrapPoint::new(self.end_line().as_u32(), Pixels::zero()),
                        |paragraph| paragraph.offset_to_softwrap_point(offset),
                    )
            }
            BlockItem::MermaidDiagram { .. } => {
                SoftWrapPoint::new(self.start_line.as_u32(), Pixels::zero())
            }
            BlockItem::TrailingNewLine(_)
            | BlockItem::Embedded(_)
            | BlockItem::HorizontalRule(_)
            | BlockItem::Image { .. }
            | BlockItem::TemporaryBlock { .. }
            | BlockItem::Hidden { .. } => {
                SoftWrapPoint::new(self.start_line.as_u32(), Pixels::zero())
            }
            BlockItem::Table(laid_out_table) => {
                let relative_offset = offset.saturating_sub(&self.start_char_offset);
                let row = laid_out_table
                    .offset_map
                    .cell_at_offset(relative_offset)
                    .map(|c| c.row)
                    .unwrap_or(0);
                SoftWrapPoint::new(self.start_line.as_u32() + row as u32, Pixels::zero())
            }
        }
    }

    /// Given a [`CharOffset`], finds the bounding box of the character at that offset (to the extent possible).
    /// The bounding box is relative to the buffer origin.
    fn character_bounds(&self, offset: CharOffset) -> Option<RectF> {
        match self.item {
            BlockItem::Paragraph(inner) => self.paragraph(inner).character_bounds(offset),
            BlockItem::TextBlock { paragraph_block } => {
                let text_block = self.text_block(paragraph_block);
                text_block
                    .paragraphs()
                    .find_or_last(|paragraph| paragraph.end_char_offset() > offset)
                    .and_then(|paragraph| paragraph.character_bounds(offset))
            }
            BlockItem::UnorderedList { paragraph, .. } => {
                self.unordered_list(paragraph).character_bounds(offset)
            }
            BlockItem::OrderedList { paragraph, .. } => {
                self.ordered_list(paragraph).character_bounds(offset)
            }
            BlockItem::TaskList { paragraph, .. } => {
                self.task_list(paragraph).character_bounds(offset)
            }
            BlockItem::Header { paragraph, .. } => self.header(paragraph).character_bounds(offset),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => {
                let code_block = self.code_block(paragraph_block);
                code_block
                    .paragraphs()
                    .find_or_last(|paragraph| paragraph.end_char_offset() > offset)
                    .and_then(|paragraph| paragraph.character_bounds(offset))
            }
            BlockItem::MermaidDiagram { config, .. } => {
                let origin = self.content_origin();
                Some(RectF::new(
                    origin,
                    vec2f(config.width.as_f32(), config.height.as_f32()),
                ))
            }
            BlockItem::TrailingNewLine(cursor) => {
                let origin = self.content_origin();
                Some(RectF::new(origin, cursor.size()))
            }
            BlockItem::HorizontalRule(rule) => {
                let origin = self.content_origin();
                Some(RectF::new(origin, rule.line_size()))
            }
            BlockItem::Image { config, .. } => {
                let origin = self.content_origin();
                Some(RectF::new(
                    origin,
                    vec2f(config.width.as_f32(), config.height.as_f32()),
                ))
            }
            BlockItem::Table(laid_out_table) => {
                if offset < self.start_char_offset || offset >= self.end_char_offset() {
                    None
                } else {
                    laid_out_table
                        .character_bounds(offset - self.start_char_offset, self.content_origin())
                }
            }
            BlockItem::Embedded(embedded_item) => {
                let origin = self.content_origin();
                Some(RectF::new(origin, embedded_item.size()))
            }
            BlockItem::TemporaryBlock { .. } | BlockItem::Hidden { .. } => None,
        }
    }

    /// The bounds of the first line of this item, relative to the buffer origin.
    pub fn first_line_bounds(&self) -> Option<RectF> {
        let line_bounds = match self.item {
            BlockItem::Paragraph(inner) => self.paragraph(inner).first_line_bounds()?,
            BlockItem::TextBlock { paragraph_block } => {
                self.text_block(paragraph_block).first_line_bounds()?
            }
            BlockItem::UnorderedList { paragraph, .. } => {
                self.unordered_list(paragraph).first_line_bounds()?
            }
            BlockItem::TaskList { paragraph, .. } => {
                self.task_list(paragraph).first_line_bounds()?
            }
            BlockItem::OrderedList { paragraph, .. } => {
                self.ordered_list(paragraph).first_line_bounds()?
            }
            BlockItem::Header { paragraph, .. } => self.header(paragraph).first_line_bounds()?,
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => {
                // For code blocks, we treat the top padding as the first line.
                let first_line = self.code_block(paragraph_block).first_line_bounds()?;
                RectF::from_points(self.visible_origin(), first_line.upper_right())
            }
            BlockItem::MermaidDiagram { config, .. } => {
                let origin = self.visible_origin();
                RectF::new(origin, vec2f(config.width.as_f32(), config.height.as_f32()))
            }
            BlockItem::TrailingNewLine(cursor) => {
                let origin = self.trailing_newline(cursor).content_origin();
                RectF::new(origin, cursor.size())
            }
            BlockItem::HorizontalRule(rule) => {
                let origin = self.content_origin();
                RectF::new(origin, rule.line_size())
            }
            BlockItem::Image { config, .. } => {
                let origin = self.content_origin();
                RectF::new(origin, vec2f(config.width.as_f32(), config.height.as_f32()))
            }
            BlockItem::Table(laid_out_table) => {
                let origin = self.content_origin();
                RectF::new(
                    origin,
                    vec2f(
                        laid_out_table.width().as_f32(),
                        laid_out_table.height().as_f32(),
                    ),
                )
            }
            BlockItem::Embedded(embedded_item) => {
                let origin = self.visible_origin();
                RectF::new(origin, embedded_item.first_line_bound())
            }
            BlockItem::TemporaryBlock { .. } | BlockItem::Hidden { .. } => return None,
        };

        // At the block level, we want to include any space for list bullets and other
        // decorations/padding in the first line.
        let flush_origin = vec2f(self.reserved_origin().x(), line_bounds.origin_y());
        Some(RectF::from_points(flush_origin, line_bounds.lower_right()))
    }
}

impl sum_tree::Item for BlockItem {
    type Summary = LayoutSummary;

    fn summary(&self) -> Self::Summary {
        LayoutSummary {
            content_length: self.content_length(),
            // We use f64 to represent heights to avoid cumulative precision error from iteratively adding
            // block heights when querying / constructing the tree. In rendering, we don't actually need
            // high decimal point precisions so it's fine to cast from f32 to f64 here.
            height: self.height().as_f32() as f64,
            width: self.width(),
            lines: self.lines(),
            item_count: 1,
        }
    }
}

impl<'a> Positioned<'a, Paragraph> {
    /// Iterator over the lines of a paragraph, along with positioning information.
    fn lines(&self) -> impl Iterator<Item = Positioned<'a, Line>> + '_ {
        self.item.frame.lines().iter().scan(
            (
                self.start_y_offset + self.style.top_offset(),
                self.start_line,
            ),
            |(y_offset_acc, line_acc), line| {
                let positioned = Some(Positioned {
                    start_y_offset: *y_offset_acc,
                    // Lines record their character index relative to the TextFrame start,
                    // so we can keep its starting offset for each.
                    start_char_offset: self.start_char_offset,
                    start_line: *line_acc,
                    // Remove y axis offset here because the paragraph level styling should
                    // not apply to each text frame line.
                    style: self.style.without_y_axis_offsets(),
                    item: line,
                });
                *y_offset_acc += line_height(line).into_pixels();
                *line_acc += LineCount(1);
                positioned
            },
        )
    }

    /// The bounds of the first line of this paragraph, relative to the content origin. This is
    /// useful for UI elements positioned alongside the start of the paragraph.
    fn first_line_bounds(&self) -> Option<RectF> {
        let first_line = self.lines().next()?;
        let size = vec2f(first_line.item.width, self.item.first_line_height());
        Some(RectF::new(first_line.content_origin(), size))
    }

    pub fn end_char_offset(&self) -> CharOffset {
        self.start_char_offset + self.item.content_length
    }

    pub fn end_line(&self) -> LineCount {
        self.start_line + self.item.lines()
    }

    // Maximum softwrap point in the paragraph on the last line.
    pub fn max_softwrap_point(&self) -> SoftWrapPoint {
        match self.lines().last() {
            Some(line) => SoftWrapPoint::new(
                line.start_line.as_u32(),
                line.item.width.into_pixels() + self.style.left_offset(),
            ),
            None => SoftWrapPoint::new(self.start_line.as_u32(), Pixels::zero()),
        }
    }

    pub fn end_y_offset(&self) -> Pixels {
        self.start_y_offset + self.item.height + self.style.top_offset()
    }

    /// The bounding box of the character at `offset`, if within this paragraph.
    fn character_bounds(&self, offset: CharOffset) -> Option<RectF> {
        let (line, frame_offset) = self.offset_to_frame_location(offset)?;
        let x_offset = line.item.caret_position_for_index(frame_offset.as_usize());
        // We don't have easy access to the width of a given glyph, so use the next caret position
        // instead. This will clamp to the end of the line if needed.
        let next_x_offset = line
            .item
            .caret_position_for_index(frame_offset.as_usize() + 1);
        let origin = line.content_origin() + vec2f(x_offset, 0.);
        let size = vec2f(next_x_offset - x_offset, line_height(line.item));
        Some(RectF::new(origin, size))
    }

    /// Resolves a content `CharOffset` to the corresponding frame offset and containing line, if
    /// it's in bounds for this paragraph.
    fn offset_to_frame_location(
        &self,
        offset: CharOffset,
    ) -> Option<(Positioned<'_, Line>, FrameOffset)> {
        if offset < self.start_char_offset || offset >= self.end_char_offset() {
            return None;
        }
        let offset_within_frame = self.frame_index(offset);

        // We've already checked that `offset` is in bounds. In that case,
        // if no line contains the offset, it's probably at the newline
        // ending the paragraph (which doesn't have a caret position of
        // its own). If we're at the newline, then use the end of the
        // paragraph as the pixel position.
        let line = self.lines().find_or_last(|line| {
            // TODO(ben): handle clamping above/below
            line.item.last_index() >= offset_within_frame.as_usize()
        })?;

        Some((line, offset_within_frame))
    }

    /// Draws any selection highlights and cursors that are within this paragraph.
    pub(super) fn draw_selection(&self, model: &RenderState, ctx: &mut RenderContext) {
        let styles = &model.styles;
        for (i, selection) in model.selections().iter().enumerate() {
            let start = selection.start();
            let end = selection.end();
            let bias = selection.cursor_bias();
            // Where we should paint the cursor. This is not always the same as `start`.
            let cursor_offset = selection.head;

            // If the selection is a cursor, don't also draw a selection highlight.
            if start != end {
                self.draw_highlight(start, end, styles.selection_fill, ctx, model.max_line());
            } else if let Some(VimMode::Visual(_)) = ctx.vim_mode {
                // If we're in Vim visual mode, render the visual mode selection.
                let Some(visual_tail) = ctx.vim_visual_tails.get(i) else {
                    continue;
                };

                let visual_start = *visual_tail;
                let visual_end = selection.head;
                let (visual_start, visual_end) = if visual_start > visual_end {
                    (visual_end, visual_start)
                } else {
                    (visual_start, visual_end)
                };

                self.draw_highlight(
                    visual_start,
                    visual_end,
                    styles.selection_fill,
                    ctx,
                    model.max_line(),
                );
            }

            if cursor_offset >= self.start_char_offset && cursor_offset < self.end_char_offset() {
                self.draw_cursor(cursor_offset, bias, styles, ctx);
            }
        }
    }

    fn draw_cursor(
        &self,
        offset: CharOffset,
        bias: Option<RenderedSelectionBias>,
        styles: &RichTextStyles,
        ctx: &mut RenderContext,
    ) {
        let Some((line, frame_offset)) = self.offset_to_frame_location(offset) else {
            // The cursor isn't within this paragraph.
            return;
        };

        let delta = match bias {
            Some(RenderedSelectionBias::Left) => -line.item.font_size / 8.,
            Some(RenderedSelectionBias::Right) => line.item.font_size / 8.,
            None => 0.,
        };

        // TODO: Instead of tracking content_origin and text_origin separately, it might be
        // simpler to track a start_position (rather than start_y_offset) in Positioned. Then,
        // blocks with padding (like a code block) could position their children with both
        // horizontal and vertical offsets.
        let cursor_position = line.content_origin()
            + vec2f(
                line.item.caret_position_for_index(frame_offset.as_usize()) + delta,
                0.,
            );

        let cursor_type = ctx.cursor_type;
        let block_width = line
            .item
            .width_for_index(frame_offset.as_usize())
            .filter(|width| *width > 0.0);

        let cursor_data = CursorData {
            block_width,
            font_size: Some(line.item.font_size),
        };

        ctx.draw_and_save_cursor(
            cursor_type,
            cursor_position,
            vec2f(styles.cursor_width, line_height(line.item)),
            cursor_data,
            styles,
        );
    }

    /// Draws a background highlight over the portion of this block that overlaps with the given
    /// range.
    pub(super) fn draw_highlight(
        &self,
        start: CharOffset,
        end: CharOffset,
        fill: Fill,
        ctx: &mut RenderContext,
        buffer_max_line: LineCount,
    ) {
        match ctx.vim_mode {
            Some(VimMode::Visual(MotionType::Linewise)) => {
                // For linewise visual mode, we want to highlight entire lines from the starting row to ending row.
                self.draw_linewise_highlight(start, end, fill, ctx, buffer_max_line)
            }
            Some(VimMode::Visual(MotionType::Charwise)) => {
                // Charwise visual mode should include the character under the block cursor.
                self.draw_charwise_highlight(start, end + 1, fill, ctx, buffer_max_line)
            }
            _ => self.draw_charwise_highlight(start, end, fill, ctx, buffer_max_line),
        }
    }

    /// Draws linewise visual selection highlighting - highlights complete lines from start row to end row
    fn draw_linewise_highlight(
        &self,
        start: CharOffset,
        end: CharOffset,
        fill: Fill,
        ctx: &mut RenderContext,
        _buffer_max_line: LineCount,
    ) {
        for line in self.lines() {
            let line_start_within_buffer = self.buffer_index(line.item.first_index().into());
            let line_end_within_buffer = self.buffer_index(line.item.end_index().into());

            // Check if this line intersects with the selection range.
            if line_end_within_buffer >= start && line_start_within_buffer <= end {
                // Start at beginning of line, go to last char in line
                let start_x = 0.0;
                let end_x = line.item.width;

                ctx.paint
                    .scene
                    .draw_rect_with_hit_recording(RectF::new(
                        ctx.content_to_screen(line.content_origin()) + vec2f(start_x, 0.),
                        vec2f(end_x - start_x, line_height(line.item)),
                    ))
                    .with_background(fill);
            }
        }
    }

    /// Computes the x positions for a given character offset range within a line.
    ///
    /// Returns `Some((start_x, end_x))` if the line intersects with the range,
    /// or `None` if:
    /// - The line starts after the end of the range (signals iteration should stop)
    /// - The line doesn't intersect the range (signals this line should be skipped)
    ///
    /// The second return value indicates whether iteration should stop entirely.
    fn offsets_to_line_x_position(
        &self,
        line: &Positioned<'_, Line>,
        start: CharOffset,
        end: CharOffset,
    ) -> Option<(f32, f32)> {
        let line_start_within_buffer = self.buffer_index(line.item.first_index().into());
        let line_end_within_buffer = self.buffer_index(line.item.end_index().into());

        // If the line starts after the end of the range, this and all
        // following lines of the paragraph are not part of the range.
        if line_start_within_buffer >= end {
            return None;
        }

        // Skip lines that don't intersect the range
        if line_end_within_buffer <= start {
            return None;
        }

        // Clamp the start and end positions to the bounds of this specific line
        let start_x = line.item.caret_position_for_index(
            self.frame_index(start.max(line_start_within_buffer))
                .as_usize(),
        );
        let end_x = line
            .item
            .caret_position_for_index(self.frame_index(end.min(line_end_within_buffer)).as_usize());

        Some((start_x, end_x))
    }

    /// Draws highlighting for typical selections going charwise from start to end.
    fn draw_charwise_highlight(
        &self,
        start: CharOffset,
        end: CharOffset,
        fill: Fill,
        ctx: &mut RenderContext,
        buffer_max_line: LineCount,
    ) {
        for line in self.lines() {
            let Some((start_x, end_x)) = self.offsets_to_line_x_position(&line, start, end) else {
                let line_start_within_buffer = self.buffer_index(line.item.first_index().into());
                // If line starts after range end, stop iteration
                if line_start_within_buffer >= end {
                    break;
                }
                // Otherwise, skip this line
                continue;
            };

            ctx.paint
                .scene
                .draw_rect_with_hit_recording(RectF::new(
                    ctx.content_to_screen(line.content_origin()) + vec2f(start_x, 0.),
                    vec2f(end_x - start_x, line_height(line.item)),
                ))
                .with_background(fill);

            let line_start_within_buffer = self.buffer_index(line.item.first_index().into());
            let line_end_within_buffer = self.buffer_index(line.item.end_index().into());
            let is_last_line_of_buffer =
                line.start_line == buffer_max_line.saturating_sub(&LineCount(1));
            let selection_crosses_newline = selection_crosses_newline_offset_based(
                is_last_line_of_buffer,
                start.as_usize(),
                end.as_usize(),
                line_start_within_buffer.as_usize(),
                line_end_within_buffer.as_usize(),
            );
            if selection_crosses_newline {
                let tick_width = calculate_tick_width(line.item.font_size);
                let tick_origin =
                    ctx.content_to_screen(line.content_origin()) + vec2f(line.item.width, 0.);
                ctx.paint
                    .scene
                    .draw_rect_with_hit_recording(create_newline_tick_rect(NewlineTickParams {
                        tick_origin,
                        tick_width,
                        tick_height: line_height(line.item),
                    }))
                    .with_background(fill);
            }
        }
    }

    /// Draws a dashed underline decoration over the portion of this block that overlaps with the
    /// given range. Used for diagnostics.
    pub(super) fn draw_dashed_underline(
        &self,
        start: CharOffset,
        end: CharOffset,
        color: ColorU,
        ctx: &mut RenderContext,
    ) {
        for line in self.lines() {
            let Some((start_x, end_x)) = self.offsets_to_line_x_position(&line, start, end) else {
                let line_start_within_buffer = self.buffer_index(line.item.first_index().into());
                // If line starts after range end, stop iteration
                if line_start_within_buffer >= end {
                    break;
                }
                // Otherwise, skip this line
                continue;
            };

            let underline_width = end_x - start_x;
            if underline_width <= 0. {
                continue;
            }

            // Position underline at the baseline of the text
            let underline_origin = ctx.content_to_screen(line.content_origin())
                + vec2f(start_x, line_height(line.item) - UNDERLINE_THICKNESS);

            let underline_rect = RectF::new(
                underline_origin,
                vec2f(underline_width, UNDERLINE_THICKNESS),
            );

            let dash = warpui::scene::Dash {
                dash_length: DASHED_UNDERLINE_DASH_LENGTH,
                gap_length: DASHED_UNDERLINE_GAP_LENGTH,
                force_consistent_gap_length: true,
            };
            ctx.paint
                .scene
                .draw_rect_without_hit_recording(underline_rect)
                .with_border(
                    warpui::scene::Border::bottom(UNDERLINE_THICKNESS)
                        .with_dashed_border(dash)
                        .with_border_color(color),
                );
        }
    }

    fn coordinate_to_location(&self, x: Pixels, y: Pixels) -> Location {
        let (line, clamped_on_y) = match (
            self.lines().find(|line| line.end_y_offset() > y),
            self.lines().last(),
        ) {
            (Some(line), _) => (line.item, false),
            // When the location is below the paragraph, clamp to the character matching its
            // x-axis pixel position on the last line.
            (None, Some(last_line)) => (last_line.item, true),
            (None, None) => {
                // If the paragraph is empty, clamp to the end of the paragraph.
                return Location::Text {
                    char_offset: self.end_char_offset().saturating_sub(&1.into()),
                    clamped: true,
                    wrap_direction: WrapDirection::Up,
                    block_start: self.start_char_offset,
                    link: None,
                };
            }
        };

        let (frame_index, clamped_on_x, wrap_direction) = match line.caret_index_for_x(x.as_f32()) {
            Some(index) => (FrameOffset::from(index), false, WrapDirection::Down),
            None => {
                // If a line contained `y`, but it does not contain `x`, clamp to
                // the extremes of the line.
                let frame_offset = FrameOffset::from(if x <= Pixels::zero() {
                    0
                } else {
                    line.end_index()
                });
                (frame_offset, true, WrapDirection::Up)
            }
        };

        // Clamp to the last character within this block (which may not be within the line).
        // This prevents an issue with hit-testing on empty blocks where we would instead return
        // the start of the next block.
        let buffer_index = self.buffer_index(frame_index);
        let end = self.end_char_offset().saturating_sub(&CharOffset::from(1));

        Location::Text {
            char_offset: buffer_index.min(end),
            clamped: clamped_on_x || clamped_on_y,
            wrap_direction,
            block_start: self.start_char_offset,
            link: self.link(frame_index),
        }
    }

    fn offset_to_softwrap_point(&self, offset: CharOffset) -> SoftWrapPoint {
        let Some(line) = self
            .lines()
            .find(|line| self.buffer_index(line.item.end_index().into()) > offset)
        else {
            return self.max_softwrap_point();
        };

        SoftWrapPoint::new(
            line.start_line.as_u32(),
            line.item
                .caret_position_for_index(self.frame_index(offset).as_usize())
                .into_pixels()
                + self.style.left_offset(),
        )
    }

    fn softwrap_point_to_offset(&self, point: SoftWrapPoint) -> CharOffset {
        let line = match self
            .lines()
            .find(|line| line.start_line.as_u32() >= point.row)
        {
            Some(line) => line.item,
            None => return self.end_char_offset(),
        };

        let line_end = self.buffer_index(line.end_index().into());
        let paragraph_last_char = self.end_char_offset().saturating_sub(&1.into());

        let adjusted_x = point.column() - self.style.left_offset();
        let frame_index = match line.caret_index_for_x(adjusted_x.as_f32()) {
            Some(caret) => FrameOffset::from(caret),
            None => {
                // caret_index_for_x returns None if the position is out of bounds. Check which side
                // it's out of bounds on to decide how to clamp.
                if adjusted_x <= Pixels::zero() {
                    FrameOffset::from(line.first_index())
                } else {
                    FrameOffset::from(line.end_index())
                }
            }
        };

        let offset = self.buffer_index(frame_index);

        // The upper bound on the offset depends on whether the line is soft-wrapped or hard-wrapped:
        // - If soft-wrapped, it's the index just after the last character on the line
        // - If hard-wrapped, it's the last non-newline character in the paragraph
        // In both cases, visually this is the caret position just after the last glyph in the line.
        offset.min(line_end).min(paragraph_last_char)
    }

    /// Converts a `CharOffset` to the corresponding character index in the text frame.
    fn frame_index(&self, offset: CharOffset) -> FrameOffset {
        self.item
            .offsets
            .to_frame(offset.saturating_sub(&self.start_char_offset))
    }

    /// Converts a text frame character index to the corresponding content `CharOffset`.
    fn buffer_index(&self, offset: FrameOffset) -> CharOffset {
        self.item.offsets.to_content(offset) + self.start_char_offset
    }

    fn link(&self, offset: FrameOffset) -> Option<String> {
        self.item.detected_url.iter().find_map(|url| {
            if url.url_range().contains(&offset.as_usize()) {
                Some(url.link())
            } else {
                None
            }
        })
    }
}

impl<'a> Positioned<'a, ParagraphBlock> {
    pub(super) fn paragraphs(&self) -> impl Iterator<Item = Positioned<'a, Paragraph>> + '_ {
        self.item.paragraphs.iter().scan(
            (
                self.start_char_offset,
                self.start_y_offset + self.style.top_offset(),
                self.start_line,
            ),
            |(char_offset_acc, y_offset_acc, line_acc), paragraph| {
                let positioned = Some(Positioned {
                    start_y_offset: *y_offset_acc,
                    // Lines record their character index relative to the TextFrame start,
                    // so we can keep its starting offset for each.
                    start_char_offset: *char_offset_acc,
                    start_line: *line_acc,
                    style: self.style.without_y_axis_offsets(),
                    item: paragraph,
                });
                *char_offset_acc += paragraph.content_length;
                *y_offset_acc += paragraph.height;
                *line_acc += paragraph.lines();
                positioned
            },
        )
    }

    /// The bounds of the first line of the first paragraph. See [`Positioned::<Paragraph>::first_line_bounds`].
    fn first_line_bounds(&self) -> Option<RectF> {
        self.paragraphs().next()?.first_line_bounds()
    }
}

impl<'a> sum_tree::Dimension<'a, LayoutSummary> for CharOffset {
    fn add_summary(&mut self, summary: &'a LayoutSummary) {
        *self += summary.content_length;
    }
}

impl<'a> sum_tree::Dimension<'a, LayoutSummary> for Height {
    fn add_summary(&mut self, summary: &'a LayoutSummary) {
        self.0 += summary.height
    }
}

impl<'a> sum_tree::Dimension<'a, LayoutSummary> for Width {
    fn add_summary(&mut self, summary: &'a LayoutSummary) {
        *self.0 = self.0.0.max(summary.width);
    }
}

impl<'a> sum_tree::Dimension<'a, LayoutSummary> for LayoutSummary {
    fn add_summary(&mut self, summary: &'a LayoutSummary) {
        *self += summary
    }
}

impl<'a> sum_tree::Dimension<'a, LayoutSummary> for LineCount {
    fn add_summary(&mut self, summary: &'a LayoutSummary) {
        *self += summary.lines;
    }
}

impl fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Paragraph")
            .field("lines", &self.frame.lines().len())
            .field("offsets", &self.offsets)
            .field("max_width", &self.frame.max_width())
            .field("height", &self.height)
            .field("content_length", &self.content_length)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedSelectionBias {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSelection {
    /// Head is the position of the cursor.
    pub head: CharOffset,
    pub tail: CharOffset,
    pub cursor_bias: Option<RenderedSelectionBias>,
}

impl RenderedSelection {
    pub fn new(head: CharOffset, tail: CharOffset) -> Self {
        Self {
            head,
            tail,
            cursor_bias: None,
        }
    }

    pub fn new_with_cursor_bias(
        head: CharOffset,
        tail: CharOffset,
        bias: RenderedSelectionBias,
    ) -> Self {
        Self {
            head,
            tail,
            cursor_bias: Some(bias),
        }
    }

    // Starting offset of the selection. Note that start is different from head because
    // the cursor could be at either the start or end of the selection.
    pub fn start(&self) -> CharOffset {
        if self.head > self.tail {
            self.tail
        } else {
            self.head
        }
    }

    // Ending offset of the selection.
    pub fn end(&self) -> CharOffset {
        if self.head > self.tail {
            self.head
        } else {
            self.tail
        }
    }

    pub fn is_cursor(&self) -> bool {
        self.head == self.tail
    }

    pub fn cursor_bias(&self) -> Option<RenderedSelectionBias> {
        self.cursor_bias
    }

    // The cursor position, if this selection is a single cursor.
    pub fn single_cursor(&self) -> Option<CharOffset> {
        self.is_cursor().then_some(self.head)
    }
}

impl Default for RenderedSelection {
    fn default() -> Self {
        Self {
            head: CharOffset::zero(),
            tail: CharOffset::zero(),
            cursor_bias: None,
        }
    }
}

impl fmt::Display for RenderedSelection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}..{}", self.start(), self.end())?;
        if let Some(bias) = self.cursor_bias {
            write!(f, " ({bias:?})")?;
        }
        Ok(())
    }
}

/// A set of all selections in the buffer.  There must be at least
/// one selection at all times.
#[derive(Eq, PartialEq, Debug, Clone, Default)]
pub struct RenderedSelectionSet {
    selections: Vec1<RenderedSelection>,
}

// Vec1 can never be empty, so we can ignore the warning to add an is_empty method.
#[allow(clippy::len_without_is_empty)]
impl RenderedSelectionSet {
    pub fn iter(&self) -> impl Iterator<Item = &RenderedSelection> {
        self.selections.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut RenderedSelection> {
        self.selections.iter_mut()
    }

    pub fn new(selection: RenderedSelection) -> Self {
        Self {
            selections: Vec1::new(selection),
        }
    }

    pub fn first(&self) -> &RenderedSelection {
        self.selections.first()
    }

    pub fn selection_map<T, F>(&self, f: F) -> Vec1<T>
    where
        F: Fn(&RenderedSelection) -> T,
    {
        self.selections.mapped_ref(f)
    }

    pub fn len(&self) -> usize {
        self.selections.len()
    }
}

impl From<Vec1<RenderedSelection>> for RenderedSelectionSet {
    fn from(selections: Vec1<RenderedSelection>) -> RenderedSelectionSet {
        RenderedSelectionSet { selections }
    }
}

impl fmt::Display for RenderedSelectionSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, selection) in self.selections.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{selection}")?;
        }
        write!(f, "]")
    }
}

impl IntoIterator for RenderedSelectionSet {
    type Item = RenderedSelection;
    type IntoIter = std::vec::IntoIter<RenderedSelection>;

    fn into_iter(self) -> Self::IntoIter {
        self.selections.into_iter()
    }
}

impl<'a> IntoIterator for &'a RenderedSelectionSet {
    type Item = &'a RenderedSelection;
    type IntoIter = slice::Iter<'a, RenderedSelection>;

    fn into_iter(self) -> Self::IntoIter {
        self.selections.iter()
    }
}

#[derive(Debug, Clone)]
pub enum UpdateDecorationAfterLayout {
    Line(Vec<LineDecoration>),
    LineAndText {
        line: Vec<LineDecoration>,
        text: Vec<Decoration>,
    },
}

impl UpdateDecorationAfterLayout {
    pub fn sort(&mut self) {
        match self {
            Self::Line(decorations) => {
                decorations.sort_unstable_by_key(|decoration| decoration.end)
            }
            Self::LineAndText { line, text } => {
                line.sort_unstable_by_key(|decoration| decoration.end);
                text.sort_unstable_by_key(|decoration| decoration.end);
            }
        }
    }
}

/// A render-time line decoration, such as highlighting the line with active cursor.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct LineDecoration {
    /// Start of the decorated range (inclusive).
    pub start: LineCount,
    /// End of the decorated range (exclusive).
    pub end: LineCount,
    pub overlay: ThemeFill,
}

impl LineDecoration {
    pub fn new(start: LineCount, end: LineCount, overlay: ThemeFill) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            overlay,
        }
    }
}

/// A render-time text decoration, such as a highlighted search result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decoration {
    /// Start of the decorated range (inclusive).
    pub start: CharOffset,
    /// End of the decorated range (exclusive).
    pub end: CharOffset,
    pub background: Option<ThemeFill>,
    /// Color for a dashed underline (used for diagnostics).
    pub dashed_underline: Option<ColorU>,
}

impl Decoration {
    pub fn new(start: CharOffset, end: CharOffset) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            background: None,
            dashed_underline: None,
        }
    }

    pub fn with_background(mut self, background: ThemeFill) -> Self {
        self.background = Some(background);
        self
    }

    pub fn with_dashed_underline(mut self, color: ColorU) -> Self {
        self.dashed_underline = Some(color);
        self
    }
}

#[derive(Debug)]
pub struct BrokenBlockEmbedding {
    width: Pixels,
    height: Pixels,
}

impl BrokenBlockEmbedding {
    pub fn new(width: Pixels, font_size: f32) -> Self {
        Self {
            width,
            height: (font_size + 2.).into_pixels(),
        }
    }
}

impl LaidOutEmbeddedItem for BrokenBlockEmbedding {
    fn height(&self) -> Pixels {
        self.height
    }

    fn size(&self) -> Vector2F {
        vec2f(self.width.as_f32(), self.height().as_f32())
    }

    fn first_line_bound(&self) -> Vector2F {
        vec2f(self.width.as_f32(), EMBEDDED_ITEM_FIRST_LINE_HEIGHT)
    }

    fn element(
        &self,
        state: &RenderState,
        viewport_item: ViewportItem,
        model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        Box::new(RenderableBrokenEmbedding::new(
            viewport_item,
            state.styles(),
            model,
            ctx,
        ))
    }

    fn spacing(&self) -> BlockSpacing {
        BROKEN_LINK_SPACING
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
