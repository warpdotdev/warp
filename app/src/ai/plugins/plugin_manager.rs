//! The client-side owner of discovered Agent Plugin packages.
//!
//! `PluginManager` watches and scans the fixed plugin search roots, keeps the winning package for
//! each manifest name, and turns the `Agent Plugin discovery` preference into the teardown the
//! rest of the client has to perform. It never launches anything itself: a discovered stdio
//! server becomes an installation for the existing file-based MCP surfaces, which own starting
//! it.
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use ai::plugins::{
    LocalPluginDataLocator, PluginCandidate, PluginComponentId, PluginDiagnostic, PluginFrontend,
    PluginSearchRoot, PluginSkillComponent, repository_search_roots, resolve_active_packages,
    scan_search_root, user_search_roots,
};
use async_channel::Sender;
use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
use repo_metadata::repository::{Repository, RepositorySubscriber, SubscriberId};
use repo_metadata::{DirectoryWatcher, RepositoryUpdate};
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};
use watcher::{BulkFilesystemWatcherEvent, HomeDirectoryWatcher, HomeDirectoryWatcherEvent};

use super::registry::{PluginDiscoveryPolicy, PluginRegistry, PluginTeardownStep};
use crate::settings::AISettingsChangedEvent;
use crate::settings::ai::AISettings;

/// What the plugin manager tells the rest of the client.
pub enum PluginManagerEvent {
    /// The active plugin set changed. Skill and MCP surfaces re-read it.
    PluginsChanged,
    /// Plugin skills must leave the model catalog and the explicit invocation resolver.
    WithdrawSkills,
    /// In-flight plugin MCP tool calls must be cancelled with `agent_plugin_discovery_disabled`.
    CancelInFlightMcpCalls,
    /// These plugin MCP installations must be stopped and unregistered.
    UnregisterMcpInstallations { components: Vec<PluginComponentId> },
}

enum PluginWatchMessage {
    SearchRootsChanged,
}

struct PluginSearchRootSubscriber {
    search_roots: Vec<PathBuf>,
    message_tx: Sender<PluginWatchMessage>,
}

impl RepositorySubscriber for PluginSearchRootSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let message_tx = self.message_tx.clone();
        Box::pin(async move {
            let _ = message_tx
                .send(PluginWatchMessage::SearchRootsChanged)
                .await;
        })
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        if !update_affects_search_roots(update, &self.search_roots) {
            return Box::pin(async {});
        }

        let message_tx = self.message_tx.clone();
        Box::pin(async move {
            let _ = message_tx
                .send(PluginWatchMessage::SearchRootsChanged)
                .await;
        })
    }
}

pub struct PluginManager {
    registry: PluginRegistry,
    policy: PluginDiscoveryPolicy,
    /// Repository roots currently in scope. Plugins from a repository are only active while its
    /// repository is, matching the existing skill scoping rules.
    repository_roots: BTreeSet<PathBuf>,
    watcher_message_tx: Sender<PluginWatchMessage>,
    watcher_subscriptions: BTreeMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
    data_locator: LocalPluginDataLocator,
}

