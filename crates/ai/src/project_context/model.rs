use std::collections::HashMap;
#[cfg(feature = "local_fs")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use warpui_core::{Entity, ModelContext, SingletonEntity};

use super::GlobalRules;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use repo_metadata::{RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
        use warp_util::standardized_path::StandardizedPath;
    }
}

#[derive(Debug, Default, Clone)]
pub struct ProjectRule {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Default, Clone)]
struct RuleAtPath {
    parent_path: PathBuf,
    warp_md: Option<ProjectRule>,
    agents_md: Option<ProjectRule>,
}

impl RuleAtPath {
    fn respected_rule(&self) -> Option<&ProjectRule> {
        self.warp_md.as_ref().or(self.agents_md.as_ref())
    }
}

#[derive(Debug, Default, Clone)]
pub struct ProjectRulesResult {
    pub root_path: PathBuf,
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
    #[cfg(feature = "local_fs")]
    fn all_rule_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.rules.iter().flat_map(|rule| {
            rule.warp_md
                .iter()
                .chain(rule.agents_md.iter())
                .map(|rule| &rule.path)
        })
    }
    #[cfg(feature = "local_fs")]
    fn retain_rule_paths(&mut self, retained_paths: &HashSet<PathBuf>) {
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
    fn find_active_or_applicable_rules(&self, path: &Path) -> FindRulesResult {
        let mut active_rules = Vec::new();
        let mut available_rule_paths = Vec::new();

        // Collect all applicable rules (rules in directories that are ancestors of the target path)
        for rule in &self.rules {
            if let Some(respected_rule) = rule.respected_rule() {
                // Check if the rule's directory is an ancestor of or equal to the target path
                if path.starts_with(&rule.parent_path) {
                    active_rules.push(respected_rule.clone());
                } else {
                    available_rule_paths.push(respected_rule.path.to_string_lossy().to_string());
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
    fn upsert_rule(&mut self, path: &Path, content: String) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };

        let existing_rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent);

        let rule_file = Some(ProjectRule {
            path: path.to_path_buf(),
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
                    parent_path: parent.to_path_buf(),
                    ..Default::default()
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
#[derive(Debug, Default)]
pub struct ProjectContextModel {
    /// Mapping from directory path to list of rule files found in that directory
    path_to_rules: HashMap<PathBuf, ProjectRules>,
    /// Latest metadata-backed async refresh per project root.
    #[cfg(feature = "local_fs")]
    rule_refresh_generations: HashMap<PathBuf, u64>,
    #[cfg(feature = "local_fs")]
    next_rule_refresh_generation: u64,
    /// File-based global rules and their local watcher state. Kept separate
    /// from `path_to_rules`, which is project-scoped.
    pub(super) global_rules: GlobalRules,
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
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        {
            ctx.subscribe_to_model(
                &RepoMetadataModel::handle(ctx),
                |me, event, ctx| match event {
                    RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Local(repo_path),
                    } => me.refresh_project_rules_for_repo(repo_path.clone(), ctx),
                    RepoMetadataEvent::StandingQueryResultsUpdated {
                        id: RepositoryIdentifier::Local(repo_path),
                        delta,
                    } => {
                        if delta.project_rules_changed() {
                            me.refresh_project_rules_for_repo(repo_path.clone(), ctx);
                        }
                    }
                    RepoMetadataEvent::RepositoryRemoved {
                        id: RepositoryIdentifier::Local(repo_path),
                    } => me.remove_project_rules_for_repo(repo_path, ctx),
                    RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Remote(_),
                    }
                    | RepoMetadataEvent::RepositoryRemoved {
                        id: RepositoryIdentifier::Remote(_),
                    }
                    | RepoMetadataEvent::StandingQueryResultsUpdated {
                        id: RepositoryIdentifier::Remote(_),
                        ..
                    }
                    | RepoMetadataEvent::FileTreeUpdated { .. }
                    | RepoMetadataEvent::FileTreeEntryUpdated { .. }
                    | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                    | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
                },
            );

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
        }

        Self::default()
    }

    /// Reconciles project rule contents from the repository metadata standing result set.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn index_and_store_rules(
        &mut self,
        root_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        #[cfg(feature = "local_fs")]
        {
            let repo_path = StandardizedPath::from_local_canonicalized(&root_path)?;
            let repo_id = RepositoryIdentifier::local(repo_path.clone());
            if RepoMetadataModel::as_ref(ctx)
                .standing_query_results(&repo_id, ctx)
                .is_none()
            {
                RepoMetadataModel::handle(ctx).update(ctx, |metadata, ctx| {
                    metadata.index_lazy_loaded_path(&repo_path, ctx)
                })?;
            }
            self.refresh_project_rules_for_repo(repo_path, ctx);
        }
        Ok(())
    }

    #[cfg(feature = "local_fs")]
    fn refresh_project_rules_for_repo(
        &mut self,
        repo_path: StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(project_root) = repo_path.to_local_path() else {
            return;
        };
        let id = RepositoryIdentifier::local(repo_path);
        let rule_paths = RepoMetadataModel::as_ref(ctx)
            .standing_query_results(&id, ctx)
            .into_iter()
            .flat_map(|results| results.project_rules())
            .filter(|content| !content.is_directory)
            .filter_map(|content| content.path.to_local_path())
            .collect::<Vec<_>>();
        let existing_rules = self
            .path_to_rules
            .get(&project_root)
            .cloned()
            .unwrap_or_default();

        self.next_rule_refresh_generation += 1;
        let refresh_generation = self.next_rule_refresh_generation;
        self.rule_refresh_generations
            .insert(project_root.clone(), refresh_generation);
        let project_root_for_read = project_root.clone();
        ctx.spawn(
            async move { Self::read_standing_project_rules(rule_paths, existing_rules).await },
            move |me, rules, ctx| {
                if me.rule_refresh_generations.get(&project_root_for_read)
                    != Some(&refresh_generation)
                {
                    return;
                }
                let new_paths = rules.all_rule_paths().cloned().collect::<Vec<_>>();
                let previous = me
                    .path_to_rules
                    .insert(project_root_for_read.clone(), rules)
                    .unwrap_or_default();
                let deleted_rules = previous
                    .all_rule_paths()
                    .filter(|path| !new_paths.contains(path))
                    .cloned()
                    .collect();
                let discovered_rules = new_paths
                    .into_iter()
                    .map(|path| ProjectRulePath {
                        path,
                        project_root: project_root_for_read.clone(),
                    })
                    .collect();
                ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                    discovered_rules,
                    deleted_rules,
                }));
                ctx.emit(ProjectContextModelEvent::PathIndexed);
            },
        );
    }

    #[cfg(feature = "local_fs")]
    async fn read_standing_project_rules(
        rule_paths: Vec<PathBuf>,
        mut existing_rules: ProjectRules,
    ) -> ProjectRules {
        let retained_paths = rule_paths.iter().cloned().collect::<HashSet<_>>();
        existing_rules.retain_rule_paths(&retained_paths);

        for rule_path in rule_paths {
            match async_fs::read_to_string(&rule_path).await {
                Ok(content) => existing_rules.upsert_rule(&rule_path, content),
                Err(error) => log::debug!(
                    "Failed to read project rule file {}: {error}",
                    rule_path.display()
                ),
            }
        }
        existing_rules
    }

    #[cfg(feature = "local_fs")]
    fn remove_project_rules_for_repo(
        &mut self,
        repo_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(project_root) = repo_path.to_local_path() else {
            return;
        };
        self.rule_refresh_generations.remove(&project_root);
        if let Some(rules) = self.path_to_rules.remove(&project_root) {
            let deleted_rules = rules.all_rule_paths().cloned().collect();
            ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                discovered_rules: Vec::new(),
                deleted_rules,
            }));
            ctx.emit(ProjectContextModelEvent::PathIndexed);
        }
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
    pub fn find_applicable_project_rules(&self, path: &Path) -> Option<ProjectRulesResult> {
        let mut current_path = path.to_owned();

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

            if !current_path.pop() {
                return None;
            }
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
    pub fn find_applicable_rules(&self, path: &Path) -> Option<ProjectRulesResult> {
        let project_result = self.find_applicable_project_rules(path);

        // Layered precedence: global rules are always included alongside
        // project rules. `global_rules` is a `BTreeMap`, so iteration is
        // sorted by path — deterministic without needing a separate
        // ordering pass.
        let mut active_rules: Vec<ProjectRule> = self.global_rules.active_rules().collect();
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
        // the parent of the first global rule (or empty).
        let root_path = project_root
            .unwrap_or_else(|| self.global_rules.first_rule_parent().unwrap_or_default());

        Some(ProjectRulesResult {
            root_path,
            active_rules,
            additional_rule_paths,
        })
    }

    #[cfg(feature = "local_fs")]
    async fn read_persisted_rules(
        rule_paths: Vec<ProjectRulePath>,
    ) -> HashMap<PathBuf, ProjectRules> {
        let mut rules: HashMap<PathBuf, ProjectRules> = HashMap::new();

        for rule in rule_paths {
            match async_fs::read_to_string(&rule.path).await {
                Ok(content) => {
                    let existing_rules = rules.entry(rule.project_root).or_default();
                    existing_rules.upsert_rule(&rule.path, content);
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

    pub fn indexed_rules(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.path_to_rules.values().flat_map(|rules| {
            rules.rules.iter().filter_map(|rules| {
                rules
                    .respected_rule()
                    .map(|project_rule| project_rule.path.clone())
            })
        })
    }

    /// Absolute paths of every indexed global rule file (e.g. `~/.agents/AGENTS.md`).
    /// Iteration order is sorted by path because global rules are backed by a `BTreeMap`.
    pub fn global_rule_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.global_rules.paths()
    }

    /// Returns the rule file paths associated with a specific workspace root path.
    pub fn rules_for_workspace(&self, workspace_path: &Path) -> Vec<PathBuf> {
        self.path_to_rules
            .get(workspace_path)
            .into_iter()
            .flat_map(|rules| {
                rules.rules.iter().filter_map(|rule| {
                    rule.respected_rule()
                        .map(|project_rule| project_rule.path.clone())
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
