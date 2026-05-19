use std::{
    path::{Path, PathBuf},
    pin::Pin,
};

use futures::Future;

use ai::skills::{read_skills, ParsedSkill, SKILL_PROVIDER_DEFINITIONS};
use async_channel::Sender;
use repo_metadata::{repository::RepositorySubscriber, Repository, RepositoryUpdate};
use warpui::ModelContext;

/// Resolve the skills directory for a home provider and read its skills.
///
/// `provider_path` is the user-facing (original) provider directory — e.g.
/// `~/.agents`. Must be in original form, not the canonical (symlink-resolved)
/// form, because `home_skills_path()`-derived candidates are built off
/// `dirs::home_dir()` and the un-resolved `definition.skills_path`. A canonical
/// `provider_path` would never prefix-match the candidate and the function
/// would return `None`, missing all skills under symlinked providers — the
/// shape of the original hot-reload bug at the startup-scan layer.
///
/// Returns `None` if `home_dir` is absent or no provider definition matches.
pub(super) fn scan_skills_for_home_provider(
    provider_path: &Path,
    home_dir: Option<&Path>,
) -> Option<Vec<ParsedSkill>> {
    let home_dir = home_dir?;
    let skills_path = SKILL_PROVIDER_DEFINITIONS.iter().find_map(|definition| {
        let candidate_path = home_dir.join(definition.skills_path.clone());
        if candidate_path.starts_with(provider_path) {
            Some(candidate_path)
        } else {
            None
        }
    })?;
    Some(read_skills(&skills_path))
}

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
    /// User-facing provider path (e.g. `~/.agents`). Used by `on_scan` to
    /// derive the skills directory; differs from the watched repository's
    /// canonical root when the provider parent is symlinked. Incremental
    /// events take a different path (translated in
    /// `SkillWatcher::handle_repository_update`).
    pub provider_path: PathBuf,
}

impl RepositorySubscriber for HomeSkillSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let provider_path = self.provider_path.clone();
        let home_dir = dirs::home_dir();
        let tx = self.message_tx.clone();

        Box::pin(async move {
            if let Some(skills) = scan_skills_for_home_provider(&provider_path, home_dir.as_deref())
            {
                let _ = tx
                    .send(SkillRepositoryMessage::HomeInitialScan { skills })
                    .await;
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Regression test: when the provider parent is symlinked, `on_scan` must
    /// derive the skills directory from the *original* (user-facing)
    /// `provider_path`, not from the repository's canonical root. A real
    /// `~/.agents` symlinked to a dotfiles location must still scan its skills
    /// at startup. This pins the contract on the helper that `on_scan` delegates to.
    #[test]
    fn scan_skills_for_home_provider_finds_skills_under_original_path() {
        let temp_dir = TempDir::new().unwrap();
        let home = temp_dir.path();
        let skill_dir = home.join(".agents").join("skills").join("test");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test\ndescription: Test skill\n---\nbody\n",
        )
        .unwrap();

        let provider_path = home.join(".agents");

        let skills = scan_skills_for_home_provider(&provider_path, Some(home)).expect(
            "expected a matching SKILL_PROVIDER_DEFINITIONS entry under the original provider path",
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test");
    }

    /// Inverse of the above: a path that doesn't match any provider definition
    /// (e.g. the canonical-form path the buggy code would have passed) returns
    /// `None`. Pins that the `provider_path` parameter is what drives the
    /// lookup — substituting a non-matching path (the bug shape) suppresses the
    /// scan entirely.
    #[test]
    fn scan_skills_for_home_provider_returns_none_for_unmatched_path() {
        let temp_dir = TempDir::new().unwrap();
        let home = temp_dir.path();
        let skill_dir = home.join(".agents").join("skills").join("test");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test\ndescription: Test skill\n---\nbody\n",
        )
        .unwrap();

        let canonical_provider_no_match = home.join("dotfiles-agents");
        let skills = scan_skills_for_home_provider(&canonical_provider_no_match, Some(home));
        assert!(skills.is_none());
    }
}
