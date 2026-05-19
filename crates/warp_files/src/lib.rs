use std::collections::HashSet;
/// A model for operating on files.
///
/// Allows opening and saving files in a single, central model.  Subscribers can watch for content
/// when files are loaded, and request that content be saved to disk.
use std::future::Future;
use std::io;
use std::ops::Range;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, SystemTime};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use remote_server::client::RemoteServerClient;
use remote_server::manager::RemoteServerManager;
use warp_core::HostId;
use warp_util::standardized_path::StandardizedPath;

use futures::io::{AsyncBufReadExt, BufReader};
use futures::StreamExt;

use async_channel::Sender;
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
use repo_metadata::{
    repositories::DetectedRepositories,
    repository::{RepositorySubscriber, SubscriberId},
    CanonicalizedPath, Repository, RepositoryUpdate,
};
use warp_util::content_version::ContentVersion;
use warp_util::file::FileSaveError;
use warp_util::file::{FileId, FileLoadError};
use warpui::ModelHandle;
use warpui::{r#async::SpawnedFutureHandle, AppContext, Entity, ModelContext, SingletonEntity};
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

pub mod text_file_reader;
pub use text_file_reader::{TextFileReadResult, TextFileSegment};

#[derive(Debug)]
pub enum FileModelEvent {
    FileLoaded {
        content: String,
        id: FileId,
        version: ContentVersion,
    },
    FailedToLoad {
        id: FileId,
        error: Rc<FileLoadError>,
    },
    FileSaved {
        id: FileId,
        version: ContentVersion,
    },
    FailedToSave {
        id: FileId,
        error: Rc<FileSaveError>,
    },
    FileUpdated {
        id: FileId,
        content: String,
        base_version: ContentVersion,
        new_version: ContentVersion,
    },
}

impl FileModelEvent {
    pub fn file_id(&self) -> FileId {
        match self {
            Self::FileLoaded { id, .. } => *id,
            Self::FailedToLoad { id, .. } => *id,
            Self::FileSaved { id, .. } => *id,
            Self::FailedToSave { id, .. } => *id,
            Self::FileUpdated { id, .. } => *id,
        }
    }
}

/// Tracks how a file is being watched for changes.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum WatcherType {
    /// File is not being watched.
    #[default]
    None,
    /// File is watched via individual file watcher.
    Individual,
    /// File is watched via repository-level subscription.
    Repository,
}

/// Per-file backing store.
/// Remote files dispatch through [`RemoteServerClient`] via [`RemoteServerManager`].
enum FileBackend {
    Local(LocalFile),
    Remote {
        /// Identifies the remote host. The actual client is looked up from
        /// [`RemoteServerManager`] at call time, which naturally handles
        /// disconnect (lookup returns `Err`) without holding an `Arc` alive
        /// per file.
        host_id: HostId,
        /// Platform-aware path on the remote host.
        path: StandardizedPath,
    },
}

impl FileBackend {
    fn as_local(&self) -> Option<&LocalFile> {
        match self {
            FileBackend::Local(f) => Some(f),
            FileBackend::Remote { .. } => None,
        }
    }

    fn version(&self) -> Option<ContentVersion> {
        match self {
            FileBackend::Local(f) => f.version,
            FileBackend::Remote { .. } => None,
        }
    }

    fn set_version(&mut self, version: ContentVersion) {
        match self {
            FileBackend::Local(f) => f.version = Some(version),
            FileBackend::Remote { .. } => {}
        }
    }
}

#[derive(Default)]
struct LocalFile {
    path: Option<PathBuf>,
    version: Option<ContentVersion>,
    /// How this file is being watched for changes.
    watcher_type: WatcherType,
}

impl LocalFile {
    fn should_receive_update_for_path(&self, path: &Path) -> bool {
        self.path.as_deref() == Some(path) && self.subscribes_to_updates() && self.version.is_some()
    }
}

/// Tracks an active subscription to a repository for file change events.
struct RepositorySubscription {
    repository: ModelHandle<Repository>,
    /// The subscriber ID, set once the async subscription completes.
    subscriber_id: Option<SubscriberId>,
}

impl LocalFile {
    fn new(path: PathBuf, watcher_type: WatcherType) -> Self {
        // If we cannot canonicalize the path, it could be because the file does not exist on disk yet.
        // In this case, keep its original input path.
        let canonicalized_path = CanonicalizedPath::try_from(&path)
            .map(|path| path.as_path_buf().to_path_buf())
            .unwrap_or(path);
        Self {
            path: Some(canonicalized_path),
            version: None,
            watcher_type,
        }
    }

