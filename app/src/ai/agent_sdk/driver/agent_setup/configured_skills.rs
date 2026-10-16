//! Shared skill resolution for normal run configuration and standby preparation.

use std::path::{Path, PathBuf};

use warp_cli::skill::SkillSpec;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelSpawner};

use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::skills::{
    ResolveSkillError, ResolvedSkill, clone_repo_for_skill, resolve_skill_spec,
};

/// Resolves one skill, optionally cloning its repository under the workspace root first.
pub(crate) async fn resolve_skill<H: Entity + 'static>(
    foreground: &ModelSpawner<H>,
    spec: SkillSpec,
    clone_repository: bool,
    working_dir: &Path,
    setup_events: &SetupClientEventReporter,
) -> Result<ResolvedSkill, AgentDriverError> {
    if clone_repository && let (Some(org), Some(repo)) = (&spec.org, &spec.repo) {
        setup_events
            .record_result(SetupStep::SkillRepoClone, async {
                clone_repo_for_skill(org, repo, working_dir)
                    .await
                    .map_err(|error| {
                        AgentDriverError::SkillResolutionFailed(format_skill_resolution_error(
                            error,
                        ))
                    })
            })
            .await?;
    }
    let working_dir = working_dir.to_path_buf();
    let skill = foreground
        .spawn(move |_, ctx| resolve_skill_spec(&spec, &working_dir, ctx))
        .await?
        .map_err(|error| {
            AgentDriverError::SkillResolutionFailed(format_skill_resolution_error(error))
        })?;
    log::debug!(
        "Resolved skill '{}' from {}",
        skill.name,
        skill.skill_path.display()
    );
    Ok(skill)
}

/// Resolves skills in order and returns the first skill plus all skill search directories.
pub(crate) async fn resolve_configured_skills<H: Entity + 'static>(
    foreground: &ModelSpawner<H>,
    specs: Vec<SkillSpec>,
    clone_repositories: bool,
    working_dir: &Path,
    setup_events: &SetupClientEventReporter,
) -> Result<(Option<ResolvedSkill>, Vec<PathBuf>), AgentDriverError> {
    if !specs.is_empty() && !FeatureFlag::OzPlatformSkills.is_enabled() {
        return Err(AgentDriverError::SkillResolutionFailed(
            "Skills are not supported in this client build".to_owned(),
        ));
    }
    let mut resolved = Vec::with_capacity(specs.len());
    for spec in specs {
        resolved.push(
            resolve_skill(
                foreground,
                spec,
                clone_repositories,
                working_dir,
                setup_events,
            )
            .await?,
        );
    }
    selected_execution_skills(resolved)
}

/// Keeps the first skill for prompt context and collects unique search paths for `WARP_SKILL_DIRS`.
pub(crate) fn selected_execution_skills(
    skills: Vec<ResolvedSkill>,
) -> Result<(Option<ResolvedSkill>, Vec<PathBuf>), AgentDriverError> {
    let mut first = None;
    let mut discovery_dirs = Vec::new();
    for skill in skills {
        let skills_dir = skill
            .skill_path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                AgentDriverError::SkillResolutionFailed(
                    "Resolved skill has no parent skill directory".to_owned(),
                )
            })?
            .to_path_buf();
        if skills_dir.to_string_lossy().contains(',') {
            return Err(AgentDriverError::SkillResolutionFailed(
                "Resolved skill directory contains a comma".to_owned(),
            ));
        }
        if !discovery_dirs.contains(&skills_dir) {
            discovery_dirs.push(skills_dir);
        }
        if first.is_none() {
            first = Some(skill);
        }
    }
    Ok((first, discovery_dirs))
}

/// Adds skill search directories without duplicates or changes to the existing search order.
pub(crate) fn merge_skill_dirs(dirs: &mut Vec<PathBuf>, discovery_dirs: Vec<PathBuf>) {
    for dir in discovery_dirs {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
}

fn format_skill_resolution_error(err: ResolveSkillError) -> String {
    match err {
        ResolveSkillError::NotFound { skill } => {
            format!("Skill '{skill}' not found")
        }
        ResolveSkillError::RepoNotFound { repo } => {
            format!("Repository '{repo}' not found")
        }
        ResolveSkillError::Ambiguous { skill, candidates } => {
            let mut msg = format!(
                "Skill '{skill}' is ambiguous; specify as repo:skill_name\n\nCandidates:\n"
            );
            for path in candidates {
                msg.push_str(&format!("- {}\n", path.display()));
            }
            msg
        }
        ResolveSkillError::OrgMismatch {
            repo,
            expected,
            found,
        } => {
            format!("Repository '{repo}' found but belongs to org '{found}', expected '{expected}'")
        }
        ResolveSkillError::ParseFailed { path, message } => {
            format!("Failed to parse skill file {}: {message}", path.display())
        }
        ResolveSkillError::CloneFailed { org, repo, message } => {
            format!("Failed to clone repository '{org}/{repo}': {message}")
        }
    }
}
