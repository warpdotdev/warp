//! Helpers for resolving per-agent "global" skill specs into repos to
//! ensure are available on disk before the agent runs.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use ai::skills::{provider_rank, ParsedSkill};
use warp_cli::skill::SkillSpec;

use crate::ai::cloud_environments::GithubRepo;

/// Parse raw skill spec strings and resolve the unique set of GitHub repos they reference.
///
/// Specs without an org/repo qualifier are not cloneable, so they are skipped
/// for repo resolution and left to normal on-disk skill discovery.
pub fn resolve_skill_repos(raw_specs: &[String]) -> (Vec<SkillSpec>, Vec<GithubRepo>) {
    let specs: Vec<SkillSpec> = raw_specs
        .iter()
        .filter_map(|raw| match SkillSpec::from_str(raw) {
            Ok(spec) => Some(spec),
            Err(err) => {
                log::warn!("Failed to parse global skill spec '{raw}': {err}");
                None
            }
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut repos = Vec::new();
    for spec in &specs {
        let (Some(org), Some(repo)) = (spec.org.as_ref(), spec.repo.as_ref()) else {
            continue;
        };

        if seen.insert((org.clone(), repo.clone())) {
            repos.push(GithubRepo::new(org.clone(), repo.clone()));
        }
    }

    (specs, repos)
}

/// Returns the subset of skills that were explicitly requested by the given skill specs.
///
/// For simple skill names, this mirrors cached skill resolution by checking parsed skill names
/// in provider precedence order. For full-path specs, it matches the exact path relative to the
/// repo root.
pub fn filter_skills_by_spec(
    repo_path: &Path,
    skills: Vec<ParsedSkill>,
    specs: &[SkillSpec],
) -> Vec<ParsedSkill> {
    if specs.is_empty() || skills.is_empty() {
        return Vec::new();
    }

    let skills_by_path = skills
        .iter()
        .map(|skill| (skill.path.clone(), skill))
        .collect::<HashMap<_, _>>();
    let mut selected_paths = Vec::new();
    let mut seen_paths = HashSet::new();

    for spec in specs {
        if let Some(path) = matching_skill_path(repo_path, &skills_by_path, spec) {
            if seen_paths.insert(path.clone()) {
                selected_paths.push(path);
            }
        }
    }

    let selected_paths = selected_paths.into_iter().collect::<HashSet<_>>();
    skills
        .into_iter()
        .filter(|skill| selected_paths.contains(&skill.path))
        .collect()
}

fn matching_skill_path(
    repo_path: &Path,
    skills_by_path: &HashMap<PathBuf, &ParsedSkill>,
    spec: &SkillSpec,
) -> Option<PathBuf> {
    if spec.is_full_path() {
        let path = repo_path.join(&spec.skill_identifier);
        return skills_by_path.contains_key(&path).then_some(path);
    }
    matching_simple_skill_path(repo_path, skills_by_path, &spec.skill_identifier)
}

fn matching_simple_skill_path(
    repo_path: &Path,
    skills_by_path: &HashMap<PathBuf, &ParsedSkill>,
    skill_name: &str,
) -> Option<PathBuf> {
    let mut matches = skills_by_path
        .values()
        .copied()
        .filter(|skill| skill.path.starts_with(repo_path) && skill.name == skill_name)
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        provider_rank(left.provider)
            .cmp(&provider_rank(right.provider))
            .then_with(|| left.path.cmp(&right.path))
    });
    matches.into_iter().map(|skill| skill.path.clone()).next()
}

#[cfg(test)]
#[path = "global_skills_tests.rs"]
mod tests;
