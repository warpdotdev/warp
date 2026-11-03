use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef};

use super::super::environment_checkout_protocol::{CheckoutFailureKind, CheckoutOutcome};
use super::*;

const AGENT_SKILLS: &str = "agents/triage/skills";
const FACTORY_SKILLS: &str = "skills";
const GITHUB: Option<SkillSourceForge> = Some(SkillSourceForge::GitHub);

/// What the fake checkout helper does for a source repository, keyed by repository name.
enum Source {
    /// Succeeds with a skill in each folder, named after the folder; a trailing `!` writes an
    /// invalid `SKILL.md` instead.
    Skills(&'static [&'static str]),
    Fails(CheckoutFailureKind),
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let temp = TempDir::new().unwrap();
        let fixture = Self {
            root: temp.path().canonicalize().unwrap(),
            _temp: temp,
        };
        for (path, contents) in files {
            let path = fixture.root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fixture
    }

    fn discovery(&self, forge: Option<SkillSourceForge>) -> SkillReferenceDiscovery {
        let configured_dirs = vec![PathBuf::from(AGENT_SKILLS), PathBuf::from(FACTORY_SKILLS)];
        let resolved_dirs = resolve_skills_dirs(&self.root, configured_dirs.clone());
        SkillReferenceDiscovery {
            root: self.root.join("out"),
            levels: discover(&resolved_dirs, forge),
            configured_dirs,
            resolved_dirs,
        }
    }

    /// Resolves the reference files, standing in for the checkout helper with `sources`. A source
    /// without an entry produces no outcome; successful checkouts are git repositories whose HEAD
    /// the helper doesn't report.
    fn resolve(
        &self,
        forge: Option<SkillSourceForge>,
        sources: &[(&str, Source)],
    ) -> (Vec<PathBuf>, SkillSourcesReport) {
        let discovery = self.discovery(forge);
        let (batch, planned) = plan_checkouts(&discovery.levels, &discovery.root.join("sources"));
        let outcomes = batch
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, request)| {
                let (_, source) = sources
                    .iter()
                    .find(|(name, _)| *name == request.source.repo_name)?;
                let failure = match source {
                    Source::Skills(folders) => {
                        let checkout = batch.working_dir.join(&request.checkout_name);
                        for folder in *folders {
                            write_skill(&checkout.join(folder));
                        }
                        let repository = git2::Repository::init(&checkout).unwrap();
                        let signature =
                            git2::Signature::now("Fixture", "fixture@example.com").unwrap();
                        let tree = repository.index().unwrap().write_tree().unwrap();
                        let tree = repository.find_tree(tree).unwrap();
                        repository
                            .commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
                            .unwrap();
                        None
                    }
                    Source::Fails(kind) => Some(*kind),
                };
                Some(CheckoutOutcome {
                    request_index: index,
                    failure,
                    diagnostics: String::new(),
                    duration_ms: 0,
                    resolved_head: None,
                })
            });
        let report = CheckoutReport {
            requests: Vec::new(),
            outcomes: outcomes.collect(),
            identity_diagnostics: None,
        };
        materialize(&discovery, &checkouts(planned, &batch, Some(&report)))
    }

    fn generated(&self, index: usize) -> PathBuf {
        self.root.join("out/skills").join(index.to_string())
    }
}

fn write_skill(folder: &Path) {
    let (folder, contents) = match folder.to_str().unwrap().strip_suffix('!') {
        Some(folder) => (
            Path::new(folder),
            "---\nname: [unterminated\n---\n".to_owned(),
        ),
        None => {
            let name = folder.file_name().unwrap().to_str().unwrap();
            (
                folder,
                format!("---\nname: {name}\ndescription: test\n---\nBody\n"),
            )
        }
    };
    fs::create_dir_all(folder).unwrap();
    fs::write(folder.join(SKILL_FILE), contents).unwrap();
}

fn loaded(report: &ReferenceFileReport) -> Vec<&str> {
    report
        .loaded
        .iter()
        .map(|skill| skill.path.as_str())
        .collect()
}

fn unresolved(report: &ReferenceFileReport) -> Vec<(&str, UnresolvedReason)> {
    report
        .unresolved
        .iter()
        .map(|entry| (entry.entry.as_str(), entry.reason))
        .collect()
}

