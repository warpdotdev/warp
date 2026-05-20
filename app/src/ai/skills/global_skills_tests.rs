use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use ai::skills::{get_provider_for_path, ParsedSkill, SkillProvider, SkillScope};
use warp_cli::skill::SkillSpec;

use super::{filter_skills_by_spec, resolve_skill_repos};
use crate::ai::cloud_environments::GithubRepo;

#[test]
fn resolve_skill_repos_returns_empty_for_empty_input() {
    let (specs, repos) = resolve_skill_repos(&[]);

    assert!(specs.is_empty());
    assert_eq!(repos, Vec::<GithubRepo>::new());
}

#[test]
fn resolve_skill_repos_skips_parse_failures() {
    let (specs, repos) = resolve_skill_repos(&[
        String::new(),
        "warpdotdev/warp-internal:read-google-doc".to_string(),
    ]);

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].skill_identifier, "read-google-doc");
    assert_eq!(
        repos,
        vec![GithubRepo::new(
            "warpdotdev".to_string(),
            "warp-internal".to_string(),
        )]
    );
}

#[test]
fn resolve_skill_repos_skips_unqualified_and_repo_only_specs() {
    let (_specs, repos) = resolve_skill_repos(&[
        "bare-name".to_string(),
        ".agents/skills/read-google-doc/SKILL.md".to_string(),
        "warp-internal:read-google-doc".to_string(),
    ]);

    assert_eq!(repos, Vec::<GithubRepo>::new());
}

#[test]
fn resolve_skill_repos_collects_org_qualified_repos() {
    let (_specs, repos) = resolve_skill_repos(&[
        "warpdotdev/warp-internal:read-google-doc".to_string(),
        "warpdotdev/warp-server:deploy".to_string(),
    ]);

    assert_eq!(
        repos,
        vec![
            GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string()),
            GithubRepo::new("warpdotdev".to_string(), "warp-server".to_string()),
        ]
    );
}

#[test]
fn filter_skills_by_spec_only_loads_requested_simple_names() {
    let repo_path = Path::new("/work/warp-internal");
    let requested_skill_path = skill_path(repo_path, ".agents", "read-google-doc");
    let other_skill_path = skill_path(repo_path, ".agents", "deploy");
    let skills = vec![
        parsed_skill(requested_skill_path.clone(), "read-google-doc"),
        parsed_skill(other_skill_path, "deploy"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:read-google-doc".to_string()]);

    let filtered = filter_skills_by_spec(repo_path, skills, &specs);

    assert_eq!(skill_paths(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_matches_simple_names_by_parsed_skill_name() {
    let repo_path = Path::new("/work/warp-internal");
    let requested_skill_path = skill_path(repo_path, ".agents", "google-doc");
    let directory_name_match_path = skill_path(repo_path, ".agents", "read-google-doc");
    let skills = vec![
        parsed_skill(requested_skill_path.clone(), "read-google-doc"),
        parsed_skill(directory_name_match_path, "unrelated-skill"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:read-google-doc".to_string()]);

    let filtered = filter_skills_by_spec(repo_path, skills, &specs);

    assert_eq!(skill_paths(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_uses_provider_precedence_for_simple_names() {
    let repo_path = Path::new("/work/warp-internal");
    let agents_skill_path = skill_path(repo_path, ".agents", "deploy");
    let claude_skill_path = skill_path(repo_path, ".claude", "deploy");
    let skills = vec![
        parsed_skill(claude_skill_path, "deploy"),
        parsed_skill(agents_skill_path.clone(), "deploy"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:deploy".to_string()]);

    let filtered = filter_skills_by_spec(repo_path, skills, &specs);

    assert_eq!(skill_paths(filtered), vec![agents_skill_path]);
}

#[test]
fn filter_skills_by_spec_matches_full_path_specs() {
    let repo_path = Path::new("/work/warp-internal");
    let requested_relative_path = PathBuf::from(".claude")
        .join("skills")
        .join("deploy")
        .join("SKILL.md");
    let requested_skill_path = repo_path.join(&requested_relative_path);
    let other_skill_path = skill_path(repo_path, ".agents", "deploy");
    let skills = vec![
        parsed_skill(other_skill_path, "deploy"),
        parsed_skill(requested_skill_path.clone(), "deploy-from-full-path"),
    ];
    let specs = global_specs(&[format!(
        "warpdotdev/warp-internal:{}",
        requested_relative_path.display()
    )]);

    let filtered = filter_skills_by_spec(repo_path, skills, &specs);

    assert_eq!(skill_paths(filtered), vec![requested_skill_path]);
}

fn global_specs(raw_specs: &[String]) -> Vec<SkillSpec> {
    raw_specs
        .iter()
        .map(|raw| SkillSpec::from_str(raw).unwrap())
        .collect()
}

fn skill_path(repo_path: &Path, provider_dir: &str, skill_name: &str) -> PathBuf {
    repo_path
        .join(provider_dir)
        .join("skills")
        .join(skill_name)
        .join("SKILL.md")
}

fn parsed_skill(path: PathBuf, name: &str) -> ParsedSkill {
    let provider = get_provider_for_path(&path).unwrap_or(SkillProvider::Agents);
    ParsedSkill {
        path,
        name: name.to_string(),
        description: String::new(),
        content: String::new(),
        line_range: None,
        provider,
        scope: SkillScope::Project,
    }
}

fn skill_paths(skills: Vec<ParsedSkill>) -> Vec<PathBuf> {
    skills.into_iter().map(|skill| skill.path).collect()
}

#[test]
fn resolve_skill_repos_collapses_duplicates_preserving_first_seen_order() {
    let (_specs, repos) = resolve_skill_repos(&[
        "org-b/foo:first".to_string(),
        "org-a/foo:second".to_string(),
        "org-b/foo:third".to_string(),
    ]);

    assert_eq!(
        repos,
        vec![
            GithubRepo::new("org-b".to_string(), "foo".to_string()),
            GithubRepo::new("org-a".to_string(), "foo".to_string()),
        ]
    );
}
