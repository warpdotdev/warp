pub mod data_source;
#[cfg(not(target_family = "wasm"))]
pub mod search_item;

#[cfg(not(target_family = "wasm"))]
use std::path::Path;

use warpui::AppContext;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;

#[cfg(not(target_family = "wasm"))]
use crate::ai::outline::{OutlineStatus, RepoOutlines};
#[cfg(not(target_family = "wasm"))]
use crate::workspace::ActiveSession;

/// Checks if the code symbols (outline) are currently being indexed for the active directory.
/// Returns true if the outline is in a pending state, false otherwise.
#[cfg(not(target_family = "wasm"))]
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

/// WASM stub for the indexing check function.
#[cfg(target_family = "wasm")]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub fn is_code_symbols_indexing(_app: &AppContext) -> bool {
    false
}
