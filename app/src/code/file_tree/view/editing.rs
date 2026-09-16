//! Module for utilities related to editing items in the file tree.

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;

use std::cmp::Ordering;
use std::io;
use std::path::Path;
use std::sync::Arc;

use repo_metadata::file_tree_store::FileTreeEntryState;
use repo_metadata::{FileMetadata, FileTreeEntry};
use warp_util::standardized_path::StandardizedPath;
use warpui::ViewContext;
use warpui::elements::MouseStateHandle;

use super::{FileTreeIdentifier, FileTreeItem, FileTreeView};
use crate::code::file_tree::FileTreeEvent;
use crate::code::file_tree::view::{PendingEdit, PendingEditKind};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;

/// Custom ordering function for items in the file tree.
///
/// Directories are ordered first, sorted by natural (numeric-aware) order.
/// Files are ordered second, sorted by natural (numeric-aware) order.
/// Within each group, dotfiles (entries starting with a dot) are ordered first.
pub(super) fn sort_entries_for_file_tree(
    entry_1: &StandardizedPath,
    entry_2: &StandardizedPath,
    entry_map: &FileTreeEntry,
) -> Ordering {
    use std::cmp::Ordering;

    // Entries missing from the map sort before present entries, and compare
    // equal to each other. Using the same `Ordering` on both sides would
    // violate antisymmetry and cause `sorted_by` to panic with
    // "user-provided comparison function does not correctly implement a total order".
    let (entry_1, entry_2) = match (entry_map.get(entry_1), entry_map.get(entry_2)) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Less,
        (Some(_), None) => return Ordering::Greater,
        (Some(e1), Some(e2)) => (e1, e2),
    };

    let is_dir_1 = matches!(entry_1, FileTreeEntryState::Directory(_));
    let is_dir_2 = matches!(entry_2, FileTreeEntryState::Directory(_));

    // Order directories before any files.
    match (is_dir_1, is_dir_2) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        // Both are same type, continue with alphabetical sort.
        _ => {}
    }

    // Same antisymmetry requirement for missing file names.
    let (name_1, name_2) = match (entry_1.path().file_name(), entry_2.path().file_name()) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Less,
        (Some(_), None) => return Ordering::Greater,
        (Some(n1), Some(n2)) => (n1, n2),
    };

    let starts_with_dot_1 = name_1.starts_with('.');
    let starts_with_dot_2 = name_2.starts_with('.');

    // Items starting with "." come first.
    match (starts_with_dot_1, starts_with_dot_2) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => alphanumeric_sort::compare_str(name_1, name_2),
    }
}

