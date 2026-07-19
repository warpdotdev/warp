use std::cell::RefCell;
use std::ops::AddAssign;
use std::rc::Rc;
use std::sync::Arc;

use derive_more::AddAssign as DeriveAddAssign;
use ordered_float::OrderedFloat;
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use sum_tree::SumTree;

use super::{
    Point, ScrollData, ScrollableElement, SelectableElement, Selection, SelectionFragment,
    SmartSelectFn,
};
use crate::event::DispatchedEvent;
use crate::scene::ClipBounds;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use crate::units::{IntoPixels, Pixels};
use crate::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

// ============================================================================
// Constants
// ============================================================================

/// Default estimated row height used when no rows have been measured yet.
const DEFAULT_ROW_HEIGHT_ESTIMATE: f32 = 32.0;

/// Callback type for rendering a single row on demand.
/// Takes the row index and returns a vector of cell elements for that row.
pub type TableRowRenderFn = dyn Fn(usize, &AppContext) -> Vec<Box<dyn Element>> + 'static;

// ============================================================================
// SumTree Types for Row Virtualization
// ============================================================================

/// A single row item in the table's sum tree.
/// Stores the measured height of a row, or None if not yet measured.
#[derive(Clone, Debug)]
struct TableRowItem {
    height: Option<Pixels>,
}

/// Summary for the sum tree, tracking total height and measurement state.
#[derive(Debug, Clone, Default)]
struct RowLayoutSummary {
    /// Total height of all items in the sum tree (only measured items contribute).
    height: Pixels,
    /// Total number of items (rows) in the sum tree.
    count: usize,
    /// Number of items that have been measured (have Some(height)).
    measured_count: usize,
}

impl sum_tree::Item for TableRowItem {
    type Summary = RowLayoutSummary;

    fn summary(&self) -> Self::Summary {
        RowLayoutSummary {
            height: self.height.unwrap_or(Pixels::zero()),
            count: 1,
            measured_count: if self.height.is_some() { 1 } else { 0 },
        }
    }
}

impl AddAssign<&RowLayoutSummary> for RowLayoutSummary {
    fn add_assign(&mut self, rhs: &RowLayoutSummary) {
        self.height += rhs.height;
        self.count += rhs.count;
        self.measured_count += rhs.measured_count;
    }
}

/// Height dimension for sum tree seeking.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Height(OrderedFloat<Pixels>);

impl From<Pixels> for Height {
    fn from(value: Pixels) -> Self {
        Self(OrderedFloat(value))
    }
}

impl<'a> sum_tree::Dimension<'a, RowLayoutSummary> for Height {
    fn add_summary(&mut self, summary: &'a RowLayoutSummary) {
        self.0 .0 += summary.height;
    }
}

/// Count dimension for sum tree seeking by row index.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, DeriveAddAssign)]
struct RowCount(usize);

impl<'a> sum_tree::Dimension<'a, RowLayoutSummary> for RowCount {
    fn add_summary(&mut self, summary: &'a RowLayoutSummary) {
        self.0 += summary.count;
    }
}

/// Scroll position in the table, similar to ViewportedList's ScrollOffset.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
struct RowScrollOffset {
    /// The row that is at the top of the viewport.
    row_index: RowCount,
    /// Number of pixels offset from the start of that row.
    offset_from_start: Pixels,
}

// ============================================================================
// Public Table Types
// ============================================================================

/// Column width specification for table columns.
#[derive(Debug, Clone, Copy)]
pub enum TableColumnWidth {
    /// Fixed pixel width.
    Fixed(f32),
    /// Proportional share of remaining space after fixed columns are allocated.
    /// Default is Flex(1.0).
    Flex(f32),
    /// Fraction of total table width (0.0 to 1.0).
    Fraction(f32),
    /// Width determined by measuring the intrinsic width of cell content.
    /// The column will be sized to fit the widest cell in that column.
    Intrinsic,
}

