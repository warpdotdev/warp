//! Opens the TUI window and wires up the agent view + bridge.
//!
//! This is invoked from `crate::launch` when the process was started in TUI mode
//! (see `warp::run_tui`). It runs inside the normal app init, so all the agent's
//! dependencies (auth, AI client, `ai::init`, ...) are already set up.

use warpui::{AddWindowOptions, AppContext};

use crate::tui::agent_bridge::TuiAgentBridge;
use crate::tui::agent_view::TuiAgentView;

/// Creates the agent bridge model and opens a TUI-backed window whose root view
/// is the [`TuiAgentView`].
pub fn open_tui_window(ctx: &mut AppContext) {
    let bridge = ctx.add_model(TuiAgentBridge::new);
    let (_window_id, _root_view) = ctx.add_window(AddWindowOptions::default(), move |ctx| {
        TuiAgentView::new(bridge, ctx)
    });
}