pub(super) fn move_destination(
    source: &StandardizedPath,
    target_directory: &StandardizedPath,
) -> Option<StandardizedPath> {
    if source == target_directory
        || target_directory.starts_with(source)
        || source.parent().as_ref() == Some(target_directory)
    {
        return None;
    }

    Some(target_directory.join(source.file_name()?))
}
pub(super) fn destination_is_vacant(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_to_c_string(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

#[cfg(target_os = "linux")]
fn rename_exclusive(old_path: &Path, new_path: &Path) -> io::Result<()> {
    let old_path = path_to_c_string(old_path)?;
    let new_path = path_to_c_string(new_path)?;
    // SAFETY: Both pointers reference valid NUL-terminated paths for the duration of the call.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_exclusive(old_path: &Path, new_path: &Path) -> io::Result<()> {
    let old_path = path_to_c_string(old_path)?;
    let new_path = path_to_c_string(new_path)?;
    // SAFETY: Both pointers reference valid NUL-terminated paths for the duration of the call.
    let result =
        unsafe { libc::renamex_np(old_path.as_ptr(), new_path.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "windows")]
fn rename_exclusive(old_path: &Path, new_path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
    use windows::core::PCWSTR;

    let old_path: Vec<_> = old_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let new_path: Vec<_> = new_path.as_os_str().encode_wide().chain(Some(0)).collect();

    // SAFETY: Both pointers reference valid NUL-terminated paths for the duration of the call.
    unsafe {
        MoveFileExW(
            PCWSTR(old_path.as_ptr()),
            PCWSTR(new_path.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|_| io::Error::last_os_error())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn rename_exclusive(_old_path: &Path, _new_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn paths_refer_to_same_entry(old_path: &Path, new_path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (
        std::fs::symlink_metadata(old_path),
        std::fs::symlink_metadata(new_path),
    ) {
        (Ok(old_metadata), Ok(new_metadata)) => {
            old_metadata.dev() == new_metadata.dev() && old_metadata.ino() == new_metadata.ino()
        }
        _ => false,
    }
}
#[cfg(target_os = "windows")]
fn paths_refer_to_same_entry(old_path: &Path, new_path: &Path) -> bool {
    windows_file_identity(old_path)
        .zip(windows_file_identity(new_path))
        .is_some_and(|(old_identity, new_identity)| old_identity == new_identity)
}

#[cfg(target_os = "windows")]
fn windows_file_identity(path: &Path) -> Option<(u32, u32, u32)> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
    };
    use windows::core::{Owned, PCWSTR};

    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: The path pointer remains valid for the call, and the returned handle is owned here.
    let handle = unsafe {
        Owned::new(
            CreateFileW(
                PCWSTR(path.as_ptr()),
                FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
            .ok()?,
        )
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The handle is valid and the output pointer refers to initialized writable memory.
    unsafe { GetFileInformationByHandle(*handle, &mut information) }.ok()?;
    Some((
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
    ))
}

#[cfg(not(any(unix, target_os = "windows")))]
fn paths_refer_to_same_entry(_old_path: &Path, _new_path: &Path) -> bool {
    false
}

fn rename_noreplace(old_path: &Path, new_path: &Path) -> io::Result<()> {
    if !paths_refer_to_same_entry(old_path, new_path) {
        return rename_exclusive(old_path, new_path);
    }

    let temporary_path = old_path.with_file_name(format!(".warp-rename-{}", uuid::Uuid::new_v4()));

    rename_exclusive(old_path, &temporary_path)?;
    if let Err(error) = rename_exclusive(&temporary_path, new_path) {
        if let Err(rollback_error) = rename_exclusive(&temporary_path, old_path) {
            log::error!(
                "Failed to restore {} after case-only rename failed: {rollback_error}",
                old_path.display()
            );
        }
        return Err(error);
    }

    Ok(())
}

impl FileTreeView {
    /// Creates a new file below the directory at the given identifier.
    pub(super) fn create_new_file(&mut self, id: &FileTreeIdentifier, ctx: &mut ViewContext<Self>) {
        let Some(root_dir) = self.root_directories.get_mut(&id.root) else {
            return;
        };
        let (path, depth) = match root_dir.items.get(id.index) {
            Some(FileTreeItem::File { .. }) => {
                log::warn!("Cannot create a new file below a file");
                return;
            }
            Some(FileTreeItem::DirectoryHeader {
                directory, depth, ..
            }) => (directory.path.clone(), *depth),
            _ => return,
        };

        // Ensure the parent directory is expanded before creating a file beneath it.
        if !self.is_folder_expanded(&id.root, &path) {
            self.toggle_folder_expansion(&id.root, &path, ctx);
        }

        // Create a dummy FileTreeItem for the file we are about to create--we'll replace
        // this with something real once the user types in the actual file.
        let new_item_index = id.index + 1;
        let Some(root_dir) = self.root_directories.get_mut(&id.root) else {
            return;
        };
        root_dir.items.insert(
            new_item_index,
            FileTreeItem::File {
                metadata: FileMetadata::from_standardized(path.join("new_file"), false).into(),
                depth: depth + 1,
                mouse_state_handle: MouseStateHandle::default(),
                draggable_state: warpui::elements::DraggableState::default(),
            },
        );

        // Ensure the new item we just created is selected.
        let new_id = FileTreeIdentifier {
            root: id.root.clone(),
            index: new_item_index,
        };
        self.select_id(&new_id, ctx);

        // Ensure the editor is focused.
        ctx.focus(&self.editor_view);
        self.pending_edit = Some(PendingEdit {
            id: new_id,
            kind: PendingEditKind::CreateNewFile,
        });
    }

    /// Starts a rename edit on the item at the given identifier.
    pub(super) fn start_rename(&mut self, id: &FileTreeIdentifier, ctx: &mut ViewContext<Self>) {
        let Some(root_dir) = self.root_directories.get(&id.root) else {
            return;
        };
        let Some(item) = root_dir.items.get(id.index) else {
            return;
        };
        // Prefill the editor with the current file or directory name.
        let current_name = item
            .path()
            .file_name()
            .map(|s| s.to_owned())
            .unwrap_or_default();

        self.pending_edit = Some(PendingEdit {
            id: id.clone(),
            kind: PendingEditKind::RenameExisting,
        });

        self.editor_view.update(ctx, |view, ctx| {
            view.set_buffer_text(&current_name, ctx);
        });
        ctx.focus(&self.editor_view);
    }

    /// Commits a pending edit to the file tree.
    pub(super) fn commit_pending_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(pending_edit) = self.pending_edit.take() else {
            return;
        };

        let file_tree_id = pending_edit.id.clone();

        let buffer_content = self.editor_view.as_ref(ctx).buffer_text(ctx);
        self.editor_view.update(ctx, |view, ctx| {
            view.clear_buffer(ctx);
        });

        match pending_edit.kind {
            PendingEditKind::CreateNewFile => {
                let new_entry = {
                    let Some(root_dir) = self.root_directories.get_mut(&file_tree_id.root) else {
                        return;
                    };
                    let Some(item) = root_dir.items.get_mut(file_tree_id.index) else {
                        return;
                    };

                    if let FileTreeItem::File { metadata, .. } = item {
                        let mut new_std = (*metadata.path).clone();
                        new_std.set_file_name(&buffer_content);
                        let local_path = new_std.to_local_path_lossy();
                        metadata.path = Arc::new(new_std);

                        if let Err(e) = std::fs::File::create_new(&local_path) {
                            log::warn!("Failed to create file: {e}");
                            return;
                        }

                        send_telemetry_from_ctx!(TelemetryEvent::FileTreeItemCreated, ctx);

                        FileTreeEntryState::File(metadata.clone())
                    } else {
                        return;
                    }
                };

                if let Some(root_dir) = self.root_directories.get_mut(&file_tree_id.root) {
                    // Ensure the file tree has the new item we've just created.
                    Self::insert_entry(&mut root_dir.entry, new_entry);
                }

                self.open_in_new_pane(&file_tree_id, ctx);
                self.rebuild_flattened_items();
            }
            PendingEditKind::RenameExisting => {
                let Some(root_dir) = self.root_directories.get(&file_tree_id.root) else {
                    return;
                };
                let Some(item) = root_dir.items.get(file_tree_id.index) else {
                    return;
                };
                if buffer_content.is_empty() {
                    return;
                }
                let old_std_path = item.path().clone();
                let mut new_std_path = old_std_path.clone();
                new_std_path.set_file_name(&buffer_content);
                self.move_item(&file_tree_id, old_std_path, new_std_path, ctx);
            }
        }
    }

    pub(super) fn move_item_to_directory(
        &mut self,
        id: &FileTreeIdentifier,
        target_directory: &StandardizedPath,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(root_dir) = self.root_directories.get(&id.root) else {
            return;
        };
        if !matches!(
            root_dir.entry.get(target_directory),
            Some(FileTreeEntryState::Directory(_))
        ) {
            return;
        }
        let Some(source) = root_dir.items.get(id.index).map(|item| item.path().clone()) else {
            return;
        };
        let Some(destination) = move_destination(&source, target_directory) else {
            return;
        };
        if !destination_is_vacant(&destination.to_local_path_lossy()) {
            return;
        }

        self.move_item(id, source, destination, ctx);
    }

    fn move_item(
        &mut self,
        id: &FileTreeIdentifier,
        old_std_path: StandardizedPath,
        new_std_path: StandardizedPath,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(repository_root) = self
            .root_directories
            .get(&id.root)
            .map(|root_dir| root_dir.entry.root_directory().as_ref().clone())
        else {
            return;
        };
        let old_path = old_std_path.to_local_path_lossy();
        let new_path = new_std_path.to_local_path_lossy();
        if let Err(e) = rename_noreplace(&old_path, &new_path) {
            log::warn!(
                "Failed to move {} -> {}: {e}",
                old_path.display(),
                new_path.display()
            );
            return;
        }
        #[cfg(feature = "local_fs")]
        self.repository_metadata_model.update(ctx, |model, ctx| {
            model.rename_local_entry_path(&repository_root, &old_std_path, &new_std_path, ctx);
        });

        if let Some(root_dir) = self.root_directories.get_mut(&id.root) {
            root_dir.entry.rename_path(&old_std_path, &new_std_path);
        }

        ctx.emit(FileTreeEvent::FileRenamed {
            old_path: old_path.clone(),
            new_path: new_path.clone(),
        });

        self.rebuild_flatten_items_impl(Some(id), None, None);
        ctx.notify();
    }

    /// Cancels a pending edit and discards any changes.
    pub(super) fn cancel_pending_edit(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(pending_edit) = self.pending_edit.take() {
            let id = &pending_edit.id;
            if self.selected_item.as_ref() == Some(id) {
                self.selected_item = None;
            }
            self.editor_view.update(ctx, |view, ctx| {
                view.clear_buffer(ctx);
            });
            // Only remove placeholder in the create-new-file flow.
            if pending_edit.kind == PendingEditKind::CreateNewFile
                && let Some(root_dir) = self.root_directories.get_mut(&id.root)
            {
                root_dir.items.remove(id.index);
            }
        }
        ctx.notify();
    }

    /// Inserts a new entry into the tree.
    fn insert_entry(root_entry: &mut FileTreeEntry, child_entry: FileTreeEntryState) {
        let Some(parent) = child_entry.path().parent() else {
            return;
        };

        root_entry.insert_child_state(&parent, child_entry);
    }

    pub(super) fn handle_pending_edit(&mut self, ctx: &mut ViewContext<Self>) {
        if self.pending_edit.is_none() {
            return;
        };

        let editor_contents = self.editor_view.as_ref(ctx).buffer_text(ctx);
        // If the editor is empty and the editor was dismissed, cancel the editor.
        // Otherwise commit the editor. This matches VSCode's behavior.
        if editor_contents.is_empty() {
            self.cancel_pending_edit(ctx);
        } else {
            self.commit_pending_edit(ctx);
        }
    }
}
