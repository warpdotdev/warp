use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::LazyLock;

use async_channel::Sender;
use futures::Future;
use futures::stream::AbortHandle;
use regex::Regex;
use repo_metadata::RepositoryWatchMode;
use repo_metadata::repositories::{
    DetectedRepositories, DetectedRepositoriesEvent, RepoDetectionSource,
};
use repo_metadata::repository::{Repository, RepositorySubscriber, SubscriberId};
use repo_metadata::watcher::{DirectoryWatcher, RepositoryUpdate};
use strum::IntoEnumIterator;
use warp_core::safe_warn;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};
use watcher::HomeDirectoryWatcherEvent;

use crate::HomeDirectoryWatcher;
use crate::ai::mcp::parsing::normalize_codex_toml_to_json;
use crate::ai::mcp::{
    MCPProvider, ParsedTemplatableMCPServerResult, home_config_file_path, home_dir,
};
use crate::warp_managed_paths_watcher::{
    WarpManagedPathsWatcher, WarpManagedPathsWatcherEvent, warp_managed_mcp_config_path,
};

static ENV_VAR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([^}]+)\}").expect("Regex is valid"));

/// Matches home config paths that are exactly one directory deep (e.g. `.codex/config.toml`,
/// `.warp/.mcp.json`), capturing the parent directory component.
static HOME_SUBDIR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/]+)/[^/]+$").expect("Regex is valid"));

/// Returns the subdirectory under the home directory that needs its own [`DirectoryWatcher`],
/// inferred from the provider's home config path. Matches paths that are exactly one directory
/// deep (e.g. `.codex/config.toml` → `.codex`, `.warp/.mcp.json` → `.warp`). Returns `None`
/// when the config file lives directly in the home dir (e.g. `.claude.json`) and is already
/// covered by `HomeDirectoryWatcher`.
fn home_subdir_to_watch(provider: MCPProvider) -> Option<PathBuf> {
    let path_str = provider.home_config_path().to_str()?;
    HOME_SUBDIR_REGEX
        .captures(path_str)
        .and_then(|caps| caps.get(1))
        .map(|m| PathBuf::from(m.as_str()))
}

/// Messages sent from `RepositorySubscriber`s to detect file-based MCPs.
enum FileMCPDetectionMessage {
    /// Initial scan of a watched directory.
    InitialScan {
        /// The directory the watcher is registered on.
        /// Can be different from the directory that detected servers are stored in, i.e. for home subdir watchers.
        watched_dir: PathBuf,
        /// The directory that detected servers are stored in.
        /// Either the home directory for home watchers, or the repository root for project watchers.
        stored_dir: PathBuf,
    },
    /// Incremental file system updates from a watched directory.
    Update {
        watched_dir: PathBuf,
        stored_dir: PathBuf,
        update: RepositoryUpdate,
    },
}

/// Single repository subscriber type used for all watched directories (project repos and home
/// provider subdirs). Carries the logical `stored_dir` key captured at registration time.
struct FileMCPSubscriber {
    // Maps to the key in `file_based_servers_by_root` that contains servers detected by this subscriber.
    // For home provider subdirs, this is the home directory.
    // For project repos, this is the repository root.
    stored_dir: PathBuf,
    message_tx: Sender<FileMCPDetectionMessage>,
}

impl RepositorySubscriber for FileMCPSubscriber {
    fn on_scan(
        &mut self,
        repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let watched_dir = repository.root_dir().to_local_path_lossy();
        let stored_dir = self.stored_dir.clone();
        let tx = self.message_tx.clone();

        Box::pin(async move {
            let _ = tx
                .send(FileMCPDetectionMessage::InitialScan {
                    watched_dir,
                    stored_dir,
                })
                .await;
        })
    }

    fn on_files_updated(
        &mut self,
        repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let watched_dir = repository.root_dir().to_local_path_lossy();
        let stored_dir = self.stored_dir.clone();
        let tx = self.message_tx.clone();
        let update = update.clone();

        Box::pin(async move {
            let _ = tx
                .send(FileMCPDetectionMessage::Update {
                    watched_dir,
                    stored_dir,
                    update,
                })
                .await;
        })
    }
}

