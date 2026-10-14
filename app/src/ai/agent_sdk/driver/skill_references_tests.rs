use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef};
use warpui::App;

use super::super::environment_checkout_protocol::{
    CheckoutFailureKind, CheckoutOutcome, CheckoutReport, CheckoutRequest,
};
use super::*;
use crate::server::server_api::ai::MockAIClient;

const INVALID_SKILL_MD: &str = "---\nname: [unterminated\n---\nBody\n";

fn fake_sha(seed: char) -> String {
    seed.to_string().repeat(40)
}

/// Writes `<dir>/<folder>/SKILL.md` with frontmatter `name`.
fn write_skill(dir: &Path, folder: &str, name: &str) -> PathBuf {
    let skill_dir = dir.join(folder);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test skill\n---\nBody\n"),
    )
    .unwrap();
    skill_dir
}

fn write_reference(dir: &Path, file_name: &str, contents: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(file_name), contents).unwrap();
}

/// What the fake checkout helper does for a source repository, keyed by repository name.
enum SourceFixture {
    /// Succeeds; each `(folder, contents)` becomes `<folder>/SKILL.md`.
    Skills(Vec<(&'static str, String)>),
    Fails(CheckoutFailureKind),
}

fn skill(folder: &'static str, name: &str) -> (&'static str, String) {
    (folder, format!("---\nname: {name}\n---\nBody\n"))
}

struct Workspace {
    _temp: TempDir,
    root: PathBuf,
    factory_skills: PathBuf,
    agent_skills: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let factory_skills = root.join("definition/skills");
        let agent_skills = root.join("definition/agents/triage/skills");
        fs::create_dir_all(&factory_skills).unwrap();
        fs::create_dir_all(&agent_skills).unwrap();
        Self {
            _temp: temp,
            root,
            factory_skills,
            agent_skills,
        }
    }

    fn skill_dirs(&self) -> Vec<PathBuf> {
        vec![self.agent_skills.clone(), self.factory_skills.clone()]
    }

    fn sources(&self) -> PathBuf {
        self.root.join("skill-sources/sources")
    }

    fn generated(&self) -> PathBuf {
        self.root.join("skill-sources/skills")
    }

    /// Discovers and resolves the reference files, standing in for the checkout helper with
    /// `fixtures`. Sources without a fixture produce no outcome.
    fn resolve(&self, fixtures: &[(&str, SourceFixture)]) -> SkillReferenceResolution {
        let dirs = self.skill_dirs();
        let discovered = discover(&dirs, RepositoryForge::GitHub).expect("reference files");
        let plan = discovered.checkout_plan(&self.sources());
        let report = self.run_fake_checkouts(&plan.batch.repositories, fixtures);
        let checkouts = plan.into_checkouts(Some(&report));
        discovered.materialize(&dirs, &dirs, &checkouts, &self.generated())
    }

    fn run_fake_checkouts(
        &self,
        requests: &[CheckoutRequest],
        fixtures: &[(&str, SourceFixture)],
    ) -> CheckoutReport {
        let outcomes = requests
            .iter()
            .enumerate()
            .filter_map(|(request_index, request)| {
                let (_, fixture) = fixtures
                    .iter()
                    .find(|(name, _)| *name == request.source.repo_name)?;
                let failure = match fixture {
                    SourceFixture::Skills(skills) => {
                        let checkout = self.sources().join(&request.checkout_name);
                        fs::create_dir_all(&checkout).unwrap();
                        for (folder, contents) in skills {
                            fs::create_dir_all(checkout.join(folder)).unwrap();
                            fs::write(checkout.join(folder).join("SKILL.md"), contents).unwrap();
                        }
                        None
                    }
                    SourceFixture::Fails(kind) => Some(*kind),
                };
                Some(CheckoutOutcome {
                    request_index,
                    failure,
                    diagnostics: String::new(),
                    duration_ms: 0,
                    resolved_head: failure.is_none().then(|| fake_sha('a')),
                })
            })
            .collect();
        CheckoutReport {
            outcomes,
            identity_diagnostics: None,
        }
    }
}

fn loaded_names(report: &ReferenceFileReport) -> Vec<&str> {
    report
        .loaded
        .iter()
        .map(|skill| skill.name.as_str())
        .collect()
}

fn unresolved(report: &ReferenceFileReport) -> Vec<(&str, UnresolvedReason)> {
    report
        .unresolved
        .iter()
        .map(|entry| (entry.entry.as_str(), entry.reason))
        .collect()
}

fn linked_skills(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn folders_without_reference_files_are_left_alone() {
    let workspace = Workspace::new();
    write_skill(&workspace.factory_skills, "deploy", "deploy");
    // YAML nested inside a skill folder is part of that skill, not a reference file.
    write_reference(
        &workspace.factory_skills.join("deploy"),
        "config.yaml",
        "source: acme/shared-skills\ninclude: [x]\n",
    );
    assert!(discover(&workspace.skill_dirs(), RepositoryForge::GitHub).is_none());
}

#[test]
fn entries_resolve_by_name_path_and_glob_and_excludes_apply_last() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "shared.yaml",
        "source: acme/shared-skills@v1.4.0\ninclude:\n  - safe-secrets\n  - tools/release-notes/SKILL.md\n  - \"frontend/*\"\n  - \"legacy-*\"\nexclude:\n  - \"legacy-old\"\n  - frontend/forms\n",
    );
    let resolution = workspace.resolve(&[(
        "shared-skills",
        SourceFixture::Skills(vec![
            skill(".agents/skills/safe-secrets", "safe-secrets"),
            skill("tools/release-notes", "release-notes"),
            skill("frontend/buttons", "buttons"),
            skill("frontend/forms", "forms"),
            skill("frontend/nested/deep", "deep"),
            skill(".claude/skills/legacy-new", "legacy-new"),
            skill(".claude/skills/legacy-old", "legacy-old"),
        ]),
    )]);

