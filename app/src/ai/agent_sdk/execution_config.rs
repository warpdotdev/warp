use std::path::{Component, PathBuf};

use ai::hooks::{HookFormat, HookOnAsk, HookSourceKind, HookSourcePolicy};
use anyhow::{Context as _, bail};
use cloud_object_models::CodeForge as RuntimeForge;
use serde_json::{Map, Value};
use warp_cli::agent::{Harness, RepositoryHeadRef};
use warp_cli::mcp::MCPSpec;
use warp_cli::share::{ShareAccessLevel, ShareRequest, ShareSubject};
use warp_graphql::ai::AgentHarness;
use warp_graphql::queries::execution_config::{
    AgentHookFormat, AgentHookLevel, AgentHookOnAsk, AgentHookSource, CloudProviderClientConfigs,
    CodeForge, ExecutionRepository, ExecutionRepositoryRefType, FactoryBranchExecutionConfig,
    SessionSharingAccessLevel, SessionSharingAclSpec, SessionSharingSubjectType,
    SourceRepo as ConfigSourceRepo,
};

use super::driver::environment::ResolvedRepository;
use super::driver::hooks::{ConfiguredHookLocation, ConfiguredHookSource};
use super::{config_file, mcp_config};
use crate::ai::cloud_environments::{
    AwsProviderConfig, GcpProviderConfig, ProvidersConfig, SourceRepo,
};
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::HeadlessTeamScope;

pub(super) fn team_scope(team_id: Option<&cynic::Id>) -> anyhow::Result<HeadlessTeamScope> {
    match team_id {
        Some(team_id) => Ok(HeadlessTeamScope::Team(
            ServerId::try_from(team_id.inner())
                .context("Execution task has an invalid team UID")?,
        )),
        None => Ok(HeadlessTeamScope::Personal),
    }
}

pub(super) fn harness(value: &AgentHarness) -> anyhow::Result<Harness> {
    match value {
        AgentHarness::Oz => Ok(Harness::Oz),
        AgentHarness::ClaudeCode => Ok(Harness::Claude),
        AgentHarness::Gemini => Ok(Harness::Gemini),
        AgentHarness::Pi => Ok(Harness::Pi),
        AgentHarness::Codex => Ok(Harness::Codex),
        AgentHarness::Other(name) => bail!("Unsupported execution harness {name}"),
    }
}

pub(super) fn mcp_specs(json: &str) -> anyhow::Result<Vec<MCPSpec>> {
    let map = serde_json::from_str::<Map<String, Value>>(json)
        .context("Invalid execution MCP server configuration")?;
    mcp_config::validate_mcp_servers(&map)?;
    config_file::mcp_specs_from_mcp_servers(&map)
}

fn forge(forge: CodeForge) -> anyhow::Result<RuntimeForge> {
    match forge {
        CodeForge::GitHub => Ok(RuntimeForge::GitHub),
        CodeForge::GitLab => Ok(RuntimeForge::GitLab),
        CodeForge::AzureDevOps => Ok(RuntimeForge::AzureDevOps),
        CodeForge::Unknown => bail!("Unsupported execution repository forge"),
    }
}

fn source_repo(source: ConfigSourceRepo) -> anyhow::Result<SourceRepo> {
    Ok(SourceRepo::new(
        forge(source.code_forge)?,
        source.owner,
        source.repo,
    ))
}

pub(super) fn repositories(
    repositories: Vec<ExecutionRepository>,
) -> anyhow::Result<Vec<ResolvedRepository>> {
    repositories
        .into_iter()
        .map(|repo| {
            let source = SourceRepo::new(forge(repo.forge)?, repo.owner, repo.name);
            let checkout = repo
                .ref_
                .map(|reference| match reference.type_ {
                    ExecutionRepositoryRefType::Branch => {
                        Ok(RepositoryHeadRef::Branch(reference.value))
                    }
                    ExecutionRepositoryRefType::CommitSha => {
                        Ok(RepositoryHeadRef::CommitSha(reference.value))
                    }
                    ExecutionRepositoryRefType::Unknown => {
                        bail!("Unsupported execution repository ref")
                    }
                })
                .transpose()?;
            let clone_from = repo.clone_from.map(source_repo).transpose()?;
            Ok(ResolvedRepository {
                source,
                checkout,
                clone_from,
                preserve_origin: repo.preserve_origin,
                expected_sha: repo.expected_sha,
            })
        })
        .collect()
}

pub(super) fn deferred_repositories(
    repositories: Vec<ConfigSourceRepo>,
) -> anyhow::Result<Vec<SourceRepo>> {
    repositories.into_iter().map(source_repo).collect()
}

pub(super) fn sharing_acls(acls: Vec<SessionSharingAclSpec>) -> anyhow::Result<Vec<ShareRequest>> {
    acls.into_iter()
        .map(|acl| {
            let subject = match acl.subject_type {
                SessionSharingSubjectType::Team if acl.email.is_none() => ShareSubject::Team,
                SessionSharingSubjectType::Public if acl.email.is_none() => ShareSubject::Public,
                SessionSharingSubjectType::UserEmail => {
                    let email = acl.email.filter(|email| !email.trim().is_empty());
                    ShareSubject::User {
                        email: email.context("User sharing ACL has no email")?,
                    }
                }
                SessionSharingSubjectType::Unknown => {
                    bail!("Unsupported execution sharing subject")
                }
                SessionSharingSubjectType::Team | SessionSharingSubjectType::Public => {
                    bail!("Invalid execution sharing ACL")
                }
            };
            let access_level = match acl.access {
                SessionSharingAccessLevel::View => ShareAccessLevel::View,
                SessionSharingAccessLevel::Edit => ShareAccessLevel::Edit,
                SessionSharingAccessLevel::Unknown => {
                    bail!("Unsupported execution sharing access")
                }
            };
            Ok(ShareRequest {
                subject,
                access_level,
            })
        })
        .collect()
}

