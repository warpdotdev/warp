use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use futures::future::BoxFuture;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui_core::{AppContext, Entity, ModelContext, SingletonEntity};

use super::GlobalRules;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use std::pin::Pin;

        use async_channel::Sender;
        use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
        use repo_metadata::repository::{Repository, RepositorySubscriber, SubscriberId};
        use repo_metadata::{
            evaluate_standing_queries, DirectoryWatcher, RepoMetadataEvent, RepoMetadataModel,
            RepositoryIdentifier, RepositoryUpdate, StandingQueryContent,
            StandingQueryDefinitions, StandingQueryWalkOptions, STANDING_QUERY_WALK_MAX_DEPTH,
        };
        use warp_core::features::FeatureFlag;
        use warp_util::remote_path::RemotePath;
        use warp_util::standardized_path::StandardizedPath;
        use warpui_core::ModelHandle;
    }
}

pub type ProjectRuleContents = Vec<(LocalOrRemotePath, String)>;
/// App-provided transport for reading the exact rule paths discovered by repository metadata.
///
/// This remains injected because remote file reads are implemented in the app crate.
pub type ProjectRuleContentReader = fn(
    Vec<LocalOrRemotePath>,
    &AppContext,
) -> BoxFuture<'static, anyhow::Result<ProjectRuleContents>>;

/// A repository update from a directly watched local rule root, tagged with
/// the watched root so the model can re-walk the right repository.
#[cfg(feature = "local_fs")]
type LocalRuleWatchMessage = (StandardizedPath, RepositoryUpdate);

/// Repository subscriber that forwards file change events for a watched local
/// rule root back to [`ProjectContextModel`].
#[cfg(feature = "local_fs")]
struct ProjectRuleSubscriber {
    message_tx: Sender<LocalRuleWatchMessage>,
}

#[cfg(feature = "local_fs")]
impl RepositorySubscriber for ProjectRuleSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        // The initial walk is triggered directly when discovery starts; this
        // subscriber only keeps rule results fresh afterward.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let tx = self.message_tx.clone();
        let root_dir = repository.root_dir().clone();
        let update = update.clone();
        Box::pin(async move {
            let _ = tx.send((root_dir, update)).await;
        })
    }
}

/// Returns whether the walk-based standing-query path handles this repository:
/// local repositories only, and only when `OnTheFlyStandingQueries` is enabled.
/// Remote repositories always use the metadata-backed standing results.
#[cfg(feature = "local_fs")]
fn local_uses_standing_query_walk(repo_id: &RepositoryIdentifier) -> bool {
    matches!(repo_id, RepositoryIdentifier::Local(_))
        && FeatureFlag::OnTheFlyStandingQueries.is_enabled()
}

/// Returns whether a changed path can affect project rule discovery.
#[cfg(feature = "local_fs")]
fn path_is_project_rule_relevant(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("WARP.md") || name.eq_ignore_ascii_case("AGENTS.md")
        })
}

