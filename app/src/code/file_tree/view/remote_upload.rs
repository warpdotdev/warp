use std::any::Any;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use futures::AsyncReadExt as _;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use remote_server::client::RemoteServerClient;
use remote_server::proto::{CompleteRemoteUploadSuccess, RemoteUploadManifestEntry};
use warp_core::{HostId, SessionId};
use warp_util::standardized_path::StandardizedPath;
use warpui::event::DispatchedEvent;
use warpui::{
    elements::Point, AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};

use super::{FileTreeAction, FileTreeIdentifier};

pub const REMOTE_UPLOAD_CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct RemoteUploadTarget {
    pub host_id: HostId,
    pub session_id: SessionId,
    pub root: StandardizedPath,
    pub repo_root: String,
    pub target_dir: StandardizedPath,
}

#[derive(Clone, Debug)]
pub struct LocalUploadManifest {
    pub entries: Vec<LocalUploadEntry>,
    pub file_count: u64,
    pub directory_count: u64,
    pub total_bytes: u64,
    pub failures: Vec<String>,
}

impl LocalUploadManifest {
    pub fn proto_entries(&self) -> Vec<RemoteUploadManifestEntry> {
        self.entries
            .iter()
            .map(|entry| RemoteUploadManifestEntry {
                relative_path: entry.relative_path.clone(),
                is_directory: entry.is_directory,
                size: entry.size,
                unix_mode: entry.unix_mode,
            })
            .collect()
    }

    pub fn first_relative_path(&self) -> Option<&str> {
        self.entries
            .first()
            .map(|entry| entry.relative_path.as_str())
    }

    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }
}

#[derive(Clone, Debug)]
pub struct LocalUploadEntry {
    pub local_path: PathBuf,
    pub relative_path: String,
    pub is_directory: bool,
    pub size: u64,
    pub unix_mode: Option<u32>,
}

pub fn build_local_upload_manifest(paths: Vec<PathBuf>) -> Result<LocalUploadManifest, String> {
    if paths.is_empty() {
        return Err("No local files were provided for upload.".to_string());
    }

    let mut entries = Vec::new();
    let mut failures = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let Some(file_name) = path.file_name() else {
            let path = path.display();
            failures.push(format!("Skipped path without a file name: {path}"));
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&path) else {
            let path = path.display();
            failures.push(format!("Skipped unreadable local path: {path}"));
            continue;
        };
        let base = PathBuf::from(file_name);
        collect_upload_entries(
            &path,
            &base,
            metadata,
            &mut seen,
            &mut entries,
            &mut failures,
        )?;
    }

    if entries.is_empty() {
        return Err(failures
            .first()
            .cloned()
            .unwrap_or_else(|| "No readable local files were provided for upload.".to_string()));
    }

    let mut file_count = 0;
    let mut directory_count = 0;
    let mut total_bytes = 0;
    for entry in &entries {
        if entry.is_directory {
            directory_count += 1;
        } else {
            file_count += 1;
            total_bytes += entry.size;
        }
    }

    Ok(LocalUploadManifest {
        entries,
        file_count,
        directory_count,
        total_bytes,
        failures,
    })
}

fn collect_upload_entries(
    local_path: &Path,
    relative_path: &Path,
    metadata: std::fs::Metadata,
    seen: &mut HashSet<String>,
    entries: &mut Vec<LocalUploadEntry>,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let relative_path = normalize_relative_path(relative_path)?;
    if !seen.insert(relative_path.clone()) {
        return Err(format!("Duplicate local upload path: {relative_path}"));
    }

    let is_directory = metadata.is_dir();
    entries.push(LocalUploadEntry {
        local_path: local_path.to_path_buf(),
        relative_path: relative_path.clone(),
        is_directory,
        size: if is_directory { 0 } else { metadata.len() },
        unix_mode: unix_mode(&metadata),
    });

    if is_directory {
        let Ok(read_dir) = std::fs::read_dir(local_path) else {
            let path = local_path.display();
            failures.push(format!(
                "Skipped unreadable local directory contents: {path}"
            ));
            return Ok(());
        };
        let mut children = match read_dir.collect::<Result<Vec<_>, _>>() {
            Ok(children) => children,
            Err(_) => {
                let path = local_path.display();
                failures.push(format!(
                    "Skipped unreadable local directory contents: {path}"
                ));
                return Ok(());
            }
        };
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let child_path = child.path();
            let Ok(metadata) = std::fs::metadata(&child_path) else {
                let path = child_path.display();
                failures.push(format!("Skipped unreadable local path: {path}"));
                continue;
            };
            collect_upload_entries(
                &child_path,
                &Path::new(&relative_path).join(child.file_name()),
                metadata,
                seen,
                entries,
                failures,
            )?;
        }
    }

    Ok(())
}

fn normalize_relative_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| "Local upload path contains invalid UTF-8.".to_string())?;
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Local upload path cannot escape its source root.".to_string());
            }
        }
    }

    if parts.is_empty() {
        return Err("Local upload path is empty.".to_string());
    }

    Ok(parts.join("/"))
}

#[cfg(unix)]
fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

pub async fn upload_manifest(
    client: Arc<RemoteServerClient>,
    upload_id: String,
    target: RemoteUploadTarget,
    manifest: LocalUploadManifest,
    overwrite: bool,
) -> Result<CompleteRemoteUploadSuccess, String> {
    client
        .begin_remote_upload(
            upload_id.clone(),
            target.repo_root.clone(),
            target.target_dir.to_string(),
            manifest.proto_entries(),
            overwrite,
        )
        .await
        .map_err(|e| e.to_string())?;

    for entry in manifest.entries.iter().filter(|entry| !entry.is_directory) {
        if let Err(message) = upload_file_entry(&client, &upload_id, entry).await {
            let _ = client.complete_remote_upload(upload_id).await;
            return Err(message);
        }
    }

    client
        .complete_remote_upload(upload_id)
        .await
        .map_err(|e| e.to_string())
}