pub(super) fn providers(config: Option<CloudProviderClientConfigs>) -> ProvidersConfig {
    let Some(config) = config else {
        return ProvidersConfig::default();
    };
    ProvidersConfig {
        gcp: config.gcp.map(|gcp| GcpProviderConfig {
            project_number: gcp.project_number,
            workload_identity_federation_pool_id: gcp.pool_id,
            workload_identity_federation_provider_id: gcp.provider_id,
            service_account_email: gcp.service_account_email,
        }),
        aws: config.aws.map(|aws| AwsProviderConfig {
            role_arn: aws.role_arn,
        }),
    }
}

pub(super) fn idle_duration(seconds: Option<i32>) -> anyhow::Result<Option<std::time::Duration>> {
    seconds
        .map(|value| {
            u64::try_from(value)
                .map(std::time::Duration::from_secs)
                .context("Execution idle window must not be negative")
        })
        .transpose()
}

pub(super) fn factory_skill_dirs(dirs: Vec<String>) -> anyhow::Result<Vec<PathBuf>> {
    dirs.into_iter()
        .map(|dir| {
            if dir.is_empty()
                || dir.trim() != dir
                || dir.contains([',', '\0'])
                || dir.starts_with('~')
            {
                bail!("Execution skill directory is not an unambiguous relative path");
            }
            let path = PathBuf::from(dir);
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
            {
                bail!("Execution skill directory must stay within the working directory");
            }
            Ok(path)
        })
        .collect()
}

/// The hook directories the execution configuration selected, in the order the server sent them.
/// A value this client does not know (a newer format, level, on-ask setting, or forge) fails the
/// run rather than dropping the entry, because declared hooks are never silently skipped. Paths are
/// checked later, when each entry is found on disk, so a malformed one fails only that entry.
pub(super) fn agent_hook_sources(
    sources: Vec<AgentHookSource>,
) -> anyhow::Result<Vec<ConfiguredHookSource>> {
    sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let number = index + 1;
            let format = match source.format {
                AgentHookFormat::Cursor => HookFormat::Cursor,
                AgentHookFormat::Unknown => {
                    bail!("Hook entry {number} uses a hook format this Warp version does not support")
                }
            };
            let kind = match source.level {
                AgentHookLevel::Agent => HookSourceKind::FactoryAgent,
                AgentHookLevel::Factory => HookSourceKind::FactoryWide,
                AgentHookLevel::Unknown => {
                    bail!("Hook entry {number} has a level this Warp version does not support")
                }
            };
            let on_ask = match source.on_ask {
                AgentHookOnAsk::Deny => HookOnAsk::Deny,
                AgentHookOnAsk::FailRun => HookOnAsk::FailRun,
                AgentHookOnAsk::Unknown => {
                    bail!("Hook entry {number} has an onAsk setting this Warp version does not support")
                }
            };
            let location = match source.repository {
                Some(repository) => ConfiguredHookLocation::Repository {
                    repo: SourceRepo::new(
                        forge(repository.forge)
                            .with_context(|| format!("Hook entry {number}"))?,
                        repository.owner,
                        repository.name,
                    ),
                    path: source.path,
                },
                None => ConfiguredHookLocation::Worker { path: source.path },
            };
            Ok(ConfiguredHookSource {
                format,
                location,
                kind,
                policy: HookSourcePolicy {
                    strict: source.strict,
                    on_ask,
                },
            })
        })
        .collect()
}

/// A hook directory inside a repository: relative, with no parent, root, or prefix components.
/// `.` is the repository root.
pub(crate) fn hook_repository_path(path: &str) -> anyhow::Result<PathBuf> {
    if path.is_empty() || path.trim() != path || path.contains('\0') {
        bail!("hook directory {path:?} is not an unambiguous relative path");
    }
    let path = PathBuf::from(path);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!(
            "hook directory {} must stay within its repository",
            path.display()
        );
    }
    Ok(path)
}

/// A hook directory on the worker outside any repository: an absolute path.
pub(crate) fn hook_worker_path(path: &str) -> anyhow::Result<PathBuf> {
    if path.trim() != path || path.contains('\0') {
        bail!("hook directory {path:?} is not an unambiguous absolute path");
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        bail!(
            "hook directory {} must be an absolute path on the worker",
            path.display()
        );
    }
    Ok(path)
}

pub(super) fn factory_definition_sha(
    branch: Option<FactoryBranchExecutionConfig>,
) -> anyhow::Result<Option<String>> {
    branch
        .map(|branch| {
            let sha = branch.source_sha;
            // The commit is inlined into a setup command, so only a full commit ID is accepted.
            if sha.len() != 40
                || !sha
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                bail!("Execution Factory branch commit is not a full commit SHA");
            }
            Ok(sha)
        })
        .transpose()
}

#[cfg(test)]
#[path = "execution_config_tests.rs"]
mod tests;
