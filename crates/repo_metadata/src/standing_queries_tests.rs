use super::*;
fn repo_path(path: &str) -> PathBuf {
    std::env::temp_dir()
        .join("repo_metadata_standing_queries_tests")
        .join(path)
}

fn standardized(path: &Path) -> StandardizedPath {
    StandardizedPath::try_from_local(path).unwrap()
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
    let skills_provider = repo_path(".agents/skills");
    let skill_file = repo_path(".agents/skills/review/SKILL.md");
    let root_rule = repo_path("WARP.md");
    let nested_rule = repo_path("packages/api/AGENTS.md");

    results.record_path(&skills_provider, true, &definitions);
    results.record_path(&skill_file, false, &definitions);
    results.record_path(&root_rule, false, &definitions);
    results.record_path(&nested_rule, false, &definitions);

    assert!(
        results
            .project_skills()
            .any(|content| content
                == &StandingQueryContent::directory(standardized(&skills_provider)))
    );
    assert!(
        results
            .project_skills()
            .any(|content| { content == &StandingQueryContent::file(standardized(&skill_file)) })
    );
    assert!(
        results
            .project_rules()
            .any(|content| content == &StandingQueryContent::file(standardized(&root_rule)))
    );
    assert!(
        results
            .project_rules()
            .any(|content| { content == &StandingQueryContent::file(standardized(&nested_rule)) })
    );
}

#[test]
fn replacing_removed_direct_skill_child_can_reupsert_provider_for_hydration() {
    let definitions = definitions();
    let provider_path = repo_path(".agents/skills");
    let skill_path = repo_path(".agents/skills/review/SKILL.md");
    let removed_skill_dir = repo_path(".agents/skills/review");
    let provider = StandingQueryContent::directory(standardized(&provider_path));
    let skill = StandingQueryContent::file(standardized(&skill_path));
    let mut results = StandingQueryResults::default();
    results.insert_project_skill(provider.clone());
    results.insert_project_skill(skill.clone());

    let mut discovered = StandingQueryResults::default();
    discovered.record_direct_project_skill_provider_child_change(&removed_skill_dir, &definitions);
    let delta = results.replace_subtrees(&[standardized(&removed_skill_dir)], discovered);

    assert_eq!(delta.removed_project_skills, vec![skill]);
    assert_eq!(delta.upserted_project_skills, vec![provider.clone()]);
    assert!(results.project_skills().any(|content| content == &provider));
    assert!(
        !results
            .project_skills()
            .any(|content| content.path == standardized(&skill_path))
    );
}

#[test]
fn support_file_beneath_skill_does_not_synthesize_provider_update() {
    let definitions = definitions();
    let mut results = StandingQueryResults::default();
    let support_file = repo_path(".agents/skills/review/README.md");

    results.record_path(&support_file, false, &definitions);

    assert!(results.project_skills().next().is_none());
}

/// Emulates the open `AGENTS.md` discovery contract that non-Warp agents follow:
/// walk from a working directory up to the repository root, collecting any
/// `AGENTS.md` rule files via the same predicate Warp uses to index project
/// rules. Guards the `WARP.md` → `AGENTS.md` rename so the repo-root agent
/// context file stays present, non-empty, and discoverable.
#[test]
fn repo_root_agents_md_is_discovered_by_rule_file_contract() {
    let definitions = StandingQueryDefinitions::default();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Locate the repository root (the ancestor that holds `.git`).
    let repo_root = manifest_dir
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .expect("repo_metadata crate should live inside the warp git checkout");

    // Walk from the crate dir up to (and including) the repo root, collecting
    // every `AGENTS.md` the rule-file predicate recognizes — the same
    // nearest-file-up-the-tree contract a conformant non-Warp agent uses.
    let discovered: Vec<PathBuf> = manifest_dir
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(repo_root))
        .map(|ancestor| ancestor.join("AGENTS.md"))
        .filter(|candidate| candidate.is_file() && definitions.is_project_rule_file(candidate))
        .collect();

    let root_agents_md = repo_root.join("AGENTS.md");
    assert!(
        discovered.contains(&root_agents_md),
        "repo-root AGENTS.md should be discovered by the rule-file contract; found {discovered:?}"
    );

    let contents =
        std::fs::read_to_string(&root_agents_md).expect("repo-root AGENTS.md should be readable");
    assert!(
        !contents.trim().is_empty(),
        "repo-root AGENTS.md should not be empty"
    );

    // Clean rename: the repo no longer ships a root WARP.md.
    assert!(
        !repo_root.join("WARP.md").exists(),
        "repo-root WARP.md should have been renamed to AGENTS.md"
    );
}
