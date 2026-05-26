use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ai::skills::{
    home_skills_path, parse_skill, ParsedSkill, SkillProvider, SKILL_PROVIDER_DEFINITIONS,
};
use async_channel::Sender;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::repository::{Repository, SubscriberId};
use repo_metadata::{DirectoryWatcher, RepoMetadataModel, RepositoryIdentifier, RepositoryUpdate};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};
use watcher::{BulkFilesystemWatcherEvent, HomeDirectoryWatcher, HomeDirectoryWatcherEvent};

use super::subscribers::{
    HomeSkillSubscriber, ProjectSkillSubscriber, SkillRepositoryMessage, SymlinkSkillSubscriber,
};
use super::utils::{
    find_local_skill_files_on_filesystem, find_skill_files_in_tree,
    find_symlinked_skill_files_in_tree, is_home_provider_path, is_home_skill_directory,
    is_skill_file, read_skills_from_directories, read_skills_from_files,
};
use crate::warp_managed_paths_watcher::{
    filter_repository_update_by_prefix, warp_managed_skill_dirs, WarpManagedPathsWatcher,
    WarpManagedPathsWatcherEvent,
};

#[derive(Debug, PartialEq)]
pub enum SkillWatcherEvent {
    SkillsAdded { skills: Vec<ParsedSkill> },
    SkillsDeleted { paths: Vec<PathBuf> },
}
pub struct SkillWatcher {
    // Channel for sending repository messages from subscribers.
    repository_message_tx: Sender<SkillRepositoryMessage>,
    /// Last known project skill files by repository. Project skill counts are small,
    /// so repo metadata changes trigger a full refresh instead of a subtree diff.
    project_skill_files_by_repo: HashMap<RepositoryIdentifier, HashSet<LocalOrRemotePath>>,
    /// Failed local repos still need the project file watcher path because
    /// repo metadata indexing can fail for oversized repos. This replaces the
    /// previous `watched_repos` set so we only subscribe when fallback is active
    /// and can also clean up the subscriber on repo removal.
    failed_local_project_watchers: HashMap<PathBuf, (ModelHandle<Repository>, SubscriberId)>,
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
        let skill_files: Vec<PathBuf> = repo_paths
            .iter()
            .filter_map(|repo_path| RepositoryIdentifier::try_local(repo_path))
            .flat_map(|repo_id| {
                let mut skill_files = find_skill_files_in_tree(&repo_id, repo_metadata, ctx);
                skill_files.extend(
                    find_symlinked_skill_files_in_tree(&repo_id, repo_metadata, ctx)
                        .into_iter()
                        .map(LocalOrRemotePath::Local),
                );
                skill_files
            })
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
            .collect();
        read_skills_from_files(skill_files)
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
        // TODO: Migrate home/global skill watching onto RepoMetadataModel as well.
        // Project skills have moved there first so local and remote project
        // behavior share one path and avoid a separate local FileWatcher. Home
        // provider directories and symlink target watches still use
        // DirectoryWatcher/HomeDirectoryWatcher for now, but should eventually
        // follow the same model for consistency.
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
        // built, so we can query it for skill files. Project skill updates use
        // RepoMetadataModel for both local and remote repos when available, while
        // local repos fall back to a direct project watcher only if metadata
        // indexing fails.
        ctx.subscribe_to_model(&RepoMetadataModel::handle(ctx), |me, event, ctx| {
            use repo_metadata::wrapper_model::RepoMetadataEvent;
            match event {
                RepoMetadataEvent::RepositoryUpdated { id } => {
                    me.refresh_project_skills_for_repo(id, ctx);
                }
                RepoMetadataEvent::FileTreeEntryUpdated { id } => {
                    me.refresh_project_skills_for_repo(id, ctx);
                }
                RepoMetadataEvent::RepositoryRemoved { id } => {
                    me.remove_project_skills_for_repo(id);
                    me.stop_failed_local_project_watcher(id, ctx);
                }
                RepoMetadataEvent::UpdatingRepositoryFailed { id } => {
                    me.fallback_to_local_project_watcher(id, ctx);
                }
                RepoMetadataEvent::FileTreeUpdated { .. }
                | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
            }
        });