#[test]
fn entries_resolve_by_name_path_and_glob_and_excludes_apply_last() {
    let fixture = Fixture::new(&[
        (
            "skills/shared.yaml",
            "source: acme/shared@v1.4.0\ninclude: [safe, tools/notes/SKILL.md, \"web/*\", \"old-*\"]\n\
             exclude: [old-b, web/forms]\n",
        ),
        // YAML nested inside a skill folder is part of that skill, not a reference file.
        (
            "skills/deploy/config.yaml",
            "source: acme/other\ninclude: [x]\n",
        ),
    ]);
    let (skill_dirs, report) = fixture.resolve(
        GITHUB,
        &[(
            "shared",
            // Names search the standard folders in provider order, not path order.
            Source::Skills(&[
                ".claude/skills/safe",
                ".warp/skills/safe",
                "tools/notes",
                "web/buttons",
                "web/forms",
                "web/nested/deep",
                ".claude/skills/old-a",
                ".claude/skills/old-b",
            ]),
        )],
    );
    let [file] = report.references.as_slice() else {
        panic!("{report:?}");
    };
    assert_eq!(
        (file.reference_file.as_str(), file.level),
        ("skills/shared.yaml", ReferenceLevel::Factory)
    );
    assert_eq!(
        loaded(file),
        [
            ".warp/skills/safe",
            "tools/notes",
            "web/buttons",
            ".claude/skills/old-a"
        ]
    );
    assert!(file.unresolved.is_empty());
    let source = file.source.as_ref().unwrap();
    assert_eq!(source.requested_ref.as_deref(), Some("v1.4.0"));
    assert_eq!(source.resolved_sha.as_ref().map(String::len), Some(40));
    assert_eq!(
        skill_dirs,
        [
            AGENT_SKILLS.into(),
            FACTORY_SKILLS.into(),
            fixture.generated(1)
        ]
    );
    assert!(fixture.generated(1).join("notes/SKILL.md").is_file());
}

#[test]
fn levels_and_files_apply_in_order_and_earlier_names_win() {
    let star = "include: [\"*\"]\n";
    let fixture = Fixture::new(&[
        (
            &format!("{AGENT_SKILLS}/agent.yaml"),
            &format!("source: acme/agent\n{star}"),
        ),
        ("skills/b.yml", &format!("source: acme/second\n{star}")),
        ("skills/a.yaml", &format!("source: acme/first\n{star}")),
    ]);
    write_skill(&fixture.root.join(AGENT_SKILLS).join("own"));
    let (skill_dirs, report) = fixture.resolve(
        GITHUB,
        &[
            (
                "agent",
                Source::Skills(&[".agents/skills/own", ".agents/skills/shared"]),
            ),
            (
                "first",
                Source::Skills(&[".agents/skills/shared", ".agents/skills/lint"]),
            ),
            (
                "second",
                Source::Skills(&[".agents/skills/lint", ".agents/skills/extra"]),
            ),
        ],
    );
    let [agent, first, second] = report.references.as_slice() else {
        panic!("{report:?}");
    };
    assert_eq!(
        (agent.reference_file.as_str(), agent.agent.as_deref()),
        ("agents/triage/skills/agent.yaml", Some("triage"))
    );
    // The agent's own folder wins over its references, which win over the Factory's. Losing on
    // name isn't a warning; two files at one level selecting the same name is.
    assert_eq!(loaded(agent), [".agents/skills/shared"]);
    assert_eq!(first.reference_file, "skills/a.yaml");
    assert_eq!(
        (loaded(first), unresolved(first)),
        (vec![".agents/skills/lint"], vec![])
    );
    assert_eq!(loaded(second), [".agents/skills/extra"]);
    assert_eq!(unresolved(second), [("*", UnresolvedReason::Collision)]);
    assert_eq!(
        skill_dirs,
        [
            AGENT_SKILLS.into(),
            fixture.generated(0),
            FACTORY_SKILLS.into(),
            fixture.generated(1),
        ]
    );
}

