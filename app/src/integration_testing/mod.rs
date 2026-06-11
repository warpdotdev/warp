use std::borrow::Cow;

use warpui::{App, AssetProvider, View, ViewHandle, WindowId};

pub mod agent_mode;
pub mod assertions;
pub mod block;
pub mod block_filtering;
pub mod clipboard;
pub mod cloud_object;
pub mod code_review;
pub mod codebase_context;
pub mod command_palette;
pub mod command_search;
pub mod context_chips;
pub mod find;
pub mod goto_line;
pub mod input;
pub mod keybindings;
pub mod launch_configs;
pub mod navigation_palette;
pub mod notebook;
pub mod pane_group;
pub mod persistence;
#[cfg(target_os = "macos")]
pub mod preview_config_migration;
pub mod remote_server;
pub mod rules;
pub mod secret_redaction;
pub mod settings;
pub mod step;
pub mod subshell;
pub mod tab;
pub mod terminal;
pub mod themes;
pub mod type_getters;
pub mod view_getters;
pub mod warp_drive;
pub mod window;
pub mod workflow;
pub mod workspace;

pub fn view_of_type<T: View>(app: &App, window_id: WindowId, tab_index: usize) -> ViewHandle<T> {
    app.views_of_type(window_id)
        .expect("should be views for window")
        .get(tab_index)
        .expect("should be an input view at index")
        .clone()
}

pub fn create_file_from_assets(
    assets: impl AssetProvider,
    asset_src: &str,
    dest_path: &std::path::Path,
) {
    let bytes = assets
        .get(asset_src)
        .expect("Should be able to retrieve file");
    create_file_with_contents(<Cow<'_, [u8]> as AsRef<[u8]>>::as_ref(&bytes), dest_path);
}

pub fn create_file_with_contents(contents: impl AsRef<[u8]>, file_path: &std::path::Path) {
    let mut file =
        crate::util::file::create_file(file_path).expect("Should be able to create file");
    std::io::Write::write_all(&mut file, contents.as_ref())
        .expect("Should be able to write to file");
}
