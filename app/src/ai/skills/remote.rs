use remote_server::proto::{BundledSkillMetadata, RemoteSkillProto, remote_skill_proto};

use super::bundled::{BundledSkill, BundledSkillActivation};
use crate::ai::mcp::McpIntegration;

/// Stable wire identifier for an MCP integration in [`BundledSkillMetadata`].
fn mcp_integration_wire_id(integration: McpIntegration) -> &'static str {
    match integration {
        McpIntegration::Figma => "figma",
    }
}

/// Serializes a daemon-side bundled catalog for the aggregate remote Agent Mode snapshot.
///
/// `RequiresFile` activations are evaluated here — the daemon owns the
/// files — so the client only ever receives `Always` or `RequiresMcp`
/// conditions. The result is sorted by skill path so pushes are
/// deterministic across daemon restarts.
pub(crate) fn bundled_skill_snapshot_protos(catalog: &BundledSkill) -> Vec<RemoteSkillProto> {
    let mut protos: Vec<RemoteSkillProto> = catalog
        .iter_definitions()
        .filter_map(|(id, skill, activation)| {
            let requires_mcp = match activation {
                BundledSkillActivation::Always => None,
                BundledSkillActivation::RequiresMcp(integration) => {
                    Some(mcp_integration_wire_id(*integration).to_owned())
                }
                BundledSkillActivation::RequiresFeature(feature) => {
                    if !feature.is_enabled() {
                        return None;
                    }
                    None
                }
                BundledSkillActivation::RequiresFile(path) => {
                    if !path.exists() {
                        return None;
                    }
                    None
                }
            };
            Some(RemoteSkillProto {
                path: skill.path.display_path(),
                content: skill.content.clone(),
                source: Some(remote_skill_proto::Source::Bundled(BundledSkillMetadata {
                    id: id.to_owned(),
                    requires_mcp,
                })),
            })
        })
        .collect();
    protos.sort_by(|a, b| a.path.cmp(&b.path));
    protos
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
