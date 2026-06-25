use warpui::elements::{DraggableState, MouseStateHandle};

use super::FileTreeItem;
use crate::appearance::Appearance;
use crate::code::icon_from_file_path;
use crate::code_review::diff_state::GitFileStatus;
use crate::ui_components::icons::Icon;
use crate::ui_components::item_highlight::ImageOrIcon;

impl FileTreeItem {
    pub(super) fn to_render_state(
        &self,
        is_expanded: Option<bool>,
        appearance: &Appearance,
        git_status: Option<GitFileStatus>,
    ) -> RenderState {
        match self {
            FileTreeItem::File {
                metadata,
                mouse_state_handle,
                depth,
                draggable_state,
            } => {
                let display_name = metadata
                    .path
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| String::from("File"));

                let icon_from_file_path =
                    icon_from_file_path(metadata.path.as_str(), appearance).map(ImageOrIcon::Image);

                RenderState {
                    display_name,
                    icon: icon_from_file_path.unwrap_or(ImageOrIcon::Icon(Icon::File)),
                    is_expanded,
                    depth: *depth,
                    mouse_state: mouse_state_handle.clone(),
                    draggable_state: draggable_state.clone(),
                    is_ignored: metadata.ignored,
                    is_directory: false,
                    git_status,
                }
            }
            FileTreeItem::DirectoryHeader {
                directory,
                mouse_state_handle,
                depth,
                draggable_state,
            } => {
                let display_name = directory
                    .path
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| String::from("Folder"));
                RenderState {
                    display_name,
                    icon: ImageOrIcon::Icon(Icon::Folder),
                    is_expanded,
                    depth: *depth,
                    mouse_state: mouse_state_handle.clone(),
                    draggable_state: draggable_state.clone(),
                    is_ignored: directory.ignored,
                    is_directory: true,
                    git_status,
                }
            }
        }
    }
}

pub(super) struct RenderState {
    pub display_name: String,
    pub icon: ImageOrIcon,
    pub is_expanded: Option<bool>,
    pub depth: usize,
    pub mouse_state: MouseStateHandle,
    pub draggable_state: DraggableState,
    pub is_ignored: bool,
    /// Whether this row is a directory header (vs. a file). Drives whether the
    /// git decoration renders as a folder dot or a per-file letter badge.
    pub is_directory: bool,
    /// Working-tree git status for this entry, if any. `None` when the entry is
    /// clean or git decorations are disabled.
    pub git_status: Option<GitFileStatus>,
}