/// A single source's currently-scheduled parse. Generation-tagged so a completion callback
/// can tell whether it is still current or was superseded by a newer parse scheduled for the
/// same source: `AbortHandle::abort` only takes effect the next time the background future is
/// polled, so a superseded parse's completion can already be queued on the foreground executor
/// by the time a replacement is scheduled. See
/// [`FileMCPWatcher::update_servers_from_config_file`].
struct InFlightParse {
    generation: u64,
    abort_handle: AbortHandle,
}

/// A batch of config sources whose first parse a consumer waits on before proceeding.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PendingScan {
    /// Every provider config file in a cloud-environment repo, keyed by repo root.
    CloudEnvRepo(PathBuf),
    /// Every global (home-directory and Warp-managed) config source read at startup.
    InitialGlobal,
}

/// Model that watches the filesystem for file-based MCP config changes and emits
/// [`FileMCPWatcherEvent`]s.
pub struct FileMCPWatcher {
    file_mcp_tx: Sender<FileMCPDetectionMessage>,
    /// The current in-flight parse for each source, if any. Starting a replacement removes
    /// and aborts the previous record; only a completion callback whose captured generation
    /// still matches the record here may act on its result. See
    /// [`Self::update_servers_from_config_file`].
    in_flight_parses: HashMap<(PathBuf, MCPProvider), InFlightParse>,
    /// Monotonically increasing counter; each scheduled parse claims the next value as its
    /// generation. Never reused, even after a source's record is removed, so a long-superseded
    /// parse's callback can never appear current again by coincidence.
    next_parse_generation: u64,
    /// Watcher handles for home provider subdirectories (e.g. `~/.codex`), keyed by subdir path.
    /// Used to cleanup watchers when the subdir is deleted at runtime.
    home_provider_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
    /// Set of project repository root paths we are already watching for file-based MCP configs.
    /// Used purely for deduplication — we never tear down project watchers during the session.
    project_repo_watchers: HashSet<PathBuf>,
    /// Config sources each in-progress scan still owes a first result for. A source is settled
    /// by its current parse completing or by an explicit removal; once a scan's set drains, its
    /// entry is removed and [`FileMCPWatcherEvent::ScanComplete`] is emitted.
    pending_scans: HashMap<PendingScan, HashSet<(PathBuf, MCPProvider)>>,
}