impl Default for TableColumnWidth {
    fn default() -> Self {
        Self::Flex(1.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableVerticalSizing {
    #[default]
    Viewported,
    ExpandToContent,
}

/// Header cell definition that includes content and column width.
pub struct TableHeader {
    pub content: Box<dyn Element>,
    pub width: TableColumnWidth,
}

impl TableHeader {
    pub fn new(content: Box<dyn Element>) -> Self {
        Self {
            content,
            width: TableColumnWidth::default(),
        }
    }

    pub fn with_width(mut self, width: TableColumnWidth) -> Self {
        self.width = width;
        self
    }
}

/// Background color configuration for table rows.
#[derive(Debug, Clone)]
pub struct RowBackground {
    /// Background color for even-indexed rows (0, 2, 4, ...).
    pub primary: ColorU,
    /// If set, odd-indexed rows use this background color instead of `primary`.
    pub alternating: Option<ColorU>,
}

impl Default for RowBackground {
    fn default() -> Self {
        Self {
            primary: ColorU::white(),
            alternating: None,
        }
    }
}

impl RowBackground {
    pub fn uniform(color: ColorU) -> Self {
        Self {
            primary: color,
            alternating: None,
        }
    }

    pub fn striped(even: ColorU, odd: ColorU) -> Self {
        Self {
            primary: even,
            alternating: Some(odd),
        }
    }

    pub fn color_for_row(&self, row_index: usize) -> ColorU {
        if self.alternating.is_some() && row_index % 2 == 1 {
            self.alternating.unwrap_or(self.primary)
        } else {
            self.primary
        }
    }
}

/// Configuration for table styling.
#[derive(Debug, Clone)]
pub struct TableConfig {
    /// Width of the outer table border and column dividers in pixels.
    pub border_width: f32,
    /// Color for the outer table border and column dividers.
    pub border_color: ColorU,
    /// Whether to draw the outer table border.
    pub outer_border: bool,
    /// Whether to draw vertical dividers between columns.
    pub column_dividers: bool,
    /// Whether to draw horizontal dividers between rows.
    pub row_dividers: bool,
    /// Padding applied uniformly to all sides of each cell (top, right, bottom, left).
    pub cell_padding: f32,
    /// Background color for the header row.
    pub header_background: ColorU,
    /// Background colors for data rows.
    pub row_background: RowBackground,
    /// When true, the header row stays fixed at the top of the viewport while the body
    /// scrolls. The header is painted at a fixed position relative to the viewport.
    pub fixed_header: bool,
    /// Controls whether the table behaves as an internally viewported widget or expands to its
    /// full content height and relies on the parent to handle vertical scrolling.
    pub vertical_sizing: TableVerticalSizing,
    /// When true, intrinsic-width columns are measured against body cells in addition to headers.
    pub measure_body_cells_for_intrinsic_widths: bool,
}

/// Note: In production code, prefer creating configs using theme colors rather than Default.
/// This default implementation uses hardcoded colors and is primarily intended for tests and examples.
impl Default for TableConfig {
    fn default() -> Self {
        let border_light_gray = ColorU::new(208, 215, 222, 255);
        let header_background_light_gray = ColorU::new(246, 248, 250, 255);

        Self {
            border_width: 1.0,
            border_color: border_light_gray,
            outer_border: true,
            column_dividers: true,
            row_dividers: false,
            cell_padding: 8.0,
            header_background: header_background_light_gray,
            row_background: RowBackground::default(),
            fixed_header: false,
            vertical_sizing: TableVerticalSizing::Viewported,
            measure_body_cells_for_intrinsic_widths: false,
        }
    }
}

/// Internal state for table layout computations using sum tree for row virtualization.
///
/// This state persists across renders to maintain measured heights and scroll position.
pub struct TableState {
    // Row dimension - uses sum tree for efficient virtualization
    /// Sum tree storing row heights. Only measured rows contribute to height calculations.
    rows: SumTree<TableRowItem>,
    /// The last known measured row where every row up to this index has been measured.
    /// Can differ from measured_count in tree if a row in the middle was invalidated.
    last_measured_row_index: usize,
    /// Current scroll position in the table.
    scroll_top: RowScrollOffset,
    /// Height of the viewport, set during layout.
    viewport_height: Pixels,
    /// The row index of the first visible row (for painting and selection).
    /// Updated during layout to track which rows are in self.children.
    visible_start_row_idx: usize,

    // Column dimension - stays as flat vectors since columns are typically small and always visible
    column_widths: Vec<f32>,

    // Intrinsic column width cache
    /// Cached intrinsic widths for columns. Some(width) = measured, None = needs measurement.
    intrinsic_column_widths: Vec<Option<f32>>,

    // Header height (separate from row sum tree)
    header_height: Pixels,

    /// The actual rendered height of the table content (header + visible rows).
    /// Used in paint() to draw borders at the correct height when content < viewport.
    rendered_height: Pixels,

    /// Callback for rendering rows on demand. Rows are generated lazily during layout.
    row_render_fn: Arc<TableRowRenderFn>,
    /// Total number of rows.
    row_count: usize,
}

impl TableState {
    fn new<F>(row_count: usize, row_render_fn: F) -> Self
    where
        F: Fn(usize, &AppContext) -> Vec<Box<dyn Element>> + 'static,
    {
        Self {
            rows: SumTree::new(),
            last_measured_row_index: 0,
            scroll_top: RowScrollOffset::default(),
            viewport_height: Pixels::zero(),
            visible_start_row_idx: 0,
            column_widths: Vec::new(),
            intrinsic_column_widths: Vec::new(),
            header_height: Pixels::zero(),
            rendered_height: Pixels::zero(),
            row_render_fn: Arc::new(row_render_fn),
            row_count,
        }
    }
}

impl TableState {
    /// Get the absolute pixel position of the current scroll top (including header).
    fn scroll_top_pixels(&self) -> Pixels {
        let mut cursor = self.rows.cursor::<RowCount, Height>();
        cursor.seek(&self.scroll_top.row_index, sum_tree::SeekBias::Right);
        let row_start_height = cursor.start().0 .0;
        self.header_height + row_start_height + self.scroll_top.offset_from_start
    }

    /// Returns the approximate total height of the table (header + all rows).
    /// If all rows are measured, returns exact height. Otherwise estimates unmeasured rows.
    fn approximate_height(&self) -> Pixels {
        let summary = self.rows.summary();

        // If all rows are measured, return exact height
        if summary.count == summary.measured_count {
            return self.header_height + summary.height;
        }

        // Otherwise, estimate total height based on average
        if summary.measured_count == 0 {
            return self.header_height;
        }

        let avg_height = summary.height.as_f32() / summary.measured_count as f32;
        let estimated_total = avg_height * summary.count as f32;
        self.header_height + estimated_total.into_pixels()
    }

    /// Returns the average height of measured rows.
    fn average_height_per_measured_row(&self) -> Pixels {
        let summary = self.rows.summary();
        if summary.measured_count == 0 {
            return Pixels::new(DEFAULT_ROW_HEIGHT_ESTIMATE);
        }
        (summary.height.as_f32() / summary.measured_count as f32).into_pixels()
    }

    /// Convert an absolute pixel position (from top of table, including header) to a scroll offset.
    fn absolute_pixels_to_scroll_offset(&self, absolute_pixels: Pixels) -> RowScrollOffset {
        // Subtract header height to get position in row content
        let pixels_in_rows = (absolute_pixels - self.header_height).max(Pixels::zero());

        let summary = self.rows.summary();
        if summary.count == 0 {
            return RowScrollOffset::default();
        }

        // If scrolling beyond measured region, use average to estimate
        let avg_height = self.average_height_per_measured_row();
        let approximate_row = (pixels_in_rows.as_f32() / avg_height.as_f32()).floor() as usize;
        let approximate_row = approximate_row.min(summary.count);

        // If the target is within the measured region, seek to exact position
        if approximate_row <= self.last_measured_row_index {
            let mut cursor = self.rows.cursor::<Height, RowCount>();
            cursor.seek(&Height::from(pixels_in_rows), sum_tree::SeekBias::Right);
            let row_index = *cursor.start();

            let mut height_cursor = self.rows.cursor::<RowCount, Height>();
            height_cursor.seek(&row_index, sum_tree::SeekBias::Right);
            let row_start_height = height_cursor.start().0 .0;

            let offset = pixels_in_rows - row_start_height;

            return RowScrollOffset {
                row_index,
                offset_from_start: offset,
            };
        }

        // Beyond measured region, use estimate
        let approximate_offset =
            (pixels_in_rows.as_f32() - approximate_row as f32 * avg_height.as_f32()).into_pixels();
        RowScrollOffset {
            row_index: RowCount(approximate_row),
            offset_from_start: approximate_offset,
        }
    }

    /// Invalidate the height of a specific row.
    fn invalidate_row_height(&mut self, row_idx: usize) {
        let summary = self.rows.summary();
        if row_idx >= summary.count {
            return;
        }

        let (new_tree, last_measured) = {
            let mut cursor = self.rows.cursor::<RowCount, ()>();
            let mut new_items = cursor.slice(&RowCount(row_idx), sum_tree::SeekBias::Right);
            let last_measured = new_items.summary().measured_count.saturating_sub(1);

            new_items.push(TableRowItem { height: None });
            cursor.next();
            new_items.push_tree(cursor.suffix());
            (new_items, last_measured)
        };

        self.rows = new_tree;
        self.last_measured_row_index = self.last_measured_row_index.min(last_measured);
    }

    /// Get the maximum scroll offset (to prevent scrolling beyond content).
    fn max_scroll_offset(&self) -> RowScrollOffset {
        let max_scroll_pixels =
            (self.approximate_height() - self.viewport_height).max(Pixels::zero());
        self.absolute_pixels_to_scroll_offset(max_scroll_pixels)
    }
}

/// Handle for shared table state across renders.
/// This follows the same pattern as MouseStateHandle - the handle should be
/// created once by the containing view and passed into the table on each render.
#[derive(Clone)]
pub struct TableStateHandle {
    inner: Rc<RefCell<TableState>>,
}

impl TableStateHandle {
    /// Create a new table state handle with the given row count and render function.
    /// The render function is called lazily during layout to generate row elements.
    pub fn new<F>(row_count: usize, row_render_fn: F) -> Self
    where
        F: Fn(usize, &AppContext) -> Vec<Box<dyn Element>> + 'static,
    {
        Self {
            inner: Rc::new(RefCell::new(TableState::new(row_count, row_render_fn))),
        }
    }

    pub fn column_widths(&self) -> Vec<f32> {
        self.inner.borrow().column_widths.clone()
    }

    /// Invalidate the height of a specific row, forcing it to be re-measured on next layout.
    pub fn invalidate_row_height(&self, row_idx: usize) {
        let mut state = self.inner.borrow_mut();
        state.invalidate_row_height(row_idx);
    }

    /// Invalidate the cached intrinsic width for a specific column.
    ///
    /// Call this when the content of cells in an `Intrinsic` width column changes
    /// and you want the column to resize to fit the new content. Without calling
    /// this, intrinsic column widths are cached and won't update automatically.
    pub fn invalidate_intrinsic_width(&self, col_idx: usize) {
        let mut state = self.inner.borrow_mut();
        if col_idx < state.intrinsic_column_widths.len() {
            state.intrinsic_column_widths[col_idx] = None;
        }
    }

    /// Invalidate all intrinsic column widths.
    ///
    /// Call this when content changes across multiple `Intrinsic` width columns
    /// and you want all columns to resize to fit their new content.
    pub fn invalidate_all_intrinsic_widths(&self) {
        let mut state = self.inner.borrow_mut();
        for width in &mut state.intrinsic_column_widths {
            *width = None;
        }
    }

    /// Scroll to a specific row with an optional offset.
    pub fn scroll_to_row(&self, row_idx: usize, offset_from_start: Option<Pixels>) {
        let mut state = self.inner.borrow_mut();
        state.scroll_top = RowScrollOffset {
            row_index: RowCount(row_idx),
            offset_from_start: offset_from_start.unwrap_or(Pixels::zero()),
        };
    }

    /// Update the total number of rows.
    /// Call this when the data source changes size.
    pub fn set_row_count(&self, count: usize) {
        let mut state = self.inner.borrow_mut();
        state.row_count = count;
    }

    /// Update the row render function.
    /// Call this when the render logic needs to change (e.g., data source changes).
    pub fn set_row_render_fn<F>(&self, f: F)
    where
        F: Fn(usize, &AppContext) -> Vec<Box<dyn Element>> + 'static,
    {
        let mut state = self.inner.borrow_mut();
        state.row_render_fn = Arc::new(f);
    }

    /// Get the render function.
    pub(crate) fn row_render_fn(&self) -> Arc<TableRowRenderFn> {
        self.inner.borrow().row_render_fn.clone()
    }

    /// Get the row count for on-demand rendering.
    pub(crate) fn row_count(&self) -> usize {
        self.inner.borrow().row_count
    }
}

impl Default for TableStateHandle {
    fn default() -> Self {
        Self::new(0, |_, _| vec![])
    }
}

/// A table element that renders a header row followed by data rows.
///
/// ## Layout Algorithm
/// 1. **Intrinsic width measurement**: For columns with `TableColumnWidth::Intrinsic`,
///    measure each cell with unconstrained width to find the widest content.
/// 2. **Column width allocation**: Allocate widths based on `TableColumnWidth` specs:
///    - `Fixed(px)`: exact pixel width
///    - `Fraction(f)`: fraction of total available width
///    - `Intrinsic`: measured width (scaled down if total exceeds available)
///    - `Flex(f)`: proportional share of remaining space after fixed/fraction/intrinsic
/// 3. **Row layout**: Layout each cell with its column's computed width, track max height per row.
/// 4. **Position computation**: Compute cumulative column left positions and row top positions.
///
/// ## Virtualization
/// Uses a sumtree to track row heights and efficiently viewport large tables.
/// Only visible rows are laid out and painted.
pub struct Table {
    state: TableStateHandle,
    headers: Vec<TableHeader>,
    config: TableConfig,
    /// Width to use when the table has unconstrained width and no intrinsic content to measure.
    unconstrained_width: f32,
    /// Viewport height to use when height constraint is not finite.
    /// Used for virtualization and clipping calculations.
    unconstrained_height: f32,
    /// Computed during `layout()`. None before layout is called.
    size: Option<Vector2F>,
    /// Computed during `paint()`. None before paint is called.
    origin: Option<Point>,
    /// Stores only the visible row elements for the current frame after layout.
    /// These are the rows that will be painted.
    children: Vec<Vec<Box<dyn Element>>>,
    /// Heights of the children rows (parallel array to children).
    /// Stored to avoid sumtree queries during paint.
    children_heights: Vec<Pixels>,
}

impl Table {
    pub fn new(
        state: TableStateHandle,
        unconstrained_width: f32,
        unconstrained_height: f32,
    ) -> Self {
        Self {
            state,
            headers: Vec::new(),
            config: TableConfig::default(),
            unconstrained_width,
            unconstrained_height,
            size: None,
            origin: None,
            children: Vec::new(),
            children_heights: Vec::new(),
        }
    }

    pub fn with_headers(mut self, headers: Vec<TableHeader>) -> Self {
        self.headers = headers;
        self
    }

    /// Update the total number of rows.
    /// Useful when the row count differs from what was set at TableStateHandle creation.
    pub fn with_row_count(self, count: usize) -> Self {
        self.state.set_row_count(count);
        self
    }

    /// Update the row render function.
    /// Useful when the render function needs to capture different data than at creation.
    pub fn with_row_render_fn<F>(self, f: F) -> Self
    where
        F: Fn(usize, &AppContext) -> Vec<Box<dyn Element>> + 'static,
    {
        self.state.set_row_render_fn(f);
        self
    }

    pub fn with_config(mut self, config: TableConfig) -> Self {
        self.config = config;
        self
    }

    /// Get total row count (for scroll calculations).
    pub fn total_row_count(&self) -> usize {
        self.state.row_count()
    }

    fn column_count(&self) -> usize {
        self.headers.len()
    }

    /// Compute column widths based on TableColumnWidth specifications.
    /// `intrinsic_widths` should contain pre-measured widths for Intrinsic columns.
    /// If total intrinsic width exceeds available width, intrinsic columns are scaled down.
    fn compute_column_widths(&self, available_width: f32, intrinsic_widths: &[f32]) -> Vec<f32> {
        let column_count = self.column_count();
        if column_count == 0 {
            return Vec::new();
        }

        let mut widths = vec![0.0f32; column_count];
        let mut fixed_width = 0.0f32;
        let mut total_intrinsic = 0.0f32;
        let mut total_flex = 0.0f32;

        for (i, header) in self.headers.iter().enumerate() {
            match header.width {
                TableColumnWidth::Fixed(w) => {
                    widths[i] = w;
                    fixed_width += w;
                }
                TableColumnWidth::Fraction(f) => {
                    let w = available_width * f;
                    widths[i] = w;
                    fixed_width += w;
                }
                TableColumnWidth::Flex(flex) => {
                    total_flex += flex;
                }
                TableColumnWidth::Intrinsic => {
                    let w = intrinsic_widths.get(i).copied().unwrap_or(0.0);
                    total_intrinsic += w;
                }
            }
        }

        let available_for_intrinsic = (available_width - fixed_width).max(0.0);
        let intrinsic_scale = if total_intrinsic > available_for_intrinsic && total_intrinsic > 0.0
        {
            available_for_intrinsic / total_intrinsic
        } else {
            1.0
        };

        let mut remaining_width = available_width - fixed_width;
        for (i, header) in self.headers.iter().enumerate() {
            if let TableColumnWidth::Intrinsic = header.width {
                let w = intrinsic_widths.get(i).copied().unwrap_or(0.0) * intrinsic_scale;
                widths[i] = w;
                remaining_width -= w;
            }
        }

        remaining_width = remaining_width.max(0.0);
        if total_flex > 0.0 {
            for (i, header) in self.headers.iter().enumerate() {
                if let TableColumnWidth::Flex(flex) = header.width {
                    widths[i] = (remaining_width * flex) / total_flex;
                }
            }
        }

        widths
    }

    /// Measure the intrinsic width contribution from a header cell.
    fn measure_header_intrinsic_width(
        &mut self,
        col_idx: usize,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> f32 {
        let padding = self.config.cell_padding * 2.0;
        let mut max_width = 0.0f32;

        if col_idx < self.headers.len() {
            let header_content = std::mem::replace(
                &mut self.headers[col_idx].content,
                Box::new(super::Empty::new()),
            );
            let mut header_box = header_content;
            let unconstrained =
                SizeConstraint::new(vec2f(0.0, 0.0), vec2f(f32::INFINITY, f32::INFINITY));
            let size = header_box.layout(unconstrained, ctx, app);
            max_width = max_width.max(size.x() + padding);
            self.headers[col_idx].content = header_box;
        }

        max_width
    }

    /// Measure intrinsic width for a single column by laying out the header cell and, when
    /// configured, every body cell in the column.
    fn measure_column_intrinsic_width(
        &mut self,
        col_idx: usize,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> f32 {
        let padding = self.config.cell_padding * 2.0;
        let mut max_width = self.measure_header_intrinsic_width(col_idx, ctx, app);

        if self.config.measure_body_cells_for_intrinsic_widths {
            let render_fn = self.state.row_render_fn();
            let row_count = self.state.row_count();
            for row_idx in 0..row_count {
                let mut row_elements = render_fn(row_idx, app);
                let Some(cell) = row_elements.get_mut(col_idx) else {
                    continue;
                };
                let unconstrained =
                    SizeConstraint::new(vec2f(0.0, 0.0), vec2f(f32::INFINITY, f32::INFINITY));
                let size = cell.layout(unconstrained, ctx, app);
                max_width = max_width.max(size.x() + padding);
            }
        }

        max_width
    }

    /// Prepare intrinsic widths for all columns that have `Intrinsic` sizing.
    /// Returns the current widths plus the intrinsic columns whose widths should be cached after
    /// the caller finishes any additional measurement work.
    fn prepare_intrinsic_widths(
        &mut self,
        header_only_for_uncached_columns: bool,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> (Vec<f32>, Vec<usize>) {
        let column_count = self.column_count();
        let mut intrinsic_widths = vec![0.0f32; column_count];

        let intrinsic_indices: Vec<usize> = self
            .headers
            .iter()
            .enumerate()
            .filter_map(|(i, header)| {
                if matches!(header.width, TableColumnWidth::Intrinsic) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        if intrinsic_indices.is_empty() {
            return (intrinsic_widths, Vec::new());
        }

        let mut state = self.state.inner.borrow_mut();
        if state.intrinsic_column_widths.len() != column_count {
            state.intrinsic_column_widths = vec![None; column_count];
        }

        let mut uncached_intrinsic_indices = Vec::new();
        for &idx in &intrinsic_indices {
            if let Some(cached_width) = state.intrinsic_column_widths[idx] {
                intrinsic_widths[idx] = cached_width;
            } else {
                uncached_intrinsic_indices.push(idx);
            }
        }
        drop(state);

        for &idx in &uncached_intrinsic_indices {
            intrinsic_widths[idx] = if header_only_for_uncached_columns {
                self.measure_header_intrinsic_width(idx, ctx, app)
            } else {
                self.measure_column_intrinsic_width(idx, ctx, app)
            };
        }

        (intrinsic_widths, uncached_intrinsic_indices)
    }

    fn cache_intrinsic_widths(&self, intrinsic_indices: &[usize], intrinsic_widths: &[f32]) {
        if intrinsic_indices.is_empty() {
            return;
        }

        let mut state = self.state.inner.borrow_mut();
        for &idx in intrinsic_indices {
            if idx < intrinsic_widths.len() {
                state.intrinsic_column_widths[idx] = Some(intrinsic_widths[idx]);
            }
        }
    }

    fn measure_body_intrinsic_widths_from_rows(
        rows: &mut [Vec<Box<dyn Element>>],
        intrinsic_indices: &[usize],
        intrinsic_widths: &mut [f32],
        cell_padding: f32,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) {
        if intrinsic_indices.is_empty() {
            return;
        }

        let unconstrained =
            SizeConstraint::new(vec2f(0.0, 0.0), vec2f(f32::INFINITY, f32::INFINITY));
        let padding = cell_padding * 2.0;

        for row in rows {
            for &col_idx in intrinsic_indices {
                let Some(cell) = row.get_mut(col_idx) else {
                    continue;
                };
                let size = cell.layout(unconstrained, ctx, app);
                intrinsic_widths[col_idx] = intrinsic_widths[col_idx].max(size.x() + padding);
            }
        }
    }

    /// Compute cumulative column left positions from widths.
    fn compute_column_lefts(widths: &[f32]) -> Vec<f32> {
        let mut lefts = Vec::with_capacity(widths.len());
        let mut x = 0.0;
        for &w in widths {
            lefts.push(x);
            x += w;
        }
        lefts
    }

    /// Layout a single row and return its height.
    fn layout_row(
        cells: &mut [Box<dyn Element>],
        column_widths: &[f32],
        cell_padding: f32,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> f32 {
        let mut max_height = 0.0f32;
        let cell_count = cells.len().min(column_widths.len());

        for (i, cell) in cells.iter_mut().take(cell_count).enumerate() {
            let content_width = (column_widths[i] - cell_padding * 2.0).max(0.0);
            let cell_constraint = SizeConstraint::new(
                vec2f(content_width, 0.0),
                vec2f(content_width, f32::INFINITY),
            );
            let cell_size = cell.layout(cell_constraint, ctx, app);
            max_height = max_height.max(cell_size.y());
        }

        max_height
    }
}

impl Element for Table {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.children.clear();
        self.children_heights.clear();

        let column_count = self.column_count();
        if column_count == 0 {
            self.size = Some(Vector2F::zero());
            return Vector2F::zero();
        }

        let cell_padding = self.config.cell_padding;

        // Update viewport height
        let viewport_height = if constraint.max.y().is_finite() {
            constraint.max.y().into_pixels()
        } else {
            self.unconstrained_height.into_pixels()
        };

        // Get render function and row count
        let render_fn = self.state.row_render_fn();
        let effective_row_count = self.state.row_count();

        // Initialize or update sumtree if row count changed
        let mut state = self.state.inner.borrow_mut();
        let current_row_count = state.rows.summary().count;
        if current_row_count != effective_row_count {
            let mut new_tree = SumTree::new();
            for _ in 0..effective_row_count {
                new_tree.push(TableRowItem { height: None });
            }
            state.rows = new_tree;
            state.last_measured_row_index = 0;
        }
        state.viewport_height = viewport_height;
        drop(state);

        // Measure intrinsic columns (with caching). In full-content mode with body-aware intrinsic
        // sizing enabled, we only measure headers here and fold body-cell measurement into the
        // single full-content row render below.
        let uses_expand_to_content = matches!(
            self.config.vertical_sizing,
            TableVerticalSizing::ExpandToContent
        );
        let header_only_intrinsic_measurement =
            uses_expand_to_content && self.config.measure_body_cells_for_intrinsic_widths;
        let (mut intrinsic_widths, uncached_intrinsic_indices) =
            self.prepare_intrinsic_widths(header_only_intrinsic_measurement, ctx, app);
        if !header_only_intrinsic_measurement {
            self.cache_intrinsic_widths(&uncached_intrinsic_indices, &intrinsic_widths);
        }
        let total_intrinsic: f32 = intrinsic_widths.iter().sum();

        let mut state = self.state.inner.borrow_mut();
        if uses_expand_to_content {
            state.scroll_top = RowScrollOffset::default();
            drop(state);

            let mut all_row_elements = (0..effective_row_count)
                .map(|row_idx| render_fn(row_idx, app))
                .collect::<Vec<_>>();

            if header_only_intrinsic_measurement {
                Self::measure_body_intrinsic_widths_from_rows(
                    &mut all_row_elements,
                    &uncached_intrinsic_indices,
                    &mut intrinsic_widths,
                    cell_padding,
                    ctx,
                    app,
                );
                self.cache_intrinsic_widths(&uncached_intrinsic_indices, &intrinsic_widths);
            }

            let total_intrinsic: f32 = intrinsic_widths.iter().sum();
            let available_width = if constraint.max.x().is_finite() {
                constraint.max.x()
            } else {
                total_intrinsic.max(self.unconstrained_width)
            };
            let column_widths = self.compute_column_widths(available_width, &intrinsic_widths);

            let mut header_content_height = 0.0f32;
            for (i, header) in self.headers.iter_mut().enumerate() {
                let col_width = column_widths.get(i).copied().unwrap_or(0.0);
                let content_width = (col_width - cell_padding * 2.0).max(0.0);
                let cell_constraint = SizeConstraint::new(
                    vec2f(content_width, 0.0),
                    vec2f(content_width, f32::INFINITY),
                );
                let cell_size = header.content.layout(cell_constraint, ctx, app);
                header_content_height = header_content_height.max(cell_size.y());
            }
            let header_height = (header_content_height + cell_padding * 2.0).into_pixels();

            let mut rows_tree = SumTree::new();
            let mut rows_height = Pixels::zero();
            for mut row_elements in all_row_elements {
                let row_content_height =
                    Self::layout_row(&mut row_elements, &column_widths, cell_padding, ctx, app);
                let row_height = (row_content_height + cell_padding * 2.0).into_pixels();
                rows_tree.push(TableRowItem {
                    height: Some(row_height),
                });
                rows_height += row_height;
                self.children.push(row_elements);
                self.children_heights.push(row_height);
            }

            let total_width: f32 = column_widths.iter().sum();
            let content_height = header_height + rows_height;
            let mut state = self.state.inner.borrow_mut();
            state.header_height = header_height;
            state.rows = rows_tree;
            state.last_measured_row_index = effective_row_count.saturating_sub(1);
            state.column_widths = column_widths;
            state.visible_start_row_idx = 0;
            state.viewport_height = content_height;
            state.rendered_height = content_height;
            drop(state);

            let size = vec2f(total_width, content_height.as_f32());
            self.size = Some(size);
            return size;
        }

        // Compute column widths
        let available_width = if constraint.max.x().is_finite() {
            constraint.max.x()
        } else {
            total_intrinsic.max(self.unconstrained_width)
        };
        let column_widths = self.compute_column_widths(available_width, &intrinsic_widths);

        // Layout header cells directly
        let mut header_content_height = 0.0f32;
        for (i, header) in self.headers.iter_mut().enumerate() {
            let col_width = column_widths.get(i).copied().unwrap_or(0.0);
            let content_width = (col_width - cell_padding * 2.0).max(0.0);
            let cell_constraint = SizeConstraint::new(
                vec2f(content_width, 0.0),
                vec2f(content_width, f32::INFINITY),
            );
            let cell_size = header.content.layout(cell_constraint, ctx, app);
            header_content_height = header_content_height.max(cell_size.y());
        }
        let header_height = (header_content_height + cell_padding * 2.0).into_pixels();

        // Store header height in state
        let mut state = self.state.inner.borrow_mut();
        state.header_height = header_height;

        // Get current scroll position and ensure it's valid
        let scroll_top = state.scroll_top;
        let max_scroll = state.max_scroll_offset();
        if scroll_top > max_scroll {
            state.scroll_top = max_scroll;
        }
        let scroll_top = state.scroll_top;
        let last_measured = state.last_measured_row_index;
        drop(state);

        // Pre-scroll measurement: ensure all rows up to scroll position are measured
        if scroll_top.row_index.0 > last_measured {
            for row_idx in last_measured..scroll_top.row_index.0.min(effective_row_count) {
                let mut row_elements = render_fn(row_idx, app);

                let row_content_height =
                    Self::layout_row(&mut row_elements, &column_widths, cell_padding, ctx, app);
                let row_height = (row_content_height + cell_padding * 2.0).into_pixels();

                let mut state = self.state.inner.borrow_mut();
                let (new_tree, new_last_measured) = {
                    let mut cursor = state.rows.cursor::<RowCount, ()>();
                    let mut new_items = cursor.slice(&RowCount(row_idx), sum_tree::SeekBias::Right);
                    new_items.push(TableRowItem {
                        height: Some(row_height),
                    });
                    cursor.next();
                    new_items.push_tree(cursor.suffix());
                    let new_last_measured = new_items.summary().measured_count.saturating_sub(1);
                    (new_items, new_last_measured)
                };
                state.rows = new_tree;
                state.last_measured_row_index = new_last_measured;
            }
        }

        // Layout visible rows using sumtree seeking
        let mut rendered_height = Pixels::zero();

        let state = self.state.inner.borrow();
        let scroll_top = state.scroll_top;
        // Determine starting row index from cursor
        let start_row_idx = scroll_top.row_index.0;
        drop(state);

        let mut measured_items = Vec::new();

        // Calculate maximum rows we might need to render to fill the viewport.
        // We break early once rendered_height >= viewport_height.
        let max_possible_rows =
            (viewport_height.as_f32() / DEFAULT_ROW_HEIGHT_ESTIMATE).ceil() as usize + 1;
        let end_row_idx = (start_row_idx + max_possible_rows).min(effective_row_count);

        for row_idx in start_row_idx..end_row_idx {
            if rendered_height >= viewport_height {
                break;
            }

            let mut row_elements = render_fn(row_idx, app);

            let row_content_height =
                Self::layout_row(&mut row_elements, &column_widths, cell_padding, ctx, app);
            let row_height = (row_content_height + cell_padding * 2.0).into_pixels();

            self.children.push(row_elements);
            self.children_heights.push(row_height);
            measured_items.push(TableRowItem {
                height: Some(row_height),
            });

            if row_idx == start_row_idx {
                rendered_height += row_height - scroll_top.offset_from_start;
            } else {
                rendered_height += row_height;
            }
        }

        // Update sumtree with newly measured rows
        let measured_range = start_row_idx..(start_row_idx + measured_items.len());
        let mut state = self.state.inner.borrow_mut();
        let new_tree = {
            let mut cursor = state.rows.cursor::<RowCount, ()>();
            let mut new_items =
                cursor.slice(&RowCount(measured_range.start), sum_tree::SeekBias::Right);
            new_items.extend(measured_items);
            cursor.seek(&RowCount(measured_range.end), sum_tree::SeekBias::Right);
            new_items.push_tree(cursor.suffix());
            new_items
        };
        state.rows = new_tree;
        state.last_measured_row_index = state.rows.summary().measured_count.saturating_sub(1);

        // Store column info and visible range
        state.column_widths = column_widths.clone();
        state.visible_start_row_idx = start_row_idx;

        // Calculate actual content height (header + rendered rows)
        let mut rows_height = Pixels::zero();
        for h in &self.children_heights {
            rows_height += *h;
        }
        let content_height = header_height + rows_height;

        // Store rendered height for paint() to use
        // Use min(content, viewport) so table shrinks when content is smaller
        let rendered_height = if content_height < viewport_height {
            content_height
        } else {
            viewport_height
        };
        state.rendered_height = rendered_height;
        drop(state);

        let total_width: f32 = column_widths.iter().sum();
        // Return the rendered height (shrinks to content when content < viewport)
        let size = vec2f(total_width, rendered_height.as_f32());
        self.size = Some(size);

        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for header in &mut self.headers {
            header.content.after_layout(ctx, app);
        }
        for row_elements in &mut self.children {
            for cell in row_elements {
                cell.after_layout(ctx, app);
            }
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let state = self.state.inner.borrow();
        let column_widths = state.column_widths.clone();
        let header_height = state.header_height;
        let scroll_top = state.scroll_top;
        let rendered_height = state.rendered_height;
        let visible_start_row_idx = state.visible_start_row_idx;
        drop(state);

        // Compute column left positions from widths
        let column_lefts = Self::compute_column_lefts(&column_widths);

        if column_widths.is_empty() {
            return;
        }

        let total_width: f32 = column_widths.iter().sum();
        let scroll_offset = scroll_top.offset_from_start;
        let uses_fixed_header = self.config.fixed_header;
        let padding = self.config.cell_padding;
        let total_row_count = self.total_row_count();

        // Determine header and content origins based on scroll mode
        // Use rendered_height for clip bounds (shrinks to content when content < viewport)
        let (header_origin, content_origin, header_visible, body_clip_rect) = if uses_fixed_header {
            // Fixed header: header at origin, body below (clipped to exclude header area)
            let body_origin = origin + vec2f(0.0, header_height.as_f32());
            let body_height = rendered_height.as_f32() - header_height.as_f32();
            let body_clip = RectF::new(body_origin, vec2f(total_width, body_height));
            (origin, body_origin, true, body_clip)
        } else {
            // Scrolling header: clip excludes header area so borders paint on top
            let header_visible = scroll_top.row_index.0 == 0;
            let content_start_y = -scroll_offset.as_f32();
            let header_origin = origin + vec2f(0.0, content_start_y);
            let content_origin = if header_visible {
                origin + vec2f(0.0, content_start_y + header_height.as_f32())
            } else {
                origin + vec2f(0.0, content_start_y)
            };
            // Clip starts below header (when visible) so header border isn't covered by layer
            let clip_start_y = if uses_fixed_header && header_visible {
                header_height.as_f32()
            } else {
                0.0
            };
            let clip_origin = origin + vec2f(0.0, clip_start_y);
            let clip_height = rendered_height.as_f32() - clip_start_y;
            let clip = RectF::new(clip_origin, vec2f(total_width, clip_height));
            (header_origin, content_origin, header_visible, clip)
        };

        // Start clipping layer for body content
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(body_clip_rect));
        ctx.scene.set_active_layer_click_through();

        // In scrolling mode, paint header INSIDE the clip so it gets clipped at table bounds
        if !uses_fixed_header && header_visible {
            let header_rect = RectF::new(header_origin, vec2f(total_width, header_height.as_f32()));
            ctx.scene
                .draw_rect_with_hit_recording(header_rect)
                .with_background(self.config.header_background);

            for (col_idx, header) in self.headers.iter_mut().enumerate() {
                let col_left = column_lefts.get(col_idx).copied().unwrap_or(0.0);
                let cell_origin = header_origin + vec2f(col_left + padding, padding);
                header.content.paint(cell_origin, ctx, app);
            }

            if self.config.border_width > 0.0 && self.config.row_dividers {
                let divider_y = Self::snap_to_pixel(
                    header_origin.y() + header_height.as_f32() - self.config.border_width,
                );
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(origin.x(), divider_y),
                        vec2f(total_width, self.config.border_width),
                    ))
                    .with_background(self.config.border_color);
            }
        }

        // Paint rows
        let row_scroll_offset = if uses_fixed_header {
            scroll_offset
        } else {
            Pixels::zero()
        };
        let mut current_y = -row_scroll_offset.as_f32();

        for (child_idx, (row_elements, row_height)) in self
            .children
            .iter_mut()
            .zip(&self.children_heights)
            .enumerate()
        {
            let absolute_row_idx = visible_start_row_idx + child_idx;
            let row_height_f32 = row_height.as_f32();

            let bg_color = self.config.row_background.color_for_row(absolute_row_idx);
            let row_rect = RectF::new(
                content_origin + vec2f(0.0, current_y),
                vec2f(total_width, row_height_f32),
            );
            ctx.scene
                .draw_rect_with_hit_recording(row_rect)
                .with_background(bg_color);

            for (col_idx, cell) in row_elements.iter_mut().enumerate() {
                let col_left = column_lefts.get(col_idx).copied().unwrap_or(0.0);
                let cell_origin = content_origin + vec2f(col_left + padding, current_y + padding);
                cell.paint(cell_origin, ctx, app);
            }

            if self.config.border_width > 0.0
                && self.config.row_dividers
                && absolute_row_idx + 1 < total_row_count
            {
                let divider_y = Self::snap_to_pixel(
                    content_origin.y() + current_y + row_height_f32 - self.config.border_width,
                );
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(origin.x(), divider_y),
                        vec2f(total_width, self.config.border_width),
                    ))
                    .with_background(self.config.border_color);
            }

            current_y += row_height_f32;
        }

        // Stop body clip layer
        ctx.scene.stop_layer();

        // In fixed mode, paint header OUTSIDE the clip so it stays on top while body scrolls
        if uses_fixed_header && header_visible {
            let header_rect = RectF::new(header_origin, vec2f(total_width, header_height.as_f32()));
            ctx.scene
                .draw_rect_with_hit_recording(header_rect)
                .with_background(self.config.header_background);

            for (col_idx, header) in self.headers.iter_mut().enumerate() {
                let col_left = column_lefts.get(col_idx).copied().unwrap_or(0.0);
                let cell_origin = header_origin + vec2f(col_left + padding, padding);
                header.content.paint(cell_origin, ctx, app);
            }

            if self.config.border_width > 0.0 && self.config.row_dividers {
                let bw = self.config.border_width;
                let header_bottom_y =
                    Self::snap_to_pixel(header_origin.y() + header_height.as_f32() - bw);
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(origin.x(), header_bottom_y),
                        vec2f(total_width, bw),
                    ))
                    .with_background(self.config.border_color);
            }
        }

        // Draw borders using rendered_height (shrinks to content when content < viewport)
        if self.config.border_width > 0.0 {
            let bw = self.config.border_width;
            let border_height = rendered_height.as_f32();
            let border_color = self.config.border_color;
            let top_y = Self::snap_to_pixel(origin.y());
            let bottom_y = Self::snap_to_pixel(origin.y() + border_height - bw);
            let left_x = Self::snap_to_pixel(origin.x());
            let right_x = Self::snap_to_pixel(origin.x() + total_width - bw);
            let inner_top_y = top_y + bw;
            let inner_height = (bottom_y - inner_top_y).max(0.0);

            if self.config.column_dividers {
                for &col_left in column_lefts.iter().skip(1) {
                    let divider_x = Self::snap_to_pixel(origin.x() + col_left);
                    let (divider_top_y, divider_height) = if self.config.outer_border {
                        (inner_top_y, inner_height)
                    } else {
                        (top_y, border_height)
                    };
                    let line_rect =
                        RectF::new(vec2f(divider_x, divider_top_y), vec2f(bw, divider_height));
                    ctx.scene
                        .draw_rect_with_hit_recording(line_rect)
                        .with_background(border_color);
                }
            }

            if self.config.outer_border {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(left_x, top_y),
                        vec2f(total_width, bw),
                    ))
                    .with_background(border_color);
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(left_x, bottom_y),
                        vec2f(total_width, bw),
                    ))
                    .with_background(border_color);
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(left_x, top_y),
                        vec2f(bw, border_height),
                    ))
                    .with_background(border_color);
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(right_x, top_y),
                        vec2f(bw, border_height),
                    ))
                    .with_background(border_color);
            }
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let mut handled = false;
        for header in &mut self.headers {
            handled |= header.content.dispatch_event(event, ctx, app);
        }
        for row_elements in &mut self.children {
            for cell in row_elements {
                handled |= cell.dispatch_event(event, ctx, app);
            }
        }
        handled
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }
}