        Self {
            repository_message_tx,
            project_skill_files_by_repo: HashMap::new(),
            failed_local_project_watchers: HashMap::new(),
            watcher_event_tx,
            home_provider_watchers,
            symlink_canonical_to_originals: HashMap::new(),
            symlink_target_watchers: HashMap::new(),
        }
    }

    fn refresh_project_skills_for_repo(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_skill_files: HashSet<LocalOrRemotePath> = {
            let repo_metadata = RepoMetadataModel::as_ref(ctx);
            let mut skill_files = find_skill_files_in_tree(repo_id, repo_metadata, ctx);
            skill_files.extend(
                find_symlinked_skill_files_in_tree(repo_id, repo_metadata, ctx)
                    .into_iter()
                    .map(LocalOrRemotePath::Local),
            );
            skill_files.into_iter().collect()
        };

        let previous_skill_files = self
            .project_skill_files_by_repo
            .get(repo_id)
            .cloned()
            .unwrap_or_default();

        let deleted_paths = previous_skill_files
            .difference(&current_skill_files)
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        if !deleted_paths.is_empty() {
            self.cleanup_symlink_watches(&deleted_paths);
            let _ = self
                .watcher_event_tx
                .try_send(SkillWatcherEvent::SkillsDeleted {
                    paths: deleted_paths,
                });
        }

        // Local hydration intentionally only parses local paths. Remote project
        // skill discovery now comes from RepoMetadataModel, but hydrating/parsing
        // remote content is coming in a subsequent PR with a remote-aware skill
        // identity pipeline.
        let skill_files = current_skill_files
            .iter()
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
            .collect::<Vec<_>>();

        // Project skill counts are expected to be small, so repo metadata updates
        // intentionally trigger a full refresh instead of attempting to diff the
        // changed subtree. This keeps local and remote project-skill behavior on
        // the same RepoMetadataModel path and avoids a separate FileWatcher update
        // path for local repos.
        Self::spawn_read_skills_from_files(skill_files, ctx);

        self.project_skill_files_by_repo
            .insert(repo_id.clone(), current_skill_files);
    }

    fn fallback_to_local_project_watcher(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Local(repo_path) = repo_id else {
            return;
        };
        let Some(local_path) = repo_path.to_local_path() else {
            return;
        };

        self.scan_local_project_skills_from_filesystem(&local_path, ctx);
        self.watch_failed_local_project_repo(local_path, ctx);
    }

    /// Register a failed local project root to watch for skill file changes.
    fn watch_failed_local_project_repo(
        &mut self,
        repo_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.failed_local_project_watchers.contains_key(&repo_path) {
            return;
        }

        let Some(repo_handle) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(&repo_path, ctx)
        else {
            log::warn!(
                "Could not start local project skill fallback watcher for {}; repo is not watched",
                repo_path.display()
            );
            return;
        };

        let subscriber = Box::new(ProjectSkillSubscriber {
            message_tx: self.repository_message_tx.clone(),
        });
        let start = repo_handle.update(ctx, |repo, ctx| repo.start_watching(subscriber, ctx));
        let subscriber_id = start.subscriber_id;
        self.failed_local_project_watchers
            .insert(repo_path.clone(), (repo_handle.clone(), subscriber_id));

        ctx.spawn(start.registration_future, move |me, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start local project skill fallback watcher for {}: {err}",
                    repo_path.display()
                );
                if let Some((repo_handle, subscriber_id)) =
                    me.failed_local_project_watchers.remove(&repo_path)
                {
                    repo_handle.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });
    }

    fn scan_local_project_skills_from_filesystem(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) {
        let skills = read_skills_from_files(find_local_skill_files_on_filesystem(repo_path));
        if skills.is_empty() {
            return;
        }

        self.register_symlink_watches(&skills, ctx);
        let _ = self
            .watcher_event_tx
            .try_send(SkillWatcherEvent::SkillsAdded { skills });
    }

    fn stop_failed_local_project_watcher(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Local(repo_path) = repo_id else {
            return;
        };
        let Some(local_path) = repo_path.to_local_path() else {
            return;
        };
        let Some((repo_handle, subscriber_id)) =
            self.failed_local_project_watchers.remove(&local_path)
        else {
            return;
        };

        repo_handle.update(ctx, |repo, ctx| {
            repo.stop_watching(subscriber_id, ctx);
        });
    }

    fn remove_project_skills_for_repo(&mut self, repo_id: &RepositoryIdentifier) {
        let Some(skill_files) = self.project_skill_files_by_repo.remove(repo_id) else {
            return;
        };
        let deleted_paths = skill_files
            .into_iter()
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        if !deleted_paths.is_empty() {
            self.cleanup_symlink_watches(&deleted_paths);
            let _ = self
                .watcher_event_tx
                .try_send(SkillWatcherEvent::SkillsDeleted {
                    paths: deleted_paths,
                });
        }
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

    fn spawn_read_skills_from_files(
        skill_files: impl IntoIterator<Item = PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let skill_files: Vec<_> = skill_files.into_iter().collect();
        if skill_files.is_empty() {
            return;
        }

        ctx.spawn(
            async move { read_skills_from_files(skill_files) },
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
            SkillRepositoryMessage::ProjectRepositoryUpdate { update } => {
                self.handle_failed_local_project_update(&update, ctx);
            }
            SkillRepositoryMessage::HomeRepositoryUpdate { update } => {
                self.handle_repository_update(&update, ctx);
            }
            SkillRepositoryMessage::SymlinkTargetUpdate { update } => {
                self.handle_symlink_target_update(&update, ctx);
            }
        }
    }

    fn handle_failed_local_project_update(
        &mut self,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut deleted_paths = Vec::new();

        // Process deleted files
        for target_file in &update.deleted {
            deleted_paths.push(target_file.path.clone());
        }

        // Process moved files
        for (to_target, from_target) in &update.moved {
            deleted_paths.push(from_target.path.clone());
            self.handle_failed_local_project_added_or_modified_path(&to_target.path, ctx);
        }

        // Process added or modified files
        for target_file in update.added_or_modified() {
            self.handle_failed_local_project_added_or_modified_path(&target_file.path, ctx);
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
    }

    fn handle_failed_local_project_added_or_modified_path(
        &mut self,
        path: &Path,
        ctx: &mut ModelContext<Self>,
    ) {
        if is_skill_file(path) {
            let skill_file_path = path.to_path_buf();
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
        } else if path.is_symlink() && path.is_dir() && path.join("SKILL.md").exists() {
            let skill_file_path = path.join("SKILL.md");
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
        } else if path.is_dir() {
            // The old local project path queued directory creations until repo
            // metadata caught up. In fallback mode metadata may never catch up
            // for oversized repos, so scan the changed directory directly.
            self.scan_local_project_skills_from_filesystem(path, ctx);
        }
    }
    fn handle_repository_update(
        &mut self,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
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
                home_path_additions.insert(to_target.path.clone());
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
                home_path_additions.insert(target_file.path.clone());
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
