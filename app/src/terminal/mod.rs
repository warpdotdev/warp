use std::cmp::max;

pub use block_list_element::GridType;
use model::alt_screen::AltScreen;
use model::blocks::BlockList;
use model::index::Side;
pub use model::terminal_model::TerminalModel;
use ordered_float::Float;
use pathfinder_geometry::vector::vec2f;
use serde::{Deserialize, Serialize};
mod package_installers;
pub(crate) use history::UpArrowHistoryConfig;
pub use history::{History, HistoryEntry, HistoryEvent, ShellHost};
pub use view::{Event, TerminalView};
pub use warp_terminal::shell::{self, ShellLaunchData};
use warpui::geometry::vector::Vector2F;
use warpui::units::{IntoPixels, Lines, Pixels};
use warpui::{AppContext, WindowId};
mod block_list_settings;

mod alias;
mod alt_screen;
pub mod alt_screen_reporting;
mod audible_bell;
pub use audible_bell::AudibleBell;
pub mod available_shells;

mod block_filter;
pub mod block_list_element;
pub mod block_list_viewport;
pub mod blockgrid_element;
mod blockgrid_renderer;
mod bootstrap;
mod buy_credits_banner;
pub mod color;
mod command_corrections_denylist;
pub mod dynamic_enum_suggestions;
pub mod enable_auto_reload_modal;
pub mod event;
pub mod event_listener;
pub mod find;
pub(crate) mod focus_env;
pub mod general_settings;
pub mod grid_renderer;
pub mod grid_size_util;
pub mod history;
pub mod input;
pub mod keys;
pub mod keys_settings;
pub mod ligature_settings;
mod line_editor_status;
pub mod links;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
pub mod local_shell;
#[cfg(feature = "local_tty")]
pub mod local_tty;
mod meta_shortcuts;
pub mod mock_terminal_manager;
pub mod model;
pub mod model_events;
pub mod platform;
pub mod profile_model_selector;
pub mod prompt;
pub mod prompt_render_helper;
pub mod recorder;
pub mod remote_tty;
pub mod resizable_data;
pub mod rich_history;
pub mod safe_mode_settings;
mod secret_regex_updater;
pub mod session_settings;
pub mod settings;
mod share_block_modal;
pub mod shared_session;
mod shell_launch_state;
pub mod universal_developer_input;

pub mod ssh;
pub mod terminal_manager;
mod terminal_size_element;
pub mod view;
pub mod warpify;
mod waterfall_gap_element;
mod writeable_pty;
#[cfg(feature = "tui")]
pub use writeable_pty::{PtyIntent, PtyIntentEvent, TerminalSurface};
#[cfg(windows)]
pub mod wsl;

pub mod cli_agent;
pub use cli_agent::CLIAgent;
pub(crate) mod cli_agent_sessions;

pub use block_list_settings::*;
pub use mock_terminal_manager::MockTerminalManager;
use model_events::{ModelEvent, ModelEventDispatcher};
pub use secret_regex_updater::CustomSecretRegexUpdater;
pub use share_block_modal::{ShareBlockModal, ShareBlockModalEvent, ShareBlockType};
pub use shell_launch_state::ShellLaunchState;
pub use terminal_manager::TerminalManager;
pub use view::{
    CANCEL_COMMAND_KEYBINDING, TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
    TOGGLE_HIDE_CLI_RESPONSES_KEYBINDING, TOGGLE_QUEUE_NEXT_PROMPT_KEYBINDING,
};

/// Minimum number of visible lines.
const MIN_ROWS: usize = 1;

/// Minimum number of columns.
///
/// A minimum of 2 is necessary to hold fullwidth unicode characters.
const MIN_COLUMNS: usize = 2;

/// The broadcast channel capacity for PTY reads.
/// This constant was picked arbitrarily. We really shouldn't
/// fall more than this many reads behind the PTY itself anyways.
/// We also don't want to make this too large because we
/// have to pay the cost of pre-allocating memory for the channel
/// (and the larger this is, the more memory we eagerly allocate).
/// TODO: investigate if we can reduce the number of PTY reads we need to buffer
/// per event loop run.
pub const PTY_READS_BROADCAST_CHANNEL_SIZE: usize = 1024;

