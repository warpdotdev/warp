use super::*;

fn path(path: &str) -> StandardizedPath {
    StandardizedPath::try_new(path).unwrap()
}

fn definitions() -> StandingQueryDefinitions {
    let mut definitions = StandingQueryDefinitions::default();
    definitions.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
    definitions
}

#[test]
fn records_provider_skill_files_and_project_rules() {
    let definitions = definitions();
    let mut results = StandingQueryResults::default();

    results.record_path(Path::new("/repo/.agents/skills"), true, &definitions);
    results.record_path(
        Path::new("/repo/.agents/skills/review/SKILL.md"),
        false,
        &definitions,
    );
    results.record_path(Path::new("/repo/WARP.md"), false, &definitions);
    results.record_path(
        Path::new("/repo/packages/api/AGENTS.md"),
        false,
        &definitions,
    );

    assert!(results
        .project_skills()
        .any(|content| content == &StandingQueryContent::directory(path("/repo/.agents/skills"))));
    assert!(results.project_skills().any(|content| {
        content == &StandingQueryContent::file(path("/repo/.agents/skills/review/SKILL.md"))
    }));
    assert!(results
        .project_rules()
        .any(|content| content == &StandingQueryContent::file(path("/repo/WARP.md"))));
    assert!(results.project_rules().any(|content| {
        content == &StandingQueryContent::file(path("/repo/packages/api/AGENTS.md"))
    }));
}

#[test]
fn replacing_removed_direct_skill_child_can_reupsert_provider_for_hydration() {
    let definitions = definitions();
    let provider = StandingQueryContent::directory(path("/repo/.agents/skills"));
    let skill = StandingQueryContent::file(path("/repo/.agents/skills/review/SKILL.md"));
    let mut results = StandingQueryResults::default();
    results.insert_project_skill(provider.clone());
    results.insert_project_skill(skill.clone());

    let mut discovered = StandingQueryResults::default();
    discovered.record_direct_project_skill_provider_child_change(
        Path::new("/repo/.agents/skills/review"),
        &definitions,
    );
    let delta = results.replace_subtrees(&[path("/repo/.agents/skills/review")], discovered);

    assert_eq!(delta.removed_project_skills, vec![skill]);
    assert_eq!(delta.upserted_project_skills, vec![provider.clone()]);
    assert!(results.project_skills().any(|content| content == &provider));
    assert!(!results
        .project_skills()
        .any(|content| content.path == path("/repo/.agents/skills/review/SKILL.md")));
}

#[test]
fn support_file_beneath_skill_does_not_synthesize_provider_update() {
    let definitions = definitions();
    let mut results = StandingQueryResults::default();

    results.record_path(
        Path::new("/repo/.agents/skills/review/README.md"),
        false,
        &definitions,
    );

    assert!(results.project_skills().next().is_none());
}
