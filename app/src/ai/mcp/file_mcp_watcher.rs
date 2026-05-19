use async_channel::Sender;
use futures::Future;
use regex::Regex;
use repo_metadata::{
    repositories::{DetectedRepositories, DetectedRepositoriesEvent, RepoDetectionSource},
    repository::{Repository, RepositorySubscriber, SubscriberId},
    watcher::{DirectoryWatcher, RepositoryUpdate},
};
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::LazyLock;
use warp_core::safe_warn;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};
use watcher::HomeDirectoryWatcherEvent;

use crate::ai::mcp::{
    home_config_file_path, parsing::normalize_codex_toml_to_json, MCPProvider,
    ParsedTemplatableMCPServerResult,
};
use crate::warp_managed_paths_watcher::{
    warp_managed_mcp_config_path, WarpManagedPathsWatcher, WarpManagedPathsWatcherEvent,
};
use crate::HomeDirectoryWatcher;
use strum::IntoEnumIterator;

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

/// Model that watches the filesystem for file-based MCP config changes and emits
/// [`FileMCPWatcherEvent`]s.
pub struct FileMCPWatcher {
    file_mcp_tx: Sender<FileMCPDetectionMessage>,
    /// Watcher handles for home provider subdirectories (e.g. `~/.codex`), keyed by subdir path.
    /// Used to cleanup watchers when the subdir is deleted at runtime.
    home_provider_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
    /// Set of project repository root paths we are already watching for file-based MCP configs.
    /// Used purely for deduplication — we never tear down project watchers during the session.
    project_repo_watchers: HashSet<PathBuf>,
    /// Tracks how many provider config files remain to be parsed for each cloud environment repo.
    /// When the count reaches zero, a `CloudEnvironmentScanComplete` event is emitted.
    cloud_env_pending: HashMap<PathBuf, usize>,
}