impl SelectableElement for Table {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let mut selection_fragments: Vec<SelectionFragment> = Vec::new();
        let mut had_prior_row = false;

        let mut header_fragments: Vec<SelectionFragment> = Vec::new();
        for header in &self.headers {
            if let Some(selectable_child) = header.content.as_selectable_element() {
                if let Some(child_fragments) =
                    selectable_child.get_selection(selection_start, selection_end, is_rect)
                {
                    if !header_fragments.is_empty() {
                        if let Some(last_fragment) = header_fragments.last() {
                            header_fragments.push(SelectionFragment {
                                text: "\t".to_string(),
                                origin: last_fragment.origin,
                            });
                        }
                    }
                    header_fragments.extend(child_fragments);
                }
            }
        }

        if !header_fragments.is_empty() {
            selection_fragments.extend(header_fragments);
            had_prior_row = true;
        }

        // VIRTUALIZATION SELECTION LIMITATION:
        // When a table is virtualized, only the rendered rows can be selected.
        // Non-visible rows are not laid out or painted, so their content cannot be
        // included in the selection. This is an expected limitation.
        //
        // To maintain logical consistency, we insert placeholder newlines for non-visible
        // rows between the header and first visible row. This preserves row structure in
        // the selection text, but the actual cell content of non-visible rows is unavailable.
        let state = self.state.inner.borrow();
        let visible_start_row_idx = state.visible_start_row_idx;
        drop(state);

