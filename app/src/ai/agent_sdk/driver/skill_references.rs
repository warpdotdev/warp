//! Resolves the skill reference files in a Factory definition's skill folders. Sources are checked
//! out outside the working directory, and the selected skills are linked into a generated skill
//! folder placed right after the folder that owns the reference files. Resolution never fails a
//! run: each problem is recorded as an unresolved entry and the rest still loads.

use std::collections::{HashMap, HashSet, hash_map};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use ai::skills::{
    SKILL_PROVIDER_DEFINITIONS, SkillGlobMatcher, SkillReferenceEntry, SkillReferenceFile,
    SkillReferenceParseError, SkillSource, SkillSourceRef, is_skill_reference_file_name,
    parse_code_forge, parse_skill, parse_skill_reference_file, read_skills, resolve_skills_dirs,
};
use serde::Serialize;
use serde_json::{Value, json};
use walkdir::WalkDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};
use warpui::ModelSpawner;

use super::environment_checkout_protocol::{
    CheckoutBatch, CheckoutFailureKind, CheckoutReport, CheckoutRequest, MAX_CHECKOUT_REQUESTS,
};
use super::harness::create_symlink;
use super::{AgentDriver, environment};

/// Names the Factory's code forge; set only when the Factory has exactly one.
const FACTORY_CODE_FORGE_ENV: &str = "WARP_FACTORY_CODE_FORGE";
const SKILL_FILE: &str = "SKILL.md";
const FACTORY_FILE: &str = "factory.yaml";
/// Bounds how much of a source repository a path glob walks.
const MAX_WALKED_ENTRIES: usize = 200_000;

/// Bounds on the run record, kept within what the server stores for one timeline event.
const MAX_RECORDED_REFERENCE_FILES: usize = 200;
const MAX_RECORDED_ENTRIES: usize = 256;
const MAX_RECORDED_STRING_BYTES: usize = 512;
const MAX_RECORDED_PAYLOAD_BYTES: usize = 60 * 1024;

/// Whether a reference file applies to the whole Factory or to one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReferenceLevel {
    Factory,
    Agent,
}

/// Why an entry, or a whole reference file, didn't load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnresolvedReason {
    CloneFailed,
    RefNotFound,
    /// Neither the source nor the run names the source's code forge.
    UnknownForge,
    NoMatch,
    InvalidSkill,
    InvalidReferenceFile,
    Collision,
}

/// What a run resolved from its reference files.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct SkillSourcesReport {
    /// In skill folder order, then file-name order.
    pub references: Vec<ReferenceFileReport>,
}

impl SkillSourcesReport {
    pub fn warning_count(&self) -> usize {
        self.references
            .iter()
            .map(|file| file.unresolved.len())
            .sum()
    }

    /// The `skill_sources_resolved` timeline payload. Strings lose control characters and strings
    /// and lists are capped, while the counts keep every file, loaded skill and unresolved entry.
    /// If the record is still too large, every file's loaded skills are dropped, then trailing
    /// files.
    pub fn timeline_payload(&self) -> Value {
        let mut references = self
            .references
            .iter()
            .take(MAX_RECORDED_REFERENCE_FILES)
            .map(|file| {
                let mut recorded = file.clone();
                recorded.loaded.truncate(MAX_RECORDED_ENTRIES);
                recorded.unresolved.truncate(MAX_RECORDED_ENTRIES);
                let mut value = serde_json::to_value(recorded).unwrap_or_default();
                value["loaded_count"] = file.loaded.len().into();
                value["unresolved_count"] = file.unresolved.len().into();
                sanitize_strings(&mut value);
                value
            })
            .collect::<Vec<_>>();
        let payload = |references: &[Value]| json!({ "file_count": self.references.len(), "references": references });
        let mut size = payload(&references).to_string().len();
        if size > MAX_RECORDED_PAYLOAD_BYTES {
            for file in &mut references {
                file["loaded"] = json!([]);
            }
            size = payload(&references).to_string().len();
        }
        while size > MAX_RECORDED_PAYLOAD_BYTES {
            let Some(file) = references.pop() else {
                break;
            };
            size -= file.to_string().len();
        }
        payload(&references)
    }
}