impl FileMCPWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (file_mcp_tx, file_mcp_rx) = async_channel::unbounded::<FileMCPDetectionMessage>();

        ctx.spawn_stream_local(
            file_mcp_rx,
            |me, message, ctx| {
                me.handle_file_mcp_detection_message(message, ctx);
            },
            |_, _| {},
        );

        // Subscribe to changes to detected repositories.
        ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), {
            let file_mcp_tx = file_mcp_tx.clone();
            move |me, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, source } = event;
                // Register MCP servers for repos the user actively navigated to, and for
                // repos cloned during cloud agent environment preparation.
                if matches!(
                    source,
                    RepoDetectionSource::TerminalNavigation
                        | RepoDetectionSource::CloudEnvironmentPrep
                ) {
                    let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();
                    if matches!(source, RepoDetectionSource::CloudEnvironmentPrep) {
                        // Track how many MCP config files remain to be parsed for the cloud environment repo.
                        let count =
                            providers_in_scope(repo_path.clone(), repo_path.clone()).count();
                        me.cloud_env_pending.insert(repo_path.clone(), count);
                    }
                    me.register_repo_for_file_mcp_watching(repo_path, ctx, file_mcp_tx.clone());
                }
            }
        });

        // Subscribe to changes to top-level files in the home directory.
        ctx.subscribe_to_model(&HomeDirectoryWatcher::handle(ctx), |me, event, ctx| {
            me.handle_home_directory_watcher_event(event, ctx);
        });
        ctx.subscribe_to_model(&WarpManagedPathsWatcher::handle(ctx), |me, event, ctx| {
            me.handle_warp_managed_paths_event(event, ctx);
        });

        let mut home_provider_watchers = HashMap::new();
        if let Some(mcp_config_path) = warp_managed_mcp_config_path() {
            Self::spawn_config_parse(
                mcp_config_path.config_path,
                mcp_config_path.root_path,
                MCPProvider::Warp,
                ctx,
            );
        }

        if let Some(home_dir) = dirs::home_dir() {
            for provider in MCPProvider::iter() {
                if provider == MCPProvider::Warp {
                    continue;
                }
                match home_subdir_to_watch(provider) {
                    None => {
                        // Initial scan of config files for providers whose config lives directly in
                        // home (i.e. ~/.claude.json). HomeDirectoryWatcher handles incremental updates.
                        let Some(config_path) = home_config_file_path(provider) else {
                            continue;
                        };
                        Self::spawn_config_parse(config_path, home_dir.clone(), provider, ctx);
                    }
                    Some(subdir) => {
                        // For providers whose home config lives in a subdir (e.g. ~/.codex for Codex)
                        // start watching the subdir for file-based MCP servers, if it exists.
                        let subdir_path = home_dir.join(&subdir);
                        // Note: this will fail if the subdir doesn't exist yet.
                        // We register upon creation of the subdir via HomeDirectoryWatcher.
                        Self::watch_home_provider_dir(
                            &subdir_path,
                            home_dir.clone(),
                            file_mcp_tx.clone(),
                            &mut home_provider_watchers,
                            ctx,
                        );
                    }
                }
            }
        }

        Self {
            file_mcp_tx,
            home_provider_watchers,
            project_repo_watchers: HashSet::new(),
            cloud_env_pending: HashMap::new(),
        }
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
        // If the subdir is already being watched, return early.
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
        let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
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
        let Some(home_dir) = dirs::home_dir() else {
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
                    if was_deleted {
                        ctx.emit(FileMCPWatcherEvent::ConfigRemoved {
                            root_path: home_dir.clone(),
                            provider,
                        });
                    }

                    let was_added = fs_event.added_or_updated_iter().any(|p| p == &config_path)
                        || fs_event.moved.contains_key(&config_path);
                    if was_added {
                        self.update_servers_from_config_file(
                            &config_path,
                            home_dir.clone(),
                            provider,
                            ctx,
                        );
                    }
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
                        ctx.emit(FileMCPWatcherEvent::ConfigRemoved {
                            root_path: home_dir.clone(),
                            provider,
                        });
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
            let was_deleted = update.deleted.iter().any(|f| f.path == config_path)
                || update.moved.values().any(|f| f.path == config_path);
            let was_added = update.added_or_modified().any(|f| f.path == config_path)
                || update.moved.keys().any(|f| f.path == config_path);
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
        if was_deleted {
            ctx.emit(FileMCPWatcherEvent::ConfigRemoved {
                root_path: root_path.clone(),
                provider,
            });
        }
        if was_added {
            self.update_servers_from_config_file(&config_path, root_path, provider, ctx);
        }
    }

    fn spawn_config_parse(
        config_path: PathBuf,
        root_path: PathBuf,
        provider: MCPProvider,
        ctx: &mut ModelContext<Self>,
    ) {
        let root_path_for_callback = root_path.clone();
        let _ = ctx.spawn(
            async move { parse_mcp_config_file(&config_path, provider).await },
            move |_me, parsed, ctx| {
                ctx.emit(FileMCPWatcherEvent::ConfigParsed {
                    root_path: root_path_for_callback,
                    provider,
                    servers: parsed,
                });
            },
        );
    }

    /// Asynchronously reads and parses the MCP configuration file at `config_file_path`,
    /// then emits a [`FileMCPWatcherEvent::ConfigParsed`] event.
    fn update_servers_from_config_file(
        &mut self,
        config_file_path: &Path,
        root_path: PathBuf,
        provider: MCPProvider,
        ctx: &mut ModelContext<Self>,
    ) {
        let config_file_path = config_file_path.to_path_buf();
        let _ = ctx.spawn(
            async move { parse_mcp_config_file(&config_file_path, provider).await },
            move |me, servers, ctx| {
                let repo_path_for_countdown = root_path.clone();
                ctx.emit(FileMCPWatcherEvent::ConfigParsed {
                    root_path,
                    provider,
                    servers,
                });
                if let Some(count) = me.cloud_env_pending.get_mut(&repo_path_for_countdown) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        // If we've parsed all MCP config files for the cloud environment repo, emit a `CloudEnvironmentScanComplete` event.
                        me.cloud_env_pending.remove(&repo_path_for_countdown);
                        ctx.emit(FileMCPWatcherEvent::CloudEnvMcpScanComplete {
                            repo_path: repo_path_for_countdown,
                        });
                    }
                }
            },
        );
    }
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

/// Asynchronously reads and parses an MCP config file and returns parsed MCP servers.
/// Dispatches to the appropriate parser based on `provider` rather than inferring from path.
/// Returns an empty vec if the file doesn't exist or parsing fails.
async fn parse_mcp_config_file(
    file_path: &Path,
    provider: MCPProvider,
) -> Vec<ParsedTemplatableMCPServerResult> {
    let file_contents = match async_fs::read_to_string(file_path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return vec![],
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
            return vec![];
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
                return vec![];
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
            return vec![];
        }
    };

    match ParsedTemplatableMCPServerResult::from_config_file_json(&resolved_contents) {
        Ok(parsed_servers) => parsed_servers,
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
            vec![]
        }
    }
}

/// Events sent from [`FileMCPWatcher`] to [`FileBasedMCPManager`] via the watcher channel.
pub enum FileMCPWatcherEvent {
    /// A config file was successfully parsed; delivers the full snapshot for `(root_path, provider)`.
    ConfigParsed {
        root_path: PathBuf,
        provider: MCPProvider,
        servers: Vec<ParsedTemplatableMCPServerResult>,
    },
    /// A config file was deleted; all servers for `(root_path, provider)` should be removed.
    ConfigRemoved {
        root_path: PathBuf,
        provider: MCPProvider,
    },
    /// All provider config files for a cloud environment repo have been parsed.
    CloudEnvMcpScanComplete { repo_path: PathBuf },
}

impl Entity for FileMCPWatcher {
    type Event = FileMCPWatcherEvent;
}

impl SingletonEntity for FileMCPWatcher {}

#[cfg(test)]
#[path = "file_mcp_watcher_tests.rs"]
mod tests;