impl PluginManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let policy = PluginDiscoveryPolicy::InteractivePreference;
        let enabled = FeatureFlag::AgentPlugins.is_enabled()
            && policy.is_enabled(AISettings::as_ref(ctx).is_plugin_discovery_enabled(ctx));
        let (watcher_message_tx, watcher_message_rx) = async_channel::unbounded();

        ctx.spawn_stream_local(
            watcher_message_rx,
            |me, message, ctx| match message {
                PluginWatchMessage::SearchRootsChanged => me.rescan(ctx),
            },
            |_, _| {},
        );

        if FeatureFlag::AgentPlugins.is_enabled() {
            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
                if matches!(
                    event,
                    AISettingsChangedEvent::AgentPluginDiscoveryEnabled { .. }
                ) {
                    me.handle_discovery_preference_change(ctx);
                }
            });
            ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
                let root = repository.as_ref(ctx).root_dir().to_local_path_lossy();
                let was_inserted = me.repository_roots.insert(root.clone());
                if me.registry.is_enabled() {
                    me.watch_repository(root, repository.clone(), ctx);
                }
                if was_inserted {
                    me.rescan(ctx);
                }
            });
            ctx.subscribe_to_model(&HomeDirectoryWatcher::handle(ctx), |me, _, event, ctx| {
                let HomeDirectoryWatcherEvent::HomeFilesChanged(event) = event;
                if home_event_affects_user_roots(event, &user_search_roots()) {
                    me.refresh_user_watchers(ctx);
                    me.rescan(ctx);
                }
            });
        }

        let mut manager = Self {
            registry: PluginRegistry::new(enabled),
            policy,
            repository_roots: BTreeSet::new(),
            watcher_message_tx,
            watcher_subscriptions: BTreeMap::new(),
            data_locator: LocalPluginDataLocator::new(
                warp_core::paths::data_dir(),
                active_frontend(),
            ),
        };
        if enabled {
            manager.start_watchers(ctx);
            manager.rescan(ctx);
        }
        manager
    }

    /// The persistent data directory for a plugin instance, without creating it.
    ///
    /// The directory is created immediately before a stdio server's first start, never during
    /// discovery, so validating a package can never allocate storage for it.
    pub fn data_locator(&self) -> &LocalPluginDataLocator {
        &self.data_locator
    }

    pub fn is_discovery_enabled(&self) -> bool {
        self.registry.is_enabled()
    }

    pub fn active_skills(&self) -> Vec<&PluginSkillComponent> {
        self.registry.active_skills()
    }

    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        self.registry.diagnostics()
    }

    /// Resolves an explicit skill reference that may be plugin-qualified.
    pub fn resolve_skill(
        &self,
        name: &str,
        flat_names: &[String],
    ) -> Result<&PluginSkillComponent, PluginDiagnostic> {
        self.registry.resolve_skill(name, flat_names)
    }

    /// Applies a change to the `Agent Plugin discovery` preference.
    ///
    /// The registry has already stopped answering lookups by the time the teardown events are
    /// emitted, so a turn that starts mid-teardown cannot resolve a component that is on its way
    /// out.
    fn handle_discovery_preference_change(&mut self, ctx: &mut ModelContext<Self>) {
        let enabled = self
            .policy
            .is_enabled(AISettings::as_ref(ctx).is_plugin_discovery_enabled(ctx));
        let transition = self.registry.set_enabled(enabled);
        if transition.is_noop() {
            return;
        }

        for step in transition.teardown {
            match step {
                PluginTeardownStep::StopWatchers => self.stop_watchers(ctx),
                step => {
                    if let Some(event) = teardown_event(step) {
                        ctx.emit(event);
                    }
                }
            }
        }

        if transition.rescan {
            self.start_watchers(ctx);
            self.rescan(ctx);
        } else {
            // Package files and plugin data are left on disk; only the runtime set is empty.
            ctx.emit(PluginManagerEvent::PluginsChanged);
        }
    }

    /// Rebuilds the active plugin set from every in-scope search root.
    ///
    /// Scanning reads `plugin.json`, `skills/`, and `mcp.json` and nothing else. A generation tag
    /// makes the result droppable, so a scan that finishes after discovery was turned off cannot
    /// resurrect the packages the teardown just removed.
    fn rescan(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.registry.is_enabled() {
            return;
        }
        let generation = self.registry.begin_scan();

        let mut candidates: Vec<PluginCandidate> = Vec::new();
        for root in user_search_roots() {
            candidates.extend(scan_search_root(&root));
        }
        for repo_root in &self.repository_roots {
            for root in repository_search_roots(repo_root) {
                candidates.extend(scan_search_root(&root));
            }
        }

        let resolved = resolve_active_packages(candidates);
        for diagnostic in resolved.all_diagnostics() {
            log_plugin_diagnostic(&diagnostic);
        }
        if self.registry.apply_scan(generation, resolved) {
            ctx.emit(PluginManagerEvent::PluginsChanged);
        }
    }

    fn start_watchers(&mut self, ctx: &mut ModelContext<Self>) {
        self.refresh_user_watchers(ctx);
        self.start_repository_watchers(ctx);
    }

    fn start_repository_watchers(&mut self, ctx: &mut ModelContext<Self>) {
        let repository_roots = DetectedRepositories::as_ref(ctx).local_repository_roots();
        for root in repository_roots {
            self.repository_roots.insert(root.clone());
            if let Some(repository) =
                DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&root, ctx)
            {
                self.watch_repository(root, repository, ctx);
            }
        }
    }

    fn refresh_user_watchers(&mut self, ctx: &mut ModelContext<Self>) {
        let user_roots = user_search_roots();
        let user_provider_roots = user_roots
            .iter()
            .filter_map(|root| root.path.parent())
            .map(PathBuf::from)
            .collect::<BTreeSet<_>>();
        let desired_watcher_roots = user_provider_roots
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect::<BTreeSet<_>>();
        let obsolete = self
            .watcher_subscriptions
            .keys()
            .filter(|path| user_provider_roots.contains(*path))
            .filter(|path| !desired_watcher_roots.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in obsolete {
            self.stop_watcher(&path, ctx);
        }

        for path in desired_watcher_roots {
            if self.watcher_subscriptions.contains_key(&path) {
                continue;
            }
            let Ok(standardized_path) =
                warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&path)
            else {
                continue;
            };
            let Ok(repository) = DirectoryWatcher::handle(ctx).update(ctx, |watcher, ctx| {
                watcher.add_directory(standardized_path, ctx)
            }) else {
                continue;
            };
            let search_roots = user_roots
                .iter()
                .filter(|root| root.path.starts_with(&path))
                .map(|root| root.path.clone())
                .collect();
            self.watch(path, repository, search_roots, ctx);
        }
    }

    fn watch_repository(
        &mut self,
        root: PathBuf,
        repository: ModelHandle<Repository>,
        ctx: &mut ModelContext<Self>,
    ) {
        let search_roots = repository_search_roots(&root)
            .into_iter()
            .map(|search_root| search_root.path)
            .collect();
        self.watch(root, repository, search_roots, ctx);
    }

    fn watch(
        &mut self,
        watched_root: PathBuf,
        repository: ModelHandle<Repository>,
        search_roots: Vec<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.watcher_subscriptions.contains_key(&watched_root) {
            return;
        }

        let start = repository.update(ctx, |repository, ctx| {
            repository.start_watching(
                Box::new(PluginSearchRootSubscriber {
                    search_roots,
                    message_tx: self.watcher_message_tx.clone(),
                }),
                ctx,
            )
        });
        let subscriber_id = start.subscriber_id;
        self.watcher_subscriptions
            .insert(watched_root.clone(), (repository.clone(), subscriber_id));

        ctx.spawn(start.registration_future, move |me, result, ctx| {
            if let Err(error) = result {
                log::warn!(
                    "Failed to watch {} for Agent Plugins: {error}",
                    watched_root.display()
                );
                if me
                    .watcher_subscriptions
                    .get(&watched_root)
                    .is_some_and(|(_, registered_id)| *registered_id == subscriber_id)
                {
                    me.watcher_subscriptions.remove(&watched_root);
                }
                repository.update(ctx, |repository, ctx| {
                    repository.stop_watching(subscriber_id, ctx);
                });
            }
        });
    }

    fn stop_watcher(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let Some((repository, subscriber_id)) = self.watcher_subscriptions.remove(path) else {
            return;
        };
        repository.update(ctx, |repository, ctx| {
            repository.stop_watching(subscriber_id, ctx);
        });
    }

    fn stop_watchers(&mut self, ctx: &mut ModelContext<Self>) {
        for (_, (repository, subscriber_id)) in std::mem::take(&mut self.watcher_subscriptions) {
            repository.update(ctx, |repository, ctx| {
                repository.stop_watching(subscriber_id, ctx);
            });
        }
    }
}