    let [report] = resolution.report.references.as_slice() else {
        panic!("expected one report");
    };
    assert_eq!(report.reference_file, "skills/shared.yaml");
    assert_eq!(report.level, ReferenceLevel::Factory);
    assert_eq!(
        loaded_names(report),
        ["safe-secrets", "release-notes", "buttons", "legacy-new"]
    );
    assert_eq!(report.loaded[1].path, "tools/release-notes");
    assert!(report.unresolved.is_empty());
    let source = report.source.as_ref().unwrap();
    assert_eq!(source.requested_ref.as_deref(), Some("v1.4.0"));
    assert_eq!(source.resolved_sha, Some(fake_sha('a')));

    let generated = workspace.generated().join("1");
    assert_eq!(
        resolution.skill_dirs,
        vec![
            workspace.agent_skills.clone(),
            workspace.factory_skills.clone(),
            generated.clone(),
        ]
    );
    assert_eq!(
        linked_skills(&generated),
        ["buttons", "legacy-new", "release-notes", "safe-secrets"]
    );
}

#[test]
fn names_prefer_earlier_standard_folders() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "shared.yaml",
        "source: acme/shared-skills\ninclude: [triage]\n",
    );
    let resolution = workspace.resolve(&[(
        "shared-skills",
        SourceFixture::Skills(vec![
            skill(".claude/skills/triage", "triage"),
            skill(".warp/skills/triage-v2", "triage"),
        ]),
    )]);
    assert_eq!(
        resolution.report.references[0].loaded[0].path,
        ".warp/skills/triage-v2"
    );
}

