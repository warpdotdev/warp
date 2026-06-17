use std::path::{Path, PathBuf};
use std::str::FromStr;

use ai::skills::{get_provider_for_path, ParsedSkill, SkillProvider, SkillScope};
use warp_cli::skill::SkillSpec;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

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
    let repo_path = std::env::temp_dir().join("work").join("warp-internal");
    let requested_skill_path = skill_path(&repo_path, ".agents", "read-google-doc");
    let other_skill_path = skill_path(&repo_path, ".agents", "deploy");
    let skills = vec![
        parsed_skill(requested_skill_path.clone(), "read-google-doc"),
        parsed_skill(other_skill_path, "deploy"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:read-google-doc".to_string()]);

    let filtered = filter_skills_by_spec(&LocalOrRemotePath::Local(repo_path), skills, &specs);

    assert_eq!(skill_paths(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_matches_full_path_specs_for_remote_repos() {
    let repo_path = remote_path("host-a", "/work/warp-internal");
    let requested_skill_path = repo_path.join(".claude/skills/deploy/SKILL.md");
    let other_host_skill_path =
        remote_path("host-b", "/work/warp-internal").join(".claude/skills/deploy/SKILL.md");
    let skills = vec![
        parsed_skill_at_location(other_host_skill_path, "deploy", SkillProvider::Claude),
        parsed_skill_at_location(
            requested_skill_path.clone(),
            "deploy",
            SkillProvider::Claude,
        ),
    ];
    let specs =
        global_specs(&["warpdotdev/warp-internal:.claude/skills/deploy/SKILL.md".to_string()]);

    let filtered = filter_skills_by_spec(&repo_path, skills, &specs);

    assert_eq!(skill_locations(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_scopes_simple_remote_names_to_the_repo_host() {
    let repo_path = remote_path("host-a", "/work/warp-internal");
    let requested_skill_path = repo_path.join(".claude/skills/deploy/SKILL.md");
    let other_host_skill_path =
        remote_path("host-b", "/work/warp-internal").join(".agents/skills/deploy/SKILL.md");
    let skills = vec![
        parsed_skill_at_location(other_host_skill_path, "deploy", SkillProvider::Agents),
        parsed_skill_at_location(
            requested_skill_path.clone(),
            "deploy",
            SkillProvider::Claude,
        ),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:deploy".to_string()]);

    let filtered = filter_skills_by_spec(&repo_path, skills, &specs);

    assert_eq!(skill_locations(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_matches_simple_names_by_parsed_skill_name() {
    let repo_path = std::env::temp_dir().join("work").join("warp-internal");
    let requested_skill_path = skill_path(&repo_path, ".agents", "google-doc");
    let directory_name_match_path = skill_path(&repo_path, ".agents", "read-google-doc");
    let skills = vec![
        parsed_skill(requested_skill_path.clone(), "read-google-doc"),
        parsed_skill(directory_name_match_path, "unrelated-skill"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:read-google-doc".to_string()]);
    let filtered = filter_skills_by_spec(&LocalOrRemotePath::Local(repo_path), skills, &specs);

    assert_eq!(skill_paths(filtered), vec![requested_skill_path]);
}

#[test]
fn filter_skills_by_spec_uses_provider_precedence_for_simple_names() {
    let repo_path = std::env::temp_dir().join("work").join("warp-internal");
    let agents_skill_path = skill_path(&repo_path, ".agents", "deploy");
    let claude_skill_path = skill_path(&repo_path, ".claude", "deploy");
    let skills = vec![
        parsed_skill(claude_skill_path, "deploy"),
        parsed_skill(agents_skill_path.clone(), "deploy"),
    ];
    let specs = global_specs(&["warpdotdev/warp-internal:deploy".to_string()]);
    let filtered = filter_skills_by_spec(&LocalOrRemotePath::Local(repo_path), skills, &specs);

    assert_eq!(skill_paths(filtered), vec![agents_skill_path]);
}

#[test]
fn filter_skills_by_spec_matches_full_path_specs() {
    let repo_path = std::env::temp_dir().join("work").join("warp-internal");
    let requested_relative_path = PathBuf::from(".claude")
        .join("skills")
        .join("deploy")
        .join("SKILL.md");
    let requested_skill_path = repo_path.join(&requested_relative_path);
    let other_skill_path = skill_path(&repo_path, ".agents", "deploy");
    let skills = vec![
        parsed_skill(other_skill_path, "deploy"),
        parsed_skill(requested_skill_path.clone(), "deploy-from-full-path"),
    ];
    let specs = global_specs(&[format!(
        "warpdotdev/warp-internal:{}",
        requested_relative_path.display()
    )]);
    let filtered = filter_skills_by_spec(&LocalOrRemotePath::Local(repo_path), skills, &specs);

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
    let path = LocalOrRemotePath::Local(path);
    let provider = get_provider_for_path(&path).unwrap_or(SkillProvider::Agents);
    parsed_skill_at_location(path, name, provider)
}

fn parsed_skill_at_location(
    path: LocalOrRemotePath,
    name: &str,
    provider: SkillProvider,
) -> ParsedSkill {
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
    skills
        .into_iter()
        .map(|skill| skill.path.to_local_path().unwrap().to_path_buf())
        .collect()
}

fn skill_locations(skills: Vec<ParsedSkill>) -> Vec<LocalOrRemotePath> {
    skills.into_iter().map(|skill| skill.path).collect()
}

fn remote_path(host_id: &str, path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new(host_id.to_string()),
        StandardizedPath::try_new(path).unwrap(),
    ))
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
