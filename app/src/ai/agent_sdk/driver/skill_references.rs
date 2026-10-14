//! Resolves the skill reference files in a Factory definition's skill folders.
//!
//! Each referenced repository is checked out outside the working directory, so nothing else
//! discovers it, and the selected skills are exposed through a generated skill folder that is
//! placed immediately after the skill folder owning the reference files. Oz skill loading and
//! third-party harness publishing then pick those folders up like any other skill folder.
//!
//! Resolution never fails a run: every problem is recorded as an unresolved entry and the rest
//! still loads.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ai::skills::{
    SKILL_PROVIDER_DEFINITIONS, SkillReferenceEntry, SkillReferenceFile, SkillReferenceParseError,
    SkillSourceForge, SkillSourceRef, is_skill_reference_file_name, parse_skill,
    parse_skill_reference_file, read_skills, resolve_skills_dirs,
};
use futures::channel::oneshot;
use globset::GlobMatcher;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use walkdir::WalkDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};
use warp_core::{safe_info, safe_warn};
use warpui::r#async::executor::Background;
use warpui::{ModelSpawner, SingletonEntity as _};

use super::AgentDriver;
use super::environment;
use super::environment_checkout_protocol::{
    CheckoutBatch, CheckoutFailureKind, CheckoutReport, CheckoutRequest,
};
use crate::ai::agent_sdk::setup_observability::{OzRunTimelineEvent, SetupClientEventReporter};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, TaskStatusUpdate};

/// Environment variable naming the Factory's primary code forge, used for sources that don't
/// name one.
pub(super) const FACTORY_PRIMARY_CODE_FORGE_ENV: &str = "WARP_FACTORY_PRIMARY_CODE_FORGE";

const SKILL_SOURCES_DIRECTORY: &str = "oz-skill-sources";
const SOURCE_CHECKOUTS_DIRECTORY: &str = "sources";
const GENERATED_SKILLS_DIRECTORY: &str = "skills";
const AGENTS_DIRECTORY: &str = "agents";
const SKILLS_DIRECTORY: &str = "skills";
const SKILL_FILE: &str = "SKILL.md";
const GIT_DIRECTORY: &str = ".git";
/// Bounds how much of a source repository a path glob walks.
const MAX_WALKED_ENTRIES: usize = 200_000;

/// Bounds on the run record, kept within what the server stores for one timeline event.
const MAX_RECORDED_REFERENCE_FILES: usize = 200;
const MAX_RECORDED_ENTRIES: usize = 256;
const MAX_RECORDED_STRING_BYTES: usize = 512;
const MAX_RECORDED_PAYLOAD_BYTES: usize = 60 * 1024;
/// How many unresolved entries the run status message names before summarizing the rest.
const MAX_STATUS_MESSAGE_ENTRIES: usize = 5;

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
    /// The number of unresolved entries across all reference files.
    pub fn warning_count(&self) -> usize {
        self.references
            .iter()
            .map(|reference| reference.unresolved.len())
            .sum()
    }

    /// The `skill_sources_resolved` timeline payload. Long strings are truncated and, when the
    /// record would be too large, loaded skills are dropped before warnings, and later reference
    /// files before earlier ones.
    pub fn timeline_payload(&self) -> serde_json::Value {
        let mut remaining =
            MAX_RECORDED_PAYLOAD_BYTES.saturating_sub(json_size(&SkillSourcesReport::default()));
        let mut references = Vec::new();
        for reference in self.references.iter().take(MAX_RECORDED_REFERENCE_FILES) {
            let mut recorded = ReferenceFileReport {
                reference_file: truncated(&reference.reference_file),
                level: reference.level,
                agent: reference.agent.as_deref().map(truncated),
                source: reference.source.as_ref().map(|source| ReferencedSource {
                    code_forge: source.code_forge,
                    owner: truncated(&source.owner),
                    name: truncated(&source.name),
                    requested_ref: source.requested_ref.as_deref().map(truncated),
                    resolved_sha: source.resolved_sha.clone(),
                }),
                loaded: Vec::new(),
                unresolved: Vec::new(),
            };
            // Each `+ 1` is the separator before the reference or entry.
            let mut size = json_size(&recorded) + 1;
            if size > remaining {
                break;
            }
            for entry in reference.unresolved.iter().take(MAX_RECORDED_ENTRIES) {
                let entry = UnresolvedEntry {
                    entry: truncated(&entry.entry),
                    reason: entry.reason,
                };
                let entry_size = json_size(&entry) + 1;
                if size + entry_size > remaining {
                    break;
                }
                size += entry_size;
                recorded.unresolved.push(entry);
            }
            remaining -= size;
            references.push(recorded);
        }
        for (recorded, reference) in references.iter_mut().zip(&self.references) {
            for skill in reference.loaded.iter().take(MAX_RECORDED_ENTRIES) {
                let skill = LoadedSkill {
                    name: truncated(&skill.name),
                    path: truncated(&skill.path),
                };
                let skill_size = json_size(&skill) + 1;
                if skill_size > remaining {
                    break;
                }
                remaining -= skill_size;
                recorded.loaded.push(skill);
            }
        }
        serde_json::to_value(SkillSourcesReport { references }).unwrap_or_default()
    }

    /// The run status message for a report with warnings, naming the first few.
    pub fn status_message(&self) -> Option<String> {
        let warnings = self
            .references
            .iter()
            .flat_map(|reference| {
                reference.unresolved.iter().map(move |entry| {
                    if entry.entry.is_empty() {
                        format!("{} ({})", reference.reference_file, entry.reason.code())
                    } else {
                        format!(
                            "{} '{}' ({})",
                            reference.reference_file,
                            entry.entry,
                            entry.reason.code()
                        )
                    }
                })
            })
            .collect::<Vec<_>>();
        if warnings.is_empty() {
            return None;
        }
        let mut details = warnings
            .iter()
            .take(MAX_STATUS_MESSAGE_ENTRIES)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        if warnings.len() > MAX_STATUS_MESSAGE_ENTRIES {
            details.push_str(&format!(
                "; and {} more",
                warnings.len() - MAX_STATUS_MESSAGE_ENTRIES
            ));
        }
        Some(format!(
            "Warning: some referenced skills did not load ({details}); continuing without them."
        ))
    }
}

