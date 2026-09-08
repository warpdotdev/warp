use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use super::event_listener::ChannelEventListener;
use super::model::block::BlockSize;
use super::safe_mode_settings::get_secret_obfuscation_mode;
use super::session_settings::SessionSettings;
use super::settings::TerminalSettings;
use super::view::{WARP_PROMPT_HEIGHT_LINES, create_size_info_for_blocklist};
use super::{BlockPadding, ShellLaunchState, SizeInfo, TerminalModel, color};
use crate::PrivacySettings;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::ai::blocklist::telemetry_banner::should_collect_ai_ugc_telemetry;
use crate::appearance::Appearance;
use crate::pane_group::pane::DetachType;
use crate::settings::{BlockVisibilitySettings, DebugSettings, InputModeSettings};

pub trait TerminalManager: Any {
    /// Returns the backing terminal model.
    fn model(&self) -> Arc<FairMutex<TerminalModel>>;

    /// Called when the terminal pane detaches from its pane group. This is a sensitive path -
    /// do not do anything with high latency here. Note that we cannot rely on events emitted
    /// here to be processed before the window closes.
    ///
    /// Implementations should preserve state on [`DetachType::HiddenForClose`] or
    /// [`DetachType::Moved`] and clean up only on [`DetachType::Closed`].
    fn on_view_detached(&self, _detach_type: DetachType, _app: &mut AppContext) {}

    /// Returns this [`TerminalManager`] as an [`Any`], to support downcasting.
    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl warpui::Entity for Box<dyn TerminalManager> {
    type Event = ();
}

/// Spacing baked into block heights: the per-block padding, the height
/// reserved for the rendered Warp prompt (in lines), and whether blocks
/// reserve a footer row for the debug memory-stats overlay.
///
/// [`Self::for_gui`] derives the GUI blocklist's spacing from the user's
/// settings; frontends whose rendering differs (e.g. the row-based TUI
/// transcript) define their own spacing and pass it when creating the
/// terminal model.
pub struct BlockSpacing {
    pub block_padding: BlockPadding,
    pub warp_prompt_height_lines: f32,
    pub show_memory_stats: bool,
}

impl BlockSpacing {
    /// The GUI blocklist's spacing, derived from the user's settings: padding
    /// from the terminal-spacing style, the rendered Warp prompt's height, and
    /// the debug memory-stats footer toggle.
    pub(super) fn for_gui(ctx: &AppContext) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let terminal_spacing =
            TerminalSettings::as_ref(ctx).terminal_spacing(appearance.line_height_ratio(), ctx);
        Self {
            block_padding: terminal_spacing.block_padding,
            warp_prompt_height_lines: WARP_PROMPT_HEIGHT_LINES,
            show_memory_stats: DebugSettings::as_ref(ctx).should_show_memory_stats(),
        }
    }
}

pub(super) fn compute_block_size(
    initial_size: Vector2F,
    block_spacing: &BlockSpacing,
    ctx: &mut AppContext,
) -> BlockSize {
    let appearance = Appearance::as_ref(ctx);
    let size_info = if ctx.is_headless() {
        // In headless mode, we don't actually have a font since we aren't rendering anything.
        // We skip the font-based size computation and hardcode a terminal size, so that
        // viewers of the shared session see a reasonable terminal width.
        SizeInfo::new_without_font_metrics(24, 120)
    } else {
        let font_cache = ctx.font_cache();
        create_size_info_for_blocklist(
            initial_size,
            font_cache,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
            appearance.ui_builder().line_height_ratio(),
        )
    };
    let maximum_grid_size = *TerminalSettings::as_ref(ctx).maximum_grid_size.value();
    BlockSize {
        block_padding: block_spacing.block_padding,
        size: size_info,
        max_block_scroll_limit: maximum_grid_size,
        warp_prompt_height_lines: block_spacing.warp_prompt_height_lines,
    }
}

/// Creates a [`TerminalModel`], the source of truth for the session's state.
///
/// `block_spacing` is the frontend's spacing baked into block heights;
/// the GUI's settings-driven frontends derive it via [`BlockSpacing::for_gui`].
#[allow(clippy::too_many_arguments)]
pub(super) fn create_terminal_model(
    startup_directory: Option<PathBuf>,
    restored_blocks: Option<&Vec<SerializedBlockListItem>>,
    initial_size: Vector2F,
    channel_event_proxy: ChannelEventListener,
    shell_state: ShellLaunchState,
    block_spacing: BlockSpacing,
    ctx: &mut AppContext,
) -> TerminalModel {
    let (should_show_bootstrap_block, should_show_in_band_command_blocks) = {
        let settings = BlockVisibilitySettings::as_ref(ctx);
        (
            *settings.should_show_bootstrap_block.value(),
            *settings.should_show_in_band_command_blocks.value(),
        )
    };
    let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
    let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
    let is_inverted = input_mode.is_inverted_blocklist();

    let sizes = compute_block_size(initial_size, &block_spacing, ctx);

    let obfuscate_secrets = get_secret_obfuscation_mode(ctx);
    let is_ai_ugc_telemetry_enabled =
        should_collect_ai_ugc_telemetry(ctx, PrivacySettings::as_ref(ctx).is_telemetry_enabled);

    TerminalModel::new(
        restored_blocks.map(|v| v.as_slice()),
        sizes,
        terminal_colors_list(ctx),
        channel_event_proxy,
        ctx.background_executor().clone(),
        should_show_bootstrap_block,
        should_show_in_band_command_blocks,
        block_spacing.show_memory_stats,
        honor_ps1,
        is_inverted,
        obfuscate_secrets,
        is_ai_ugc_telemetry_enabled,
        startup_directory,
        shell_state,
    )
}

pub(super) fn terminal_colors_list(ctx: &AppContext) -> color::List {
    let appearance = Appearance::as_ref(ctx);
    let theme = appearance.theme();
    color::List::from(&theme.to_owned().into())
}