#[cfg(unix)]
#[test]
fn referenced_skills_are_their_folders_inside_the_checkout() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "shared.yaml",
        "source: acme/shared-skills\ninclude:\n  - skills/inside\n  - skills/escape\n",
    );
    let outside = write_skill(&workspace.root, "outside", "escape");
    let resolution = workspace.resolve(&[(
        "shared-skills",
        SourceFixture::Skills(vec![skill("skills/inside", "inside")]),
    )]);
    let checkout = workspace
        .sources()
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(checkout.join("README.md"), "repository file").unwrap();

    // A symlinked folder can't pull in files from outside the source repository.
    std::os::unix::fs::symlink(&outside, checkout.join("skills/escape")).unwrap();
    let resolution_with_escape = {
        let dirs = workspace.skill_dirs();
        let discovered = discover(&dirs, RepositoryForge::GitHub).unwrap();
        let plan = discovered.checkout_plan(&workspace.sources());
        let report = CheckoutReport {
            outcomes: vec![CheckoutOutcome {
                request_index: 0,
                failure: None,
                diagnostics: String::new(),
                duration_ms: 0,
                resolved_head: Some(fake_sha('b')),
            }],
            identity_diagnostics: None,
        };
        let checkouts = plan.into_checkouts(Some(&report));
        fs::remove_dir_all(workspace.generated()).unwrap();
        discovered.materialize(&dirs, &dirs, &checkouts, &workspace.generated())
    };
    assert_eq!(
        unresolved(&resolution.report.references[0]),
        [("skills/escape", UnresolvedReason::NoMatch)]
    );
    assert_eq!(
        unresolved(&resolution_with_escape.report.references[0]),
        [("skills/escape", UnresolvedReason::NoMatch)]
    );

    let link = workspace.generated().join("1/inside");
    assert_eq!(
        fs::read_link(&link).unwrap(),
        checkout.join("skills/inside")
    );
    assert!(link.join("SKILL.md").is_file());
    assert_eq!(linked_skills(&workspace.generated().join("1")), ["inside"]);
}

#[test]
fn levels_apply_in_folder_order_and_earlier_names_win() {
    let workspace = Workspace::new();
    write_skill(&workspace.agent_skills, "agent-only", "agent-only");
    write_skill(&workspace.factory_skills, "factory-only", "factory-only");
    write_reference(
        &workspace.agent_skills,
        "agent.yaml",
        "source: acme/agent-skills\ninclude: [\"*\"]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "factory.yaml",
        "source: acme/factory-skills\ninclude: [\"*\"]\n",
    );
    let resolution = workspace.resolve(&[
        (
            "agent-skills",
            SourceFixture::Skills(vec![
                skill(".agents/skills/agent-only", "agent-only"),
                skill(".agents/skills/factory-only", "factory-only"),
                skill(".agents/skills/shared", "shared"),
            ]),
        ),
        (
            "factory-skills",
            SourceFixture::Skills(vec![
                skill(".agents/skills/shared", "shared"),
                skill(".agents/skills/factory-only", "factory-only"),
                skill(".agents/skills/extra", "extra"),
            ]),
        ),
    ]);

    let [agent, factory] = resolution.report.references.as_slice() else {
        panic!("expected two reports");
    };
    assert_eq!(agent.reference_file, "agents/triage/skills/agent.yaml");
    assert_eq!(agent.level, ReferenceLevel::Agent);
    assert_eq!(agent.agent.as_deref(), Some("triage"));
    // The agent's own folder wins over its references, which win over the Factory's folder.
    assert_eq!(loaded_names(agent), ["factory-only", "shared"]);
    // Everything earlier wins over the Factory's references, and losing on name isn't a warning.
    assert_eq!(loaded_names(factory), ["extra"]);
    assert_eq!(resolution.report.warning_count(), 0);
    assert_eq!(
        resolution.skill_dirs,
        vec![
            workspace.agent_skills.clone(),
            workspace.generated().join("0"),
            workspace.factory_skills.clone(),
            workspace.generated().join("1"),
        ]
    );
}