impl UnresolvedReason {
    fn code(self) -> &'static str {
        match self {
            Self::CloneFailed => "clone_failed",
            Self::RefNotFound => "ref_not_found",
            Self::NoMatch => "no_match",
            Self::InvalidSkill => "invalid_skill",
            Self::InvalidReferenceFile => "invalid_reference_file",
            Self::Collision => "collision",
        }
    }
}

fn json_size(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX / 2, |bytes| bytes.len())
}

fn truncated(value: &str) -> String {
    if value.len() <= MAX_RECORDED_STRING_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_RECORDED_STRING_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// The outcome for one reference file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ReferenceFileReport {
    /// Path of the reference file relative to the definition root.
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
    pub code_forge: RepositoryForge,
    pub owner: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    /// The commit the skills were loaded from, absent when the checkout failed.
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

/// Reference files found in a run's skill folders, ready to be resolved.
pub(super) struct SkillReferenceDiscovery {
    task_id: AmbientAgentTaskId,
    configured_dirs: Vec<PathBuf>,
    resolved_dirs: Vec<PathBuf>,
    references: DiscoveredReferences,
}

impl AgentDriver {
    /// Finds the reference files in this run's skill folders. Returns `None` for runs without a
    /// task or without reference files, which then behave exactly as if references didn't exist.
    pub(super) async fn discover_skill_references(
        environment_forges: Vec<RepositoryForge>,
        foreground: &ModelSpawner<Self>,
    ) -> Option<SkillReferenceDiscovery> {
        let (task_id, working_dir, configured_dirs) = foreground
            .spawn(|me, _| (me.task_id, me.working_dir.clone(), me.skill_dirs.clone()))
            .await
            .ok()?;
        let task_id = task_id?;
        let resolved_dirs = resolve_skills_dirs(&working_dir, configured_dirs.clone());
        let primary_forge = std::env::var(FACTORY_PRIMARY_CODE_FORGE_ENV).ok();
        let default_forge = default_source_forge(primary_forge.as_deref(), environment_forges);
        let references = discover(&resolved_dirs, default_forge)?;
        Some(SkillReferenceDiscovery {
            task_id,
            configured_dirs,
            resolved_dirs,
            references,
        })
    }

    /// Checks out the referenced repositories, generates the skill folders for the selected
    /// skills, and updates this run's skill folders to include them.
    pub(super) async fn resolve_skill_references(
        discovery: SkillReferenceDiscovery,
        foreground: &ModelSpawner<Self>,
    ) -> SkillSourcesReport {
        let SkillReferenceDiscovery {
            task_id,
            configured_dirs,
            resolved_dirs,
            references,
        } = discovery;
        let Ok(background) = foreground.spawn(|_, ctx| ctx.background_executor()).await else {
            return references.unresolved_report();
        };
        let root = skill_sources_root(&task_id);
        let sources_dir = root.join(SOURCE_CHECKOUTS_DIRECTORY);
        let prepared = run_in_background(&background, {
            let root = root.clone();
            let sources_dir = sources_dir.clone();
            move || prepare_root(&root, &sources_dir)
        })
        .await;
        let plan = references.checkout_plan(&sources_dir);
        let report = match prepared {
            Some(Ok(())) if !plan.batch.repositories.is_empty() => {
                let batch = plan.batch.clone();
                match foreground
                    .spawn(move |me, ctx| {
                        me.terminal_driver.update(ctx, |_, ctx| {
                            environment::checkout_skill_sources(batch, ctx)
                        })
                    })
                    .await
                {
                    Ok(checkout) => checkout.await,
                    Err(_) => None,
                }
            }
            Some(Ok(())) => None,
            Some(Err(error)) => {
                safe_warn!(
                    safe: ("Could not prepare the skill source directory"),
                    full: ("Could not prepare the skill source directory {}: {error}", root.display())
                );
                None
            }
            None => None,
        };
        let generated_root = root.join(GENERATED_SKILLS_DIRECTORY);
        let resolution = run_in_background(&background, move || {
            let checkouts = plan.into_checkouts(report.as_ref());
            references.materialize(
                &configured_dirs,
                &resolved_dirs,
                &checkouts,
                &generated_root,
            )
        })
        .await;
        let Some(resolution) = resolution else {
            log::warn!("Skill reference resolution did not complete");
            return SkillSourcesReport::default();
        };
        log_report(&resolution.report);
        let SkillReferenceResolution { skill_dirs, report } = resolution;
        if let Err(error) = foreground
            .spawn(move |me, _| {
                me.skill_dirs = skill_dirs;
                me.skill_dirs_first_name_wins = true;
            })
            .await
        {
            log::warn!("Could not apply resolved skill references: {error}");
        }
        report
    }

    /// Records `report` on this run. See [`post_skill_sources`].
    pub(super) async fn report_skill_sources(
        report: &SkillSourcesReport,
        setup_events: &SetupClientEventReporter,
        foreground: &ModelSpawner<Self>,
    ) {
        let Ok((Some(task_id), ai_client)) = foreground
            .spawn(|me, ctx| {
                (
                    me.task_id,
                    ServerApiProvider::as_ref(ctx).get_ai_client().clone(),
                )
            })
            .await
        else {
            return;
        };
        post_skill_sources(report, task_id, setup_events, ai_client).await;
    }
}

/// Records `report` on the run as a `skill_sources_resolved` timeline event and, when anything
/// didn't resolve, a run status message. Both are best-effort: failures are logged and the run
/// continues.
pub(super) async fn post_skill_sources(
    report: &SkillSourcesReport,
    task_id: AmbientAgentTaskId,
    setup_events: &SetupClientEventReporter,
    ai_client: Arc<dyn AIClient>,
) {
    let timeline_event = setup_events.post_timeline_event_with_payload(
        OzRunTimelineEvent::SkillSourcesResolved,
        report.timeline_payload(),
    );
    let status_message = async {
        let Some(message) = report.status_message() else {
            return;
        };
        // The server persists status messages only on terminal state transitions today, so this
        // is best-effort until message-only updates are supported.
        if let Err(error) = ai_client
            .update_agent_task(
                task_id,
                None,
                None,
                None,
                Some(TaskStatusUpdate::message(message)),
                None,
                None,
            )
            .await
        {
            log::warn!("Failed to report skill reference warnings for task {task_id}: {error:#}");
        }
    };
    futures::join!(timeline_event, status_message);
}

/// Directory holding a run's skill source checkouts and generated skill folders.
pub(super) fn skill_sources_root(task_id: &AmbientAgentTaskId) -> PathBuf {
    std::env::temp_dir()
        .join(SKILL_SOURCES_DIRECTORY)
        .join(task_id.to_string())
}

/// Picks the forge for sources that don't name one: the Factory's primary forge when the run
/// provides it, otherwise the single forge shared by the run's repositories, otherwise GitHub.
pub(super) fn default_source_forge(
    primary_forge: Option<&str>,
    environment_forges: impl IntoIterator<Item = RepositoryForge>,
) -> RepositoryForge {
    if let Some(forge) = primary_forge
        .map(str::trim)
        .and_then(SkillSourceForge::from_token)
    {
        return repository_forge(forge);
    }
    let forges = environment_forges.into_iter().collect::<HashSet<_>>();
    match forges.into_iter().collect::<Vec<_>>().as_slice() {
        [forge] => *forge,
        _ => RepositoryForge::GitHub,
    }
}

fn repository_forge(forge: SkillSourceForge) -> RepositoryForge {
    match forge {
        SkillSourceForge::GitHub => RepositoryForge::GitHub,
        SkillSourceForge::GitLab => RepositoryForge::GitLab,
        SkillSourceForge::AzureDevOps => RepositoryForge::AzureDevOps,
    }
}

async fn run_in_background<T: Send + 'static>(
    background: &Background,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = oneshot::channel();
    background
        .spawn(async move {
            let _ = sender.send(work());
        })
        .detach();
    receiver.await.ok()
}