pub fn init(app: &mut AppContext) {
    share_block_modal::init(app);
    view::init(app);
}

/// Treat rounding errors for heights within this amount as equal.
pub const HEIGHT_FUDGE_FACTOR_LINES: Lines = Lines::new(0.01);

/// Returns whether two heights in lines are approximately equal.
/// This is an annoying cludge to handle the fact that we're using floating point
/// throughout our block heights code and have to deal with the consequences of accumulated
/// rounding errors.
pub fn heights_approx_eq(a: Lines, b: Lines) -> bool {
    (a - b).abs() < HEIGHT_FUDGE_FACTOR_LINES
}

/// Returns whether height a is greater than or equal to height b, allowing
/// for a bit of fudging to account for accumulated rounding errors.
pub fn heights_approx_gte(a: Lines, b: Lines) -> bool {
    a > b || heights_approx_eq(a, b)
}

/// Returns whether height a is greater than height b, allowing for a bit of fudging to account
/// for accumulated rounding errors.
pub fn heights_approx_gt(a: Lines, b: Lines) -> bool {
    a > b && !heights_approx_eq(a, b)
}

/// Returns whether height a is less than or equal to height b, allowing
/// for a bit of fudging to account for accumulated rounding errors.
pub fn heights_approx_lte(a: Lines, b: Lines) -> bool
where
{
    a < b || heights_approx_eq(a, b)
}

/// Returns whether height a is less than height b, allowing for a bit of fudging to account
/// for accumulated rounding errors.
pub fn heights_approx_lt(a: Lines, b: Lines) -> bool {
    a < b && !heights_approx_eq(a, b)
}

/// Returns whether the given height is between the start and end heights,
/// allowing for a bit of fudging to account for accumulated rounding errors.
pub fn height_in_range_approx(height: Lines, start: Lines, end: Lines) -> bool {
    heights_approx_gte(height, start) && heights_approx_lte(height, end)
}

/// Returns the size of the `SavePosition`-ed element with the given ID from the last layout cycle.
///
/// If this is the first app layout, if if there was no laid-out element with the given ID, returns
/// `None`.
pub(crate) fn element_size_at_last_frame(
    element_position_id: &str,
    window_id: WindowId,
    app: &AppContext,
) -> Option<Vector2F> {
    app.element_position_by_id_at_last_frame(window_id, element_position_id)
        .map(|position| position.size())
}

/// The reason that the terminal size is being updated
#[derive(Debug, Copy, Clone)]
pub enum SizeUpdateReason {
    /// Updated because of some general refresh (e.g. a font-size change)
    Refresh,

    /// Updated after the temrinal has been laid out, so some of the element
    /// sizes that drive terminal size may have changed.
    AfterLayout,

    /// The shared session sharer's size changed.
    /// This is only applicable for shared session viewers.
    ///
    /// The resultant [`SizeUpdate`] will use the larger of the
    /// sharer's and viewer's size.
    SharerSizeChanged { num_rows: usize, num_cols: usize },

    /// A viewer reported its terminal size to the sharer.
    /// This is only applicable for shared session sharers.
    ///
    /// The resultant [`SizeUpdate`] will use the viewer's reported
    /// size directly (floored at 1 row and 1 column).
    ViewerSizeReported { num_rows: usize, num_cols: usize },
}

/// Encapsulates info for updating the size of the terminal.
#[derive(Debug, Copy, Clone)]
pub struct SizeUpdate {
    /// The reason for the update.
    update_reason: SizeUpdateReason,

    /// The last size info.
    last_size: SizeInfo,

    /// The new size info.
    new_size: SizeInfo,

    /// The new gap height, if there is one.
    new_gap_height: Option<Lines>,

    /// The pane-computed rows before any shared session size adjustments.
    natural_rows: usize,

    /// The pane-computed columns before any shared session size adjustments.
    natural_cols: usize,
}