#[test]
fn files_resolve_in_name_order_and_glob_collisions_warn() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "b-second.yml",
        "source: acme/second\ninclude: [\"lint-*\"]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "a-first.yaml",
        "source: acme/first\ninclude: [\"lint-*\"]\n",
    );
    let resolution = workspace.resolve(&[
        (
            "first",
            SourceFixture::Skills(vec![skill(".agents/skills/lint-rust", "lint-rust")]),
        ),
        (
            "second",
            SourceFixture::Skills(vec![
                skill(".agents/skills/lint-rust", "lint-rust"),
                skill(".agents/skills/lint-go", "lint-go"),
            ]),
        ),
    ]);

    let [first, second] = resolution.report.references.as_slice() else {
        panic!("expected two reports");
    };
    assert_eq!(first.reference_file, "skills/a-first.yaml");
    assert_eq!(loaded_names(first), ["lint-rust"]);
    assert_eq!(loaded_names(second), ["lint-go"]);
    assert_eq!(
        unresolved(second),
        [("lint-*", UnresolvedReason::Collision)]
    );
    let link = workspace.generated().join("1/lint-rust");
    assert!(
        fs::read_link(link)
            .unwrap()
            .starts_with(workspace.sources())
    );
}

#[test]
fn every_failure_is_recorded_and_the_rest_still_loads() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "a-broken.yaml",
        "source: acme/broken\ninclude: [x]\nunknown: true\n",
    );
    write_reference(
        &workspace.factory_skills,
        "b-unreachable.yaml",
        "source: acme/unreachable\ninclude: [one, two]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "c-missing-ref.yaml",
        "source: acme/pinned@v9\ninclude: [one]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "d-partial.yaml",
        "source: acme/partial\ninclude:\n  - good\n  - missing\n  - broken\n  - skills/broken-path\n  - \"nothing-*\"\n",
    );
    write_reference(
        &workspace.factory_skills,
        "e-no-outcome.yaml",
        "source: acme/silent\ninclude: [one]\n",
    );
    let resolution = workspace.resolve(&[
        (
            "unreachable",
            SourceFixture::Fails(CheckoutFailureKind::Clone),
        ),
        (
            "pinned",
            SourceFixture::Fails(CheckoutFailureKind::Checkout),
        ),
        (
            "partial",
            SourceFixture::Skills(vec![
                skill(".agents/skills/good", "good"),
                (".agents/skills/broken", INVALID_SKILL_MD.to_owned()),
                ("skills/broken-path", INVALID_SKILL_MD.to_owned()),
            ]),
        ),
    ]);

    let reports = &resolution.report.references;
    assert_eq!(reports.len(), 5);
    assert!(reports[0].source.is_none());
    assert_eq!(
        unresolved(&reports[0]),
        [("", UnresolvedReason::InvalidReferenceFile)]
    );
    assert_eq!(
        unresolved(&reports[1]),
        [
            ("one", UnresolvedReason::CloneFailed),
            ("two", UnresolvedReason::CloneFailed)
        ]
    );
    assert_eq!(reports[1].source.as_ref().unwrap().resolved_sha, None);
    assert_eq!(
        unresolved(&reports[2]),
        [("one", UnresolvedReason::RefNotFound)]
    );
    assert_eq!(loaded_names(&reports[3]), ["good"]);
    assert_eq!(
        unresolved(&reports[3]),
        [
            ("missing", UnresolvedReason::NoMatch),
            ("broken", UnresolvedReason::InvalidSkill),
            ("skills/broken-path", UnresolvedReason::InvalidSkill),
            ("nothing-*", UnresolvedReason::NoMatch),
        ]
    );
    assert_eq!(
        unresolved(&reports[4]),
        [("one", UnresolvedReason::CloneFailed)]
    );
    assert_eq!(resolution.report.warning_count(), 9);
    assert_eq!(linked_skills(&workspace.generated().join("1")), ["good"]);
}