/// Starts from an empty root so nothing from an earlier attempt in the same sandbox is reused.
fn prepare_root(root: &Path, sources_dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(sources_dir)
}

/// The run's skill folders with the generated folders inserted, and what was resolved.
pub(super) struct SkillReferenceResolution {
    pub(super) skill_dirs: Vec<PathBuf>,
    pub(super) report: SkillSourcesReport,
}

/// Reference files grouped by the skill folder that holds them.
pub(super) struct DiscoveredReferences {
    levels: Vec<DiscoveredLevel>,
}

struct DiscoveredLevel {
    skills_dir: PathBuf,
    agent: Option<String>,
    files: Vec<DiscoveredFile>,
}

struct DiscoveredFile {
    reference_file: String,
    parsed: Result<ReferenceSpec, SkillReferenceParseError>,
}

struct ReferenceSpec {
    file: SkillReferenceFile,
    source: SourceKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceKey {
    forge: RepositoryForge,
    owner: String,
    name: String,
    reference: Option<SkillSourceRef>,
}

/// Source identity for deduplication; owners and names are case-insensitive on every forge.
type SourceIdentity = (RepositoryForge, String, String, Option<SkillSourceRef>);

impl SourceKey {
    fn identity(&self) -> SourceIdentity {
        (
            self.forge,
            self.owner.to_lowercase(),
            self.name.to_lowercase(),
            self.reference.clone(),
        )
    }