impl FileMCPWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (file_mcp_tx, file_mcp_rx) = async_channel::unbounded::<FileMCPDetectionMessage>();
        let settings_mode = settings::settings_mode();

        ctx.spawn_stream_local(
            file_mcp_rx,
            |me, message, ctx| {
                me.handle_file_mcp_detection_message(message, ctx);
            },
            |_, _| {},
        );

        ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), {
            let file_mcp_tx = file_mcp_tx.clone();
            move |me, _, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, source } = event;
                if should_watch_repository(*source, settings_mode) {
                    let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();
                    if matches!(source, RepoDetectionSource::CloudEnvironmentPrep) {
                        me.pending_scans.insert(
                            PendingScan::CloudEnvRepo(repo_path.clone()),
                            providers_in_scope(repo_path.clone(), repo_path.clone())
                                .map(|(provider, config_path)| (config_path, provider))
                                .collect(),
                        );
                    }
                    me.register_repo_for_file_mcp_watching(repo_path, ctx, file_mcp_tx.clone());
                }
            }
        });
        ctx.subscribe_to_model(&HomeDirectoryWatcher::handle(ctx), |me, _, event, ctx| {
            me.handle_home_directory_watcher_event(event, ctx);
        });
        ctx.subscribe_to_model(
            &WarpManagedPathsWatcher::handle(ctx),
            |me, _, event, ctx| {
                me.handle_warp_managed_paths_event(event, ctx);
            },
        );

        let mut home_provider_watchers = HashMap::new();
        if let Some(home_dir) = home_dir() {
            for provider in MCPProvider::iter().filter(|provider| *provider != MCPProvider::Warp) {
                let Some(subdir) = home_subdir_to_watch(provider) else {
                    continue;
                };
                // Note: this will fail if the subdir doesn't exist yet.
                // We register upon creation of the subdir via HomeDirectoryWatcher.
                Self::watch_home_provider_dir(
                    &home_dir.join(subdir),
                    home_dir.clone(),
                    file_mcp_tx.clone(),
                    &mut home_provider_watchers,
                    ctx,
                );
            }
        }

        let global_configs = global_config_sources();
        let mut watcher = Self {
            file_mcp_tx,
            in_flight_parses: HashMap::new(),
            next_parse_generation: 0,
            home_provider_watchers,
            project_repo_watchers: HashSet::new(),
            pending_scans: HashMap::from([(
                PendingScan::InitialGlobal,
                global_configs
                    .iter()
                    .map(|(config_path, _, provider)| (config_path.clone(), *provider))
                    .collect(),
            )]),
        };
        // Every global source is read explicitly so the initial-global scan settles even when a
        // provider's home subdir watcher never registers. A subdir watcher's own scan of the
        // same file simply supersedes the explicit parse.
        for (config_path, root_path, provider) in global_configs {
            watcher.update_servers_from_config_file(&config_path, root_path, provider, ctx);
        }
        watcher
    }

    /// A watcher with no filesystem subscriptions and no owed scans, so tests can feed
    /// consumers a deterministic event sequence.
    #[cfg(test)]
    pub(crate) fn new_inert() -> Self {
        Self {
            file_mcp_tx: async_channel::unbounded().0,
            in_flight_parses: HashMap::new(),
            next_parse_generation: 0,
            home_provider_watchers: HashMap::new(),
            project_repo_watchers: HashSet::new(),
            pending_scans: HashMap::new(),
        }
    }

    #[cfg(feature = "tui")]
    pub fn reload_global_config(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(config) = warp_managed_mcp_config_path() else {
            return;
        };
        self.update_servers_from_config_file(
            &config.config_path,
            config.root_path,
            MCPProvider::Warp,
            ctx,
        );
    }

    /// Register a project repo for file-based MCP watching via DirectoryWatcher.
    fn register_repo_for_file_mcp_watching(
        &mut self,
        repo_path: PathBuf,
        ctx: &mut ModelContext<Self>,
        file_mcp_tx: Sender<FileMCPDetectionMessage>,
    ) {
        if self.project_repo_watchers.contains(&repo_path) {
            return;
        }

        let Some(repo_handle) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&repo_path, ctx)
        else {
            return;
        };

        let start = repo_handle.update(ctx, |repo, ctx| {
            repo.start_watching(
                RepositoryWatchMode::FilesystemOnly,
                Box::new(FileMCPSubscriber {
                    stored_dir: repo_path.clone(),
                    message_tx: file_mcp_tx,
                }),
                ctx,
            )
        });
        let subscriber_id = start.subscriber_id;
        // Store optimistically; removed in the error callback below if registration fails.
        self.project_repo_watchers.insert(repo_path.clone());

        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching {repo_path} for file-based MCP servers: {err}",
                    repo_path = repo_path.display(),
                );
                me.project_repo_watchers.remove(&repo_path);
                repo_handle.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });
            }
        });
    }

    /// Register a home provider subdir (e.g. `~/.codex`) for watching via `DirectoryWatcher`,
    /// storing the handle in `home_provider_watchers` for later cleanup.
    fn watch_home_provider_dir(
        subdir_path: &Path,
        home_dir: PathBuf,
        file_mcp_tx: Sender<FileMCPDetectionMessage>,
        home_provider_watchers: &mut HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
        ctx: &mut ModelContext<Self>,
    ) {
        if home_provider_watchers.contains_key(subdir_path) {
            return;
        }

        let Ok(std_path) =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(subdir_path)
        else {
            return;
        };

        let repo_handle = match DirectoryWatcher::handle(ctx)
            .update(ctx, |watcher, ctx| watcher.add_directory(std_path, ctx))
        {
            Ok(handle) => handle,
            Err(err) => {
                log::warn!(
                    "Failed to register {} for file-based MCP watching: {err}",
                    subdir_path.display(),
                );
                return;
            }
        };
        let subscriber = Box::new(FileMCPSubscriber {
            stored_dir: home_dir,
            message_tx: file_mcp_tx,
        });
        let start = repo_handle.update(ctx, |repo, ctx| {
            repo.start_watching(RepositoryWatchMode::FilesystemOnly, subscriber, ctx)
        });
        let subscriber_id = start.subscriber_id;
        // Store optimistically; removed in the error callback below if registration fails.
        home_provider_watchers.insert(
            subdir_path.to_path_buf(),
            (repo_handle.clone(), subscriber_id),
        );

        let subdir_path_owned = subdir_path.to_path_buf();
        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching {} for file-based MCP servers: {err}",
                    subdir_path_owned.display(),
                );
                me.home_provider_watchers.remove(&subdir_path_owned);
                repo_handle.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });
            }
        });
    }

    /// Handle incoming home directory watcher events.
    ///
    /// For providers whose config sits directly in home (no subdir), handles add/delete of
    /// the config file itself. For providers with a home subdir, handles creation and deletion
    /// of that subdir, registering or cleaning up a `DirectoryWatcher` accordingly.
    fn handle_home_directory_watcher_event(
        &mut self,
        event: &HomeDirectoryWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let HomeDirectoryWatcherEvent::HomeFilesChanged(fs_event) = event;
        let Some(home_dir) = home_dir() else {
            return;
        };

        for provider in MCPProvider::iter() {
            if provider == MCPProvider::Warp {
                continue;
            }
            match home_subdir_to_watch(provider) {
                None => {
                    // Config lives directly in home (e.g. ~/.claude.json).
                    // HomeDirectoryWatcher watches home non-recursively, so we handle
                    // add/delete/move of the config file here.
                    let Some(config_path) = home_config_file_path(provider) else {
                        continue;
                    };

                    let was_deleted = fs_event.deleted.contains(&config_path)
                        || fs_event.moved.values().any(|v| v == &config_path);

                    let was_added = fs_event.added_or_updated_iter().any(|p| p == &config_path)
                        || fs_event.moved.contains_key(&config_path);
                    self.handle_single_config_update(
                        home_dir.clone(),
                        provider,
                        config_path,
                        was_deleted,
                        was_added,
                        ctx,
                    );
                }
                Some(subdir) => {
                    // Config lives in a home subdir (e.g. ~/.codex/config.toml).
                    // HomeDirectoryWatcher detects creation/deletion of the subdir itself;
                    // file changes within it are handled by the registered DirectoryWatcher.
                    let subdir_path = home_dir.join(&subdir);

                    let subdir_added = fs_event.added.contains(&subdir_path)
                        || fs_event.moved.contains_key(&subdir_path);
                    if subdir_added {
                        // If the subdir (i.e. ~/.codex) is created, start watching it for file-based MCP servers.
                        Self::watch_home_provider_dir(
                            &subdir_path,
                            home_dir.clone(),
                            self.file_mcp_tx.clone(),
                            &mut self.home_provider_watchers,
                            ctx,
                        );
                    }

                    let subdir_deleted = fs_event.deleted.contains(&subdir_path)
                        || fs_event.moved.values().any(|v| v == &subdir_path);
                    if subdir_deleted {
                        if let Some((repo_handle, id)) =
                            self.home_provider_watchers.remove(&subdir_path)
                        {
                            repo_handle.update(ctx, |repo, ctx| repo.stop_watching(id, ctx));
                        }
                        let config_path = home_dir.join(provider.home_config_path());
                        self.remove_config(config_path, home_dir.clone(), provider, ctx);
                    }
                }
            }
        }
    }

    fn handle_warp_managed_paths_event(
        &mut self,
        event: &WarpManagedPathsWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let WarpManagedPathsWatcherEvent::FilesChanged(update) = event;
        let Some(mcp_config_path) = warp_managed_mcp_config_path() else {
            return;
        };
        let config_path = mcp_config_path.config_path;
        let was_deleted = update
            .deleted
            .iter()
            .any(|target| target.path == config_path)
            || update
                .moved
                .values()
                .any(|target| target.path == config_path);
        let was_added = update
            .added_or_modified()
            .any(|target| target.path == config_path)
            || update.moved.keys().any(|target| target.path == config_path);
        self.handle_single_config_update(
            mcp_config_path.root_path,
            MCPProvider::Warp,
            config_path,
            was_deleted,
            was_added,
            ctx,
        );
    }

    /// Handle incoming file-based MCP detection messages.
    fn handle_file_mcp_detection_message(
        &mut self,
        message: FileMCPDetectionMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        match message {
            FileMCPDetectionMessage::InitialScan {
                watched_dir,
                stored_dir: root_path,
            } => {
                self.handle_dir_initial_scan(watched_dir, root_path, ctx);
            }
            FileMCPDetectionMessage::Update {
                watched_dir,
                stored_dir: root_path,
                update,
            } => {
                self.handle_dir_update(watched_dir, root_path, update, ctx);
            }
        }
    }

    /// Handle an initial scan of a watched directory.
    ///
    /// `providers_in_scope` scopes the scan to the watcher: for a project watcher
    /// (`watched_dir == root_path`) both Claude and Codex configs are scanned; for a home
    /// Codex watcher (`watched_dir = ~/.codex`, `root_path = ~/`) only Codex's config passes.
    fn handle_dir_initial_scan(
        &mut self,
        watched_dir: PathBuf,
        root_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        for (provider, config_path) in providers_in_scope(root_path.clone(), watched_dir.clone()) {
            self.update_servers_from_config_file(&config_path, root_path.clone(), provider, ctx);
        }
    }

    /// Handle incremental file system updates from a watched directory.
    fn handle_dir_update(
        &mut self,
        watched_dir: PathBuf,
        root_path: PathBuf,
        update: RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut configs_to_update = Vec::new();

        for (provider, config_path) in providers_in_scope(root_path.clone(), watched_dir.clone()) {
            let (was_deleted, was_added) = config_change_flags(&update, &config_path);
            configs_to_update.push((provider, config_path, was_deleted, was_added));
        }

        for (provider, config_path, was_deleted, was_added) in configs_to_update {
            self.handle_single_config_update(
                root_path.clone(),
                provider,
                config_path,
                was_deleted,
                was_added,
                ctx,
            );
        }
    }

    fn handle_single_config_update(
        &mut self,
        root_path: PathBuf,
        provider: MCPProvider,
        config_path: PathBuf,
        was_deleted: bool,
        was_added: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Atomic replacements can be reported as a delete and add in the same
        // update. Parse the replacement without transiently removing the
        // last-known-good servers.
        if was_deleted && !was_added {
            self.remove_config(config_path, root_path, provider, ctx);
        } else if was_added {
            self.update_servers_from_config_file(&config_path, root_path, provider, ctx);
        }
    }

    /// Drops any in-flight parse for the source, reports it removed, and settles it for any
    /// pending scan still waiting on it.
    fn remove_config(
        &mut self,
        config_path: PathBuf,
        root_path: PathBuf,
        provider: MCPProvider,
        ctx: &mut ModelContext<Self>,
    ) {
        self.abort_config_parse(&config_path, provider);
        ctx.emit(FileMCPWatcherEvent::ConfigRemoved {
            config_path: config_path.clone(),
            root_path,
            provider,
        });
        self.settle_pending_source(&config_path, provider, ctx);
    }

    /// Marks the source as no longer owed by any pending scan, emitting
    /// [`FileMCPWatcherEvent::ScanComplete`] for each scan this drains.
    fn settle_pending_source(
        &mut self,
        config_path: &Path,
        provider: MCPProvider,
        ctx: &mut ModelContext<Self>,
    ) {
        let source = (config_path.to_path_buf(), provider);
        let mut completed = Vec::new();
        self.pending_scans.retain(|scan, sources| {
            sources.remove(&source);
            if sources.is_empty() {
                completed.push(scan.clone());
            }
            !sources.is_empty()
        });
        for scan in completed {
            ctx.emit(FileMCPWatcherEvent::ScanComplete(scan));
        }
    }

    fn abort_config_parse(&mut self, config_path: &Path, provider: MCPProvider) {
        if let Some(in_flight) = self
            .in_flight_parses
            .remove(&(config_path.to_path_buf(), provider))
        {
            in_flight.abort_handle.abort();
        }
    }

    /// Returns whether the parse identified by `key`/`generation` is still the source's
    /// current one, removing its record if so. A `false` result means it was superseded by a
    /// newer parse scheduled for the same source (or already explicitly removed) since it was
    /// scheduled: `AbortHandle::abort` only takes effect the next time the background future is
    /// polled, so a superseded parse's completion callback can still run after that. The
    /// caller must treat a `false` result as fully stale because its possibly outdated result
    /// would preempt whatever superseded it.
    fn take_current_in_flight_parse(
        &mut self,
        key: &(PathBuf, MCPProvider),
        generation: u64,
    ) -> bool {
        match self.in_flight_parses.get(key) {
            Some(in_flight) if in_flight.generation == generation => {
                self.in_flight_parses.remove(key);
                true
            }
            _ => false,
        }
    }

    /// Asynchronously reads and parses the MCP configuration file at `config_file_path`,
    /// then emits a [`FileMCPWatcherEvent::ConfigParsed`] event.
    ///
    /// Every call for a given `(config_file_path, provider)` source claims a fresh generation
    /// (see [`Self::take_current_in_flight_parse`]) and aborts any in-flight parse for that
    /// source.
    fn update_servers_from_config_file(
        &mut self,
        config_file_path: &Path,
        root_path: PathBuf,
        provider: MCPProvider,
        ctx: &mut ModelContext<Self>,
    ) {
        let config_file_path = config_file_path.to_path_buf();
        let key = (config_file_path.clone(), provider);
        let callback_key = key.clone();
        self.abort_config_parse(&config_file_path, provider);

        let generation = self.next_parse_generation;
        self.next_parse_generation += 1;

        let parse = ctx.spawn(
            async move { parse_mcp_config_file(&config_file_path, provider).await },
            move |me, outcome, ctx| {
                if !me.take_current_in_flight_parse(&callback_key, generation) {
                    // Superseded: a newer parse for this source already owns the record (or
                    // an explicit removal already settled it). Do not touch anything else.
                    return;
                }

                let (config_path, provider) = callback_key;
                emit_parse_outcome(outcome, config_path.clone(), root_path, provider, ctx);
                me.settle_pending_source(&config_path, provider, ctx);
            },
        );
        self.in_flight_parses.insert(
            key,
            InFlightParse {
                generation,
                abort_handle: parse.abort_handle(),
            },
        );
    }
}