#[test]
fn checkout_plan_dedupes_sources_and_requests_their_heads() {
    let workspace = Workspace::new();
    let sha = "0123456789abcdef".repeat(4)[..40].to_owned();
    write_reference(
        &workspace.agent_skills,
        "a.yaml",
        "source: Acme/Shared-Skills@v1\ninclude: [x]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "a.yaml",
        "source: acme/shared-skills@v1\ninclude: [x]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "b.yaml",
        &format!("source: acme/shared-skills@{sha}\ninclude: [x]\n"),
    );
    write_reference(
        &workspace.factory_skills,
        "c.yaml",
        "source:\n  owner: platform/backend\n  name: skills\n  codeForge: GITLAB\ninclude: [x]\n",
    );
    write_reference(
        &workspace.factory_skills,
        "d.yaml",
        &format!(
            "source: acme/shared-skills@{}\ninclude: [x]\n",
            "a".repeat(64)
        ),
    );
    write_reference(
        &workspace.factory_skills,
        "e.yaml",
        "source: platform/backend/skills\ninclude: [x]\n",
    );

    let discovered = discover(&workspace.skill_dirs(), RepositoryForge::GitHub).unwrap();
    let plan = discovered.checkout_plan(&workspace.sources());
    let requests = &plan.batch.repositories;
    assert_eq!(
        requests
            .iter()
            .map(|request| (
                request.source.code_forge,
                request.source.repo_owner.as_str(),
                request.head.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                RepositoryForge::GitHub,
                "Acme",
                Some(RepositoryHeadRef::NamedRef("v1".to_owned()))
            ),
            (
                RepositoryForge::GitHub,
                "acme",
                Some(RepositoryHeadRef::CommitSha(sha))
            ),
            (RepositoryForge::GitLab, "platform/backend", None),
        ]
    );
    assert!(plan.batch.validate().is_ok());

    let checkouts = plan.into_checkouts(None);
    let reasons = |file: &str| {
        let resolution_reports = discover(&workspace.skill_dirs(), RepositoryForge::GitHub)
            .unwrap()
            .materialize(
                &workspace.skill_dirs(),
                &workspace.skill_dirs(),
                &checkouts,
                &workspace.generated(),
            )
            .report
            .references;
        resolution_reports
            .into_iter()
            .find(|report| report.reference_file == file)
            .unwrap()
            .unresolved
            .into_iter()
            .map(|entry| entry.reason)
            .collect::<Vec<_>>()
    };
    // A 64-character SHA can't be checked out, and a GitHub owner can't contain `/`.
    assert_eq!(reasons("skills/d.yaml"), [UnresolvedReason::RefNotFound]);
    assert_eq!(reasons("skills/e.yaml"), [UnresolvedReason::CloneFailed]);
}

#[test]
fn checkouts_without_a_reported_head_read_it_from_the_repository() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "shared.yaml",
        "source: acme/shared-skills\ninclude: [x]\n",
    );
    let discovered = discover(&workspace.skill_dirs(), RepositoryForge::GitHub).unwrap();
    let plan = discovered.checkout_plan(&workspace.sources());
    let checkout = workspace
        .sources()
        .join(&plan.batch.repositories[0].checkout_name);
    let repository = git2::Repository::init(&checkout).unwrap();
    let signature = git2::Signature::now("Fixture", "fixture@example.com").unwrap();
    let tree = repository
        .find_tree(repository.index().unwrap().write_tree().unwrap())
        .unwrap();
    let commit = repository
        .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    let report = CheckoutReport {
        outcomes: vec![CheckoutOutcome {
            request_index: 0,
            failure: None,
            diagnostics: String::new(),
            duration_ms: 0,
            resolved_head: None,
        }],
        identity_diagnostics: None,
    };
    let resolution = discovered.materialize(
        &workspace.skill_dirs(),
        &workspace.skill_dirs(),
        &plan.into_checkouts(Some(&report)),
        &workspace.generated(),
    );
    assert_eq!(
        resolution.report.references[0]
            .source
            .as_ref()
            .unwrap()
            .resolved_sha,
        Some(commit.to_string())
    );
}