    /// Returns true if this file is subscribed to receive update events.
    fn subscribes_to_updates(&self) -> bool {
        self.watcher_type != WatcherType::None
    }
}

/// Tracks files by ID with O(1) path reference counting.
///
/// Encapsulates the file map and path refcount together so callers
/// cannot forget to maintain the refcount invariant.
#[derive(Default)]
struct FileState {
    files: HashMap<FileId, FileBackend>,
    /// Tracks how many FileIds reference each path, for O(1) "path still used" checks.
    path_refcount: HashMap<PathBuf, usize>,
}

impl FileState {
    fn insert_local(&mut self, file_id: FileId, local_file: LocalFile) {
        if let Some(ref path) = local_file.path {
            *self.path_refcount.entry(path.clone()).or_insert(0) += 1;
        }
        self.files.insert(file_id, FileBackend::Local(local_file));
    }

    fn insert_remote(&mut self, file_id: FileId, host_id: HostId, path: StandardizedPath) {
        self.files
            .insert(file_id, FileBackend::Remote { host_id, path });
    }

    /// Removes a file and returns the backend along with whether the local path
    /// is still referenced (always `false` for remote files).
    fn remove(&mut self, file_id: FileId) -> Option<(FileBackend, bool)> {
        let backend = self.files.remove(&file_id)?;
        let path_still_used = match &backend {
            FileBackend::Local(file) => {
                if let Some(ref path) = file.path {
                    match self.path_refcount.get_mut(path) {
                        Some(count) => {
                            *count -= 1;
                            if *count == 0 {
                                self.path_refcount.remove(path);
                                false
                            } else {
                                true
                            }
                        }
                        None => false,
                    }
                } else {
                    false
                }
            }
            FileBackend::Remote { .. } => false,
        };
        Some((backend, path_still_used))
    }

    fn get(&self, file_id: FileId) -> Option<&FileBackend> {
        self.files.get(&file_id)
    }

    fn get_mut(&mut self, file_id: FileId) -> Option<&mut FileBackend> {
        self.files.get_mut(&file_id)
    }

    fn get_local(&self, file_id: FileId) -> Option<&LocalFile> {
        self.get(file_id).and_then(FileBackend::as_local)
    }

    fn local_values(&self) -> impl Iterator<Item = &LocalFile> {
        self.files.values().filter_map(FileBackend::as_local)
    }

    fn local_iter_mut(&mut self) -> impl Iterator<Item = (&FileId, &mut LocalFile)> {
        self.files
            .iter_mut()
            .filter_map(|(id, backend)| match backend {
                FileBackend::Local(f) => Some((id, f)),
                FileBackend::Remote { .. } => None,
            })
    }
}

/// Tracks which file paths belong to which repository roots, with O(1) repo cleanup checks.
///
/// Encapsulates the path-to-repo mapping and repo path count together so callers
/// cannot forget to maintain the count invariant.
#[derive(Default)]
struct RepoPathMappingState {
    /// Maps file path to its repository root for quick lookup during cleanup.
    path_to_repo: HashMap<PathBuf, PathBuf>,
    /// Tracks how many distinct paths reference each repo root.
    repo_path_count: HashMap<PathBuf, usize>,
}

impl RepoPathMappingState {
    fn insert(&mut self, path: PathBuf, repo_root: PathBuf) {
        // Only increment count if this is a genuinely new path entry.
        if self.path_to_repo.insert(path, repo_root.clone()).is_none() {
            *self.repo_path_count.entry(repo_root).or_insert(0) += 1;
        }
    }

    fn remove(&mut self, path: &Path) -> Option<(PathBuf, bool)> {
        if let Some(repo_root) = self.path_to_repo.remove(path) {
            let mut unused_repo = true;
            if let Some(count) = self.repo_path_count.get_mut(&repo_root) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.repo_path_count.remove(&repo_root);
                } else {
                    unused_repo = false;
                }
            }

            Some((repo_root, unused_repo))
        } else {
            None
        }
    }

    /// Returns all paths mapped to the given repository root.
    fn paths_for_repo(&self, repo_root: &Path) -> Vec<PathBuf> {
        self.path_to_repo
            .iter()
            .filter(|(_, root)| root.as_path() == repo_root)
            .map(|(path, _)| path.clone())
            .collect()
    }
}

