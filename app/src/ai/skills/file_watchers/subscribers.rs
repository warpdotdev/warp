use std::pin::Pin;

use ai::skills::{read_skills, ParsedSkill, SKILL_PROVIDER_DEFINITIONS};
use async_channel::Sender;
use futures::Future;
use repo_metadata::repository::RepositorySubscriber;
use repo_metadata::{Repository, RepositoryUpdate};
use warpui::ModelContext;

/// Messages sent from [`RepositorySubscriber`]s to [`SkillManager`].
pub enum SkillRepositoryMessage {
    /// Initial scan of a home skills directory (e.g., `~/.agents`).
    HomeInitialScan { skills: Vec<ParsedSkill> },
    /// Incremental file system updates from either a home provider directory or a project skills directory.
    RepositoryUpdate { update: RepositoryUpdate },
    /// File changes detected in a resolved symlink target directory.
    SymlinkTargetUpdate { update: RepositoryUpdate },
}

/// A repository subscriber for project directories that forwards file change events to [`SkillManager`].
pub struct ProjectSkillSubscriber {
    pub message_tx: Sender<SkillRepositoryMessage>,
}

impl RepositorySubscriber for ProjectSkillSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // Initial skill scanning is handled via RepositoryMetadataEvent::RepositoryUpdated,
        // which fires AFTER the file tree is built. This subscriber is only used for
        // incremental file change updates via on_files_updated.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let tx = self.message_tx.clone();
        let update = update.clone();

        Box::pin(async move {
            let _ = tx
                .send(SkillRepositoryMessage::RepositoryUpdate { update })
                .await;
        })
    }
}

/// A repository subscriber for resolved symlink target directories.
/// Forwards file change events as `SymlinkTargetUpdate` so the `SkillWatcher`
/// can map canonical paths back to their original symlink-based skill paths.
pub struct SymlinkSkillSubscriber {
    pub message_tx: Sender<SkillRepositoryMessage>,
}

impl RepositorySubscriber for SymlinkSkillSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // Skills are already loaded at the symlink path; no initial scan needed.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let tx = self.message_tx.clone();
        let update = update.clone();

        Box::pin(async move {
            let _ = tx
                .send(SkillRepositoryMessage::SymlinkTargetUpdate { update })
                .await;
        })
    }
}

/// A repository subscriber for home skills directories that forwards file change events to [`SkillManager`].
pub struct HomeSkillSubscriber {
    pub message_tx: Sender<SkillRepositoryMessage>,
}

impl RepositorySubscriber for HomeSkillSubscriber {
    fn on_scan(
        &mut self,
        repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // For a home skills directory, this is the provider directory. E.g. ~/.agents
        let repo_path = repository.root_dir().to_local_path_lossy();
        let home_dir = dirs::home_dir();
        let tx = self.message_tx.clone();

        Box::pin(async move {
            if let Some(home_dir) = home_dir {
                let skills_path = SKILL_PROVIDER_DEFINITIONS.iter().find_map(|definition| {
                    let candidate_path = home_dir.join(definition.skills_path.clone());
                    if candidate_path.starts_with(&repo_path) {
                        Some(candidate_path)
                    } else {
                        None
                    }
                });
                if let Some(skills_path) = skills_path {
                    let skills = read_skills(&skills_path);
                    let _ = tx
                        .send(SkillRepositoryMessage::HomeInitialScan { skills })
                        .await;
                }
            }
        })
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let tx = self.message_tx.clone();
        let update = update.clone();

        Box::pin(async move {
            let _ = tx
                .send(SkillRepositoryMessage::RepositoryUpdate { update })
                .await;
        })
    }
}