/// The server rejects control characters, which skill names from a source's frontmatter can hold.
fn sanitize_strings(value: &mut Value) {
    match value {
        Value::String(string) => {
            string.retain(|character| !character.is_control());
            string.truncate(string.floor_char_boundary(MAX_RECORDED_STRING_BYTES));
        }
        Value::Array(items) => items.iter_mut().for_each(sanitize_strings),
        Value::Object(fields) => fields.values_mut().for_each(sanitize_strings),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The outcome for one reference file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ReferenceFileReport {
    /// Relative to the definition root.
    pub reference_file: String,
    pub level: ReferenceLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Absent when the file itself couldn't be parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ReferencedSource>,
    pub loaded: Vec<LoadedSkill>,
    pub unresolved: Vec<UnresolvedEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ReferencedSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_forge: Option<RepositoryForge>,
    pub owner: String,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    /// Absent when the checkout failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LoadedSkill {
    pub name: String,
    /// The skill folder, relative to the source repository root.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UnresolvedEntry {
    /// The `include` entry, or empty when the whole file is invalid.
    pub entry: String,
    pub reason: UnresolvedReason,
}

/// Reference files found in a run's skill folders.
pub(super) struct SkillReferenceDiscovery {
    /// Holds the source checkouts and generated skill folders.
    root: PathBuf,
    configured_dirs: Vec<PathBuf>,
    resolved_dirs: Vec<PathBuf>,
    levels: Vec<Level>,
}

/// The reference files directly in one skill folder, in file-name order.
struct Level {
    /// The owning folder's index in the run's skill folders.
    index: usize,
    agent: Option<String>,
    files: Vec<(String, Result<SkillReferenceFile, SkillReferenceParseError>)>,
}

/// Case-insensitive repository identity plus the requested ref.
type SourceIdentity = (
    Option<RepositoryForge>,
    String,
    String,
    Option<SkillSourceRef>,
);

/// Each source's request index in the checkout batch, or why it has no request.
type Planned = HashMap<SourceIdentity, Result<usize, UnresolvedReason>>;

/// A source's checkout directory and commit, or why it has none.
type Checkout = Result<(PathBuf, Option<String>), UnresolvedReason>;

impl AgentDriver {
    /// Finds the reference files in this run's Factory definition skill folders. `None` for runs
    /// without a task or without reference files, which then behave as if references didn't exist.
    pub(super) async fn discover_skill_references(
        foreground: &ModelSpawner<Self>,
    ) -> Option<SkillReferenceDiscovery> {
        let (task_id, working_dir, configured_dirs, reference_dirs) = foreground
            .spawn(|me, _| {
                (
                    me.task_id,
                    me.working_dir.clone(),
                    me.skill_dirs.clone(),
                    me.skill_reference_dirs.clone(),
                )
            })
            .await
            .ok()?;
        let task_id = task_id?;
        let resolved_dirs = resolve_skills_dirs(&working_dir, configured_dirs.clone());
        let reference_dirs = resolve_skills_dirs(&working_dir, reference_dirs);
        let factory_forge = std::env::var(FACTORY_CODE_FORGE_ENV)
            .ok()
            .and_then(|value| parse_code_forge(value.trim()));
        let levels = discover(&resolved_dirs, &reference_dirs, factory_forge);
        (!levels.is_empty()).then(|| SkillReferenceDiscovery {
            root: std::env::temp_dir()
                .join("oz-skill-sources")
                .join(task_id.to_string()),
            configured_dirs,
            resolved_dirs,
            levels,
        })
    }

    /// Checks out the referenced repositories, links the selected skills, and adds the generated
    /// folders to this run's skill folders.
    pub(super) async fn resolve_skill_references(
        discovery: SkillReferenceDiscovery,
        foreground: &ModelSpawner<Self>,
    ) -> SkillSourcesReport {
        // Starts empty so nothing from an earlier attempt in the same sandbox is reused.
        let _ = fs::remove_dir_all(&discovery.root);
        let (batch, sources) = plan_checkouts(&discovery.levels, &discovery.root.join("sources"));
        let report = match fs::create_dir_all(&batch.working_dir) {
            Ok(()) if !batch.repositories.is_empty() => {
                let request = batch.clone();
                let checkout = foreground.spawn(move |me, ctx| {
                    me.terminal_driver.update(ctx, |_, ctx| {
                        environment::checkout_skill_sources(request, ctx)
                    })
                });
                match checkout.await {
                    Ok(checkout) => checkout.await,
                    Err(_) => None,
                }
            }
            Ok(()) | Err(_) => None,
        };
        let checkouts = checkouts(sources, &batch, report.as_ref());
        let (skill_dirs, report) = materialize(&discovery, &checkouts);
        log::info!(
            "Skill references: {} file(s), {} warning(s)",
            report.references.len(),
            report.warning_count()
        );
        let _ = foreground
            .spawn(move |me, _| {
                me.skill_dirs = skill_dirs;
                me.skill_dirs_first_name_wins = true;
            })
            .await;
        report
    }
}

/// Reads the reference files directly in each of `skill_dirs` that is one of `reference_dirs` and
/// a definition level. `factory_forge` fills in the forge of sources that don't name one.
fn discover(
    skill_dirs: &[PathBuf],
    reference_dirs: &[PathBuf],
    factory_forge: Option<RepositoryForge>,
) -> Vec<Level> {
    let mut seen = HashSet::new();
    let mut levels = Vec::new();
    for (index, skills_dir) in skill_dirs.iter().enumerate() {
        if !reference_dirs.contains(skills_dir) {
            continue;
        }
        let Some(agent) = definition_level(skills_dir) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(skills_dir) else {
            continue;
        };
        if !seen.insert(skills_dir) {
            continue;
        }
        let mut file_names = entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_skill_reference_file_name(name))
            .collect::<Vec<_>>();
        if file_names.is_empty() {
            continue;
        }
        file_names.sort();
        let files = file_names
            .into_iter()
            .map(|name| {
                let parsed = fs::read_to_string(skills_dir.join(&name))
                    .map_err(|_| SkillReferenceParseError::InvalidYaml)
                    .and_then(|contents| parse_skill_reference_file(&contents))
                    .map(|mut file| {
                        file.source.code_forge = file.source.code_forge.or(factory_forge);
                        file
                    });
                let reference_file = match &agent {
                    Some(agent) => format!("agents/{agent}/skills/{name}"),
                    None => format!("skills/{name}"),
                };
                (reference_file, parsed)
            })
            .collect();
        levels.push(Level {
            index,
            agent,
            files,
        });
    }
    levels
}

/// `Some(None)` for `<root>/skills` and `Some(Some(name))` for `<root>/agents/<name>/skills`,
/// where `<root>` holds `factory.yaml`. No other folder holds reference files.
fn definition_level(skills_dir: &Path) -> Option<Option<String>> {
    let is_root = |root: &Path| root.join(FACTORY_FILE).is_file();
    if skills_dir.file_name()? != "skills" {
        return None;
    }
    let parent = skills_dir.parent()?;
    let agents = parent
        .parent()
        .filter(|agents| agents.file_name() == Some(OsStr::new("agents")));
    if let Some(root) = agents.and_then(Path::parent)
        && is_root(root)
    {
        return Some(Some(parent.file_name()?.to_str()?.to_owned()));
    }
    is_root(parent).then_some(None)
}

fn identity(source: &SkillSource) -> SourceIdentity {
    (
        source.code_forge,
        source.owner.to_lowercase(),
        source.repo.to_lowercase(),
        source.reference.clone(),
    )
}

/// One checkout request per distinct source, checked out into `sources_dir`.
fn plan_checkouts(levels: &[Level], sources_dir: &Path) -> (CheckoutBatch, Planned) {
    let mut repositories = Vec::new();
    let mut planned = HashMap::new();
    let sources = levels
        .iter()
        .flat_map(|level| &level.files)
        .filter_map(|(_, parsed)| parsed.as_ref().ok());
    for SkillReferenceFile { source, .. } in sources {
        let hash_map::Entry::Vacant(slot) = planned.entry(identity(source)) else {
            continue;
        };
        let Some(forge) = source.code_forge else {
            slot.insert(Err(UnresolvedReason::UnknownForge));
            continue;
        };
        let repo = source
            .repo
            .replace(|character: char| !character.is_ascii_alphanumeric(), "-");
        let request = CheckoutRequest {
            source: RepositoryIdentity {
                code_forge: forge,
                repo_owner: source.owner.clone(),
                repo_name: source.repo.clone(),
            },
            checkout_name: format!("{}-{repo}", repositories.len()),
            // Tags ride branch heads; the checkout helper falls back to a tag of the same name.
            head: source.reference.as_ref().map(|reference| match reference {
                SkillSourceRef::Sha(sha) => RepositoryHeadRef::CommitSha(sha.clone()),
                SkillSourceRef::Named(name) => RepositoryHeadRef::Branch(name.clone()),
            }),
            fetch_branch_only: false,
            expected_sha: None,
            remove_origin: false,
        };
        // An invalid request would fail the helper's whole batch, so it's rejected here alone.
        slot.insert(match request.validate() {
            Ok(()) if repositories.len() < MAX_CHECKOUT_REQUESTS => {
                repositories.push(request);
                Ok(repositories.len() - 1)
            }
            Ok(()) => Err(UnresolvedReason::CloneFailed),
            Err("invalid checkout ref" | "invalid checkout commit") => {
                Err(UnresolvedReason::RefNotFound)
            }
            Err(_) => Err(UnresolvedReason::CloneFailed),
        });
    }
    let batch = CheckoutBatch {
        working_dir: sources_dir.to_owned(),
        repositories,
    };
    (batch, planned)
}

/// Pairs each planned source with its outcome in the helper's report. Without an outcome, as when
/// the helper failed or was interrupted, a checkout counts only if it finished.
fn checkouts(
    planned: Planned,
    batch: &CheckoutBatch,
    report: Option<&CheckoutReport>,
) -> HashMap<SourceIdentity, Checkout> {
    let checkout = |index: usize| {
        let request = &batch.repositories[index];
        let dir = batch.working_dir.join(&request.checkout_name);
        let outcomes = report.map_or(&[][..], |report| &report.outcomes);
        let Some(outcome) = outcomes
            .iter()
            .find(|outcome| outcome.request_index == index)
        else {
            let sha = finished_head(&dir, request.head.as_ref());
            return sha
                .map(|sha| (dir, Some(sha)))
                .ok_or(UnresolvedReason::CloneFailed);
        };
        match outcome.failure {
            // The helper reports heads only when the whole batch succeeds.
            None => {
                let sha = outcome.resolved_head.clone();
                let sha = sha.or_else(|| read_head(&dir).map(|(sha, _)| sha));
                Ok((dir, sha))
            }
            Some(CheckoutFailureKind::Checkout) if request.head.is_some() => {
                Err(UnresolvedReason::RefNotFound)
            }
            Some(
                CheckoutFailureKind::Clone
                | CheckoutFailureKind::Checkout
                | CheckoutFailureKind::RemoveOrigin
                | CheckoutFailureKind::VerifyHead,
            ) => Err(UnresolvedReason::CloneFailed),
        }
    };
    planned
        .into_iter()
        .map(|(identity, planned)| (identity, planned.and_then(checkout)))
        .collect()
}

/// `dir`'s HEAD commit, and whether HEAD is detached.
fn read_head(dir: &Path) -> Option<(String, bool)> {
    let repository = git2::Repository::open(dir).ok()?;
    let sha = repository
        .head()
        .ok()?
        .peel_to_commit()
        .ok()?
        .id()
        .to_string();
    Some((sha, repository.head_detached().ok()?))
}

/// The commit of a finished checkout of `head` in `dir`. The helper ends every requested head with
/// a detached checkout, and an interrupted clone removes its directory.
fn finished_head(dir: &Path, head: Option<&RepositoryHeadRef>) -> Option<String> {
    let (sha, detached) = read_head(dir)?;
    let finished = match head {
        None => !detached,
        Some(RepositoryHeadRef::Branch(_)) => detached,
        Some(RepositoryHeadRef::CommitSha(expected)) => detached && *expected == sha,
    };
    finished.then_some(sha)
}

/// Selects each reference file's skills, links them into a generated folder per level, and returns
/// the run's skill folders with each generated folder after the folder that owns it. A skill
/// whose name an earlier folder or level already provides is skipped.
fn materialize(
    discovery: &SkillReferenceDiscovery,
    checkouts: &HashMap<SourceIdentity, Checkout>,
) -> (Vec<PathBuf>, SkillSourcesReport) {
    let mut skill_dirs = Vec::new();
    let mut claimed = HashSet::new();
    let mut references = Vec::new();
    let mut levels = discovery.levels.iter().peekable();
    let dirs = discovery
        .configured_dirs
        .iter()
        .zip(&discovery.resolved_dirs);
    for (index, (configured, resolved)) in dirs.enumerate() {
        skill_dirs.push(configured.clone());
        claimed.extend(read_skills(resolved).into_iter().map(|skill| skill.name));
        let Some(level) = levels.next_if(|level| level.index == index) else {
            continue;
        };
        let generated = discovery.root.join("skills").join(index.to_string());
        let mut level_names = HashSet::new();
        for (reference_file, parsed) in &level.files {
            let mut report = ReferenceFileReport {
                reference_file: reference_file.clone(),
                level: match level.agent {
                    Some(_) => ReferenceLevel::Agent,
                    None => ReferenceLevel::Factory,
                },
                agent: level.agent.clone(),
                source: None,
                loaded: Vec::new(),
                unresolved: Vec::new(),
            };
            let unresolve = |entry: &str, reason| UnresolvedEntry {
                entry: entry.to_owned(),
                reason,
            };
            let Ok(file) = parsed else {
                report
                    .unresolved
                    .push(unresolve("", UnresolvedReason::InvalidReferenceFile));
                references.push(report);
                continue;
            };
            let checkout = checkouts
                .get(&identity(&file.source))
                .cloned()
                .unwrap_or(Err(UnresolvedReason::CloneFailed));
            report.source = Some(ReferencedSource {
                code_forge: file.source.code_forge,
                owner: file.source.owner.clone(),
                repo: file.source.repo.clone(),
                requested_ref: file
                    .source
                    .reference
                    .as_ref()
                    .map(|reference| reference.value().to_owned()),
                resolved_sha: checkout.as_ref().ok().and_then(|(_, sha)| sha.clone()),
            });
            let checkout_dir = match checkout {
                Ok((dir, _)) => dir,
                Err(reason) => {
                    report.unresolved = file
                        .include
                        .iter()
                        .map(|entry| unresolve(entry.value(), reason))
                        .collect();
                    references.push(report);
                    continue;
                }
            };
            let (candidates, unresolved) = select_skills(file, &checkout_dir);
            report.unresolved = unresolved;
            for candidate in candidates {
                let skill_dir = checkout_dir.join(&candidate.rel_dir);
                let reason = if level_names.contains(&candidate.name) {
                    Some(UnresolvedReason::Collision)
                } else if claimed.contains(&candidate.name) {
                    continue;
                } else if !is_valid_link_name(&candidate.name)
                    || fs::create_dir_all(&generated)
                        .and_then(|()| create_symlink(&skill_dir, &generated.join(&candidate.name)))
                        .is_err()
                {
                    Some(UnresolvedReason::InvalidSkill)
                } else {
                    None
                };
                match reason {
                    Some(reason) => report.unresolved.push(unresolve(&candidate.entry, reason)),
                    None => {
                        level_names.insert(candidate.name.clone());
                        report.loaded.push(LoadedSkill {
                            name: candidate.name,
                            path: candidate.rel_dir,
                        });
                    }
                }
            }
            references.push(report);
        }
        if !level_names.is_empty() {
            skill_dirs.push(generated);
        }
        claimed.extend(level_names);
    }
    for report in &mut references {
        let mut seen = HashSet::new();
        report
            .unresolved
            .retain(|entry| seen.insert((entry.entry.clone(), entry.reason)));
    }
    (skill_dirs, SkillSourcesReport { references })
}

/// A skill folder selected by an `include` entry.
struct Candidate {
    name: String,
    rel_dir: String,
    entry: String,
}

/// Resolves `file`'s includes in the checkout at `root`, then applies its excludes. Candidates are
/// in `include` order, without repeats of the same folder.
fn select_skills(file: &SkillReferenceFile, root: &Path) -> (Vec<Candidate>, Vec<UnresolvedEntry>) {
    let standard = standard_skills(root);
    let mut walked = None;
    let mut candidates = Vec::new();
    let mut unresolved = Vec::new();
    for entry in &file.include {
        let matcher = entry.glob_matcher();
        let results: Vec<_> = match entry {
            SkillReferenceEntry::Name(_) | SkillReferenceEntry::NameGlob(_) => standard
                .iter()
                .filter(|(name, rel_dir)| selects(entry, matcher.as_ref(), name, rel_dir))
                .map(|skill| Ok(skill.clone()))
                .collect(),
            SkillReferenceEntry::Path(path) => vec![skill_at(root, normalized_path(path))],
            SkillReferenceEntry::PathGlob(_) => walked
                .get_or_insert_with(|| skill_folders(root))
                .iter()
                .filter(|rel_dir| selects(entry, matcher.as_ref(), "", rel_dir))
                .map(|rel_dir| skill_at(root, rel_dir.clone()))
                .collect(),
        };
        let results = match results.is_empty() {
            true => vec![Err(UnresolvedReason::NoMatch)],
            false => results,
        };
        for result in results {
            match result {
                Ok((name, rel_dir)) => candidates.push(Candidate {
                    name,
                    rel_dir,
                    entry: entry.value().to_owned(),
                }),
                Err(reason) => unresolved.push(UnresolvedEntry {
                    entry: entry.value().to_owned(),
                    reason,
                }),
            }
        }
    }
    let excludes = file
        .exclude
        .iter()
        .map(|entry| (entry, entry.glob_matcher()))
        .collect::<Vec<_>>();
    let mut folders = HashSet::new();
    candidates.retain(|candidate| {
        folders.insert(candidate.rel_dir.clone())
            && !excludes.iter().any(|(entry, matcher)| {
                selects(entry, matcher.as_ref(), &candidate.name, &candidate.rel_dir)
            })
    });
    (candidates, unresolved)
}

/// Valid skills in the standard skill folders as `(name, folder)`, keeping the first of each name
/// in provider order.
fn standard_skills(root: &Path) -> Vec<(String, String)> {
    let mut names = HashSet::new();
    let mut skills = Vec::new();
    for provider in SKILL_PROVIDER_DEFINITIONS.iter() {
        let Ok(entries) = fs::read_dir(root.join(&provider.skills_path)) else {
            continue;
        };
        let mut dirs = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        dirs.sort();
        skills.extend(dirs.into_iter().filter_map(|dir| {
            is_contained(root, &dir).then_some(())?;
            let name = parse_skill(&dir.join(SKILL_FILE)).ok()?.name;
            names.insert(name.clone()).then_some(())?;
            Some((name, relative(root, &dir)?))
        }));
    }
    skills
}

/// The skill in `rel_dir`, as `(name, folder)`.
fn skill_at(root: &Path, rel_dir: String) -> Result<(String, String), UnresolvedReason> {
    let skill_file = root.join(&rel_dir).join(SKILL_FILE);
    if !skill_file.is_file() || !is_contained(root, &root.join(&rel_dir)) {
        return Err(UnresolvedReason::NoMatch);
    }
    let skill = parse_skill(&skill_file).map_err(|_| UnresolvedReason::InvalidSkill)?;
    Ok((skill.name, rel_dir))
}

/// Every folder in the checkout that holds a `SKILL.md`, relative to its root.
fn skill_folders(root: &Path) -> Vec<String> {
    WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .take(MAX_WALKED_ENTRIES)
        .flatten()
        .filter(|entry| entry.file_type().is_dir() && entry.path().join(SKILL_FILE).is_file())
        .filter_map(|entry| relative(root, entry.path()))
        .collect()
}

/// Whether `entry` selects the skill `name` in folder `rel_dir`; `matcher` is its compiled glob.
fn selects(
    entry: &SkillReferenceEntry,
    matcher: Option<&SkillGlobMatcher>,
    name: &str,
    rel_dir: &str,
) -> bool {
    match entry {
        SkillReferenceEntry::Name(entry_name) => entry_name == name,
        SkillReferenceEntry::Path(path) => normalized_path(path) == rel_dir,
        SkillReferenceEntry::NameGlob(_) => matcher.is_some_and(|matcher| matcher.is_match(name)),
        // A path glob selects a skill by its folder or its `SKILL.md`.
        SkillReferenceEntry::PathGlob(_) => matcher.is_some_and(|matcher| {
            matcher.is_match(rel_dir) || matcher.is_match(&format!("{rel_dir}/{SKILL_FILE}"))
        }),
    }
}

/// A path entry as a skill folder relative to the repository root, which is `.`.
fn normalized_path(path: &str) -> String {
    let mut segments = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    if segments.last() == Some(&SKILL_FILE) {
        segments.pop();
    }
    match segments.is_empty() {
        true => ".".to_owned(),
        false => segments.join("/"),
    }
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    Some(path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/"))
}

/// Whether the skill folder `dir` and every symlink reachable in it resolve inside `root`, so a
/// source can't expose files from elsewhere on the machine. A folder too large to check, or with
/// a dangling or looping link, doesn't count.
fn is_contained(root: &Path, dir: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let inside = |path: &Path| {
        path.canonicalize()
            .is_ok_and(|path| path.starts_with(&root))
    };
    let mut entries = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git");
    inside(dir)
        && entries
            .by_ref()
            .take(MAX_WALKED_ENTRIES)
            .all(|entry| entry.is_ok_and(|entry| !entry.path_is_symlink() || inside(entry.path())))
        && entries.next().is_none()
}

/// Whether a skill name can name a folder; names come from untrusted frontmatter.
fn is_valid_link_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..")
        && name.len() <= 255
        && !name
            .contains(|character: char| character.is_control() || matches!(character, '/' | '\\'))
}

#[cfg(test)]
#[path = "skill_references_tests.rs"]
mod tests;