async fn upload_file_entry(
    client: &Arc<RemoteServerClient>,
    upload_id: &str,
    entry: &LocalUploadEntry,
) -> Result<(), String> {
    let mut file = async_fs::File::open(&entry.local_path)
        .await
        .map_err(|e| format!("Failed to open local file for upload: {e}"))?;
    let mut buffer = vec![0; REMOTE_UPLOAD_CHUNK_BYTES];
    let mut offset = 0;

    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|e| format!("Failed to read local file for upload: {e}"))?;
        if read == 0 {
            break;
        }
        client
            .upload_remote_file_chunk(
                upload_id.to_string(),
                entry.relative_path.clone(),
                offset,
                buffer[..read].to_vec(),
            )
            .await
            .map_err(|e| e.to_string())?;
        offset += read as u64;
    }

    Ok(())
}

pub struct FileTreeLocalFileDropTarget {
    child: Box<dyn Element>,
    id: FileTreeIdentifier,
    is_valid_target: bool,
}

impl FileTreeLocalFileDropTarget {
    pub fn new(child: Box<dyn Element>, id: FileTreeIdentifier, is_valid_target: bool) -> Self {
        Self {
            child,
            id,
            is_valid_target,
        }
    }

    fn mouse_position_is_in_bounds(&self, position: Vector2F) -> bool {
        let Some(bounds) = self.bounds() else {
            return false;
        };

        bounds.contains_point(position)
    }
}

impl Element for FileTreeLocalFileDropTarget {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn bounds(&self) -> Option<RectF> {
        self.child.bounds()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        self.child.parent_data()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled_by_child = self.child.dispatch_event(event, ctx, app);
        if handled_by_child {
            return true;
        }

        let Some(z_index) = self.z_index() else {
            return false;
        };
        let Some(event_at_z_index) = event.at_z_index(z_index, ctx) else {
            return false;
        };

        match event_at_z_index {
            Event::DragFiles { location } if self.mouse_position_is_in_bounds(*location) => {
                if self.is_valid_target {
                    ctx.dispatch_typed_action(FileTreeAction::LocalFilesDragged {
                        id: self.id.clone(),
                    });
                } else {
                    ctx.dispatch_typed_action(FileTreeAction::LocalFileDragExited);
                }
                true
            }
            Event::DragFileExit => {
                ctx.dispatch_typed_action(FileTreeAction::LocalFileDragExited);
                true
            }
            Event::DragAndDropFiles { paths, location }
                if self.is_valid_target
                    && self.mouse_position_is_in_bounds(*location)
                    && !paths.is_empty() =>
            {
                ctx.dispatch_typed_action(FileTreeAction::LocalFilesDropped {
                    id: self.id.clone(),
                    paths: paths.clone(),
                });
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preserves_nested_directory_structure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assets");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/icon.bin"), [0, 159, 146, 150]).unwrap();

        let manifest = build_local_upload_manifest(vec![root.clone()]).unwrap();
        let paths = manifest
            .entries
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry.is_directory, entry.size))
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                ("assets", true, 0),
                ("assets/nested", true, 0),
                ("assets/nested/icon.bin", false, 4),
            ]
        );
        assert_eq!(manifest.file_count, 1);
        assert_eq!(manifest.directory_count, 2);
        assert_eq!(manifest.total_bytes, 4);
    }

    #[test]
    fn manifest_rejects_duplicate_top_level_names() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("one");
        let second_parent = temp.path().join("two");
        let second = second_parent.join("shared.txt");
        std::fs::create_dir_all(&second_parent).unwrap();
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();

        let first_as_shared = temp.path().join("shared.txt");
        std::fs::rename(&first, &first_as_shared).unwrap();

        let error = build_local_upload_manifest(vec![first_as_shared, second])
            .expect_err("duplicate names should be rejected");
        assert!(error.contains("Duplicate local upload path"));
    }

    #[test]
    fn manifest_reports_unreadable_sources_without_aborting_valid_entries() {
        let temp = tempfile::tempdir().unwrap();
        let valid = temp.path().join("valid.bin");
        let missing = temp.path().join("missing.bin");
        std::fs::write(&valid, [1, 2, 3]).unwrap();

        let manifest = build_local_upload_manifest(vec![missing, valid]).unwrap();

        assert_eq!(manifest.file_count, 1);
        assert_eq!(manifest.failure_count(), 1);
        assert_eq!(manifest.entries[0].relative_path, "valid.bin");
        assert!(manifest.failures[0].contains("Skipped unreadable local path"));
    }

    #[cfg(unix)]
    #[test]
    fn manifest_follows_valid_symlink_sources() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.bin");
        let link = temp.path().join("link.bin");
        std::fs::write(&target, [1, 2, 3, 4]).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let manifest = build_local_upload_manifest(vec![link]).unwrap();

        assert_eq!(manifest.file_count, 1);
        assert_eq!(manifest.entries[0].relative_path, "link.bin");
        assert_eq!(manifest.entries[0].size, 4);
    }

    #[test]
    fn normalize_relative_path_rejects_parent_escape() {
        let error = normalize_relative_path(Path::new("../secret.txt"))
            .expect_err("parent traversal should be rejected");
        assert!(error.contains("cannot escape"));
    }
}