#[cfg(feature = "local_fs")]
fn standing_project_rule_paths<'a>(
    repo_id: &RepositoryIdentifier,
    contents: impl IntoIterator<Item = &'a StandingQueryContent>,
) -> Vec<LocalOrRemotePath> {
    contents
        .into_iter()
        .filter(|content| !content.is_directory)
        .filter_map(|content| match repo_id {
            RepositoryIdentifier::Local(_) => {
                content.path.to_local_path().map(LocalOrRemotePath::Local)
            }
            RepositoryIdentifier::Remote(remote_root) => Some(LocalOrRemotePath::Remote(
                RemotePath::new(remote_root.host_id.clone(), content.path.clone()),
            )),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ProjectRule {
    pub path: LocalOrRemotePath,
    pub content: String,
}

#[derive(Debug, Clone)]
struct RuleAtPath {
    parent_path: LocalOrRemotePath,
    warp_md: Option<ProjectRule>,
    agents_md: Option<ProjectRule>,
}

impl RuleAtPath {
    fn respected_rule(&self) -> Option<&ProjectRule> {
        self.warp_md.as_ref().or(self.agents_md.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct ProjectRulesResult {
    pub root_path: LocalOrRemotePath,
    pub active_rules: Vec<ProjectRule>,
    pub additional_rule_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRulePath {
    pub path: PathBuf,
    pub project_root: PathBuf,
}

struct FindRulesResult {
    /// Rules that are active and should be eagerly applied.
    active_rules: Vec<ProjectRule>,
    /// Rule paths that are currently not active but available to be applied if
    /// a file under its directory is edited.
    available_rule_paths: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct ProjectRules {
    rules: Vec<RuleAtPath>,
}

impl ProjectRules {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn rule_paths(&self) -> impl Iterator<Item = &LocalOrRemotePath> {
        self.rules.iter().flat_map(|rule| {
            rule.warp_md
                .iter()
                .chain(rule.agents_md.iter())
                .map(|rule| &rule.path)
        })
    }
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn local_rule_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.rule_paths()
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
    }
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn retain_rule_paths(&mut self, retained_paths: &HashSet<LocalOrRemotePath>) {
        self.rules.retain_mut(|rule| {
            if rule
                .warp_md
                .as_ref()
                .is_some_and(|rule| !retained_paths.contains(&rule.path))
            {
                rule.warp_md = None;
            }
            if rule
                .agents_md
                .as_ref()
                .is_some_and(|rule| !retained_paths.contains(&rule.path))
            {
                rule.agents_md = None;
            }
            rule.warp_md.is_some() || rule.agents_md.is_some()
        });
    }
    /// Finds the set of rules that are active in the given path and the set that are available to be applied.
    fn find_active_or_applicable_rules(&self, path: &LocalOrRemotePath) -> FindRulesResult {
        let mut active_rules = Vec::new();
        let mut available_rule_paths = Vec::new();

        // Collect all applicable rules (rules in directories that are ancestors of the target path)
        for rule in &self.rules {
            if let Some(respected_rule) = rule.respected_rule() {
                // Check if the rule's directory is an ancestor of or equal to the target path
                if path.starts_with(&rule.parent_path) {
                    active_rules.push(respected_rule.clone());
                } else {
                    available_rule_paths.push(respected_rule.path.display_path());
                }
            }
        }

        FindRulesResult {
            active_rules,
            available_rule_paths,
        }
    }

    /// Upsert a rule to the set of project rules. This will create a new RuleAtPath entry if none exists and update the existing one
    /// otherwise.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn upsert_rule(&mut self, path: &LocalOrRemotePath, content: String) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Some(file_name) = path.file_name() else {
            return;
        };

        let existing_rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent);

        let rule_file = Some(ProjectRule {
            path: path.clone(),
            content,
        });

        match existing_rule {
            Some(rule) => {
                if file_name.to_lowercase() == "warp.md" {
                    rule.warp_md = rule_file;
                } else if file_name.to_lowercase() == "agents.md" {
                    rule.agents_md = rule_file;
                }
            }
            None => {
                let mut rule = RuleAtPath {
                    parent_path: parent,
                    warp_md: None,
                    agents_md: None,
                };
                if file_name.to_lowercase() == "warp.md" {
                    rule.warp_md = rule_file;
                } else if file_name.to_lowercase() == "agents.md" {
                    rule.agents_md = rule_file;
                }
                self.rules.push(rule);
            }
        };
    }
}

/// Singleton model that keeps track of mapping between paths and rule files
/// Currently supports WARP.md files, but designed to be extensible
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Default)]
pub struct ProjectContextModel {
    /// Mapping from directory path to list of rule files found in that directory
    path_to_rules: HashMap<LocalOrRemotePath, ProjectRules>,
    /// Latest metadata-backed async refresh per exact repository identity.
    /// This uses the same identifier carried by metadata events rather than an arbitrary file path.
    #[cfg(feature = "local_fs")]
    rule_refresh_generations: HashMap<RepositoryIdentifier, u64>,
    #[cfg(feature = "local_fs")]
    next_rule_refresh_generation: u64,
    /// App-provided rule content reader, stored so walk-based refreshes
    /// triggered outside metadata events can read rule contents.
    #[cfg(feature = "local_fs")]
    project_rule_content_reader: Option<ProjectRuleContentReader>,
    /// Sender for updates from directly watched local rule roots. Present only
    /// after `new_from_persisted` wires up the receiving stream.
    #[cfg(feature = "local_fs")]
    rule_walk_message_tx: Option<Sender<LocalRuleWatchMessage>>,
    /// Local rule roots with a direct `Repository` watcher. Only populated when
    /// `OnTheFlyStandingQueries` is enabled: the watcher triggers standing-query
    /// re-walks when WARP.md/AGENTS.md files change.
    #[cfg(feature = "local_fs")]
    local_rule_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
    /// File-based global rules and their local watcher state. Kept separate
    /// from `path_to_rules`, which is project-scoped.
    pub(super) global_rules: GlobalRules,
    /// File-based global rules published by connected remote hosts. Kept
    /// separate from local globals so existing local Rules UI accessors remain
    /// local-only.
    remote_global_rules: HashMap<HostId, Vec<ProjectRule>>,
}

#[derive(Default, Debug)]
pub struct RulesDelta {
    pub discovered_rules: Vec<ProjectRulePath>,
    pub deleted_rules: Vec<PathBuf>,
}

impl RulesDelta {
    /// Merge another delta into this one, preserving the ordering of operations.
    ///
    /// When the same path appears across sequential deltas the *last* operation
    /// wins. For example:
    ///   - (add A, delete A) → net effect is **delete**
    ///   - (delete A, add A) → net effect is **add**
    ///
    /// This is important because consumers (e.g. persistence) apply the delta
    /// incrementally; a symmetric "cancel both sides" approach would silently
    /// drop real state changes.
    #[cfg(test)]
    fn merge(&mut self, other: RulesDelta) {
        // Each newly-discovered path supersedes any prior deletion or earlier
        // discovery of the same path.
        for discovered in &other.discovered_rules {
            self.deleted_rules.retain(|p| *p != discovered.path);
            self.discovered_rules.retain(|r| r.path != discovered.path);
        }
        // Each newly-deleted path supersedes any prior discovery or earlier
        // deletion of the same path.
        for deleted in &other.deleted_rules {
            self.discovered_rules.retain(|r| r.path != *deleted);
            self.deleted_rules.retain(|p| *p != *deleted);
        }
        self.discovered_rules.extend(other.discovered_rules);
        self.deleted_rules.extend(other.deleted_rules);
    }
}

#[derive(Default, Debug)]
pub struct GlobalRulesDelta {
    pub discovered_rules: Vec<PathBuf>,
    pub deleted_rules: Vec<PathBuf>,
}

/// Events emitted by the ProjectContextModel
pub enum ProjectContextModelEvent {
    /// Emitted when a path has been indexed
    PathIndexed,
    /// Emitted when the known set of rule files changed
    KnownRulesChanged(RulesDelta),
    /// Emitted when the set of indexed global rule files changed
    GlobalRulesChanged(GlobalRulesDelta),
}

impl ProjectContextModel {
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn new_from_persisted(
        persisted_rules: Vec<ProjectRulePath>,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg_attr(not(feature = "local_fs"), allow(unused_mut))]
        let mut model = Self::default();
        #[cfg(feature = "local_fs")]
        {
            model.project_rule_content_reader = Some(project_rule_content_reader);

            // Receive updates from directly watched local rule roots (used by
            // the flag-on walk-based discovery path).
            let (rule_walk_tx, rule_walk_rx) = async_channel::unbounded();
            model.rule_walk_message_tx = Some(rule_walk_tx);
            ctx.spawn_stream_local(
                rule_walk_rx,
                |me, (root_dir, update): LocalRuleWatchMessage, ctx| {
                    if FeatureFlag::OnTheFlyStandingQueries.is_enabled() {
                        me.handle_local_rule_watch_update(root_dir, &update, ctx);
                    }
                },
                |_, _| {},
            );

            // With `OnTheFlyStandingQueries` enabled, local rule discovery is
            // driven by repo detection and the direct rule-root watchers rather
            // than eager repo metadata indexing. Remote repos keep the
            // metadata-backed path in both modes.
            ctx.subscribe_to_model(&RepoMetadataModel::handle(ctx), move |me, _, event, ctx| {
                match event {
                    RepoMetadataEvent::RepositoryUpdated { id } => {
                        if local_uses_standing_query_walk(id) {
                            me.ensure_local_rule_discovery(id, ctx);
                        } else {
                            me.refresh_project_rules_for_repo(
                                id.clone(),
                                project_rule_content_reader,
                                ctx,
                            );
                        }
                    }
                    RepoMetadataEvent::StandingQueryResultsUpdated { id, delta } => {
                        if local_uses_standing_query_walk(id) {
                            // Freshness is driven by the rule-root watcher re-walks.
                        } else if delta.project_rules_changed() {
                            me.refresh_project_rules_for_repo(
                                id.clone(),
                                project_rule_content_reader,
                                ctx,
                            );
                        }
                    }
                    RepoMetadataEvent::RepositoryRemoved { id } => {
                        me.remove_project_rules_for_repo(id, ctx);
                    }
                    RepoMetadataEvent::FileTreeUpdated { .. }
                    | RepoMetadataEvent::FileTreeEntryUpdated { .. }
                    | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                    | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
                }
            });

            // With `OnTheFlyStandingQueries` enabled, repo detection is the
            // primary trigger for local rule discovery.
            ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
                if !FeatureFlag::OnTheFlyStandingQueries.is_enabled() {
                    return;
                }
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
                let repo_id =
                    RepositoryIdentifier::local(repository.as_ref(ctx).root_dir().clone());
                me.ensure_local_rule_discovery(&repo_id, ctx);
            });

            ctx.spawn(
                async move { Self::read_persisted_rules(persisted_rules).await },
                |me, mut res, ctx| {
                    // Metadata refreshes may have completed before persistence loads; retain
                    // the fresher metadata-backed state for overlapping roots.
                    res.extend(me.path_to_rules.drain());
                    me.path_to_rules = res;
                    ctx.emit(ProjectContextModelEvent::PathIndexed);
                },
            );

            // Remote snapshots may have arrived before this model subscribed to metadata events,
            // so hydrate any remote repositories that are already tracked.
            let remote_repo_ids = RepoMetadataModel::as_ref(ctx)
                .remote_repository_ids(ctx)
                .cloned()
                .map(RepositoryIdentifier::Remote)
                .collect::<Vec<_>>();
            for repo_id in remote_repo_ids {
                model.refresh_project_rules_for_repo(repo_id, project_rule_content_reader, ctx);
            }
        }

        model
    }

    /// Reconciles project rule contents from the repository metadata standing result set.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn index_and_store_rules(
        &mut self,
        root_path: PathBuf,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        #[cfg(feature = "local_fs")]
        {
            let repo_path = StandardizedPath::from_local_canonicalized(&root_path)?;
            let repo_id = RepositoryIdentifier::local(repo_path.clone());
            if FeatureFlag::OnTheFlyStandingQueries.is_enabled() {
                // Walk-based discovery: no repo metadata tracking needed.
                self.ensure_local_rule_discovery(&repo_id, ctx);
                return Ok(());
            }
            if RepoMetadataModel::as_ref(ctx)
                .standing_query_results(&repo_id, ctx)
                .is_none()
            {
                RepoMetadataModel::handle(ctx).update(ctx, |metadata, ctx| {
                    metadata.index_lazy_loaded_path(&repo_path, ctx)
                })?;
            }
            self.refresh_project_rules_for_repo(repo_id, project_rule_content_reader, ctx);
        }
        Ok(())
    }

    /// Ensures the walk-based rule discovery path is active for a local root:
    /// registers the direct rule-root watcher if needed and runs the initial
    /// standing-query walk on first registration. Subsequent freshness comes
    /// from the watcher's re-walks.
    #[cfg(feature = "local_fs")]
    fn ensure_local_rule_discovery(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Local(repo_root) = repo_id else {
            return;
        };
        let Some(local_path) = repo_root.to_local_path() else {
            return;
        };
        let newly_watched = !self.local_rule_watchers.contains_key(&local_path);
        self.watch_local_rule_root(local_path, ctx);
        if newly_watched {
            self.refresh_local_project_rules_via_walk(repo_id.clone(), ctx);
        }
    }

    /// Registers a direct `Repository` watcher on a local rule root so
    /// WARP.md/AGENTS.md changes trigger standing-query re-walks.
    #[cfg(feature = "local_fs")]
    fn watch_local_rule_root(&mut self, root_path: PathBuf, ctx: &mut ModelContext<Self>) {
        if self.local_rule_watchers.contains_key(&root_path) {
            return;
        }
        let Some(message_tx) = self.rule_walk_message_tx.clone() else {
            return;
        };
        let Ok(std_path) = StandardizedPath::from_local_canonicalized(&root_path) else {
            return;
        };
        // `add_directory` returns the existing registration when the root is
        // already watched (e.g. as a detected repo), so this works for both
        // git repositories and plain directories.
        let repo_handle = match DirectoryWatcher::handle(ctx)
            .update(ctx, |watcher, ctx| watcher.add_directory(std_path, ctx))
        {
            Ok(handle) => handle,
            Err(err) => {
                log::warn!(
                    "Failed to register rule root {} for watching: {err}",
                    root_path.display()
                );
                return;
            }
        };

        let subscriber = Box::new(ProjectRuleSubscriber { message_tx });
        let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
        let subscriber_id = start.subscriber_id;
        self.local_rule_watchers
            .insert(root_path.clone(), (repo_handle.clone(), subscriber_id));

        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching rule root {}: {err}",
                    root_path.display()
                );
                if let Some((repo_handle, subscriber_id)) =
                    me.local_rule_watchers.remove(&root_path)
                {
                    repo_handle.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });
    }

    /// Re-discovers a local root's project rules with the standalone
    /// standing-query walk on a background thread.
    #[cfg(feature = "local_fs")]
    fn refresh_local_project_rules_via_walk(
        &mut self,
        repo_id: RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Local(repo_root) = &repo_id else {
            return;
        };
        let Some(local_path) = repo_root.to_local_path() else {
            return;
        };
        let Some(project_rule_content_reader) = self.project_rule_content_reader else {
            return;
        };

        // Git repositories are walked in full (matching eager-index standing
        // coverage); other directories only surface first-level rules
        // (matching the lazily-loaded path coverage).
        let max_depth = if local_path.join(".git").exists() {
            STANDING_QUERY_WALK_MAX_DEPTH
        } else {
            1
        };

        self.next_rule_refresh_generation += 1;
        let refresh_generation = self.next_rule_refresh_generation;
        self.rule_refresh_generations
            .insert(repo_id.clone(), refresh_generation);

        ctx.spawn(
            async move {
                // Force-include the skill provider directories to match the
                // eager walk, which always descends into them.
                let force_included_paths: Vec<PathBuf> = crate::skills::SKILL_PROVIDER_DEFINITIONS
                    .iter()
                    .map(|provider| provider.skills_path.clone())
                    .collect();
                let options = StandingQueryWalkOptions {
                    max_depth,
                    force_included_paths,
                };
                let results = evaluate_standing_queries(
                    &local_path,
                    &StandingQueryDefinitions::default(),
                    &options,
                );
                results
                    .project_rules()
                    .filter(|content| !content.is_directory)
                    .filter_map(|content| {
                        content.path.to_local_path().map(LocalOrRemotePath::Local)
                    })
                    .collect::<Vec<_>>()
            },
            move |me, rule_paths, ctx| {
                if me.rule_refresh_generations.get(&repo_id) != Some(&refresh_generation) {
                    return;
                }
                me.read_rule_contents_and_apply(
                    repo_id,
                    refresh_generation,
                    rule_paths,
                    project_rule_content_reader,
                    ctx,
                );
            },
        );
    }

    /// Handles an update from a directly watched rule root: rule-relevant
    /// changes trigger a full re-walk of the root's standing rule queries.
    #[cfg(feature = "local_fs")]
    fn handle_local_rule_watch_update(
        &mut self,
        root_dir: StandardizedPath,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let repo_id = RepositoryIdentifier::local(root_dir);
        if !self.update_touches_project_rules(&repo_id, update) {
            return;
        }
        self.refresh_local_project_rules_via_walk(repo_id, ctx);
    }

    /// Returns whether a repository update can affect project rule discovery:
    /// a changed path is a rule file, or a deletion/move removed a directory
    /// above a known rule file.
    #[cfg(feature = "local_fs")]
    fn update_touches_project_rules(
        &self,
        repo_id: &RepositoryIdentifier,
        update: &RepositoryUpdate,
    ) -> bool {
        let changed_paths = update
            .added_or_modified()
            .chain(update.deleted.iter())
            .chain(update.moved.keys())
            .chain(update.moved.values());
        for target in changed_paths {
            if path_is_project_rule_relevant(&target.path) {
                return true;
            }
        }

        let known_rule_paths: Vec<PathBuf> = repo_id
            .to_local_or_remote_path()
            .and_then(|project_root| self.path_to_rules.get(&project_root))
            .map(|rules| rules.local_rule_paths().collect())
            .unwrap_or_default();
        if known_rule_paths.is_empty() {
            return false;
        }
        update
            .deleted
            .iter()
            .chain(update.moved.values())
            .any(|removed| {
                known_rule_paths
                    .iter()
                    .any(|rule| rule.starts_with(&removed.path))
            })
    }

    #[cfg(feature = "local_fs")]
    fn refresh_project_rules_for_repo(
        &mut self,
        repo_id: RepositoryIdentifier,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) {
        if repo_id.to_local_or_remote_path().is_none() {
            return;
        };
        let rule_paths = standing_project_rule_paths(
            &repo_id,
            RepoMetadataModel::as_ref(ctx)
                .standing_query_results(&repo_id, ctx)
                .into_iter()
                .flat_map(|results| results.project_rules()),
        );
        self.next_rule_refresh_generation += 1;
        let refresh_generation = self.next_rule_refresh_generation;
        self.rule_refresh_generations
            .insert(repo_id.clone(), refresh_generation);
        self.read_rule_contents_and_apply(
            repo_id,
            refresh_generation,
            rule_paths,
            project_rule_content_reader,
            ctx,
        );
    }

    /// Reads the contents of the given rule paths and reconciles them into the
    /// repository's rule state, guarded by the refresh generation.
    #[cfg(feature = "local_fs")]
    fn read_rule_contents_and_apply(
        &mut self,
        repo_id: RepositoryIdentifier,
        refresh_generation: u64,
        rule_paths: Vec<LocalOrRemotePath>,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) {
        let read_rule_contents = project_rule_content_reader(rule_paths.clone(), ctx);
        let repo_id_for_result = repo_id;
        ctx.spawn(read_rule_contents, move |me, result, ctx| {
            if me.rule_refresh_generations.get(&repo_id_for_result) != Some(&refresh_generation) {
                return;
            }
            match result {
                Ok(contents) => {
                    let Some(project_root) = repo_id_for_result.to_local_or_remote_path() else {
                        return;
                    };
                    let existing_rules = me
                        .path_to_rules
                        .get(&project_root)
                        .cloned()
                        .unwrap_or_default();
                    let rules = Self::reconcile_project_rules(rule_paths, contents, existing_rules);
                    me.apply_project_rules(repo_id_for_result, rules, ctx);
                }
                Err(error) => log::warn!("Failed to read project rules: {error}"),
            }
        });
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn reconcile_project_rules(
        rule_paths: Vec<LocalOrRemotePath>,
        rule_contents: ProjectRuleContents,
        mut existing_rules: ProjectRules,
    ) -> ProjectRules {
        let retained_paths = rule_paths.iter().cloned().collect::<HashSet<_>>();
        existing_rules.retain_rule_paths(&retained_paths);
        for (path, content) in rule_contents {
            existing_rules.upsert_rule(&path, content);
        }
        existing_rules
    }

    /// Stops and forgets the direct rule-root watcher for a local root, if any.
    #[cfg(feature = "local_fs")]
    fn stop_local_rule_watcher(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Local(repo_root) = repo_id else {
            return;
        };
        let Some(local_path) = repo_root.to_local_path() else {
            return;
        };
        let Some((repo_handle, subscriber_id)) = self.local_rule_watchers.remove(&local_path)
        else {
            return;
        };
        repo_handle.update(ctx, |repo, ctx| {
            repo.stop_watching(subscriber_id, ctx);
        });
    }

    #[cfg(feature = "local_fs")]
    fn remove_project_rules_for_repo(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        self.rule_refresh_generations.remove(repo_id);
        self.stop_local_rule_watcher(repo_id, ctx);
        let Some(project_root) = repo_id.to_local_or_remote_path() else {
            return;
        };
        if let Some(rules) = self.path_to_rules.remove(&project_root) {
            // KnownRulesChanged is consumed by local persistence and carries local PathBufs.
            // Remote removals still update in-memory state and emit PathIndexed below.
            if matches!(repo_id, RepositoryIdentifier::Local(_)) {
                let deleted_rules = rules.local_rule_paths().collect();
                ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                    discovered_rules: Vec::new(),
                    deleted_rules,
                }));
            }
            ctx.emit(ProjectContextModelEvent::PathIndexed);
        }
    }

    #[cfg(feature = "local_fs")]
    fn apply_project_rules(
        &mut self,
        repo_id: RepositoryIdentifier,
        rules: ProjectRules,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(project_root) = repo_id.to_local_or_remote_path() else {
            return;
        };
        if let RepositoryIdentifier::Local(local_root) = &repo_id {
            let Some(local_root) = local_root.to_local_path() else {
                return;
            };
            let new_paths = rules.local_rule_paths().collect::<Vec<_>>();
            let previous = self
                .path_to_rules
                .insert(project_root, rules)
                .unwrap_or_default();
            let deleted_rules = previous
                .local_rule_paths()
                .filter(|path| !new_paths.contains(path))
                .collect();
            let discovered_rules = new_paths
                .into_iter()
                .map(|path| ProjectRulePath {
                    path,
                    project_root: local_root.clone(),
                })
                .collect();
            ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                discovered_rules,
                deleted_rules,
            }));
        } else {
            self.path_to_rules.insert(project_root, rules);
        }
        ctx.emit(ProjectContextModelEvent::PathIndexed);
    }

    /// Index all configured global rule sources.
    ///
    /// `ProjectContextModel` remains the public rule-context facade; the
    /// global source registry, cache, and watcher plumbing live in
    /// `global_rules`.
    pub fn index_global_rules(&mut self, ctx: &mut ModelContext<Self>) {
        self.global_rules.index(ctx);
    }

    /// Project-only rule lookup. Returns `Some` only when an indexed project
    /// root above `path` actually contributes a rule — globals are
    /// deliberately ignored.
    ///
    /// Use this for callers that need a project-initialization signal rather
    /// than the full rule context sent to agents.
    pub fn find_applicable_project_rules(
        &self,
        path: &LocalOrRemotePath,
    ) -> Option<ProjectRulesResult> {
        let mut current_path = path.clone();

        // Walk upwards from `path` toward the filesystem root, stopping at the
        // first directory we have indexed project rules for. `path_to_rules`
        // is keyed by indexed project root, so popping the path produces
        // every ancestor directory until we hit a known root or `pop()`
        // returns false (we've reached the top of the path).
        loop {
            if let Some(rules) = self.path_to_rules.get(&current_path) {
                let result = rules.find_active_or_applicable_rules(path);
                if result.active_rules.is_empty() && result.available_rule_paths.is_empty() {
                    return None;
                }
                return Some(ProjectRulesResult {
                    root_path: current_path,
                    active_rules: result.active_rules,
                    additional_rule_paths: result.available_rule_paths,
                });
            }

            current_path = current_path.parent()?;
        }
    }

    /// Returns the rules applicable to `path`, layering global rules on top of
    /// any project rules discovered up the directory tree.
    ///
    /// Precedence is `global > project WARP.md > project AGENTS.md`. Globals
    /// are always included (when present) regardless of project state; the
    /// existing in-directory `WARP.md > AGENTS.md` shadow inside
    /// [`RuleAtPath::respected_rule`] still applies to project rules.
    ///
    /// This is the entry point used by `BlocklistAIContextModel` when packing
    /// `AIAgentContext::ProjectRules` for an agent query. Callers that need
    /// a project-only signal should use
    /// [`Self::find_applicable_project_rules`] instead.
    pub fn find_applicable_rules(&self, path: &LocalOrRemotePath) -> Option<ProjectRulesResult> {
        let project_result = self.find_applicable_project_rules(path);

        // Layered precedence: global rules are always included alongside
        // project rules. `global_rules` is a `BTreeMap`, so iteration is
        // sorted by path — deterministic without needing a separate
        // ordering pass.
        let mut active_rules: Vec<ProjectRule> = self.global_rules.active_rules().collect();
        if let Some(remote) = path.as_remote() {
            active_rules.extend(
                self.remote_global_rules
                    .get(&remote.host_id)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        let (project_root, additional_rule_paths) = match project_result {
            Some(project) => {
                active_rules.extend(project.active_rules);
                (Some(project.root_path), project.additional_rule_paths)
            }
            None => (None, Vec::new()),
        };

        if active_rules.is_empty() && additional_rule_paths.is_empty() {
            return None;
        }

        // Use the indexed project root when available; otherwise fall back to
        // the parent of the first local or remote global rule.
        let root_path =
            project_root.or_else(|| active_rules.first().and_then(|rule| rule.path.parent()))?;

        Some(ProjectRulesResult {
            root_path,
            active_rules,
            additional_rule_paths,
        })
    }

    #[cfg(feature = "local_fs")]
    async fn read_persisted_rules(
        rule_paths: Vec<ProjectRulePath>,
    ) -> HashMap<LocalOrRemotePath, ProjectRules> {
        let mut rules: HashMap<LocalOrRemotePath, ProjectRules> = HashMap::new();

        for rule in rule_paths {
            match async_fs::read_to_string(&rule.path).await {
                Ok(content) => {
                    let existing_rules = rules
                        .entry(LocalOrRemotePath::Local(rule.project_root))
                        .or_default();
                    existing_rules.upsert_rule(&LocalOrRemotePath::Local(rule.path), content);
                }
                Err(e) => {
                    log::debug!(
                        "Failed to read rule file from persistence {}: {}",
                        rule.path.display(),
                        e
                    );
                    // Continue processing other files even if one fails
                }
            }
        }

        rules
    }

    pub fn indexed_rules(&self) -> impl Iterator<Item = LocalOrRemotePath> + '_ {
        self.path_to_rules.values().flat_map(|rules| {
            rules.rules.iter().filter_map(|rules| {
                rules
                    .respected_rule()
                    .map(|project_rule| project_rule.path.clone())
            })
        })
    }

    /// Absolute locations of every indexed global rule file (e.g. `~/.agents/AGENTS.md`).
    /// Iteration order is sorted by path because global rules are backed by a `BTreeMap`.
    pub fn global_rule_paths(&self) -> impl Iterator<Item = LocalOrRemotePath> + '_ {
        self.global_rules.paths()
    }

    /// Returns every indexed global rule with its cached content, sorted by path.
    pub fn global_rules(&self) -> impl Iterator<Item = ProjectRule> + '_ {
        self.global_rules.active_rules()
    }
    /// Replaces the file-based global rule catalog for one remote host.
    pub fn set_remote_global_rules(&mut self, host_id: HostId, mut rules: Vec<ProjectRule>) {
        rules.sort_by_key(|rule| rule.path.display_path());
        self.remote_global_rules.insert(host_id, rules);
    }

    /// Removes the file-based global rule catalog for a disconnected remote host.
    pub fn remove_remote_global_rules(&mut self, host_id: &HostId) {
        self.remote_global_rules.remove(host_id);
    }

    /// Returns the rule file paths associated with a specific workspace root path.
    pub fn rules_for_workspace(&self, workspace_path: &Path) -> Vec<PathBuf> {
        self.path_to_rules
            .get(&LocalOrRemotePath::Local(workspace_path.to_path_buf()))
            .into_iter()
            .flat_map(|rules| {
                rules.rules.iter().filter_map(|rule| {
                    rule.respected_rule().and_then(|project_rule| {
                        project_rule.path.to_local_path().map(Path::to_path_buf)
                    })
                })
            })
            .collect()
    }
}

impl Entity for ProjectContextModel {
    type Event = ProjectContextModelEvent;
}

impl SingletonEntity for ProjectContextModel {}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