#[test]
fn sources_without_a_forge_use_the_primary_then_the_environment_forge() {
    assert_eq!(
        default_source_forge(Some("GITLAB"), [RepositoryForge::AzureDevOps]),
        RepositoryForge::GitLab
    );
    assert_eq!(
        default_source_forge(
            None,
            [RepositoryForge::AzureDevOps, RepositoryForge::AzureDevOps]
        ),
        RepositoryForge::AzureDevOps
    );
    assert_eq!(
        default_source_forge(
            Some("UNKNOWN"),
            [RepositoryForge::AzureDevOps, RepositoryForge::GitLab]
        ),
        RepositoryForge::GitHub
    );
    assert_eq!(default_source_forge(None, []), RepositoryForge::GitHub);
}

fn sample_report() -> SkillSourcesReport {
    SkillSourcesReport {
        references: vec![
            ReferenceFileReport {
                reference_file: "agents/triage/skills/broken.yaml".to_owned(),
                level: ReferenceLevel::Agent,
                agent: Some("triage".to_owned()),
                source: None,
                loaded: Vec::new(),
                unresolved: vec![UnresolvedEntry {
                    entry: String::new(),
                    reason: UnresolvedReason::InvalidReferenceFile,
                }],
            },
            ReferenceFileReport {
                reference_file: "skills/shared.yaml".to_owned(),
                level: ReferenceLevel::Factory,
                agent: None,
                source: Some(ReferencedSource {
                    code_forge: RepositoryForge::GitHub,
                    owner: "acme".to_owned(),
                    name: "shared-skills".to_owned(),
                    requested_ref: Some("v1.4.0".to_owned()),
                    resolved_sha: Some(fake_sha('c')),
                }),
                loaded: vec![LoadedSkill {
                    name: "safe-secrets".to_owned(),
                    path: ".agents/skills/safe-secrets".to_owned(),
                }],
                unresolved: vec![UnresolvedEntry {
                    entry: "legacy-*".to_owned(),
                    reason: UnresolvedReason::NoMatch,
                }],
            },
        ],
    }
}

#[test]
fn timeline_payload_has_the_agreed_shape() {
    assert_eq!(
        sample_report().timeline_payload(),
        json!({"references": [
            {
                "reference_file": "agents/triage/skills/broken.yaml",
                "level": "agent",
                "agent": "triage",
                "loaded": [],
                "unresolved": [{"entry": "", "reason": "invalid_reference_file"}],
            },
            {
                "reference_file": "skills/shared.yaml",
                "level": "factory",
                "source": {
                    "code_forge": "GITHUB",
                    "owner": "acme",
                    "name": "shared-skills",
                    "requested_ref": "v1.4.0",
                    "resolved_sha": fake_sha('c'),
                },
                "loaded": [{"name": "safe-secrets", "path": ".agents/skills/safe-secrets"}],
                "unresolved": [{"entry": "legacy-*", "reason": "no_match"}],
            },
        ]})
    );
}

#[test]
fn timeline_payload_stays_bounded_and_keeps_warnings_first() {
    let long = "x".repeat(4 * MAX_RECORDED_STRING_BYTES);
    let mut report = SkillSourcesReport::default();
    for index in 0..400 {
        let mut reference = sample_report().references.remove(1);
        reference.reference_file = format!("skills/{index}-{long}.yaml");
        reference.loaded = (0..600)
            .map(|skill| LoadedSkill {
                name: format!("skill-{skill}"),
                path: format!("skills/skill-{skill}"),
            })
            .collect();
        reference.unresolved = (0..10)
            .map(|entry| UnresolvedEntry {
                entry: format!("missing-{entry}"),
                reason: UnresolvedReason::NoMatch,
            })
            .collect();
        report.references.push(reference);
    }

    let payload = report.timeline_payload();
    assert!(serde_json::to_vec(&payload).unwrap().len() <= MAX_RECORDED_PAYLOAD_BYTES);
    let references = payload["references"].as_array().unwrap();
    assert!(references.len() > 10, "{}", references.len());
    assert!(references.iter().all(|reference| {
        reference["reference_file"].as_str().unwrap().len() == MAX_RECORDED_STRING_BYTES
            && reference["unresolved"].as_array().unwrap().len() == 10
            && reference["loaded"].as_array().unwrap().len() <= MAX_RECORDED_ENTRIES
    }));
    let loaded = references
        .iter()
        .map(|reference| reference["loaded"].as_array().unwrap().len())
        .sum::<usize>();
    assert!(loaded < 600, "{loaded}");
}