    fn head(&self) -> Option<RepositoryHeadRef> {
        self.reference.as_ref().map(|reference| match reference {
            SkillSourceRef::Sha(sha) => RepositoryHeadRef::CommitSha(sha.clone()),
            SkillSourceRef::Named(name) => RepositoryHeadRef::NamedRef(name.clone()),
        })
    }

    fn checkout_name(&self) -> String {
        let (forge, owner, name, reference) = self.identity();
        let mut hasher = Sha256::new();
        for part in [
            forge_token(forge),
            &owner,
            &name,
            reference.as_ref().map_or("", SkillSourceRef::value),
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        let digest = hasher
            .finalize()
            .iter()
            .take(6)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let readable = format!("{}-{owner}-{name}", forge_token(forge).to_lowercase())
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '-'
                }
            })
            .take(80)
            .collect::<String>();
        format!("{readable}-{digest}")
    }
}

fn forge_token(forge: RepositoryForge) -> &'static str {
    match forge {
        RepositoryForge::GitHub => SkillSourceForge::GitHub.token(),
        RepositoryForge::GitLab => SkillSourceForge::GitLab.token(),
        RepositoryForge::AzureDevOps => SkillSourceForge::AzureDevOps.token(),
    }
}

/// Reads the reference files that sit directly in each skill folder, in file-name order.
/// Returns `None` when there are none.
pub(super) fn discover(
    skill_dirs: &[PathBuf],
    default_forge: RepositoryForge,
) -> Option<DiscoveredReferences> {
    let mut seen = HashSet::new();
    let mut levels = Vec::new();
    for skills_dir in skill_dirs {
        if !seen.insert(skills_dir.clone()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(skills_dir) else {
            continue;
        };
        let mut file_names = entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|file_name| is_skill_reference_file_name(file_name))
            .collect::<Vec<_>>();
        if file_names.is_empty() {
            continue;
        }
        file_names.sort();
        let agent = agent_for_skills_dir(skills_dir);
        let files = file_names
            .into_iter()
            .map(|file_name| {
                let parsed = fs::read_to_string(skills_dir.join(&file_name))
                    .map_err(|_| SkillReferenceParseError::InvalidYaml)
                    .and_then(|contents| parse_skill_reference_file(&contents))
                    .map(|file| {
                        let source = SourceKey {
                            forge: file
                                .source
                                .code_forge
                                .map_or(default_forge, repository_forge),
                            owner: file.source.owner.clone(),
                            name: file.source.name.clone(),
                            reference: file.source.reference.clone(),
                        };
                        ReferenceSpec { file, source }
                    });
                DiscoveredFile {
                    reference_file: reference_file_path(skills_dir, agent.as_deref(), &file_name),
                    parsed,
                }
            })
            .collect();
        levels.push(DiscoveredLevel {
            skills_dir: skills_dir.clone(),
            agent,
            files,
        });
    }
    (!levels.is_empty()).then_some(DiscoveredReferences { levels })
}

/// The agent owning `skills_dir` when it is an `agents/<name>/skills` folder.
fn agent_for_skills_dir(skills_dir: &Path) -> Option<String> {
    let mut components = skills_dir.components().rev();
    let (Some(skills), Some(agent), Some(agents)) =
        (components.next(), components.next(), components.next())
    else {
        return None;
    };
    (skills.as_os_str() == SKILLS_DIRECTORY && agents.as_os_str() == AGENTS_DIRECTORY)
        .then(|| agent.as_os_str().to_str().map(str::to_owned))
        .flatten()
}

fn reference_file_path(skills_dir: &Path, agent: Option<&str>, file_name: &str) -> String {
    match agent {
        Some(agent) => format!("{AGENTS_DIRECTORY}/{agent}/{SKILLS_DIRECTORY}/{file_name}"),
        None => {
            let folder = skills_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(SKILLS_DIRECTORY);
            format!("{folder}/{file_name}")
        }
    }
}

/// The checkout batch for every distinct source, plus the sources that can't be requested.
pub(super) struct SourceCheckoutPlan {
    pub(super) batch: CheckoutBatch,
    planned: Vec<(SourceIdentity, bool)>,
    rejected: HashMap<SourceIdentity, UnresolvedReason>,
}

