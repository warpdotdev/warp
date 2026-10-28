use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ai::index::build_outline;
use anyhow::Context as _;
use async_channel::Sender;
use futures::stream::AbortHandle;
use instant::Instant;
use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
use repo_metadata::repository::{
    BufferingRepositorySubscriber, RepositorySubscriber, SubscriberId,
};
use repo_metadata::{
    CanonicalizedPath, DirectoryWatcher, Repository, RepositoryUpdate, RepositoryWatchMode,
};
use settings::Setting as _;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::OutlineStatus;
use crate::ai::persisted_workspace::all_working_directories;
use crate::settings::{
    AISettings, AISettingsChangedEvent, CodeSettings, CodeSettingsChangedEvent, InputSettings,
    InputSettingsChangedEvent,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{TelemetryEvent, safe_info, safe_warn, send_telemetry_from_ctx};

/// State for a repository outline, containing both the repository handle and the outline status.
#[derive(Debug)]
struct OutlineState {
    /// Handle to the repository model.
    repository: ModelHandle<Repository>,
    /// Current status of the outline.
    status: OutlineStatus,
    /// Subscriber ID for repository updates (if watching).
    subscriber_id: Option<SubscriberId>,
    generation: u64,
}

pub enum RepoOutlinesEvent {
    OutlinesUpdated(PathBuf),
}

const MAX_REPO_FILE_SIZE_LIMIT: usize = 5000;
const MAX_RETAINED_REPO_OUTLINES: usize = 3;

#[derive(Default)]
struct RepoRecency {
    paths: VecDeque<PathBuf>,
}

impl RepoRecency {
    fn touch(&mut self, path: &Path) -> Option<PathBuf> {
        if let Some(position) = self.paths.iter().position(|candidate| candidate == path) {
            self.paths.remove(position);
        }
        self.paths.push_back(path.to_path_buf());

        (self.paths.len() > MAX_RETAINED_REPO_OUTLINES)
            .then(|| self.paths.pop_front())
            .flatten()
    }

    fn clear(&mut self) {
        self.paths.clear();
    }
}

struct ActiveOutlineTask {
    repo_path: PathBuf,
    generation: u64,
    abort_handle: AbortHandle,
}
pub struct RepoOutlines {
    outlines: HashMap<PathBuf, OutlineState>,
    repo_recency: RepoRecency,

    /// Queue of paths to be scanned for git repo outlines.
    outline_queue: VecDeque<PathBuf>,

    active_outline_task: Option<ActiveOutlineTask>,

    indexing_enabled: bool,
    next_generation: u64,
}

const REPO_WATCHER_DEBOUNCE_DURATION: Duration = Duration::from_secs(10);

impl RepoOutlines {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self::new_with_indexing_enabled(true, ctx)
    }

    pub fn new_with_indexing_enabled(indexing_enabled: bool, ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = event {
                Self::handle_setting_change_event(me, ctx);
            }
        });

        ctx.subscribe_to_model(&CodeSettings::handle(ctx), |me, _, event, ctx| {
            if let CodeSettingsChangedEvent::CodebaseContextEnabled { .. } = event {
                Self::handle_setting_change_event(me, ctx);
            }
        });

        if indexing_enabled
            && !cfg!(any(
                test,
                feature = "fast_dev",
                feature = "integration_tests"
            ))
        {
            ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo {
                    repository,
                    source: _,
                } = event;
                me.index_repo(repository.clone(), ctx);
            });
        }

        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, _, event, ctx| {
            if let InputSettingsChangedEvent::OutlineCodebaseSymbolsForAtContextMenu { .. } = event
            {
                Self::handle_setting_change_event(me, ctx);
            }
        });

        Self {
            outlines: Default::default(),
            repo_recency: Default::default(),
            outline_queue: Default::default(),
            active_outline_task: Default::default(),
            indexing_enabled,
            next_generation: 0,
        }
    }

    #[allow(dead_code)]
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            outlines: Default::default(),
            repo_recency: Default::default(),
            outline_queue: Default::default(),
            active_outline_task: Default::default(),
            indexing_enabled: true,
            next_generation: 0,
        }
    }

    fn index_repo(&mut self, repository: ModelHandle<Repository>, ctx: &mut ModelContext<Self>) {
        let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();
        if let Some(retained_repo_path) = self
            .get_outline_internal(&repo_path)
            .map(|(_, retained_repo_path)| retained_repo_path)
        {
            self.repo_recency.touch(&retained_repo_path);
        } else if self.should_build_outlines(ctx) && !self.outline_queue.contains(&repo_path) {
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1);
            let outline_state = OutlineState {
                repository,
                status: OutlineStatus::Pending,
                subscriber_id: None,
                generation,
            };
            self.outline_queue.push_back(repo_path.clone());
            self.retain_outline_state(repo_path, outline_state, ctx);
            self.compute_next_outline(ctx);
        }
    }
    fn retain_outline_state(
        &mut self,
        repo_path: PathBuf,
        outline_state: OutlineState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.outlines.insert(repo_path.clone(), outline_state);
        if let Some(evicted_path) = self.repo_recency.touch(&repo_path) {
            self.evict_repo(&evicted_path, ctx);
        }
    }

    fn evict_repo(&mut self, repo_path: &Path, ctx: &mut ModelContext<Self>) {
        self.outline_queue
            .retain(|queued_path| queued_path != repo_path);
        if self
            .active_outline_task
            .as_ref()
            .is_some_and(|task| task.repo_path == repo_path)
            && let Some(task) = self.active_outline_task.take()
        {
            task.abort_handle.abort();
        }
        let Some(mut state) = self.outlines.remove(repo_path) else {
            return;
        };

        if let Some(subscriber_id) = state.subscriber_id.take() {
            state.repository.update(ctx, |repo, ctx| {
                repo.stop_watching(subscriber_id, ctx);
            });
        }
        ctx.emit(RepoOutlinesEvent::OutlinesUpdated(repo_path.to_path_buf()));
    }

    /// Check if outlines should be built based on if codebase context enabled OR
    /// outline codebase symbols for @ context menu settings.
    fn should_build_outlines(&self, ctx: &ModelContext<Self>) -> bool {
        self.indexing_enabled
            && (UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx)
                || *InputSettings::as_ref(ctx)
                    .outline_codebase_symbols_for_at_context_menu
                    .value())
    }

    fn handle_setting_change_event(me: &mut RepoOutlines, ctx: &mut ModelContext<Self>) {
        if me.should_build_outlines(ctx) {
            // Add all working directories to the queue and start processing.
            for dir in all_working_directories(ctx).into_iter() {
                if let Some(repository) =
                    DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&dir, ctx)
                {
                    me.index_repo(repository, ctx);
                }
            }
        } else {
            // Unsubscribe from repository updates and clear all outlines.
            for state in me.outlines.values_mut() {
                if let Some(subscriber_id) = state.subscriber_id.take() {
                    state.repository.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }

            me.outlines = HashMap::default();
            me.repo_recency.clear();
            me.outline_queue = VecDeque::default();
            if let Some(task) = me.active_outline_task.take() {
                task.abort_handle.abort();
            }
        }
    }

    /// Returns the `OutlineStatus` for the given path, if any.
    pub fn get_outline(&self, path: &Path) -> Option<(&OutlineStatus, PathBuf)> {
        let Ok(canonicalized_path) = dunce::canonicalize(path) else {
            return None;
        };
        self.get_outline_internal(&canonicalized_path)
    }

    /// Returns the `OutlineStatus` for the given path, if any. The input path has to be canonicalized.
    fn get_outline_internal(&self, path: &Path) -> Option<(&OutlineStatus, PathBuf)> {
        let mut path = path.to_owned();

        loop {
            if let Some(outline_state) = self.outlines.get(&path) {
                return Some((&outline_state.status, path));
            }

            if !path.pop() {
                break;
            }
        }
        None
    }

    pub fn is_directory_indexed(&self, directory: &Path) -> bool {
        self.get_outline(directory)
            .is_some_and(|(status, _)| matches!(status, OutlineStatus::Complete(_)))
    }

    /// Computes the outline for the repo containing the next path in the queue, if any.
    fn compute_next_outline(&mut self, ctx: &mut ModelContext<Self>) {
        if self.should_build_outlines(ctx)
            && self.active_outline_task.is_none()
            && let Some(repo_root) = self.outline_queue.pop_front()
        {
            let Some(generation) = self.outlines.get(&repo_root).map(|state| state.generation)
            else {
                self.compute_next_outline(ctx);
                return;
            };
            self.compute_outline_for_repo(repo_root, generation, ctx);
        }
    }

    /// Computes the outline for the repo with the given root path.
    ///
    /// `repo_root` is assumed to be the root of a code repository.
    fn compute_outline_for_repo(
        &mut self,
        repo_root: PathBuf,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        let root_path_clone = repo_root.clone();
        let active_repo_path = repo_root.clone();

        let scan_start = Instant::now();
        let scan_abort_handle = ctx
            .spawn(
                async move {
                    safe_info!(
                        safe: ("Parsing symbols for repo outline."),
                        full: ("Parsing symbols for repo at {}", repo_root.display())
                    );
                    let canonicalized_path = CanonicalizedPath::try_from(&repo_root)?;
                    build_outline(canonicalized_path.as_path(), Some(MAX_REPO_FILE_SIZE_LIMIT))
                        .await
                        .map(|outline| (canonicalized_path, outline, scan_start.elapsed()))
                },
                move |me, res, ctx| {
                    if !me.active_outline_task.as_ref().is_some_and(|task| {
                        task.repo_path == root_path_clone && task.generation == generation
                    }) {
                        return;
                    }
                    // Don't process this result if the setting has been disabled.
                    // The abort handle doesn't always abort.
                    if me.should_build_outlines(ctx) {
                        match res {
                            Ok((canonicalized_path, outline, parse_duration)) => {
                                let is_current_generation = me
                                    .outlines
                                    .get(canonicalized_path.as_path_buf())
                                    .is_some_and(|state| state.generation == generation);
                                if !is_current_generation {
                                    me.active_outline_task = None;
                                    me.compute_next_outline(ctx);
                                    return;
                                }
                                send_telemetry_from_ctx!(
                                    TelemetryEvent::RepoOutlineConstructionSuccess {
                                        total_parse_seconds: parse_duration.as_secs() as usize,
                                        file_count: outline.file_count(),
                                    },
                                    ctx
                                );

                                safe_info!(
                                    safe: ("Successfully constructed symbols outline for repo."),
                                    full: (
                                        "Successfully constructed symbols outline for repo: {}",
                                        canonicalized_path
                                    )
                                );
                                // Ensure the repository is registered with DirectoryWatcher.
                                let repository_handle = match DirectoryWatcher::handle(ctx)
                                    .update(ctx, |repo_watcher, ctx| {
                                        repo_watcher
                                            .add_directory(canonicalized_path.clone().into(), ctx)
                                    })
                                    .context("Failed to start tracking repository")
                                {
                                    Ok(handle) => handle,
                                    Err(e) => {
                                        report_error!(e);
                                        me.active_outline_task = None;
                                        me.compute_next_outline(ctx);
                                        return;
                                    }
                                };

                                // Start watching repository changes and route updates to this model
                                me.start_repository_subscription(
                                    &repository_handle,
                                    canonicalized_path.as_path_buf().clone(),
                                    generation,
                                    ctx,
                                );

                                if let Some(outline_state) = me
                                    .outlines
                                    .get_mut(canonicalized_path.as_path_buf())
                                    .filter(|state| state.generation == generation)
                                {
                                    outline_state.status = OutlineStatus::Complete(outline);
                                }
                                ctx.emit(RepoOutlinesEvent::OutlinesUpdated(
                                    canonicalized_path.into(),
                                ));
                            }
                            Err(e) => {
                                safe_warn!(
                                    safe: ("Failed to construct symbols outline for repo: {:?}", e),
                                    full: (
                                        "Failed to construct symbols outline for repo at {}: {:?}",
                                        root_path_clone.display(),
                                        e
                                    )
                                );

                                send_telemetry_from_ctx!(
                                    TelemetryEvent::RepoOutlineConstructionFailed {
                                        error: e.to_string()
                                    },
                                    ctx
                                );
                                if let Some(outline_state) = me
                                    .outlines
                                    .get_mut(&root_path_clone)
                                    .filter(|state| state.generation == generation)
                                {
                                    outline_state.status = OutlineStatus::Failed;
                                }
                            }
                        };
                    }

                    me.active_outline_task = None;
                    me.compute_next_outline(ctx);
                },
            )
            .abort_handle();
        self.active_outline_task = Some(ActiveOutlineTask {
            repo_path: active_repo_path,
            generation,
            abort_handle: scan_abort_handle,
        });
    }

    fn start_repository_subscription(
        &mut self,
        repository_handle: &ModelHandle<Repository>,
        repo_path: PathBuf,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        let (repository_update_tx, repository_update_rx) = async_channel::unbounded();
        let start = repository_handle.update(ctx, |repo, ctx| {
            let inner = OutlineRepositorySubscriber {
                repository_update_tx,
            };
            let debounced =
                BufferingRepositorySubscriber::new(inner, REPO_WATCHER_DEBOUNCE_DURATION);
            repo.start_watching(
                RepositoryWatchMode::FilesystemOnly,
                Box::new(debounced),
                ctx,
            )
        });
        let subscriber_id = start.subscriber_id;

        // Store subscriber id so callers can always unsubscribe.
        if let Some(state) = self
            .outlines
            .get_mut(&repo_path)
            .filter(|state| state.generation == generation)
        {
            state.subscriber_id = Some(subscriber_id);
        }

        let repo_path_for_cleanup = repo_path.clone();
        let repository_handle_for_cleanup = repository_handle.downgrade();
        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching repository for outline updates at {}: {err}",
                    repo_path_for_cleanup.display()
                );

                if let Some(repository_handle) = repository_handle_for_cleanup.upgrade(ctx) {
                    repository_handle.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }

                if let Some(state) = me
                    .outlines
                    .get_mut(&repo_path_for_cleanup)
                    .filter(|state| state.generation == generation)
                {
                    state.subscriber_id = None;
                }
            }
        });

        // Process repository updates
        let repo_path_for_updates = repo_path.clone();
        ctx.spawn_stream_local(
            repository_update_rx.clone(),
            move |me, update: RepositoryUpdate, ctx| {
                me.handle_repository_update(&repo_path_for_updates, generation, update, ctx);
            },
            |_, _| {},
        );
    }

    fn handle_repository_update(
        &mut self,
        repo_path: &Path,
        generation: u64,
        update: RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        if update.is_empty() {
            return;
        }
        if self
            .outlines
            .get(repo_path)
            .is_none_or(|state| state.generation != generation)
        {
            return;
        }

        match self.outlines.get_mut(repo_path) {
            Some(OutlineState {
                status: outline_status @ OutlineStatus::Complete(_),
                ..
            }) => {
                let mut outline = OutlineStatus::Pending;
                std::mem::swap(outline_status, &mut outline);
                let repo_path_clone_inner = repo_path.to_path_buf();

                ctx.spawn(
                    async move {
                        if let OutlineStatus::Complete(mut outline) = outline {
                            outline.update(update).await;
                            (outline, repo_path_clone_inner)
                        } else {
                            unreachable!("Expected status to be Complete(outline)")
                        }
                    },
                    move |me, (outline, repo_path), ctx| {
                        if let Some(state) = me
                            .outlines
                            .get_mut(&repo_path)
                            .filter(|state| state.generation == generation)
                        {
                            state.status = OutlineStatus::Complete(outline);
                            ctx.emit(RepoOutlinesEvent::OutlinesUpdated(repo_path));
                        }
                    },
                );
            }
            Some(_) => {
                log::warn!("Failed to update repo outline: repo outline failed or is pending")
            }
            None => log::warn!("Failed to update repo outline: repo outline not found"),
        }
    }
}

impl Entity for RepoOutlines {
    type Event = RepoOutlinesEvent;
}

impl SingletonEntity for RepoOutlines {}

struct OutlineRepositorySubscriber {
    repository_update_tx: Sender<RepositoryUpdate>,
}

impl RepositorySubscriber for OutlineRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::prelude::rust_2024::Future<Output = ()> + Send + 'static>> {
        // The model can safely ignore the initial scan because we trigger our own initial outline build.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &repo_metadata::RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::prelude::rust_2024::Future<Output = ()> + Send + 'static>> {
        let tx = self.repository_update_tx.clone();
        let update = update.clone();
        Box::pin(async move {
            let _ = tx.send(update).await;
        })
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