        if had_prior_row && !self.children.is_empty() && visible_start_row_idx > 0 {
            for _ in 0..visible_start_row_idx {
                if let Some(last_fragment) = selection_fragments.last() {
                    selection_fragments.push(SelectionFragment {
                        text: "\n".to_string(),
                        origin: last_fragment.origin,
                    });
                }
            }
        }

        for row_elements in &self.children {
            let mut row_fragments: Vec<SelectionFragment> = Vec::new();
            for cell in row_elements {
                if let Some(selectable_child) = cell.as_selectable_element() {
                    if let Some(child_fragments) =
                        selectable_child.get_selection(selection_start, selection_end, is_rect)
                    {
                        if !row_fragments.is_empty() {
                            if let Some(last_fragment) = row_fragments.last() {
                                row_fragments.push(SelectionFragment {
                                    text: "\t".to_string(),
                                    origin: last_fragment.origin,
                                });
                            }
                        }
                        row_fragments.extend(child_fragments);
                    }
                }
            }
            if !row_fragments.is_empty() {
                if had_prior_row {
                    if let Some(last_fragment) = selection_fragments.last() {
                        selection_fragments.push(SelectionFragment {
                            text: "\n".to_string(),
                            origin: last_fragment.origin,
                        });
                    }
                }
                selection_fragments.extend(row_fragments);
                had_prior_row = true;
            }
        }