#[test]
fn status_message_names_the_first_warnings_only_when_there_are_any() {
    let mut report = sample_report();
    assert_eq!(
        report.status_message().unwrap(),
        "Warning: some referenced skills did not load (agents/triage/skills/broken.yaml \
         (invalid_reference_file); skills/shared.yaml 'legacy-*' (no_match)); continuing \
         without them."
    );

    report.references[1].unresolved = (0..6)
        .map(|entry| UnresolvedEntry {
            entry: format!("missing-{entry}"),
            reason: UnresolvedReason::NoMatch,
        })
        .collect();
    assert!(report.status_message().unwrap().contains("; and 2 more)"));

    for reference in &mut report.references {
        reference.unresolved.clear();
    }
    assert_eq!(report.status_message(), None);
}

#[test]
fn posting_tolerates_a_rejected_event_and_warns_only_with_warnings() {
    App::test((), |app| async move {
        let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        for with_warnings in [true, false] {
            let mut report = sample_report();
            if !with_warnings {
                for reference in &mut report.references {
                    reference.unresolved.clear();
                }
            }
            let expected_payload = report.timeline_payload();
            let mut ai_client = MockAIClient::new();
            ai_client
                .expect_post_agent_run_client_event()
                .times(1)
                .withf(move |run_id, request| {
                    let request = serde_json::to_value(request).unwrap();
                    *run_id == task_id
                        && request["event_name"] == "skill_sources_resolved"
                        && request["payload"] == expected_payload
                })
                .returning(|_, _| Err(anyhow::anyhow!("400 Bad Request: unknown event")));
            ai_client
                .expect_update_agent_task()
                .times(usize::from(with_warnings))
                .withf(
                    move |run_id, state, session, conversation, status, debug_until, debug| {
                        *run_id == task_id
                            && state.is_none()
                            && session.is_none()
                            && conversation.is_none()
                            && status.as_ref().is_some_and(|status| {
                                status
                                    .message
                                    .starts_with("Warning: some referenced skills")
                            })
                            && debug_until.is_none()
                            && debug.is_none()
                    },
                )
                .returning(|_, _, _, _, _, _, _| Ok(()));
            let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
            let setup_events = SetupClientEventReporter::new(
                task_id,
                ai_client.clone(),
                app.background_executor(),
            );
            post_skill_sources(&report, task_id, &setup_events, ai_client).await;
        }
    });
}

#[test]
fn relative_skill_dirs_keep_their_configured_form() {
    let workspace = Workspace::new();
    write_reference(
        &workspace.factory_skills,
        "shared.yaml",
        "source: acme/shared-skills\ninclude: [one]\n",
    );
    let configured = vec![PathBuf::from("definition/skills")];
    let resolved = resolve_skills_dirs(&workspace.root, configured.clone());
    let discovered = discover(&resolved, RepositoryForge::GitHub).unwrap();
    let plan = discovered.checkout_plan(&workspace.sources());
    let report = workspace.run_fake_checkouts(
        &plan.batch.repositories,
        &[(
            "shared-skills",
            SourceFixture::Skills(vec![skill(".agents/skills/one", "one")]),
        )],
    );
    let resolution = discovered.materialize(
        &configured,
        &resolved,
        &plan.into_checkouts(Some(&report)),
        &workspace.generated(),
    );
    assert_eq!(
        resolution.skill_dirs,
        vec![
            PathBuf::from("definition/skills"),
            workspace.generated().join("0")
        ]
    );
}
