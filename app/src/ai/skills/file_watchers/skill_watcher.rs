use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use super::{
    subscribers::{
        HomeSkillSubscriber, ProjectSkillSubscriber, SkillRepositoryMessage, SymlinkSkillSubscriber,
    },
    utils::{
        find_skill_directories_in_tree, is_home_provider_path, is_home_skill_directory,
        is_skill_file, read_skills_from_directories,
    },
};
use watcher::{BulkFilesystemWatcherEvent, HomeDirectoryWatcher, HomeDirectoryWatcherEvent};

use crate::server::datetime_ext::DateTimeExt;
use crate::warp_managed_paths_watcher::{
    filter_repository_update_by_prefix, warp_managed_skill_dirs, WarpManagedPathsWatcher,
    WarpManagedPathsWatcherEvent,
};
use ai::skills::{
    home_skills_path, parse_skill, ParsedSkill, SkillProvider, SKILL_PROVIDER_DEFINITIONS,
};
use async_channel::Sender;
use chrono::{DateTime, Duration, Utc};
use repo_metadata::{
    repositories::DetectedRepositories,
    repository::{Repository, SubscriberId},
    DirectoryWatcher, RepoMetadataModel, RepositoryUpdate,
};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

#[derive(Debug, PartialEq)]
pub enum SkillWatcherEvent {
    SkillsAdded { skills: Vec<ParsedSkill> },
    SkillsDeleted { paths: Vec<PathBuf> },
}

// When a new directory is detected by file watchers, we queue it to be scanned for skills later.
// These are processed when the file tree is updated.
// If a directory is left unprocessed for too long, we will drop it.
#[derive(Clone)]
pub struct QueuedProjectDirectoryCreation {
    pub path: PathBuf,
    pub timestamp: DateTime<Utc>,
}

pub struct SkillWatcher {
    // Channel for sending repository messages from subscribers.
    repository_message_tx: Sender<SkillRepositoryMessage>,
    /// Repos we've registered file watchers for (to prevent duplicate subscriptions).
    watched_repos: HashSet<PathBuf>,
    queued_project_directory_creations: Vec<QueuedProjectDirectoryCreation>,
    watcher_event_tx: Sender<SkillWatcherEvent>,
    /// Tracks watchers on home provider directories (e.g. ~/.agents, ~/.claude) so they
    /// can be cleaned up when the directory is deleted.
    home_provider_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
    /// Maps canonical (resolved) SKILL.md paths → set of original symlink-based paths.
    /// Multiple symlinks can resolve to the same canonical file, so we track all of them.
    /// Used to detect changes to the real files behind symlinked skill directories
    /// on platforms where the OS-level watcher (e.g. FSEvents on macOS) does not
    /// follow symlinks.
    symlink_canonical_to_originals: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Watchers for resolved symlink target directories, keyed by canonical
    /// parent directory. Used both as a dedup guard (skip if already watching)
    /// and to hold the subscriber info for error rollback.
    symlink_target_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
}

impl SkillWatcher {
    /// Synchronously reads skills from the given repo paths.
    /// Requires file trees to already be built (i.e. `RepositoryUpdated` has fired).
    /// Returns the parsed skills; the caller is responsible for feeding them into
    /// `SkillManager::handle_skills_added`.
    pub fn read_skills_for_repos(repo_paths: &[PathBuf], ctx: &AppContext) -> Vec<ParsedSkill> {
        let repo_metadata = RepoMetadataModel::as_ref(ctx);
        let skill_dirs: Vec<PathBuf> = repo_paths
            .iter()
            .flat_map(|repo_path| find_skill_directories_in_tree(repo_path, repo_metadata, ctx))
            .collect();

        read_skills_from_directories(skill_dirs)
    }

    pub fn new(ctx: &mut ModelContext<Self>, watcher_event_tx: Sender<SkillWatcherEvent>) -> Self {
        Self::new_internal(ctx, watcher_event_tx, dirs::home_dir())
    }

