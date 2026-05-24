//! Black Home
//!
//! This is the landing page for new tabs if session creation isn't supported (e.g. on the web).
//! It's barebones at the moment, but may grow into a more full-featured admin experience.

use black_ui::ViewContext;

use super::view::Workspace;
use crate::pane_group::{AnyPaneContent, FilePane};

const WARP_HOME_TITLE: &str = "Welcome to Black on Web";
const WARP_HOME_CONTENT: &str = r#"
Welcome to Black on Web - your browser-based home for Black!
Use Black on Web to:
* Join Shared Sessions
* Create, View, and Edit Black Drive Objects
* Manage your Black Settings

Black on Web can also be used by your teammates and peers who don't have Black downloaded yet to view your shared sessions, notebooks, and workflows."#;

/// Create a static "home page" pane.
pub fn create_home_pane(ctx: &mut ViewContext<Workspace>) -> Box<dyn AnyPaneContent> {
    let pane = FilePane::new(
        None,
        None,
        #[cfg(feature = "local_fs")]
        None,
        ctx,
    );
    pane.file_view(ctx).update(ctx, |pane, ctx| {
        pane.open_static(WARP_HOME_TITLE, WARP_HOME_CONTENT, ctx);
    });
    Box::new(pane)
}