/// The checkout of each source, or why there isn't one.
pub(super) struct SourceCheckouts(
    HashMap<SourceIdentity, Result<CheckedOutSource, UnresolvedReason>>,
);

pub(super) struct CheckedOutSource {
    dir: PathBuf,
    resolved_sha: Option<String>,
}

impl DiscoveredReferences {
    /// One request per distinct source, checked out into `sources_dir`.
    pub(super) fn checkout_plan(&self, sources_dir: &Path) -> SourceCheckoutPlan {
        let mut seen = HashSet::new();
        let mut repositories = Vec::new();
        let mut planned = Vec::new();
        let mut rejected = HashMap::new();
        for spec in self
            .levels
            .iter()
            .flat_map(|level| &level.files)
            .filter_map(|file| file.parsed.as_ref().ok())
        {
            let identity = spec.source.identity();
            if !seen.insert(identity.clone()) {
                continue;
            }
            let request = CheckoutRequest {
                source: RepositoryIdentity {
                    code_forge: spec.source.forge,
                    repo_owner: spec.source.owner.clone(),
                    repo_name: spec.source.name.clone(),
                },
                checkout_name: spec.source.checkout_name(),
                head: spec.source.head(),
                fetch_branch_only: false,
            };
            match request.validate() {
                Ok(()) => {
                    planned.push((identity, request.head.is_some()));
                    repositories.push(request);
                }
                Err("invalid checkout commit" | "invalid checkout ref") => {
                    rejected.insert(identity, UnresolvedReason::RefNotFound);
                }
                Err(_) => {
                    rejected.insert(identity, UnresolvedReason::CloneFailed);
                }
            }
        }
        SourceCheckoutPlan {
            batch: CheckoutBatch {
                working_dir: sources_dir.to_owned(),
                repositories,
            },
            planned,
            rejected,
        }
    }

    /// A report that marks every reference as unresolved, for when resolution couldn't start.
    fn unresolved_report(&self) -> SkillSourcesReport {
        let checkouts = SourceCheckouts(HashMap::new());
        SkillSourcesReport {
            references: self
                .levels
                .iter()
                .flat_map(|level| {
                    level
                        .files
                        .iter()
                        .map(|file| file_report_without_skills(level, file, &checkouts))
                })
                .collect(),
        }
    }

    /// Selects the skills of every reference file, generates a skill folder per level, and
    /// returns `configured_dirs` with each generated folder after the folder that owns it.
    ///
    /// `resolved_dirs` are `configured_dirs` resolved against the working directory. Skills are
    /// considered in folder order, so a referenced skill whose name is already provided by an
    /// earlier folder, or an earlier level's references, is not loaded.
    pub(super) fn materialize(
        self,
        configured_dirs: &[PathBuf],
        resolved_dirs: &[PathBuf],
        checkouts: &SourceCheckouts,
        generated_root: &Path,
    ) -> SkillReferenceResolution {
        let mut levels = self
            .levels
            .into_iter()
            .map(|level| (level.skills_dir.clone(), level))
            .collect::<HashMap<_, _>>();
        let mut indexes = HashMap::<PathBuf, SourceIndex>::new();
        let mut claimed = HashSet::new();
        let mut skill_dirs = Vec::with_capacity(configured_dirs.len());
        let mut references = Vec::new();
        for (index, (configured, resolved)) in configured_dirs.iter().zip(resolved_dirs).enumerate()
        {
            skill_dirs.push(configured.clone());
            claimed.extend(read_skills(resolved).into_iter().map(|skill| skill.name));
            let Some(level) = levels.remove(resolved) else {
                continue;
            };
            let generated_dir = generated_root.join(index.to_string());
            let mut level_names = HashSet::new();
            for file in &level.files {
                let mut report = file_report_without_skills(&level, file, checkouts);
                let Ok(spec) = &file.parsed else {
                    references.push(report);
                    continue;
                };
                let Some(Ok(checkout)) = checkouts.0.get(&spec.source.identity()) else {
                    references.push(report);
                    continue;
                };
                let source_index = indexes
                    .entry(checkout.dir.clone())
                    .or_insert_with(|| SourceIndex::new(checkout.dir.clone()));
                let (candidates, unresolved) = select_skills(&spec.file, source_index);
                report.unresolved.extend(unresolved);
                for candidate in candidates {
                    if level_names.contains(&candidate.name) {
                        report.unresolved.push(UnresolvedEntry {
                            entry: candidate.entry,
                            reason: UnresolvedReason::Collision,
                        });
                        continue;
                    }
                    if claimed.contains(&candidate.name) {
                        safe_info!(
                            safe: ("A referenced skill is shadowed by a higher-precedence skill"),
                            full: (
                                "Referenced skill '{}' from {} is shadowed by a higher-precedence skill",
                                candidate.name, file.reference_file
                            )
                        );
                        continue;
                    }
                    if let Err(error) = link_skill(&generated_dir, &candidate) {
                        safe_warn!(
                            safe: ("Could not link a referenced skill"),
                            full: ("Could not link referenced skill '{}': {error}", candidate.name)
                        );
                        report.unresolved.push(UnresolvedEntry {
                            entry: candidate.entry,
                            reason: UnresolvedReason::InvalidSkill,
                        });
                        continue;
                    }
                    level_names.insert(candidate.name.clone());
                    report.loaded.push(LoadedSkill {
                        name: candidate.name,
                        path: candidate.rel_dir,
                    });
                }
                references.push(report);
            }
            if !level_names.is_empty() {
                skill_dirs.push(generated_dir);
            }
            claimed.extend(level_names);
        }
        SkillReferenceResolution {
            skill_dirs,
            report: SkillSourcesReport { references },
        }
    }
}

