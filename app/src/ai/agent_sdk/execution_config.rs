use std::path::{Component, PathBuf};

use anyhow::{Context as _, bail};
use cloud_object_models::CodeForge as RuntimeForge;
use serde_json::{Map, Value};
use warp_cli::agent::{Harness, RepositoryHeadRef};
use warp_cli::mcp::MCPSpec;
use warp_cli::share::{ShareAccessLevel, ShareRequest, ShareSubject};
use warp_graphql::ai::AgentHarness;
use warp_graphql::queries::execution_config::{
    CloudProviderClientConfigs, CodeForge, ExecutionRepository, ExecutionRepositoryRefType,
    SessionSharingAccessLevel, SessionSharingAclSpec, SessionSharingSubjectType,
    SourceRepo as ConfigSourceRepo,
};

use super::driver::environment::ResolvedRepository;
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
            })
        })
        .collect()
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

#[cfg(test)]
#[path = "execution_config_tests.rs"]
mod tests;