pub struct FileModel {
    file_state: FileState,
    abort_handles: HashMap<FileId, SpawnedFutureHandle>,
    watcher: ModelHandle<BulkFilesystemWatcher>,
    /// Maps repository root path to its subscription. One subscription per repo.
    repo_subscriptions: HashMap<PathBuf, RepositorySubscription>,
    repo_path_mapping: RepoPathMappingState,
}

impl FileModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let watcher =
            ctx.add_model(|ctx| BulkFilesystemWatcher::new(Duration::from_millis(200), ctx));

        ctx.subscribe_to_model(&watcher, |me, event, ctx| {
            me.handle_watcher_event(event, ctx);
        });

        Self {
            watcher,
            file_state: FileState::default(),
            abort_handles: HashMap::new(),
            repo_subscriptions: HashMap::new(),
            repo_path_mapping: RepoPathMappingState::default(),
        }
    }

    #[cfg(feature = "test-util")]
    pub fn get_future_handle(&self, file_id: FileId) -> Option<SpawnedFutureHandle> {
        self.abort_handles.get(&file_id).cloned()
    }

    fn handle_watcher_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let updated_files = event.added_or_updated_set();
        self.reload_file_paths(updated_files, ctx);
    }

    pub fn file_path(&self, file_id: FileId) -> Option<PathBuf> {
        self.file_state
            .get_local(file_id)
            .and_then(|x| x.path.clone())
    }

    /// Register a remote file path and return a `FileId`.
    ///
    /// The returned `FileId` can be used with `save()` and `delete()` which
    /// will dispatch to the remote backend via [`RemoteServerClient`].
    pub fn register_remote_file(&mut self, host_id: HostId, path: StandardizedPath) -> FileId {
        let file_id = FileId::new();
        self.file_state.insert_remote(file_id, host_id, path);
        file_id
    }

    /// Register a file path and immediately return a FileId without loading the file.
    /// This is useful when you need a FileId in a constructor but don't need to load the file. This will
    /// also not opt-in to receive file watcher updates.
    pub fn register_file_path(
        &mut self,
        file_path: &Path,
        subscribe_to_updates: bool,
        ctx: &mut ModelContext<Self>,
    ) -> FileId {
        let file_id = FileId::new();

        let watcher_type = if subscribe_to_updates {
            // Try to use repository-level watching if available
            if let Some(repo_root) = self.get_or_create_repo_subscription(file_path, ctx) {
                self.repo_path_mapping
                    .insert(file_path.to_path_buf(), repo_root);
                WatcherType::Repository
            } else {
                // Fallback to individual file watcher
                self.watcher.update(ctx, |watcher, _ctx| {
                    std::mem::drop(watcher.register_path(
                        file_path,
                        WatchFilter::accept_all(),
                        RecursiveMode::Recursive,
                    ));
                });
                WatcherType::Individual
            }
        } else {
            WatcherType::None
        };

        let local_file = LocalFile::new(file_path.to_owned(), watcher_type);
        self.file_state.insert_local(file_id, local_file);

        file_id
    }

    /// Open a file to get its content asynchronously. This also opts in to receiving file watcher updates.
    pub fn open(
        &mut self,
        file_path: &Path,
        subscribe_to_updates: bool,
        ctx: &mut ModelContext<Self>,
    ) -> FileId {
        let file_id = FileId::new();

        // Determine watcher type before spawning async work
        let watcher_type = if subscribe_to_updates {
            // Try to use repository-level watching if available
            if let Some(repo_root) = self.get_or_create_repo_subscription(file_path, ctx) {
                self.repo_path_mapping
                    .insert(file_path.to_path_buf(), repo_root);
                WatcherType::Repository
            } else {
                // Will register individual watcher after file loads
                WatcherType::Individual
            }
        } else {
            WatcherType::None
        };

        let file_path_buf = file_path.to_owned();
        let file_path_clone = file_path_buf.clone();
        let use_individual_watcher = watcher_type == WatcherType::Individual;
        let future = ctx.spawn(
            async move {
                let contents = async_fs::read_to_string(&file_path_buf)
                    .await
                    .map_err(FileLoadError::from);
                (file_id, contents)
            },
            move |me, (file_id, load_result), ctx| match load_result {
                Ok(content) => {
                    let version = ContentVersion::new();
                    me.set_version(file_id, version);

                    // Only register individual watcher if not using repo subscription.
                    // Watch the parent directory (NonRecursive) instead of the file
                    // itself so the watch survives editors that use a
                    // delete+create/rename pattern (vim, sed -i, etc.). Watching
                    // the file directly would lose the inotify watch when the
                    // original inode is deleted.
                    if use_individual_watcher {
                        let watch_path = file_path_clone
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| file_path_clone.clone());
                        me.watcher.update(ctx, |watcher, _ctx| {
                            std::mem::drop(watcher.register_path(
                                &watch_path,
                                WatchFilter::accept_all(),
                                RecursiveMode::NonRecursive,
                            ));
                        });
                    }

                    ctx.emit(FileModelEvent::FileLoaded {
                        content,
                        id: file_id,
                        version,
                    });
                }
                Err(err) => {
                    ctx.emit(FileModelEvent::FailedToLoad {
                        id: file_id,
                        error: Rc::new(err),
                    });
                }
            },
        );

        let local_file = LocalFile::new(file_path.to_owned(), watcher_type);
        self.file_state.insert_local(file_id, local_file);

        self.abort_handles.insert(file_id, future);
        file_id
    }

    pub async fn read_content_for_file(file_path: &Path) -> Result<String, FileLoadError> {
        if !Self::file_exists(file_path).await {
            return Err(FileLoadError::DoesNotExist);
        }
        async_fs::read_to_string(file_path)
            .await
            .map_err(FileLoadError::from)
    }

    /// Asynchronously reads specific lines from a file using BufReader.
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to read
    /// * `line_numbers` - A list of 0-based line numbers to retrieve. Supports non-consecutive lines.
    ///
    /// # Returns
    /// A vector of (line_number, line_content) tuples for each requested line that exists.
    /// Lines that don't exist in the file are omitted from the result.
    pub async fn read_lines_async(
        file_path: &Path,
        line_numbers: Vec<usize>,
    ) -> Result<Vec<(usize, String)>, FileLoadError> {
        use std::collections::HashSet;

        if line_numbers.is_empty() {
            return Ok(Vec::new());
        }

        if !Self::file_exists(file_path).await {
            return Err(FileLoadError::DoesNotExist);
        }

        let requested_lines: HashSet<usize> = line_numbers.iter().copied().collect();
        let max_line = *line_numbers.iter().max().unwrap();

        let file = async_fs::File::open(file_path)
            .await
            .map_err(FileLoadError::from)?;
        let reader = BufReader::new(file);
        let mut lines_stream = reader.lines();

        let mut result = Vec::with_capacity(line_numbers.len());
        let mut current_line = 0usize;

        while let Some(line_result) = lines_stream.next().await {
            if current_line > max_line {
                break;
            }

            if requested_lines.contains(&current_line) {
                let line = line_result.map_err(FileLoadError::from)?;
                result.push((current_line, line));
            }

            current_line += 1;
        }

        Ok(result)
    }

    /// Reads a text file by async-streaming its lines, respecting a byte budget
    /// and optional line ranges. Returns [`TextFileReadResult::NotText`] if the
    /// file is not valid UTF-8.
    ///
    /// **Line ending normalization**: `\n` and `\r\n` line endings are normalized
    /// to `\n` (LF) in the returned content. Classic Mac `\r`-only line endings
    /// are **not** recognized as line separators (matching `read_line()` behavior).
    /// A trailing newline at the end of the file is preserved so that
    /// round-tripping content through this reader and writing it back does not
    /// silently drop the final newline.
    ///
    /// This is a modified version of the loop that [`futures::io::BufReader::lines()`]
    /// uses internally (i.e. repeated `read_line()` calls with newline stripping),
    /// but additionally tracks whether each line was terminated by a newline so
    /// the accumulator can preserve the file's trailing newline.
    pub async fn read_text_file(
        path: &Path,
        max_bytes: usize,
        requested_ranges: &[Range<usize>],
        last_modified: Option<SystemTime>,
    ) -> anyhow::Result<TextFileReadResult> {
        let file = match async_fs::File::open(path).await {
            Ok(file) => file,
            Err(e) => return Err(anyhow::anyhow!(e)),
        };
        let mut reader = futures::io::BufReader::new(file);

        let file_name = path.to_string_lossy().to_string();
        let mut accumulator = text_file_reader::TextFileAccumulator::new(
            file_name,
            last_modified,
            requested_ranges,
            max_bytes,
        );

        // Use `read_line()` instead of `lines()` so we can detect whether each
        // line was terminated by a newline. `lines()` strips this information,
        // which caused trailing newlines to be silently dropped.
        let mut line_buf = String::new();
        loop {
            line_buf.clear();
            let bytes_read = match reader.read_line(&mut line_buf).await {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    // Not valid UTF-8.
                    return Ok(TextFileReadResult::NotText);
                }
                Err(e) => return Err(anyhow::anyhow!(e)),
            };
            if bytes_read == 0 {
                break; // EOF
            }

            // Strip the line terminator (`\n` or `\r\n`) and record whether
            // one was present. Note: `read_line()` only splits on `\n`, so
            // standalone `\r` (classic Mac) is not treated as a line separator.
            let has_newline = line_buf.ends_with('\n');
            if has_newline {
                line_buf.pop();
            }
            if line_buf.ends_with('\r') {
                line_buf.pop();
            }

            accumulator.push_line(std::mem::take(&mut line_buf), has_newline);
        }

        let (segments, bytes_read) = accumulator.finalize();
        Ok(TextFileReadResult::Segments {
            segments,
            bytes_read,
        })
    }

    pub async fn read_file_as_binary(file_path: &Path) -> Result<Vec<u8>, FileLoadError> {
        if !Self::file_exists(file_path).await {
            return Err(FileLoadError::DoesNotExist);
        }

        async_fs::read(file_path).await.map_err(FileLoadError::from)
    }

    pub async fn file_exists(file_path: &Path) -> bool {
        async_fs::metadata(file_path).await.is_ok()
    }

    pub async fn create_file(file_path: &Path) -> Result<(), io::Error> {
        async_fs::File::create(file_path).await.map(|_| ())
    }

    /// Ensures all parent directories of the given path exist, creating them if necessary.
    pub async fn ensure_parent_directories(path: &Path) -> Result<(), io::Error> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                async_fs::create_dir_all(parent).await?;
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self, file_id: FileId) {
        if let Some(future) = self.abort_handles.remove(&file_id) {
            future.abort()
        }
    }

    pub fn unsubscribe(&mut self, file_id: FileId, ctx: &mut ModelContext<Self>) {
        self.abort_handles.remove(&file_id);
        if let Some((FileBackend::Local(file), path_still_used)) = self.file_state.remove(file_id) {
            let path = file.path;
            let watcher_type = file.watcher_type;

            if let Some(ref path) = path {
                if !path_still_used {
                    match watcher_type {
                        WatcherType::Individual => {
                            // Unwatch the parent directory (matching the register
                            // in open() which watches the parent, not the file).
                            // Only unregister if no other individually-watched
                            // files share the same parent directory.
                            let watch_path = path
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|| path.clone());
                            let other_files_share_parent =
                                self.file_state.local_values().any(|f| {
                                    f.watcher_type == WatcherType::Individual
                                        && f.path
                                            .as_deref()
                                            .and_then(|p| p.parent())
                                            .map(|p| p == watch_path)
                                            .unwrap_or(false)
                                });
                            if !other_files_share_parent {
                                self.watcher.update(ctx, |watcher, _ctx| {
                                    std::mem::drop(watcher.unregister_path(&watch_path));
                                });
                            }
                        }
                        WatcherType::Repository => {
                            if let Some((repo_root, unused_repo)) =
                                self.repo_path_mapping.remove(path)
                            {
                                if unused_repo {
                                    self.unsubscribe_from_repo(&repo_root, ctx);
                                }
                            }
                        }
                        WatcherType::None => {}
                    }
                }
            }
        }
        // Remote files have no watcher to clean up.
    }

    pub fn save(
        &mut self,
        file_id: FileId,
        content: String,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        let backend = self
            .file_state
            .get(file_id)
            .ok_or(FileSaveError::NoFilePath(file_id))?;

        match backend {
            FileBackend::Local(_) => {
                let file_path = self
                    .file_path(file_id)
                    .ok_or(FileSaveError::NoFilePath(file_id))?;

                ctx.spawn(
                    async move {
                        if let Err(err) = Self::ensure_parent_directories(&file_path).await {
                            return Err(FileSaveError::IOError {
                                error: err,
                                path: file_path,
                            });
                        }
                        async_fs::write(&file_path, content).await.map_err(|err| {
                            FileSaveError::IOError {
                                error: err,
                                path: file_path,
                            }
                        })
                    },
                    move |me, write_result: Result<(), FileSaveError>, ctx| {
                        match write_result {
                            Ok(_) => {
                                me.set_version(file_id, version);
                                ctx.emit(FileModelEvent::FileSaved {
                                    id: file_id,
                                    version,
                                })
                            }
                            Err(err) => ctx.emit(FileModelEvent::FailedToSave {
                                id: file_id,
                                error: Rc::new(err),
                            }),
                        };
                    },
                );
            }
            FileBackend::Remote { host_id, path } => {
                let client = Self::resolve_remote_client(host_id, ctx)?;
                let path = path.as_str().to_string();
                let future = async move {
                    client
                        .write_file(path, content)
                        .await
                        .map_err(|e| e.to_string())
                };
                ctx.spawn(
                    future,
                    move |me, result: Result<(), String>, ctx| match result {
                        Ok(()) => {
                            me.set_version(file_id, version);
                            ctx.emit(FileModelEvent::FileSaved {
                                id: file_id,
                                version,
                            });
                        }
                        Err(err) => {
                            ctx.emit(FileModelEvent::FailedToSave {
                                id: file_id,
                                error: Rc::new(FileSaveError::RemoteError(err)),
                            });
                        }
                    },
                );
            }
        }

        Ok(())
    }

    /// Renames a file and also saves its content.
    // TODO: refactor this against [`FileModel::save`].
    pub fn rename_and_save(
        &mut self,
        file_id: FileId,
        new_path: PathBuf,
        content: String,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        let file_path = self
            .file_path(file_id)
            .ok_or(FileSaveError::NoFilePath(file_id))?;

        ctx.spawn(
            async move {
                // Make sure the file we're renaming exists.
                if let Err(err) = async_fs::metadata(&file_path).await {
                    return Err(FileSaveError::IOError {
                        error: err,
                        path: file_path,
                    });
                }

                // Make sure the components of the path we're moving to exists.
                if let Err(err) = Self::ensure_parent_directories(&new_path).await {
                    return Err(FileSaveError::IOError {
                        error: err,
                        path: new_path,
                    });
                }

                // Write the updated contents to the old path first.
                async_fs::write(&file_path, content).await.map_err(|err| {
                    FileSaveError::IOError {
                        error: err,
                        path: file_path.clone(),
                    }
                })?;

                // Now rename.
                async_fs::rename(&file_path, &new_path)
                    .await
                    .map_err(|err| FileSaveError::IOError {
                        error: err,
                        path: file_path.clone(),
                    })
            },
            move |me, write_result: Result<(), FileSaveError>, ctx| {
                match write_result {
                    Ok(_) => {
                        me.set_version(file_id, version);
                        ctx.emit(FileModelEvent::FileSaved {
                            id: file_id,
                            version,
                        })
                    }
                    Err(err) => ctx.emit(FileModelEvent::FailedToSave {
                        id: file_id,
                        error: Rc::new(err),
                    }),
                };
            },
        );

        Ok(())
    }

    /// Deletes the specified file.
    pub fn delete(
        &mut self,
        file_id: FileId,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        let backend = self
            .file_state
            .get(file_id)
            .ok_or(FileSaveError::NoFilePath(file_id))?;

        match backend {
            FileBackend::Local(_) => {
                let file_path = self
                    .file_path(file_id)
                    .ok_or(FileSaveError::NoFilePath(file_id))?;

                ctx.spawn(
                    async move {
                        if let Err(err) = Self::ensure_parent_directories(&file_path).await {
                            return Err(FileSaveError::IOError {
                                error: err,
                                path: file_path,
                            });
                        }
                        async_fs::remove_file(&file_path).await.map_err(|err| {
                            FileSaveError::IOError {
                                error: err,
                                path: file_path,
                            }
                        })
                    },
                    move |me, delete_result: Result<(), FileSaveError>, ctx| {
                        match delete_result {
                            Ok(_) => {
                                me.set_version(file_id, version);
                                ctx.emit(FileModelEvent::FileSaved {
                                    id: file_id,
                                    version,
                                })
                            }
                            Err(err) => ctx.emit(FileModelEvent::FailedToSave {
                                id: file_id,
                                error: Rc::new(err),
                            }),
                        };
                    },
                );
            }
            FileBackend::Remote { host_id, path } => {
                let client = Self::resolve_remote_client(host_id, ctx)?;
                let path = path.as_str().to_string();
                let future =
                    async move { client.delete_file(path).await.map_err(|e| e.to_string()) };
                ctx.spawn(
                    future,
                    move |me, result: Result<(), String>, ctx| match result {
                        Ok(()) => {
                            me.set_version(file_id, version);
                            ctx.emit(FileModelEvent::FileSaved {
                                id: file_id,
                                version,
                            });
                        }
                        Err(err) => {
                            ctx.emit(FileModelEvent::FailedToSave {
                                id: file_id,
                                error: Rc::new(FileSaveError::RemoteError(err)),
                            });
                        }
                    },
                );
            }
        }

        Ok(())
    }

    /// Look up the `RemoteServerClient` for a given host at call time.
    fn resolve_remote_client(
        host_id: &HostId,
        ctx: &AppContext,
    ) -> Result<std::sync::Arc<RemoteServerClient>, FileSaveError> {
        RemoteServerManager::as_ref(ctx)
            .client_for_host(host_id)
            .cloned()
            .ok_or_else(|| {
                FileSaveError::RemoteError(format!("Remote host {host_id} is not connected"))
            })
    }

    pub fn set_version(&mut self, file_id: FileId, version: ContentVersion) {
        if let Some(backend) = self.file_state.get_mut(file_id) {
            backend.set_version(version);
        }
    }

    pub fn version(&self, file_id: FileId) -> Option<ContentVersion> {
        self.file_state.get(file_id).and_then(|b| b.version())
    }

    /// Checks if a repository subscription exists for the given path, and creates one if needed.
    /// Returns the repository root path if a subscription exists or was created.
    fn get_or_create_repo_subscription(
        &mut self,
        file_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> Option<PathBuf> {
        // Check if we already have a subscription for a repo containing this path
        for repo_root in self.repo_subscriptions.keys() {
            if file_path.starts_with(repo_root) {
                return Some(repo_root.clone());
            }
        }

        // Try to find a repository for this path
        let repository =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(file_path, ctx)?;

        let repo_root = repository.as_ref(ctx).root_dir().to_local_path_lossy();

        // Create a new subscription
        let (repository_update_tx, repository_update_rx) = async_channel::unbounded();
        let start = repository.update(ctx, |repo, ctx| {
            repo.start_watching(
                Box::new(FileRepositorySubscriber {
                    repository_update_tx,
                }),
                ctx,
            )
        });
        let subscriber_id = start.subscriber_id;

        let repo_root_for_handler = repo_root.clone();
        let repository_for_handler = repository.clone();
        ctx.spawn(start.registration_future, move |me, result, ctx| match result {
            Ok(()) => {
                log::info!(
                    "FileModel subscribed to repository: {}",
                    repo_root_for_handler.display()
                );
            }
            Err(err) => {
                log::warn!(
                    "Failed to subscribe to repository {}: {}, falling back to individual file watchers",
                    repo_root_for_handler.display(),
                    err
                );

                repository_for_handler.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });

                // Remove the subscription entry since it failed
                me.repo_subscriptions.remove(&repo_root_for_handler);

                // Fall back to individual file watchers for all files that were expecting this repo
                me.fallback_to_individual_watchers(&repo_root_for_handler, ctx);
            }
        });

        // Set up the stream handler for repository updates
        let repo_root_for_stream = repo_root.clone();
        ctx.spawn_stream_local(
            repository_update_rx,
            move |me, update, ctx| {
                me.handle_repository_update(&repo_root_for_stream, update, ctx);
            },
            |_, _| {},
        );

        // Store the subscription (subscriber_id is available immediately; registration completion is async)
        self.repo_subscriptions.insert(
            repo_root.clone(),
            RepositorySubscription {
                repository,
                subscriber_id: Some(subscriber_id),
            },
        );

        Some(repo_root)
    }

    /// Handles file updates from a repository subscription.
    fn handle_repository_update(
        &mut self,
        _repo_root: &Path,
        update: RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        if update.is_empty() {
            return;
        }

        // Collect paths that need content reloading:
        // - Added files that match tracked paths
        // - Moved files where the to_target matches a tracked path
        let mut paths_to_reload: HashSet<PathBuf> = update
            .added_or_modified()
            .filter(|target| !target.is_ignored)
            .map(|target| target.path.clone())
            .collect();

        for to_target in update.moved.keys() {
            if !to_target.is_ignored {
                paths_to_reload.insert(to_target.path.clone());
            }
        }

        // Reload content for all affected paths
        self.reload_file_paths(paths_to_reload, ctx);

        // Handle deleted files - log that the file's backing is gone
        for target in &update.deleted {
            if target.is_ignored {
                continue;
            }
            let has_file = self
                .file_state
                .local_values()
                .any(|file| file.path.as_ref() == Some(&target.path));
            if has_file {
                log::info!(
                    "File's backing was deleted, file is now orphaned: {}",
                    target.path.display()
                );
            }
        }

        // Handle moved/renamed files - log that the file's backing was moved (for from_target)
        for (to_target, from_target) in &update.moved {
            if from_target.is_ignored {
                continue;
            }
            let has_file = self
                .file_state
                .local_values()
                .any(|file| file.path.as_ref() == Some(&from_target.path));
            if has_file {
                log::info!(
                    "File's backing was moved from {} to {}, file is now orphaned",
                    from_target.path.display(),
                    to_target.path.display()
                );
            }
        }
    }

    /// Reloads file content if the given path matches any tracked file.
    fn reload_file_paths(&mut self, file_paths: HashSet<PathBuf>, ctx: &mut ModelContext<Self>) {
        // First filter to keep only the paths that are tracked.
        let matching_files: Vec<PathBuf> = file_paths
            .into_iter()
            .filter_map(|path| {
                self.file_state
                    .local_values()
                    .any(|file| file.should_receive_update_for_path(&path))
                    .then_some(path)
            })
            .collect();

        if matching_files.is_empty() {
            return;
        }

        // Autoreload modified files.
        ctx.spawn(
            async move {
                let mut res = Vec::new();
                for file_path in matching_files {
                    if let Ok(content) = async_fs::read_to_string(&file_path).await {
                        res.push((file_path, content));
                    }
                }
                res
            },
            move |me, res, ctx| {
                for (file_path, content) in res {
                    let mut emitted_event = false;
                    for (file_id, file_state) in me.file_state.local_iter_mut() {
                        // Only set the new version of a file if it has opt-in to receiving updates.
                        if file_state.should_receive_update_for_path(&file_path) {
                            let new_version = ContentVersion::new();
                            ctx.emit(FileModelEvent::FileUpdated {
                                id: *file_id,
                                content: content.clone(),
                                base_version: file_state.version.expect("Version should be some"),
                                new_version,
                            });
                            emitted_event = true;
                            file_state.version = Some(new_version);
                        }
                    }

                    if !emitted_event {
                        log::warn!(
                            "{} is changed but there is no handler for the update event",
                            file_path.display()
                        );
                    }
                }
            },
        );
    }

    /// Falls back to individual file watchers for all files that were expecting to use the given repository.
    /// Called when a repository subscription fails.
    fn fallback_to_individual_watchers(&mut self, repo_root: &Path, ctx: &mut ModelContext<Self>) {
        let affected_paths = self.repo_path_mapping.paths_for_repo(repo_root);

        for path in affected_paths {
            self.repo_path_mapping.remove(&path);

            // Find the file(s) at this path and update their watcher type
            for (_, file) in self.file_state.local_iter_mut() {
                if file.path.as_ref() == Some(&path) && file.watcher_type == WatcherType::Repository
                {
                    file.watcher_type = WatcherType::Individual;
                }
            }

            // Register individual file watcher
            self.watcher.update(ctx, |watcher, _ctx| {
                std::mem::drop(watcher.register_path(
                    &path,
                    WatchFilter::accept_all(),
                    RecursiveMode::Recursive,
                ));
            });
        }
    }

    /// Checks if any files still reference the given repository, and unsubscribes if not.
    fn unsubscribe_from_repo(&mut self, repo_root: &Path, ctx: &mut ModelContext<Self>) {
        if let Some(subscription) = self.repo_subscriptions.remove(repo_root) {
            log::info!(
                "No more files in repository {}, unsubscribing",
                repo_root.display()
            );
            if let Some(subscriber_id) = subscription.subscriber_id {
                subscription.repository.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });
            }
        }
    }
}

impl Entity for FileModel {
    type Event = FileModelEvent;
}

impl SingletonEntity for FileModel {}

/// Subscriber for repository file change events, forwarding updates to FileModel.
struct FileRepositorySubscriber {
    repository_update_tx: Sender<RepositoryUpdate>,
}

impl RepositorySubscriber for FileRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // Files are already loaded when subscription starts, so we don't need to do anything.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let tx = self.repository_update_tx.clone();
        let update = update.clone();
        Box::pin(async move {
            let _ = tx.send(update).await;
        })
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
