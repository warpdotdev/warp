pub mod data_source;
pub mod search_item;
use crate::ai::outline::{OutlineStatus, RepoOutlines};
use crate::workspace::ActiveSession;
use std::path::Path;
use warpui::AppContext;
use warpui::SingletonEntity;

/// Checks if the code symbols (outline) are currently being indexed for the active directory.
/// Returns true if the outline is in a pending state, false otherwise.
pub fn is_code_symbols_indexing(app: &AppContext) -> bool {
    let active_window_id = app.windows().state().active_window;

    let current_dir =
        active_window_id.and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id));

    if let Some(current_dir) = current_dir {
        let repo_outlines = RepoOutlines::handle(app);
        let repo_outlines_ref = repo_outlines.as_ref(app);

        if let Some((status, _)) = repo_outlines_ref.get_outline(Path::new(current_dir)) {
            matches!(status, OutlineStatus::Pending)
        } else {
            false
        }
    } else {
        false
    }
}
