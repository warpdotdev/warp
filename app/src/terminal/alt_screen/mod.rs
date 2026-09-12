use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use super::TerminalModel;
use super::alt_screen_reporting::AltScreenReporting;
use super::model::grid::grid_handler::TermMode;

pub mod alt_screen_element;

/// Determines if mouse events should be handled by Warp instead of forwarded to the PTY.
pub fn should_intercept_mouse(model: &TerminalModel, shift: bool, ctx: &AppContext) -> bool {
    // Always intercept mouse for a shared session reader since their mouse events
    // will not be processed by the sharer's running terminal app.
    if model.shared_session_status().is_reader() || shift {
        return true;
    }
    let mouse_tracking = model.is_term_mode_set(TermMode::MOUSE_MODE);
    let mouse_reporting_enabled = *AltScreenReporting::as_ref(ctx)
        .mouse_reporting_enabled
        .value();
    !(mouse_tracking && mouse_reporting_enabled)
}

/// Determines if scroll event is intercepted. Mouse tracking and both reporting settings must be
/// enabled to report scroll events, otherwise, always intercept scroll.
pub fn should_intercept_scroll(model: &TerminalModel, ctx: &AppContext) -> bool {
    let scroll_reporting_enabled = *AltScreenReporting::as_ref(ctx)
        .scroll_reporting_enabled
        .value();
    should_intercept_mouse(model, false, ctx) || !scroll_reporting_enabled
}