impl SourceCheckoutPlan {
    /// Pairs each planned source with its outcome in the helper's report. A missing report or
    /// outcome means the source couldn't be cloned.
    pub(super) fn into_checkouts(self, report: Option<&CheckoutReport>) -> SourceCheckouts {
        let mut checkouts = self
            .rejected
            .into_iter()
            .map(|(identity, reason)| (identity, Err(reason)))
            .collect::<HashMap<_, _>>();
        for (index, (identity, has_ref)) in self.planned.into_iter().enumerate() {
            let request = &self.batch.repositories[index];
            let outcome = report.and_then(|report| {
                report
                    .outcomes
                    .iter()
                    .find(|outcome| outcome.request_index == index)
            });
            let checkout = match outcome.map(|outcome| outcome.failure) {
                Some(None) => {
                    let dir = self.batch.working_dir.join(&request.checkout_name);
                    let resolved_sha = outcome
                        .and_then(|outcome| outcome.resolved_head.clone())
                        .or_else(|| read_head(&dir));
                    Ok(CheckedOutSource { dir, resolved_sha })
                }
                Some(Some(CheckoutFailureKind::Checkout)) if has_ref => {
                    Err(UnresolvedReason::RefNotFound)
                }
                Some(Some(
                    CheckoutFailureKind::Clone
                    | CheckoutFailureKind::Checkout
                    | CheckoutFailureKind::RemoveOrigin,
                ))
                | None => Err(UnresolvedReason::CloneFailed),
            };
            checkouts.insert(identity, checkout);
        }
        SourceCheckouts(checkouts)
    }
}

/// The commit checked out at `dir`, for checkouts whose HEAD the helper didn't report.
fn read_head(dir: &Path) -> Option<String> {
    let repository = git2::Repository::open(dir).ok()?;
    let commit = repository.head().ok()?.peel_to_commit().ok()?;
    Some(commit.id().to_string())
}

/// The report for `file` before any skill is selected: its source, and the reason nothing
/// loads when the file or its checkout failed.
fn file_report_without_skills(
    level: &DiscoveredLevel,
    file: &DiscoveredFile,
    checkouts: &SourceCheckouts,
) -> ReferenceFileReport {
    let mut report = ReferenceFileReport {
        reference_file: file.reference_file.clone(),
        level: if level.agent.is_some() {
            ReferenceLevel::Agent
        } else {
            ReferenceLevel::Factory
        },
        agent: level.agent.clone(),
        source: None,
        loaded: Vec::new(),
        unresolved: Vec::new(),
    };
    let spec = match &file.parsed {
        Ok(spec) => spec,
        Err(error) => {
            safe_warn!(
                safe: ("Skipping an invalid skill reference file"),
                full: ("Skipping invalid skill reference file {}: {error}", file.reference_file)
            );
            report.unresolved.push(UnresolvedEntry {
                entry: String::new(),
                reason: UnresolvedReason::InvalidReferenceFile,
            });
            return report;
        }
    };
    let checkout = checkouts
        .0
        .get(&spec.source.identity())
        .map_or(Err(UnresolvedReason::CloneFailed), |checkout| {
            checkout.as_ref().map_err(|reason| *reason)
        });
    report.source = Some(ReferencedSource {
        code_forge: spec.source.forge,
        owner: spec.source.owner.clone(),
        name: spec.source.name.clone(),
        requested_ref: spec
            .source
            .reference
            .as_ref()
            .map(|reference| reference.value().to_owned()),
        resolved_sha: checkout
            .ok()
            .and_then(|checkout| checkout.resolved_sha.clone()),
    });
    if let Err(reason) = checkout {
        report.unresolved = spec
            .file
            .include
            .iter()
            .map(|entry| UnresolvedEntry {
                entry: entry.value().to_owned(),
                reason,
            })
            .collect();
    }
    report
}

/// A skill folder selected by an `include` entry.
#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    rel_dir: String,
    dir: PathBuf,
    entry: String,
}