fn should_watch_repository(
    source: RepoDetectionSource,
    settings_mode: settings::SettingsMode,
) -> bool {
    match settings_mode {
        settings::SettingsMode::Gui => match source {
            RepoDetectionSource::TerminalNavigation | RepoDetectionSource::CloudEnvironmentPrep => {
                true
            }
            RepoDetectionSource::ProjectRulesIndexing
            | RepoDetectionSource::CodeReviewInitialization => false,
        },
        settings::SettingsMode::Tui => match source {
            RepoDetectionSource::TerminalNavigation => true,
            RepoDetectionSource::ProjectRulesIndexing
            | RepoDetectionSource::CodeReviewInitialization
            | RepoDetectionSource::CloudEnvironmentPrep => false,
        },
    }
}

fn config_change_flags(update: &RepositoryUpdate, config_path: &Path) -> (bool, bool) {
    let was_deleted = update.deleted.iter().any(|file| file.path == config_path)
        || update.moved.values().any(|file| file.path == config_path);
    let was_added = update
        .added_or_modified()
        .any(|file| file.path == config_path)
        || update.moved.keys().any(|file| file.path == config_path);
    (was_deleted, was_added)
}

/// Every global config source as `(config_path, root_path, provider)`: the Warp-managed config
/// plus each other provider's home-directory config.
fn global_config_sources() -> Vec<(PathBuf, PathBuf, MCPProvider)> {
    let mut sources = Vec::new();
    if let Some(config) = warp_managed_mcp_config_path() {
        sources.push((config.config_path, config.root_path, MCPProvider::Warp));
    }
    if let Some(home_dir) = home_dir() {
        sources.extend(
            MCPProvider::iter()
                .filter(|provider| *provider != MCPProvider::Warp)
                .filter_map(|provider| {
                    home_config_file_path(provider)
                        .map(|config_path| (config_path, home_dir.clone(), provider))
                }),
        );
    }
    sources
}