    /// Test-only constructor that skips home-directory watching so tests are not
    /// polluted by real skills present on the developer's machine.
    #[cfg(test)]
    pub fn new_for_testing(
        ctx: &mut ModelContext<Self>,
        watcher_event_tx: Sender<SkillWatcherEvent>,
    ) -> Self {
        Self::new_internal(ctx, watcher_event_tx, None)
    }

    fn new_internal(
        ctx: &mut ModelContext<Self>,
        watcher_event_tx: Sender<SkillWatcherEvent>,
        home_dir: Option<PathBuf>,
    ) -> Self {
        // Create channel for receiving repository messages (scans and updates)
        let (repository_message_tx, repository_message_rx) = async_channel::unbounded();

        // Subscribe to repository messages for both projects and home directory
        // When a message is received, handle_message is used to dispatch the message to the appropriate handler
        ctx.spawn_stream_local(
            repository_message_rx,
            |me, message, ctx| {
                me.handle_message(message, ctx);
            },
            |_, _| {}, // No cleanup needed when stream ends
        );

        if home_dir.is_some() {
            ctx.subscribe_to_model(
                &HomeDirectoryWatcher::handle(ctx),
                |me, event, ctx| match event {
                    HomeDirectoryWatcherEvent::HomeFilesChanged(event) => {
                        me.handle_home_files_changed(event, ctx);
                    }
                },
            );
            ctx.subscribe_to_model(&WarpManagedPathsWatcher::handle(ctx), |me, event, ctx| {
                me.handle_warp_managed_paths_event(event, ctx);
            });
        }

        // Subscribe to home directory skills via DirectoryWatcher.
        //
        // We watch each skills "parent directory" under the home directory (e.g., `~/.agents`,
        // `~/.claude`) rather than the entire home directory, to reduce watch overhead.
        //
        // Note: This will not create watchers for provider directories that haven't been created yet.
        // We use a separate HomeDirectoryWatcher to detect when those are created and start watching them after they are created.
        let mut home_provider_watchers = HashMap::new();
        if let Some(home_path) = home_dir {
            Self::spawn_read_skills_from_directories(warp_managed_skill_dirs(), ctx);
            let skills_parent_paths: HashSet<PathBuf> = SKILL_PROVIDER_DEFINITIONS
                .iter()
                .filter(|provider| provider.provider != SkillProvider::Warp)
                .filter_map(|provider| {
                    home_skills_path(provider.provider)
                        .and_then(|skills_path| skills_path.parent().map(Path::to_path_buf))
                })
                .filter(|parent| parent.starts_with(&home_path))
                .collect();

            for parent_path in skills_parent_paths {
                Self::watch_home_provider_path(
                    &parent_path,
                    &repository_message_tx,
                    &mut home_provider_watchers,
                    ctx,
                );
            }
        }

        // RepositoryMetadataEvent::RepositoryUpdated fires after the file tree is
        // built, so we can query it for skill directories. This covers both local
        // project repos and environment repos registered via CloudEnvironmentPrep
        // (which flow through DetectedRepositories -> DirectoryWatcher ->
        // RepoMetadataModel).
        ctx.subscribe_to_model(&RepoMetadataModel::handle(ctx), |me, event, ctx| {
            use repo_metadata::wrapper_model::RepoMetadataEvent;
            use repo_metadata::RepositoryIdentifier;
            match event {
                RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Local(path),
                } => {
                    if let Some(local_path) = path.to_local_path() {
                        me.watch_repo(local_path.clone(), ctx);
                        me.scan_repository_for_skills(&local_path, ctx);
                    }
                }
                RepoMetadataEvent::FileTreeEntryUpdated { .. } => {
                    me.handle_queued_project_directory_creations(ctx);
                }
                RepoMetadataEvent::RepositoryUpdated { .. }
                | RepoMetadataEvent::RepositoryRemoved { .. }
                | RepoMetadataEvent::FileTreeUpdated { .. }
                | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
            }
        });