        if !selection_fragments.is_empty() {
            Some(selection_fragments)
        } else {
            None
        }
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        let mut expanded_selection = None;

        for header in &self.headers {
            if let Some(selectable_child) = header.content.as_selectable_element() {
                if let Some(selection) = selectable_child.expand_selection(
                    point,
                    direction,
                    unit,
                    word_boundaries_policy,
                ) {
                    match direction {
                        SelectionDirection::Backward => return Some(selection),
                        SelectionDirection::Forward => {
                            expanded_selection = Some(selection);
                        }
                    }
                }
            }
        }

        for row_elements in &self.children {
            for cell in row_elements {
                if let Some(selectable_child) = cell.as_selectable_element() {
                    if let Some(selection) = selectable_child.expand_selection(
                        point,
                        direction,
                        unit,
                        word_boundaries_policy,
                    ) {
                        match direction {
                            SelectionDirection::Backward => return Some(selection),
                            SelectionDirection::Forward => {
                                expanded_selection = Some(selection);
                            }
                        }
                    }
                }
            }
        }

        expanded_selection
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        for header in &self.headers {
            if let Some(selectable_child) = header.content.as_selectable_element() {
                if let Some(result) = selectable_child
                    .is_point_semantically_before(absolute_point, absolute_point_other)
                {
                    return Some(result);
                }
            }
        }

