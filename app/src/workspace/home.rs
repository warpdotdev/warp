//! Warp Home
//!
//! This is the landing page for new tabs if session creation isn't supported (e.g. on the web).
//! It's barebones at the moment, but may grow into a more full-featured admin experience.

use warpui::ViewContext;

use super::view::Workspace;
use crate::pane_group::{AnyPaneContent, FilePane};

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
        let content = i18n::t("workspace.home.content");
        pane.open_static(i18n::t("workspace.home.title"), &content, ctx);
    });
    Box::new(pane)
}