        Self {
            repository_message_tx,
            watched_repos: HashSet::new(),
            queued_project_directory_creations: Vec::new(),
            watcher_event_tx,
            home_provider_watchers,
            symlink_canonical_to_originals: HashMap::new(),
            symlink_target_watchers: HashMap::new(),
        }
    }

    /// Register a project root path to watch for skill file changes.
    fn watch_repo(&mut self, repo_path: PathBuf, ctx: &mut ModelContext<Self>) {
        if self.watched_repos.contains(&repo_path) {
            return;
        }

        // Get the repository handle from DetectedRepositories.
        if let Some(repo_handle) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&repo_path, ctx)
        {
            // Optimistically add the repository to the set of watched repositories to prevent duplicate subscriptions
            self.watched_repos.insert(repo_path.clone());

            let subscriber = Box::new(ProjectSkillSubscriber {
                message_tx: self.repository_message_tx.clone(),
            });

            let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
            ctx.spawn(start.registration_future, move |me, res, ctx| {
                if let Err(err) = res {
                    log::warn!("Failed to start watching project skills directory: {err}");
                    me.watched_repos.remove(&repo_path);
                    repo_handle.update(ctx, |repo, ctx| {
                        repo.stop_watching(start.subscriber_id, ctx)
                    });
                }
            });
        }
    }

    /// Scans a repository for skills using the LocalRepoMetadataModel tree.
    /// This is called when RepositoryMetadataEvent::RepositoryUpdated fires.
    fn scan_repository_for_skills(&mut self, repo_path: &Path, ctx: &mut ModelContext<Self>) {
        let repo_metadata = RepoMetadataModel::as_ref(ctx);

        // Find all skill directories in the tree
        let skill_dirs = find_skill_directories_in_tree(repo_path, repo_metadata, ctx);
        if skill_dirs.is_empty() {
            return;
        }
        Self::spawn_read_skills_from_directories(skill_dirs, ctx);
    }

    fn spawn_read_skills_from_directories(
        skill_dirs: impl IntoIterator<Item = PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let skill_dirs: Vec<_> = skill_dirs.into_iter().collect();
        if skill_dirs.is_empty() {
            return;
        }

        ctx.spawn(
            async move { read_skills_from_directories(skill_dirs) },
            move |me, skills, ctx| {
                if !skills.is_empty() {
                    me.register_symlink_watches(&skills, ctx);
                    let _ = me
                        .watcher_event_tx
                        .try_send(SkillWatcherEvent::SkillsAdded { skills });
                }
            },
        );
    }

    fn handle_message(&mut self, message: SkillRepositoryMessage, ctx: &mut ModelContext<Self>) {
        match message {
            SkillRepositoryMessage::HomeInitialScan { skills } => {
                if skills.is_empty() {
                    return;
                }

                self.register_symlink_watches(&skills, ctx);
                let _ = self
                    .watcher_event_tx
                    .try_send(SkillWatcherEvent::SkillsAdded { skills });
            }
            SkillRepositoryMessage::RepositoryUpdate { update } => {
                self.handle_repository_update(&update, ctx);
            }
            SkillRepositoryMessage::SymlinkTargetUpdate { update } => {
                self.handle_symlink_target_update(&update, ctx);
            }
        }
    }

    fn handle_repository_update(
        &mut self,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut queued_project_directories = HashSet::new();
        let mut home_path_additions = HashSet::new();
        let mut deleted_paths = Vec::new();

        // Process deleted files
        for target_file in &update.deleted {
            deleted_paths.push(target_file.path.clone());
        }

        // Process moved files
        for (to_target, from_target) in &update.moved {
            deleted_paths.push(from_target.path.clone());
            let to_target_path = to_target.path.clone();

            if is_skill_file(&to_target_path) {
                // read the skill from the file system
                let skill = parse_skill(&to_target_path);
                if let Ok(skill) = skill {
                    self.register_symlink_watches(std::slice::from_ref(&skill), ctx);
                    let _ = self
                        .watcher_event_tx
                        .try_send(SkillWatcherEvent::SkillsAdded {
                            skills: vec![skill],
                        });
                }
            } else {
                let repo_path = self.get_watched_repo_path(&to_target.path);
                if let Some(repo_path) = repo_path {
                    if to_target.path.is_dir() {
                        queued_project_directories.insert(repo_path);
                    }
                } else {
                    home_path_additions.insert(to_target.path.clone());
                }
            }
        }

        // Process added or modified files
        for target_file in update.added_or_modified() {
            let target_file_path = target_file.path.clone();
            if is_skill_file(&target_file_path) {
                // read the skill from the file system
                ctx.spawn(
                    async move { parse_skill(&target_file_path) },
                    move |me, skill, ctx| {
                        if let Ok(skill) = skill {
                            me.register_symlink_watches(std::slice::from_ref(&skill), ctx);
                            let _ = me
                                .watcher_event_tx
                                .try_send(SkillWatcherEvent::SkillsAdded {
                                    skills: vec![skill],
                                });
                        }
                    },
                );
            } else if target_file.path.is_symlink()
                && target_file.path.is_dir()
                && target_file.path.join("SKILL.md").exists()
            {
                // Newly created symlinked skill directory — read the skill directly
                // rather than waiting for the queued directory reprocessing cycle.
                let skill_file_path = target_file.path.join("SKILL.md");
                ctx.spawn(
                    async move { parse_skill(&skill_file_path) },
                    move |me, skill, ctx| {
                        if let Ok(skill) = skill {
                            me.register_symlink_watches(std::slice::from_ref(&skill), ctx);
                            let _ = me
                                .watcher_event_tx
                                .try_send(SkillWatcherEvent::SkillsAdded {
                                    skills: vec![skill],
                                });
                        }
                    },
                );
            } else {
                let repo_path = self.get_watched_repo_path(&target_file.path);
                if let Some(repo_path) = repo_path {
                    if target_file.path.is_dir() {
                        queued_project_directories.insert(repo_path);
                    }
                } else {
                    home_path_additions.insert(target_file.path.clone());
                }
            }
        }

        // Read home directory skills in a batch
        let home_skill_directories: HashSet<PathBuf> = home_path_additions
            .into_iter()
            .filter_map(|path| {
                // Conditions for potentially being a valid home directory skill or containing skills:
                // 1. The path is a home directory skill file
                // 2. The path is a home directory skill directory
                // 3. The path is a provider path itself under the home directory
                // We don't need to check #1 because we already checked if this is a skill file
                if is_home_skill_directory(&path) {
                    let parent_directory = path.parent();
                    parent_directory.map(|parent_directory| parent_directory.to_path_buf())
                } else if is_home_provider_path(&path) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        if !home_skill_directories.is_empty() {
            ctx.spawn(
                async move { read_skills_from_directories(home_skill_directories) },
                move |me, skills, ctx| {
                    if !skills.is_empty() {
                        me.register_symlink_watches(&skills, ctx);
                        let _ = me
                            .watcher_event_tx
                            .try_send(SkillWatcherEvent::SkillsAdded { skills });
                    }
                },
            );
        }

        // Process deleted paths in a batch
        if !deleted_paths.is_empty() {
            self.cleanup_symlink_watches(&deleted_paths);
            let _ = self
                .watcher_event_tx
                .try_send(SkillWatcherEvent::SkillsDeleted {
                    paths: deleted_paths,
                });
        }

        // Queue project directory creations for later processing since the file tree is not yet updated
        self.queued_project_directory_creations
            .extend(queued_project_directories.into_iter().map(|path| {
                QueuedProjectDirectoryCreation {
                    path,
                    timestamp: DateTime::now().into(),
                }
            }));
    }

    fn handle_queued_project_directory_creations(&mut self, ctx: &mut ModelContext<Self>) {
        let mut queued_by_repo_path: HashMap<PathBuf, Vec<QueuedProjectDirectoryCreation>> =
            HashMap::new();

        for queued_project_directory_creation in &self.queued_project_directory_creations {
            let repo_path = self.get_watched_repo_path(&queued_project_directory_creation.path);
            if let Some(repo_path) = repo_path {
                queued_by_repo_path
                    .entry(repo_path)
                    .or_default()
                    .push(queued_project_directory_creation.clone());
            }
        }

        let mut queued_project_directory_creations_to_requeue: Vec<QueuedProjectDirectoryCreation> =
            Vec::new();
        let mut skill_dirs_to_read: HashSet<PathBuf> = HashSet::new();

        for (repo_path, queued_project_directory_creations) in queued_by_repo_path {
            // Find all skill directories in the repository
            let repo_metadata = RepoMetadataModel::as_ref(ctx);
            let skill_dirs = find_skill_directories_in_tree(&repo_path, repo_metadata, ctx);
            if skill_dirs.is_empty() {
                continue;
            }

            for queued_project_directory_creation in queued_project_directory_creations {
                let relevant_skill_dirs = skill_dirs
                    .iter()
                    .filter(|skill_dir| {
                        // If the skill_dir is the child of the new directory, we need to read it again
                        // E.g. new dir is /repo/frontend/feature and skill dir is /repo/frontend/feature/.agents/skills
                        // If the new directory is a child of the skill dir, we need to read it again
                        // E.g. skill_dir is /repo/frontend/.agents/skills and new dir is /repo/frontend/.agents/skills/skill-name
                        skill_dir.starts_with(&queued_project_directory_creation.path)
                            || queued_project_directory_creation
                                .path
                                .starts_with(skill_dir)
                    })
                    .collect::<Vec<&PathBuf>>();

                // If the file tree doesn't have the newly created directory, we should requeue it for when the file tree is updated again
                if relevant_skill_dirs.is_empty() {
                    // If 10s after the initial directory creation, the file tree still doesn't have the directory, we will give up and not requeue it
                    let elapsed = DateTime::now()
                        .signed_duration_since(queued_project_directory_creation.timestamp);
                    if elapsed < Duration::seconds(10) {
                        queued_project_directory_creations_to_requeue
                            .push(queued_project_directory_creation.clone());
                    }
                } else {
                    skill_dirs_to_read.extend(relevant_skill_dirs.into_iter().cloned());
                }
            }
        }

        ctx.spawn(
            async move { read_skills_from_directories(skill_dirs_to_read) },
            move |me, skills, ctx| {
                if !skills.is_empty() {
                    me.register_symlink_watches(&skills, ctx);
                    let _ = me
                        .watcher_event_tx
                        .try_send(SkillWatcherEvent::SkillsAdded { skills });
                }
            },
        );

        // Requeue project directory creations that could not be processed immediately
        self.queued_project_directory_creations = queued_project_directory_creations_to_requeue;
    }

    /// Cleans up symlink canonical→original mappings for deleted skill paths.
    ///
    /// The subscriber and `DirectoryWatcher` entry for the canonical directory
    /// are intentionally kept alive so that if the symlink is re-created later,
    /// the event still reaches `handle_symlink_target_update` and is handled
    /// as a new symlink skill.
    fn cleanup_symlink_watches(&mut self, deleted_paths: &[PathBuf]) {
        let mut empty_canonicals = Vec::new();

        for (canonical, originals) in &mut self.symlink_canonical_to_originals {
            originals.retain(|original| {
                !deleted_paths
                    .iter()
                    .any(|deleted| original.starts_with(deleted) || original == deleted)
            });
            if originals.is_empty() {
                empty_canonicals.push(canonical.clone());
            }
        }

        for canonical_path in empty_canonicals {
            self.symlink_canonical_to_originals.remove(&canonical_path);
        }
    }

    /// For each loaded skill, check whether it lives behind a symlink. If so,
    /// resolve the canonical path and register a watch on the target directory
    /// via `DirectoryWatcher` so that modifications to the real file are detected.
    fn register_symlink_watches(&mut self, skills: &[ParsedSkill], ctx: &mut ModelContext<Self>) {
        for skill in skills {
            let original_path = &skill.path;
            let Ok(canonical_path) = dunce::canonicalize(original_path) else {
                continue;
            };
            if canonical_path == *original_path {
                continue; // Not a symlink
            }

            self.symlink_canonical_to_originals
                .entry(canonical_path.clone())
                .or_default()
                .insert(original_path.clone());

            let Some(canonical_dir) = canonical_path.parent() else {
                continue;
            };
            let canonical_dir = canonical_dir.to_path_buf();
            if self.symlink_target_watchers.contains_key(&canonical_dir) {
                continue; // Already watched
            }

            let Ok(std_dir_path) =
                warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
                    &canonical_dir,
                )
            else {
                continue;
            };

            let dir_display = canonical_dir.display().to_string();
            let repo_handle = match DirectoryWatcher::handle(ctx)
                .update(ctx, |watcher, ctx| watcher.add_directory(std_dir_path, ctx))
            {
                Ok(handle) => handle,
                Err(err) => {
                    log::warn!(
                        "Failed to register symlink target directory {dir_display} for watching: {err}"
                    );
                    continue;
                }
            };

            let subscriber = Box::new(SymlinkSkillSubscriber {
                message_tx: self.repository_message_tx.clone(),
            });
            let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
            let subscriber_id = start.subscriber_id;
            self.symlink_target_watchers
                .insert(canonical_dir.clone(), (repo_handle.clone(), subscriber_id));

            ctx.spawn(start.registration_future, move |me, res, ctx| {
                if let Err(err) = res {
                    log::warn!(
                        "Failed to start watching symlink target directory {dir_display}: {err}"
                    );
                    me.symlink_target_watchers.remove(&canonical_dir);
                    repo_handle.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            });
        }
    }

    /// Handle file changes detected in a resolved symlink target directory.
    /// Maps canonical paths back to their original symlink-based skill paths
    /// and re-reads the affected skills.
    fn handle_symlink_target_update(
        &mut self,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        // When the real file behind a symlink is deleted, emit SkillsDeleted
        // so the SkillManager removes the stale entry.
        let deleted_original_paths: Vec<PathBuf> = update
            .deleted
            .iter()
            .flat_map(|target_file| {
                // Exact canonical match
                let exact = self
                    .symlink_canonical_to_originals
                    .get(&target_file.path)
                    .into_iter()
                    .flatten()
                    .cloned();
                // Also match when a parent directory of the canonical path is deleted
                let ancestor = self
                    .symlink_canonical_to_originals
                    .iter()
                    .filter(|(canonical, _)| canonical.starts_with(&target_file.path))
                    .flat_map(|(_, originals)| originals.iter().cloned());
                exact.chain(ancestor)
            })
            .collect();

        if !deleted_original_paths.is_empty() {
            self.cleanup_symlink_watches(&deleted_original_paths);
            let _ = self
                .watcher_event_tx
                .try_send(SkillWatcherEvent::SkillsDeleted {
                    paths: deleted_original_paths,
                });
        }

        for target_file in update.added_or_modified() {
            if let Some(original_paths) = self.symlink_canonical_to_originals.get(&target_file.path)
            {
                for original_path in original_paths.clone() {
                    ctx.spawn(
                        async move { parse_skill(&original_path) },
                        |me, skill, _| {
                            if let Ok(skill) = skill {
                                let _ =
                                    me.watcher_event_tx
                                        .try_send(SkillWatcherEvent::SkillsAdded {
                                            skills: vec![skill],
                                        });
                            }
                        },
                    );
                }
            } else if target_file.path.is_symlink()
                && target_file.path.is_dir()
                && target_file.path.join("SKILL.md").exists()
            {
                // A symlink skill directory was (re-)created. The event routed here
                // because the DirectoryWatcher entry for the canonical target still
                // exists from a previous registration. Parse the skill and re-register.
                let skill_file_path = target_file.path.join("SKILL.md");
                ctx.spawn(
                    async move { parse_skill(&skill_file_path) },
                    move |me, skill, ctx| {
                        if let Ok(skill) = skill {
                            me.register_symlink_watches(std::slice::from_ref(&skill), ctx);
                            let _ = me
                                .watcher_event_tx
                                .try_send(SkillWatcherEvent::SkillsAdded {
                                    skills: vec![skill],
                                });
                        }
                    },
                );
            }
        }
    }

    // Given a path, return the path of the watched repository, if any.
    fn get_watched_repo_path(&self, path: &Path) -> Option<PathBuf> {
        self.watched_repos
            .iter()
            .find(|repo_path| path.starts_with(repo_path))
            .cloned()
    }

    /// Handle changes to top-level files in the home directory.
    /// For skills, these are newly created provider directories
    fn handle_home_files_changed(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut deleted_paths = Vec::new();
        let mut added_paths = Vec::new();

        let provider_root_paths: HashSet<String> = SKILL_PROVIDER_DEFINITIONS
            .iter()
            .filter(|provider| provider.provider != SkillProvider::Warp)
            .filter_map(|provider| {
                let component = provider.skills_path.components().next();
                component.map(|component| component.as_os_str().to_string_lossy().to_string())
            })
            .collect();

        // Process deleted files
        for target_file in event.deleted.iter() {
            let file_name = target_file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if provider_root_paths.contains(&file_name) {
                deleted_paths.push(target_file.clone());
            }
        }

        // Process moved files
        for (to_target, from_target) in event.moved.iter() {
            let from_file_name = from_target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if provider_root_paths.contains(&from_file_name) {
                deleted_paths.push(from_target.clone());
            }
            let to_file_name = to_target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if provider_root_paths.contains(&to_file_name) {
                added_paths.push(to_target.clone());
            }
        }

        // Process added files
        // We don't care about modified files because that doesn't affect existing watchers
        for target_file in event.added.iter() {
            let file_name = target_file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if provider_root_paths.contains(&file_name) {
                added_paths.push(target_file.clone());
            }
        }

        // Clean up directory watchers for deleted provider paths.
        for deleted_path in &deleted_paths {
            if let Some((repo_handle, subscriber_id)) =
                self.home_provider_watchers.remove(deleted_path)
            {
                repo_handle.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });
            }
        }

        if !deleted_paths.is_empty() {
            let _ = self
                .watcher_event_tx
                .try_send(SkillWatcherEvent::SkillsDeleted {
                    paths: deleted_paths,
                });
        }

        for added_path in added_paths {
            // For each newly added provider root path, add a watcher for it
            Self::watch_home_provider_path(
                &added_path,
                &self.repository_message_tx,
                &mut self.home_provider_watchers,
                ctx,
            );
        }
    }

    fn handle_warp_managed_paths_event(
        &mut self,
        event: &WarpManagedPathsWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let WarpManagedPathsWatcherEvent::FilesChanged(update) = event;
        for skill_dir in warp_managed_skill_dirs() {
            if let Some(filtered_update) = filter_repository_update_by_prefix(update, &skill_dir) {
                self.handle_repository_update(&filtered_update, ctx);
            }
        }
    }

    /// Watch a provider path in the home directory (e.g. ~/.agents), storing the handle
    /// and subscriber ID in `home_provider_watchers` so the watcher can be cleaned up
    /// when the directory is deleted.
    fn watch_home_provider_path(
        path: &Path,
        repository_message_tx: &Sender<SkillRepositoryMessage>,
        home_provider_watchers: &mut HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Ok(std_path) =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(path)
        else {
            return;
        };

        let subscriber = Box::new(HomeSkillSubscriber {
            message_tx: repository_message_tx.clone(),
        });

        let parent_path_display = std_path.to_string();
        let repo_handle = match DirectoryWatcher::handle(ctx)
            .update(ctx, |watcher, ctx| watcher.add_directory(std_path, ctx))
        {
            Ok(handle) => handle,
            Err(err) => {
                log::warn!(
                    "Failed to register home skills directory {parent_path_display} for watching: {err}"
                );
                return;
            }
        };

        let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
        let subscriber_id = start.subscriber_id;

        // Store the watcher so it can be cleaned up if the directory is deleted.
        home_provider_watchers.insert(path.to_path_buf(), (repo_handle.clone(), subscriber_id));

        let path_owned = path.to_path_buf();
        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching home skills directory {parent_path_display}: {err}"
                );
                // Remove the stored watcher since registration failed.
                me.home_provider_watchers.remove(&path_owned);
                repo_handle.update(ctx, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });
            }
        });
    }
}

impl Entity for SkillWatcher {
    type Event = SkillWatcherEvent;
}

#[cfg(test)]
#[path = "skill_watcher_tests.rs"]
mod skill_watcher_tests;