        for row_elements in &self.children {
            for cell in row_elements {
                if let Some(selectable_child) = cell.as_selectable_element() {
                    if let Some(result) = selectable_child
                        .is_point_semantically_before(absolute_point, absolute_point_other)
                    {
                        return Some(result);
                    }
                }
            }
        }

        None
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        for header in &self.headers {
            if let Some(selectable_child) = header.content.as_selectable_element() {
                if let Some(selection) =
                    selectable_child.smart_select(absolute_point, smart_select_fn)
                {
                    return Some(selection);
                }
            }
        }

        for row_elements in &self.children {
            for cell in row_elements {
                if let Some(selectable_child) = cell.as_selectable_element() {
                    if let Some(selection) =
                        selectable_child.smart_select(absolute_point, smart_select_fn)
                    {
                        return Some(selection);
                    }
                }
            }
        }

        None
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        let mut clickable_bounds = Vec::new();

        for header in &self.headers {
            if let Some(selectable_child) = header.content.as_selectable_element() {
                clickable_bounds
                    .append(&mut selectable_child.calculate_clickable_bounds(current_selection));
            }
        }

        for row_elements in &self.children {
            for cell in row_elements {
                if let Some(selectable_child) = cell.as_selectable_element() {
                    clickable_bounds.append(
                        &mut selectable_child.calculate_clickable_bounds(current_selection),
                    );
                }
            }
        }