#[test]
fn every_failure_is_recorded_and_the_rest_still_loads() {
    let fixture = Fixture::new(&[
        (
            "skills/a.yaml",
            "source: github.com:acme/broken\ninclude: [x]\nbad: 1\n",
        ),
        (
            "skills/b.yaml",
            "source: github.com:acme/down\ninclude: [one, two]\n",
        ),
        (
            "skills/c.yaml",
            "source: github.com:acme/pinned@v9\ninclude: [one]\n",
        ),
        (
            "skills/d.yaml",
            "source: github.com:acme/silent\ninclude: [one]\n",
        ),
        (
            "skills/e.yaml",
            "source: acme/no-forge\ninclude: [one, two]\n",
        ),
        (
            "skills/f.yaml",
            "source: github.com:acme/partial\ninclude: [good, gone, bad, skills/bad, \"none-*\"]\n",
        ),
    ]);
    let (_, report) = fixture.resolve(
        None,
        &[
            ("down", Source::Fails(CheckoutFailureKind::Clone)),
            ("pinned", Source::Fails(CheckoutFailureKind::Checkout)),
            (
                "partial",
                Source::Skills(&[".agents/skills/good", ".agents/skills/bad!", "skills/bad!"]),
            ),
        ],
    );
    use UnresolvedReason::*;
    let expected: [&[(&str, UnresolvedReason)]; 6] = [
        &[("", InvalidReferenceFile)],
        &[("one", CloneFailed), ("two", CloneFailed)],
        &[("one", RefNotFound)],
        &[("one", CloneFailed)],
        &[("one", UnknownForge), ("two", UnknownForge)],
        // An unparseable `SKILL.md` has no name to match.
        &[
            ("gone", NoMatch),
            ("bad", NoMatch),
            ("skills/bad", InvalidSkill),
            ("none-*", NoMatch),
        ],
    ];
    for (file, expected) in report.references.iter().zip(expected) {
        assert_eq!(unresolved(file), expected, "{}", file.reference_file);
    }
    assert!(report.references[0].source.is_none());
    let no_forge = report.references[4].source.as_ref().unwrap();
    assert_eq!((no_forge.code_forge, &no_forge.resolved_sha), (None, &None));
    assert_eq!(loaded(&report.references[5]), [".agents/skills/good"]);
}

#[cfg(unix)]
#[test]
fn skills_that_symlink_outside_their_checkout_are_rejected() {
    let fixture = Fixture::new(&[(
        "skills/shared.yaml",
        "source: acme/shared\ninclude: [skills/inside, skills/escape]\n",
    )]);
    write_skill(&fixture.root.join("outside"));
    let checkout = fixture.root.join("out/sources/0-shared");
    fs::create_dir_all(checkout.join("skills")).unwrap();
    std::os::unix::fs::symlink(fixture.root.join("outside"), checkout.join("skills/escape"))
        .unwrap();
    let (_, report) = fixture.resolve(GITHUB, &[("shared", Source::Skills(&["skills/inside"]))]);
    assert!(checkout.join("skills/inside/SKILL.md").is_file());
    assert_eq!(loaded(&report.references[0]), ["skills/inside"]);
    assert_eq!(
        unresolved(&report.references[0]),
        [("skills/escape", UnresolvedReason::NoMatch)]
    );
}

#[test]
fn sources_are_checked_out_once_per_identity_and_ref() {
    let sha = "0123456789abcdef".repeat(3)[..40].to_owned();
    let file = |source: &str| format!("source: {source}\ninclude: [x]\n");
    let fixture = Fixture::new(&[
        (&format!("{AGENT_SKILLS}/a.yaml"), &file("Acme/Shared@v1")),
        ("skills/a.yaml", &file("acme/shared@v1")),
        ("skills/b.yaml", &file("github.com:ACME/shared@v1")),
        ("skills/c.yaml", &file(&format!("acme/shared@{sha}"))),
        ("skills/d.yaml", &file("gitlab.com:acme/shared@v1")),
        ("skills/e.yaml", &file("platform/backend/skills")),
    ]);
    let discovery = fixture.discovery(GITHUB);
    let (batch, planned) = plan_checkouts(&discovery.levels, &discovery.root.join("sources"));
    let requests = batch
        .repositories
        .iter()
        .map(|request| {
            let source = &request.source;
            (
                source.code_forge,
                source.repo_owner.as_str(),
                request.head.clone(),
            )
        })
        .collect::<Vec<_>>();
    let branch = Some(RepositoryHeadRef::Branch("v1".to_owned()));
    assert_eq!(
        requests,
        [
            (RepositoryForge::GitHub, "Acme", branch.clone()),
            (
                RepositoryForge::GitHub,
                "acme",
                Some(RepositoryHeadRef::CommitSha(sha))
            ),
            (RepositoryForge::GitLab, "acme", branch),
        ]
    );
    assert!(batch.validate().is_ok());
    // A GitHub owner can't contain `/`, so that request is rejected without failing the batch.
    let nested = (
        GITHUB,
        "platform/backend".to_owned(),
        "skills".to_owned(),
        None,
    );
    assert_eq!(planned[&nested], Err(UnresolvedReason::CloneFailed));
}