fn update_affects_search_roots(update: &RepositoryUpdate, search_roots: &[PathBuf]) -> bool {
    let affects_root = |path: &Path| {
        search_roots
            .iter()
            .any(|root| path.starts_with(root) || root.starts_with(path))
    };

    update
        .added_or_modified()
        .any(|target| affects_root(&target.path))
        || update
            .deleted
            .iter()
            .any(|target| affects_root(&target.path))
        || update
            .moved
            .iter()
            .any(|(to, from)| affects_root(&to.path) || affects_root(&from.path))
}

fn home_event_affects_user_roots(
    event: &BulkFilesystemWatcherEvent,
    search_roots: &[PluginSearchRoot],
) -> bool {
    let provider_roots = search_roots
        .iter()
        .filter_map(|root| root.path.parent())
        .map(PathBuf::from)
        .collect::<BTreeSet<_>>();
    event
        .added
        .iter()
        .chain(&event.modified)
        .chain(&event.deleted)
        .any(|path| provider_roots.contains(path))
        || event
            .moved
            .iter()
            .any(|(to, from)| provider_roots.contains(to) || provider_roots.contains(from))
}

/// Emits a package-level diagnostic to structured logs.
///
/// Component-level status continues to reach the user through the existing Skills and MCP
/// surfaces; this is the channel for problems that leave no component behind to attach to.
fn log_plugin_diagnostic(diagnostic: &PluginDiagnostic) {
    if diagnostic.is_error() {
        log::warn!("{diagnostic}");
    } else {
        log::info!("{diagnostic}");
    }
}

fn active_frontend() -> PluginFrontend {
    match settings::settings_mode() {
        settings::SettingsMode::Tui => PluginFrontend::Tui,
        _ => PluginFrontend::Gui,
    }
}

impl Entity for PluginManager {
    type Event = PluginManagerEvent;
}

impl SingletonEntity for PluginManager {}

/// Turns a non-watcher teardown step into the event the rest of the client acts on.
///
/// `StopWatchers` is handled synchronously by [`PluginManager`] before later teardown events.
pub(crate) fn teardown_event(step: PluginTeardownStep) -> Option<PluginManagerEvent> {
    match step {
        PluginTeardownStep::StopWatchers => None,
        PluginTeardownStep::WithdrawSkills => Some(PluginManagerEvent::WithdrawSkills),
        PluginTeardownStep::CancelInFlightMcpCalls => {
            Some(PluginManagerEvent::CancelInFlightMcpCalls)
        }
        PluginTeardownStep::UnregisterMcpInstallations { components } => {
            Some(PluginManagerEvent::UnregisterMcpInstallations { components })
        }
    }
}

#[cfg(test)]
#[path = "plugin_manager_tests.rs"]
mod tests;