        clickable_bounds
    }
}

impl ScrollableElement for Table {
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        if matches!(
            self.config.vertical_sizing,
            TableVerticalSizing::ExpandToContent
        ) {
            return None;
        }
        self.vertical_scroll_data()
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        if matches!(
            self.config.vertical_sizing,
            TableVerticalSizing::ExpandToContent
        ) {
            return;
        }
        self.scroll_vertically(delta, ctx);
    }

    fn should_handle_scroll_wheel(&self) -> bool {
        !matches!(
            self.config.vertical_sizing,
            TableVerticalSizing::ExpandToContent
        )
    }
}

impl Table {
    fn snap_to_pixel(value: f32) -> f32 {
        value.round()
    }
    fn scroll_vertically(&mut self, delta: Pixels, ctx: &mut EventContext) {
        let mut state = self.state.inner.borrow_mut();

        let current_scroll_top = state.scroll_top_pixels();
        let viewport_height = state.viewport_height;
        let approximate_height = state.approximate_height();
        let scroll_max = (approximate_height - viewport_height).max(Pixels::zero());
        let new_scroll_top = (current_scroll_top - delta)
            .max(Pixels::zero())
            .min(scroll_max);

        state.scroll_top = state.absolute_pixels_to_scroll_offset(new_scroll_top);
        drop(state);

        ctx.notify();
    }

    fn vertical_scroll_data(&self) -> Option<ScrollData> {
        let state = self.state.inner.borrow();
        let viewport_height = state.viewport_height;
        let scroll_start = state.scroll_top_pixels();
        let total_size = state.approximate_height();
        drop(state);

        Some(ScrollData {
            scroll_start,
            visible_px: viewport_height,
            total_size,
        })
    }
}

impl Table {
    #[cfg(test)]
    pub(crate) fn visible_row_count(&self) -> usize {
        self.children.len()
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