impl SizeUpdate {
    /// Builds a host-terminal-driven resize for headless (TUI) frontends,
    /// where one cell maps to one "pixel" and there is no window padding
    /// (mirroring [`SizeInfo::new_without_font_metrics`]). Used to keep the
    /// PTY's winsize in sync with the host terminal while an alt-screen app
    /// is running.
    pub fn for_tui_host_resize(last_size: SizeInfo, rows: usize, cols: usize) -> Self {
        Self {
            update_reason: SizeUpdateReason::Refresh,
            last_size,
            new_size: SizeInfo::new_without_font_metrics(rows, cols),
            new_gap_height: None,
            natural_rows: rows,
            natural_cols: cols,
        }
    }

    /// Whether the reason for the update is a refresh.
    pub fn is_refresh(&self) -> bool {
        matches!(self.update_reason, SizeUpdateReason::Refresh)
    }

    /// Returns whether there was any change with this update.
    pub fn anything_changed(&self) -> bool {
        self.pane_size_changed() || self.gap_height_changed() || self.rows_or_columns_changed()
    }

    pub fn rows_or_columns_changed(&self) -> bool {
        self.last_size.columns() != self.new_size.columns()
            || self.last_size.rows() != self.new_size.rows()
    }

    /// Returns whether the pane size changed with this update
    pub fn pane_size_changed(&self) -> bool {
        // It's fine for this to be a near-exact comparison because pane size
        // is not determined by summing floats like content element size is.
        (self.last_size.pane_size_px().x() - self.new_size.pane_size_px().x()).abs() > f32::EPSILON
            || (self.last_size.pane_size_px().y() - self.new_size.pane_size_px().y()).abs()
                > f32::EPSILON
    }

    /// Returns any new gap height to set with this update
    pub fn new_gap_height(&self) -> Option<Lines> {
        self.new_gap_height
    }

    /// Returns whether the gap height changed with this update
    pub fn gap_height_changed(&self) -> bool {
        self.new_gap_height.is_some()
    }

    /// The pane-computed natural rows before shared session adjustments.
    pub fn natural_rows(&self) -> usize {
        self.natural_rows
    }

    /// The pane-computed natural columns before shared session adjustments.
    pub fn natural_cols(&self) -> usize {
        self.natural_cols
    }

    /// Returns true if this resize was caused by a sharer size change.
    pub fn is_sharer_size_change(&self) -> bool {
        matches!(
            self.update_reason,
            SizeUpdateReason::SharerSizeChanged { .. }
        )
    }
}

/// Terminal size info.
///
/// Note that this implements Serialize/Deserialize for ref tests.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeInfo {
    /// The width of the TerminalView pane.
    ///
    /// This is basically the `x` of the the incoming max size constraint on the TerminalView. This
    /// is represented as a raw float rather than a Vector2F to satisfy the
    /// `Serialize`/`Deserialize` trait requirements.
    pane_width_px: f32,

    /// The height of the TerminalView pane.
    ///
    /// This is basically the `y` of the the incoming max size constraint on the TerminalView. This
    /// is represented as a raw float rather than a Vector2F to satisfy the
    /// `Serialize`/`Deserialize` trait requirements.
    pane_height_px: f32,

    /// This height in rows that the pty and grid model thinks the terminal is.
    ///
    /// Note that *rows* is always determined as a function of pane size, not
    /// the content element size, which is somewhat counterintuitive.  The reason
    /// is that the content element size changes frequenetly as the input size
    /// changes or the input dissapears for long running commands, but many
    /// programs do not handle size changes while they are running very well.  To
    /// get around this we make them think that rows always comes from the pane size.
    rows: usize,

    /// This is width in columns that the pty and grid model thinks the terminal is.
    columns: usize,

    /// Width of an individual cell.
    cell_width_px: Pixels,

    /// Height of an individual cell.
    cell_height_px: Pixels,

    /// Horizontal window padding.
    padding_x_px: Pixels,

    /// Vertical window padding.
    padding_y_px: Pixels,
}

/// Helper struct containing the cell size and window padding info.
pub struct CellSizeAndWindowPadding {
    /// Width of an individual cell.
    pub cell_width_px: Pixels,

    /// Height of an individual cell.
    pub cell_height_px: Pixels,

    /// Horizontal window padding.
    pub padding_x_px: Pixels,

    /// Vertical window padding.
    pub padding_y_px: Pixels,
}