/// The skills a checkout offers, indexed lazily.
struct SourceIndex {
    root: PathBuf,
    /// Valid skills in the standard skill folders, in provider order.
    standard: Vec<IndexedSkill>,
    /// Standard skill folders whose `SKILL.md` doesn't parse.
    invalid_standard: Vec<IndexedSkill>,
    /// Every folder holding a `SKILL.md`, found on first use by a path glob.
    walked: Option<Vec<String>>,
}

/// A skill folder, named by its frontmatter name or, for invalid skills, its folder name.
struct IndexedSkill {
    name: String,
    rel_dir: String,
}

impl SourceIndex {
    fn new(root: PathBuf) -> Self {
        let mut standard = Vec::new();
        let mut invalid_standard = Vec::new();
        for definition in SKILL_PROVIDER_DEFINITIONS.iter() {
            let Some(base) = slash_path(&definition.skills_path) else {
                continue;
            };
            let Ok(entries) = fs::read_dir(root.join(&definition.skills_path)) else {
                continue;
            };
            let mut folders = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter(|entry| entry.path().join(SKILL_FILE).is_file())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect::<Vec<_>>();
            folders.sort();
            for folder in folders {
                let rel_dir = format!("{base}/{folder}");
                match parse_skill(&root.join(&rel_dir).join(SKILL_FILE)) {
                    Ok(skill) => standard.push(IndexedSkill {
                        name: skill.name,
                        rel_dir,
                    }),
                    Err(_) => invalid_standard.push(IndexedSkill {
                        name: folder,
                        rel_dir,
                    }),
                }
            }
        }
        Self {
            root,
            standard,
            invalid_standard,
            walked: None,
        }
    }

    fn candidate(&self, name: String, rel_dir: String, entry: &str) -> Candidate {
        Candidate {
            dir: self.root.join(&rel_dir),
            name,
            rel_dir,
            entry: entry.to_owned(),
        }
    }

    /// Standard skills deduplicated by name, keeping the first in provider order.
    fn standard_by_name(&self) -> Vec<&IndexedSkill> {
        let mut names = HashSet::new();
        self.standard
            .iter()
            .filter(|skill| names.insert(skill.name.as_str()))
            .collect()
    }

    fn walked(&mut self) -> &[String] {
        let root = &self.root;
        self.walked.get_or_insert_with(|| {
            let mut dirs = WalkDir::new(root)
                .min_depth(1)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| entry.file_name() != GIT_DIRECTORY)
                .take(MAX_WALKED_ENTRIES)
                .flatten()
                .filter(|entry| entry.file_type().is_dir())
                .filter(|entry| entry.path().join(SKILL_FILE).is_file())
                .filter_map(|entry| entry.path().strip_prefix(root).ok().and_then(slash_path))
                .collect::<Vec<_>>();
            dirs.sort();
            dirs
        })
    }

    /// Parses the skill in `rel_dir`, which must stay inside the checkout.
    fn parse(&self, rel_dir: &str, entry: &str) -> Result<Candidate, UnresolvedReason> {
        let dir = self.root.join(rel_dir);
        if !dir.join(SKILL_FILE).is_file() || !is_within(&self.root, &dir) {
            return Err(UnresolvedReason::NoMatch);
        }
        let skill =
            parse_skill(&dir.join(SKILL_FILE)).map_err(|_| UnresolvedReason::InvalidSkill)?;
        Ok(self.candidate(skill.name, rel_dir.to_owned(), entry))
    }
}

