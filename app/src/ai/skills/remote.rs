use ai::skills::{parse_skill_content_at_location, SkillProvider, SkillScope};
use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use remote_server::proto::BundledSkillProto;
use warp_core::features::FeatureFlag;
use warp_core::safe_warn;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::bundled::{BundledSkill, BundledSkillActivation};
use super::SkillManager;
use crate::ai::mcp::McpIntegration;

pub(crate) fn wire_remote_bundled_skills(ctx: &mut AppContext) {
    SkillManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.subscribe_to_remote_bundled_skills(ctx);
    });
}

impl SkillManager {
    fn subscribe_to_remote_bundled_skills(&mut self, ctx: &mut ModelContext<Self>) {
        let remote_server_manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&remote_server_manager, |me, event, _ctx| match event {
            RemoteServerManagerEvent::BundledSkillsSnapshot { host_id, skills } => {
                if !FeatureFlag::BundledSkills.is_enabled() {
                    return;
                }
                // A fresh snapshot replaces any previous catalog for the
                // host (e.g. after a reconnect following a daemon upgrade).
                me.set_remote_bundled_skill(
                    host_id.clone(),
                    bundled_skill_from_protos(host_id, skills),
                );
            }
            RemoteServerManagerEvent::HostDisconnected { host_id } => {
                me.remove_remote_bundled_skill(host_id);
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionConnected { .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionReconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::NavigatedToDirectory { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::CommitChainResponse { .. }
            | RemoteServerManagerEvent::GitPushResponse { .. }
            | RemoteServerManagerEvent::CreatePrResponse { .. }
            | RemoteServerManagerEvent::GenerateCommitMessageResponse { .. }
            | RemoteServerManagerEvent::GetPrInfoResponse { .. }
            | RemoteServerManagerEvent::GetCommittedBranchFilesResponse { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        });
    }
}

/// Stable wire identifier for an MCP integration in [`BundledSkillProto`].
fn mcp_integration_wire_id(integration: McpIntegration) -> &'static str {
    match integration {
        McpIntegration::Figma => "figma",
    }
}

fn mcp_integration_from_wire_id(wire_id: &str) -> Option<McpIntegration> {
    match wire_id {
        "figma" => Some(McpIntegration::Figma),
        _ => None,
    }
}

/// Converts a daemon-pushed snapshot into a catalog whose skill paths are
/// remote paths on `host_id`.
fn bundled_skill_from_protos(host_id: &HostId, skills: &[BundledSkillProto]) -> BundledSkill {
    let definitions = skills.iter().filter_map(|proto| {
        let path = match StandardizedPath::try_new(&proto.path) {
            Ok(path) => LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), path)),
            Err(_) => {
                safe_warn!(
                    safe: ("Skipping bundled skill with an invalid remote path"),
                    full: ("Skipping bundled skill {} with an invalid remote path: {}", proto.id, proto.path)
                );
                return None;
            }
        };
        // Re-parse the daemon-rendered content so name, description, and
        // line range are derived exactly as they are for local skills.
        let skill = match parse_skill_content_at_location(
            path,
            &proto.content,
            SkillProvider::Warp,
            SkillScope::Bundled,
        ) {
            Ok(skill) => skill,
            Err(err) => {
                safe_warn!(
                    safe: ("Skipping bundled skill that failed to parse"),
                    full: ("Skipping bundled skill {} that failed to parse: {err:#}", proto.id)
                );
                return None;
            }
        };
        let activation = match proto.requires_mcp.as_deref() {
            None => BundledSkillActivation::Always,
            Some(wire_id) => match mcp_integration_from_wire_id(wire_id) {
                Some(integration) => BundledSkillActivation::RequiresMcp(integration),
                None => {
                    // Unknown integration (e.g. a newer daemon): the client
                    // cannot evaluate the condition, so skip the skill.
                    safe_warn!(
                        safe: ("Skipping bundled skill with an unknown MCP integration"),
                        full: ("Skipping bundled skill {} with an unknown MCP integration: {wire_id}", proto.id)
                    );
                    return None;
                }
            },
        };
        Some((proto.id.clone(), skill, activation))
    });
    BundledSkill::from_definitions(definitions)
}

/// Serializes a daemon-side catalog for the `BundledSkillsSnapshot` push.
///
/// `RequiresFile` activations are evaluated here — the daemon owns the
/// files — so the client only ever receives `Always` or `RequiresMcp`
/// conditions.
pub(crate) fn bundled_skills_snapshot_protos(catalog: &BundledSkill) -> Vec<BundledSkillProto> {
    catalog
        .iter_definitions()
        .filter_map(|(id, skill, activation)| {
            let requires_mcp = match activation {
                BundledSkillActivation::Always => None,
                BundledSkillActivation::RequiresMcp(integration) => {
                    Some(mcp_integration_wire_id(*integration).to_owned())
                }
                BundledSkillActivation::RequiresFile(path) => {
                    if !path.exists() {
                        return None;
                    }
                    None
                }
            };
            Some(BundledSkillProto {
                id: id.to_owned(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                path: skill.path.display_path(),
                content: skill.content.clone(),
                requires_mcp,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
