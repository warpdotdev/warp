use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use settings::Setting as _;
use std::{any::Any, path::PathBuf, sync::Arc};
use warpui::{AppContext, SingletonEntity, ViewHandle};

use crate::PrivacySettings;
use crate::{
    ai::blocklist::{telemetry_banner::should_collect_ai_ugc_telemetry, SerializedBlockListItem},
    appearance::Appearance,
    settings::{BlockVisibilitySettings, DebugSettings, InputModeSettings},
};

use super::{
    color,
    event_listener::ChannelEventListener,
    model::block::BlockSize,
    safe_mode_settings::get_secret_obfuscation_mode,
    session_settings::SessionSettings,
    settings::TerminalSettings,
    view::{create_size_info_for_blocklist, WARP_PROMPT_HEIGHT_LINES},
    ShellLaunchState, SizeInfo, TerminalModel, TerminalView,
};
use crate::pane_group::pane::DetachType;

pub trait TerminalManager: Any {
    /// Returns the backing terminal model.
    fn model(&self) -> Arc<FairMutex<TerminalModel>>;

    /// Returns the terminal view being managed.
    fn view(&self) -> ViewHandle<TerminalView>;

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

pub(super) fn compute_block_size(initial_size: Vector2F, ctx: &mut AppContext) -> BlockSize {
    let appearance = Appearance::as_ref(ctx);
    let terminal_spacing =
        TerminalSettings::as_ref(ctx).terminal_spacing(appearance.line_height_ratio(), ctx);
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
        block_padding: terminal_spacing.block_padding,
        size: size_info,
        max_block_scroll_limit: maximum_grid_size,
        warp_prompt_height_lines: WARP_PROMPT_HEIGHT_LINES,
    }
}

/// Creates a [`TerminalModel`], the source of truth for the session's state.
#[allow(clippy::too_many_arguments)]
pub(super) fn create_terminal_model(
    startup_directory: Option<PathBuf>,
    restored_blocks: Option<&Vec<SerializedBlockListItem>>,
    initial_size: Vector2F,
    channel_event_proxy: ChannelEventListener,
    shell_state: ShellLaunchState,
    ctx: &mut AppContext,
) -> TerminalModel {
    let (should_show_bootstrap_block, should_show_in_band_command_blocks) = {
        let settings = BlockVisibilitySettings::as_ref(ctx);
        (
            *settings.should_show_bootstrap_block.value(),
            *settings.should_show_in_band_command_blocks.value(),
        )
    };
    let show_memory_stats = DebugSettings::as_ref(ctx).should_show_memory_stats();
    let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
    let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
    let is_inverted = input_mode.is_inverted_blocklist();

    let sizes = compute_block_size(initial_size, ctx);

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
        show_memory_stats,
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