impl SizeInfo {
    pub fn new(
        pane_size_px: Vector2F,
        cell_width_px: Pixels,
        cell_height_px: Pixels,
        padding_x_px: Pixels,
        padding_y_px: Pixels,
    ) -> SizeInfo {
        let rows = (pane_size_px.y() - 2. * padding_y_px.as_f32()) / cell_height_px.as_f32();
        let columns = (pane_size_px.x() - 2. * padding_x_px.as_f32()) / cell_width_px.as_f32();

        SizeInfo {
            pane_width_px: pane_size_px.x(),
            pane_height_px: pane_size_px.y(),
            columns: max(columns as usize, MIN_COLUMNS),
            rows: max(rows as usize, MIN_ROWS),
            cell_width_px,
            cell_height_px,
            padding_x_px: padding_x_px.floor(),
            padding_y_px: padding_y_px.floor(),
        }
    }

    /// Create SizeInfo for a [`TerminalModel`] instance that doesn't have font metrics,
    /// which comes from either a headless Warp instance or tests.
    pub fn new_without_font_metrics(rows: usize, cols: usize) -> Self {
        let width = cols as f32;
        let height = rows as f32;
        SizeInfo::new(
            vec2f(width, height),
            1.0.into_pixels(), /* cell_width */
            1.0.into_pixels(), /* cell_height */
            Pixels::zero(),    /* padding_x */
            Pixels::zero(),    /* padding_y */
        )
    }

    pub fn with_rows_and_columns(mut self, rows: usize, cols: usize) -> SizeInfo {
        self.columns = cols;
        self.rows = rows;
        self
    }

    /// Returns the side of the cell where the mouse is located.
    pub fn get_mouse_side(&self, position: Vector2F) -> Side {
        let x = position.x() as usize;

        let cell_x = x.saturating_sub(self.padding_x_px().as_f32() as usize)
            % self.cell_width_px().as_f32() as usize;
        let half_cell_width = (self.cell_width_px().as_f32() / 2.0) as usize;

        let additional_padding = (self.pane_width_px().as_f32()
            - self.padding_x_px().as_f32() * 2.)
            % self.cell_width_px().as_f32();
        let end_of_grid =
            self.pane_width_px() - self.padding_x_px() - additional_padding.into_pixels();

        if cell_x > half_cell_width
            // Edge case when mouse leaves the window.
            || x as f32 >= end_of_grid.as_f32()
        {
            Side::Right
        } else {
            Side::Left
        }
    }

    #[inline]
    pub fn pane_size_px(&self) -> Vector2F {
        vec2f(self.pane_width_px, self.pane_height_px)
    }

    #[inline]
    pub fn pane_width_px(&self) -> Pixels {
        self.pane_width_px.into_pixels()
    }

    #[inline]
    pub fn pane_height_px(&self) -> Pixels {
        self.pane_height_px.into_pixels()
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[inline]
    pub fn columns(&self) -> usize {
        self.columns
    }

    #[inline]
    pub fn cell_width_px(&self) -> Pixels {
        self.cell_width_px
    }

    #[inline]
    pub fn cell_height_px(&self) -> Pixels {
        self.cell_height_px
    }

    #[inline]
    pub fn padding_x_px(&self) -> Pixels {
        self.padding_x_px
    }

    #[inline]
    pub fn padding_y_px(&self) -> Pixels {
        self.padding_y_px
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ClipboardType {
    Clipboard,
    Selection,
}

/// The padding around each block, represented in fractional lines.
///
/// TODO(vorporeal): Change this to hold `Lines` instead of `f32`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BlockPadding {
    /// Padding from top of the block to the prompt.
    pub padding_top: f32,
    /// Padding from bottom of prompt to top of the command.
    pub command_padding_top: f32,
    /// Padding from bottom of command to top of output.
    pub middle: f32,
    /// Padding from bottom of output to bottom of block.
    pub bottom: f32,
}

impl BlockPadding {
    pub fn new(
        padding_top: f32,
        command_padding_top: f32,
        padding_middle: f32,
        padding_bottom: f32,
    ) -> Self {
        BlockPadding {
            padding_top,
            command_padding_top,
            middle: padding_middle,
            bottom: padding_bottom,
        }
    }
}

#[cfg(test)]
mod ref_tests;
