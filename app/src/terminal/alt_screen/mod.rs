use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use super::alt_screen_reporting::AltScreenReporting;
use super::model::grid::grid_handler::TermMode;
use super::TerminalModel;

pub mod alt_screen_element;

/// Determines if mouse event is intercepted based on SGR_MOUSE mode and mouse reporting setting.
pub fn should_intercept_mouse(model: &TerminalModel, shift: bool, ctx: &AppContext) -> bool {
    // Always intercept mouse for a shared session reader since their mouse events
    // will not be processed by the sharer's running terminal app.
    if model.shared_session_status().is_reader() || shift {
        return true;
    }
    // Require some level of mouse tracking to be enabled when the block list is active.
    let mouse_tracking = model.is_alt_screen_active()
        || model.is_term_mode_set(TermMode::MOUSE_REPORT_CLICK)
        || model.is_term_mode_set(TermMode::MOUSE_DRAG)
        || model.is_term_mode_set(TermMode::MOUSE_MOTION);
    let mouse_reporting_enabled = *AltScreenReporting::as_ref(ctx)
        .mouse_reporting_enabled
        .value();
    !(model.is_term_mode_set(TermMode::SGR_MOUSE) && mouse_tracking && mouse_reporting_enabled)
}

/// Determines if scroll event is intercepted. SGR_mouse and mouse reporting must be enabled to
/// report scroll events, otherwise, always intercept scroll.
pub fn should_intercept_scroll(model: &TerminalModel, ctx: &AppContext) -> bool {
    let scroll_reporting_enabled = *AltScreenReporting::as_ref(ctx)
        .scroll_reporting_enabled
        .value();
    should_intercept_mouse(model, false, ctx) || !scroll_reporting_enabled
}