/// Resolves `file`'s includes against a checkout and applies its excludes. Candidates are in
/// `include` order, without repeats of the same folder.
fn select_skills(
    file: &SkillReferenceFile,
    index: &mut SourceIndex,
) -> (Vec<Candidate>, Vec<UnresolvedEntry>) {
    let mut candidates = Vec::new();
    let mut unresolved = Vec::new();
    for entry in &file.include {
        let value = entry.value();
        let mut unresolve = |reason| {
            unresolved.push(UnresolvedEntry {
                entry: value.to_owned(),
                reason,
            })
        };
        match entry {
            SkillReferenceEntry::Name(name) => {
                if let Some(skill) = index.standard.iter().find(|skill| &skill.name == name) {
                    candidates.push(index.candidate(
                        skill.name.clone(),
                        skill.rel_dir.clone(),
                        value,
                    ));
                } else if index
                    .invalid_standard
                    .iter()
                    .any(|skill| &skill.name == name)
                {
                    unresolve(UnresolvedReason::InvalidSkill);
                } else {
                    unresolve(UnresolvedReason::NoMatch);
                }
            }
            SkillReferenceEntry::Path(path) => {
                match normalized_skill_path(path)
                    .ok_or(UnresolvedReason::NoMatch)
                    .and_then(|rel_dir| index.parse(&rel_dir, value))
                {
                    Ok(candidate) => candidates.push(candidate),
                    Err(reason) => unresolve(reason),
                }
            }
            SkillReferenceEntry::NameGlob(_) => {
                let Some(matcher) = entry.glob_matcher() else {
                    unresolve(UnresolvedReason::NoMatch);
                    continue;
                };
                let matched = index
                    .standard_by_name()
                    .into_iter()
                    .filter(|skill| matcher.is_match(&skill.name))
                    .map(|skill| index.candidate(skill.name.clone(), skill.rel_dir.clone(), value))
                    .collect::<Vec<_>>();
                if matched.is_empty() {
                    unresolve(UnresolvedReason::NoMatch);
                }
                candidates.extend(matched);
            }
            SkillReferenceEntry::PathGlob(_) => {
                let Some(matcher) = entry.glob_matcher() else {
                    unresolve(UnresolvedReason::NoMatch);
                    continue;
                };
                let matched = index
                    .walked()
                    .iter()
                    .filter(|rel_dir| matches_skill_path(&matcher, rel_dir))
                    .cloned()
                    .collect::<Vec<_>>();
                if matched.is_empty() {
                    unresolve(UnresolvedReason::NoMatch);
                }
                for rel_dir in matched {
                    match index.parse(&rel_dir, value) {
                        Ok(candidate) => candidates.push(candidate),
                        Err(reason) => unresolve(reason),
                    }
                }
            }
        }
    }
    let mut folders = HashSet::new();
    candidates.retain(|candidate| folders.insert(candidate.rel_dir.clone()));
    let excludes = file
        .exclude
        .iter()
        .map(|entry| (entry, entry.glob_matcher()))
        .collect::<Vec<_>>();
    candidates.retain(|candidate| {
        !excludes
            .iter()
            .any(|(entry, matcher)| excludes_candidate(entry, matcher.as_ref(), candidate))
    });
    for candidate in &mut candidates {
        if !is_valid_skill_link_name(&candidate.name) {
            unresolved.push(UnresolvedEntry {
                entry: candidate.entry.clone(),
                reason: UnresolvedReason::InvalidSkill,
            });
        }
    }
    candidates.retain(|candidate| is_valid_skill_link_name(&candidate.name));
    (candidates, unresolved)
}

fn excludes_candidate(
    entry: &SkillReferenceEntry,
    matcher: Option<&GlobMatcher>,
    candidate: &Candidate,
) -> bool {
    match entry {
        SkillReferenceEntry::Name(name) => &candidate.name == name,
        SkillReferenceEntry::Path(path) => {
            normalized_skill_path(path).is_some_and(|rel_dir| rel_dir == candidate.rel_dir)
        }
        SkillReferenceEntry::NameGlob(_) => {
            matcher.is_some_and(|matcher| matcher.is_match(&candidate.name))
        }
        SkillReferenceEntry::PathGlob(_) => {
            matcher.is_some_and(|matcher| matches_skill_path(matcher, &candidate.rel_dir))
        }
    }
}

/// Whether a path glob selects the skill folder `rel_dir`, by its folder or its `SKILL.md`.
fn matches_skill_path(matcher: &GlobMatcher, rel_dir: &str) -> bool {
    matcher.is_match(rel_dir) || matcher.is_match(format!("{rel_dir}/{SKILL_FILE}"))
}

/// A path entry as a skill folder relative to the repository root, or `None` for the root
/// itself.
fn normalized_skill_path(path: &str) -> Option<String> {
    let mut segments = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    if segments.last() == Some(&SKILL_FILE) {
        segments.pop();
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

fn slash_path(path: &Path) -> Option<String> {
    let segments = path
        .components()
        .map(|component| match component {
            Component::Normal(segment) => segment.to_str(),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// Whether `dir` resolves to a location inside `root`, so a symlinked folder can't select files
/// from elsewhere.
fn is_within(root: &Path, dir: &Path) -> bool {
    match (root.canonicalize(), dir.canonicalize()) {
        (Ok(root), Ok(dir)) => dir.starts_with(root),
        _ => false,
    }
}

/// Whether a skill name can name its folder in a generated skill folder.
fn is_valid_skill_link_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

fn link_skill(generated_dir: &Path, candidate: &Candidate) -> std::io::Result<()> {
    fs::create_dir_all(generated_dir)?;
    create_dir_symlink(&candidate.dir, &generated_dir.join(&candidate.name))
}

#[cfg(unix)]
fn create_dir_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_dir_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
}

fn log_report(report: &SkillSourcesReport) {
    let loaded = report
        .references
        .iter()
        .map(|reference| reference.loaded.len())
        .sum::<usize>();
    log::info!(
        "Skill references: {} file(s), {loaded} skill(s) loaded, {} warning(s)",
        report.references.len(),
        report.warning_count()
    );
    for reference in &report.references {
        for unresolved in &reference.unresolved {
            safe_warn!(
                safe: ("A skill reference entry did not resolve: {:?}", unresolved.reason),
                full: (
                    "Skill reference {} entry '{}' did not resolve: {:?}",
                    reference.reference_file, unresolved.entry, unresolved.reason
                )
            );
        }
    }
}

#[cfg(test)]
#[path = "skill_references_tests.rs"]
mod tests;
