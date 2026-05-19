use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use super::{
    alt_screen_reporting::AltScreenReporting, model::grid::grid_handler::TermMode, TerminalModel,
};
use warp_core::features::FeatureFlag;

pub mod alt_screen_element;

const SMART_ALT_SCREEN_MOUSE_TMUX_LAUNCHERS_ENV: &str =
    "WARP_SMART_ALT_SCREEN_MOUSE_TMUX_LAUNCHERS";

fn is_sgr_mouse_reporting_enabled(model: &TerminalModel, ctx: &AppContext) -> bool {
    // Require some level of mouse tracking to be enabled when the block list is active.
    let mouse_tracking = model.is_alt_screen_active()
        || model.is_term_mode_set(TermMode::MOUSE_REPORT_CLICK)
        || model.is_term_mode_set(TermMode::MOUSE_DRAG)
        || model.is_term_mode_set(TermMode::MOUSE_MOTION);

    model.is_term_mode_set(TermMode::SGR_MOUSE) && mouse_tracking && mouse_reporting_enabled(ctx)
}

fn mouse_reporting_enabled(ctx: &AppContext) -> bool {
    *AltScreenReporting::as_ref(ctx)
        .mouse_reporting_enabled
        .value()
}

fn normalized_command_basename(token: &str) -> Option<&str> {
    let token = token.trim_matches(|c: char| {
        matches!(
            c,
            '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ';' | '&' | '|'
        )
    });
    token.rsplit('/').next().filter(|token| !token.is_empty())
}

fn smart_mouse_tmux_launcher_names(ctx: &AppContext) -> Vec<String> {
    let mut launchers = AltScreenReporting::as_ref(ctx)
        .smart_mouse_tmux_launchers
        .value()
        .iter()
        .filter_map(|token| normalized_command_basename(token).map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    if let Ok(extra_launchers) = std::env::var(SMART_ALT_SCREEN_MOUSE_TMUX_LAUNCHERS_ENV) {
        launchers.extend(
            extra_launchers
                .split(|c: char| c == ',' || c == ':' || c == ';' || c.is_whitespace())
                .filter_map(normalized_command_basename)
                .map(ToOwned::to_owned),
        );
    }

    launchers.sort_unstable();
    launchers.dedup();
    launchers
}

fn command_token_enables_smart_mouse(token: &str, ctx: &AppContext) -> bool {
    let Some(token) = normalized_command_basename(token) else {
        return false;
    };
    smart_mouse_tmux_launcher_names(ctx)
        .iter()
        .any(|launcher| launcher == token)
}

fn skip_wrapper_options(
    tokens: &[String],
    mut index: usize,
    options_with_values: &[&str],
) -> usize {
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if token == "--" {
            return index + 1;
        }
        if !token.starts_with('-') || token == "-" {
            return index;
        }
        let option_name = token.split('=').next().unwrap_or(token);
        index += 1;
        if options_with_values.contains(&option_name) && !token.contains('=') {
            index += 1;
        }
    }
    index
}

fn command_looks_like_tmux(command: &str, ctx: &AppContext) -> bool {
    let tokens = shlex::split(command)
        .unwrap_or_else(|| command.split_whitespace().map(ToOwned::to_owned).collect());

    let mut index = 0;
    while let Some(token) = tokens.get(index) {
        let token = token.as_str();
        if token.contains('=') && !token.starts_with('-') {
            index += 1;
            continue;
        }
        match token {
            "command" | "exec" | "nohup" => {
                index += 1;
                continue;
            }
            "env" => {
                index = skip_wrapper_options(&tokens, index + 1, &["-u", "--unset", "-C", "-S"]);
                continue;
            }
            "sudo" => {
                index = skip_wrapper_options(
                    &tokens,
                    index + 1,
                    &[
                        "-a", "-C", "-c", "-D", "-g", "-h", "-p", "-R", "-r", "-T", "-t", "-U",
                        "-u",
                    ],
                );
                continue;
            }
            _ => return command_token_enables_smart_mouse(token, ctx),
        }
    }
    false
}

fn is_tmux_context(model: &TerminalModel, ctx: &AppContext) -> bool {
    // In non-control-mode tmux, Warp can only infer context from the active command. Additional
    // launcher/wrapper commands can be configured with
    // `terminal.smart_alt_screen_mouse_tmux_launchers` or the
    // WARP_SMART_ALT_SCREEN_MOUSE_TMUX_LAUNCHERS environment variable.
    model.tmux_control_mode_active()
        || command_looks_like_tmux(&model.block_list().active_block().command_to_string(), ctx)
}

/// Determines if mouse event is intercepted based on SGR_MOUSE mode and mouse reporting setting.
pub fn should_intercept_mouse(model: &TerminalModel, shift: bool, ctx: &AppContext) -> bool {
    // Always intercept mouse for a shared session reader since their mouse events
    // will not be processed by the sharer's running terminal app.
    if model.shared_session_status().is_reader() || shift {
        return true;
    }
    !is_sgr_mouse_reporting_enabled(model, ctx)
}

/// Determines if SGR mouse events should be routed through Warp's smart alt-screen gesture
/// classifier before deciding whether to consume them or replay them to the PTY.
pub fn should_use_smart_mouse_handling(
    model: &TerminalModel,
    shift: bool,
    ctx: &AppContext,
) -> bool {
    !shift && should_continue_smart_mouse_handling(model, ctx)
}

/// Determines if a no-Shift right-click should use Warp's smart tmux routing. Right-clicks do not
/// have a pending press that must wait for a later drag/click classification, but they must still
/// require observed SGR mouse tracking before replaying mouse escape bytes to the PTY. This
/// preserves ordinary alt-screen right-click context menus when the TUI has not opted into mouse
/// reporting, preserves Shift as the force-Warp context-menu path, and keeps the behavior behind
/// the rollout gate.
pub fn should_use_smart_right_mouse_handling(
    model: &TerminalModel,
    shift: bool,
    ctx: &AppContext,
) -> bool {
    FeatureFlag::SmartAltScreenMouseHandling.is_enabled()
        && !shift
        && !model.shared_session_status().is_reader()
        && model.is_alt_screen_active()
        && is_tmux_context(model, ctx)
        && is_sgr_mouse_reporting_enabled(model, ctx)
}

/// Determines if an in-flight smart SGR mouse gesture should stay on the smart path. This
/// intentionally ignores Shift so a gesture that began without Shift can still finish through
/// the stable `TerminalView` gesture state even if Shift is pressed before drag/up.
pub fn should_continue_smart_mouse_handling(model: &TerminalModel, ctx: &AppContext) -> bool {
    FeatureFlag::SmartAltScreenMouseHandling.is_enabled()
        && !model.shared_session_status().is_reader()
        && is_tmux_context(model, ctx)
        && is_sgr_mouse_reporting_enabled(model, ctx)
}

/// Determines if scroll event is intercepted. SGR_mouse and mouse reporting must be enabled to
/// report scroll events, otherwise, always intercept scroll.
pub fn should_intercept_scroll(model: &TerminalModel, ctx: &AppContext) -> bool {
    let scroll_reporting_enabled = *AltScreenReporting::as_ref(ctx)
        .scroll_reporting_enabled
        .value();
    should_intercept_mouse(model, false, ctx) || !scroll_reporting_enabled
}