/// Returns an iterator of `(provider, config_path)` pairs for MCP providers whose configuration file
/// paths fall within the watched directory.
fn providers_in_scope(
    root_path: PathBuf,
    watched_dir: PathBuf,
) -> impl Iterator<Item = (MCPProvider, PathBuf)> {
    MCPProvider::iter().flat_map(move |provider| {
        let mut results = HashSet::new();
        for path in [
            root_path.join(provider.home_config_path()),
            root_path.join(provider.project_config_path()),
        ] {
            if path.starts_with(&watched_dir) {
                results.insert((provider, path));
            }
        }
        results.into_iter()
    })
}

/// Substitutes environment variables in the format ${VAR_NAME} in the given JSON string.
/// Returns an error if any environment variable is not found, as the server cannot be started.
fn substitute_env_vars(json_content: &str) -> Result<String, anyhow::Error> {
    let mut result = json_content.to_string();

    for capture in ENV_VAR_REGEX.captures_iter(json_content) {
        if let Some(var_match) = capture.get(1) {
            let var_name = var_match.as_str();
            match std::env::var(var_name) {
                Ok(value) if !value.is_empty() => {
                    let placeholder = format!("${{{}}}", var_name);
                    result = result.replace(&placeholder, &value);
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "Missing or empty environment variable: {var_name}"
                    ));
                }
            }
        }
    }

    Ok(result)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileMCPConfigDiagnosticKind {
    Read,
    Parse,
    MissingEnvironmentVariable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMCPConfigDiagnostic {
    pub config_path: PathBuf,
    pub provider: MCPProvider,
    pub kind: FileMCPConfigDiagnosticKind,
    pub message: String,
}

enum FileMCPConfigParseOutcome {
    Missing,
    Parsed(Vec<ParsedTemplatableMCPServerResult>),
    Error(FileMCPConfigDiagnostic),
}

fn emit_parse_outcome(
    outcome: FileMCPConfigParseOutcome,
    config_path: PathBuf,
    root_path: PathBuf,
    provider: MCPProvider,
    ctx: &mut ModelContext<FileMCPWatcher>,
) {
    match outcome {
        FileMCPConfigParseOutcome::Missing => ctx.emit(FileMCPWatcherEvent::ConfigRemoved {
            config_path,
            root_path,
            provider,
        }),
        FileMCPConfigParseOutcome::Parsed(servers) => ctx.emit(FileMCPWatcherEvent::ConfigParsed {
            config_path,
            root_path,
            provider,
            servers,
        }),
        FileMCPConfigParseOutcome::Error(diagnostic) => {
            let _ = root_path;
            ctx.emit(FileMCPWatcherEvent::ConfigError { diagnostic })
        }
    }
}

/// Asynchronously reads and parses an MCP config file.
///
/// Missing files, valid snapshots, and invalid snapshots are distinct so
/// consumers can preserve the last-known-good servers on transient errors.
async fn parse_mcp_config_file(
    file_path: &Path,
    provider: MCPProvider,
) -> FileMCPConfigParseOutcome {
    let file_contents = match async_fs::read_to_string(file_path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return FileMCPConfigParseOutcome::Missing,
        Err(err) => {
            safe_warn!(
                safe: (
                    "Failed to read MCP config file: {}",
                    err
                ),
                full: (
                    "Failed to read MCP config file {}: {}",
                    file_path.display(),
                    err
                )
            );
            return FileMCPConfigParseOutcome::Error(FileMCPConfigDiagnostic {
                config_path: file_path.to_path_buf(),
                provider,
                kind: FileMCPConfigDiagnosticKind::Read,
                message: format!("Failed to read MCP config: {err}"),
            });
        }
    };

    let json = match provider {
        MCPProvider::Codex => match normalize_codex_toml_to_json(&file_contents) {
            Ok(json) => json,
            Err(err) => {
                safe_warn!(
                    safe: (
                        "Failed to normalize Codex TOML: {:#}",
                        err
                    ),
                    full: (
                        "Failed to normalize Codex TOML {}: {:#}",
                        file_path.display(),
                        err
                    )
                );
                return FileMCPConfigParseOutcome::Error(FileMCPConfigDiagnostic {
                    config_path: file_path.to_path_buf(),
                    provider,
                    kind: FileMCPConfigDiagnosticKind::Parse,
                    message: format!("Failed to parse MCP config: {err:#}"),
                });
            }
        },
        MCPProvider::Claude | MCPProvider::Warp | MCPProvider::Agents => file_contents,
    };

    let resolved_contents = match substitute_env_vars(&json) {
        Ok(resolved) => resolved,
        Err(err) => {
            safe_warn!(
                safe: (
                    "Cannot start MCP servers - missing required environment variables: {}",
                    err
                ),
                full: (
                    "Cannot start MCP servers from {} - missing required environment variables: {}",
                    file_path.display(),
                    err
                )
            );
            return FileMCPConfigParseOutcome::Error(FileMCPConfigDiagnostic {
                config_path: file_path.to_path_buf(),
                provider,
                kind: FileMCPConfigDiagnosticKind::MissingEnvironmentVariable,
                message: err.to_string(),
            });
        }
    };

    match ParsedTemplatableMCPServerResult::from_config_file_json(&resolved_contents) {
        Ok(parsed_servers) => FileMCPConfigParseOutcome::Parsed(parsed_servers),
        Err(err) => {
            safe_warn!(
                safe: (
                    "Failed to parse MCP servers: {:#}",
                    err
                ),
                full: (
                    "Failed to parse MCP servers from {}: {:#}",
                    file_path.display(),
                    err
                )
            );
            FileMCPConfigParseOutcome::Error(FileMCPConfigDiagnostic {
                config_path: file_path.to_path_buf(),
                provider,
                kind: FileMCPConfigDiagnosticKind::Parse,
                message: format!("Failed to parse MCP servers: {err:#}"),
            })
        }
    }
}

/// Events sent from [`FileMCPWatcher`] to [`FileBasedMCPManager`] via the watcher channel.
pub enum FileMCPWatcherEvent {
    /// A config file was successfully parsed; delivers the full snapshot for `(root_path, provider)`.
    ConfigParsed {
        config_path: PathBuf,
        root_path: PathBuf,
        provider: MCPProvider,
        servers: Vec<ParsedTemplatableMCPServerResult>,
    },
    /// A config file was deleted; all servers for `(root_path, provider)` should be removed.
    ConfigRemoved {
        config_path: PathBuf,
        root_path: PathBuf,
        provider: MCPProvider,
    },
    /// A config could not be read or parsed. Consumers should preserve the last-known-good state.
    ConfigError { diagnostic: FileMCPConfigDiagnostic },
    /// Every source owed by the scan has been parsed or removed. Emitted after the events
    /// carrying those sources' results.
    ScanComplete(PendingScan),
}

impl Entity for FileMCPWatcher {
    type Event = FileMCPWatcherEvent;
}

impl SingletonEntity for FileMCPWatcher {}

#[cfg(test)]
#[path = "file_mcp_watcher_tests.rs"]
mod tests;
