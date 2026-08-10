use std::fs;
use std::path::Path;

use ai::plugins::{
    MANIFEST_SCHEMA_1_0_0, MCP_SCHEMA_1_0_0, PluginCandidate, PluginScopeId, PluginSourceId,
    PluginSourceKind, resolve_active_packages,
};
use serde_json::json;
use tempfile::{TempDir, tempdir};

use super::*;

/// Writes a plugin package with the given skills and MCP servers, and returns its candidate.
fn write_package(
    search_root: &Path,
    name: &str,
    skills: &[&str],
    mcp_servers: &[&str],
) -> PluginCandidate {
    let root = search_root.join(name);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("plugin.json"),
        json!({ "$schema": MANIFEST_SCHEMA_1_0_0, "name": name }).to_string(),
    )
    .unwrap();

    for skill in skills {
        let skill_dir = root.join("skills").join(skill);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {skill}\ndescription: The {skill} skill\n---\n\n# {skill}\n"),
        )
        .unwrap();
    }

    if !mcp_servers.is_empty() {
        let servers: serde_json::Map<String, serde_json::Value> = mcp_servers
            .iter()
            .map(|server| {
                (
                    (*server).to_owned(),
                    json!({ "type": "stdio", "command": "server" }),
                )
            })
            .collect();
        fs::write(
            root.join("mcp.json"),
            json!({ "$schema": MCP_SCHEMA_1_0_0, "mcpServers": servers }).to_string(),
        )
        .unwrap();
    }

    PluginCandidate {
        root,
        scope: PluginScopeId::Repository,
        source: PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/repo"),
    }
}

/// A registry with discovery on and one package loaded.
fn loaded_registry(skills: &[&str], mcp_servers: &[&str]) -> (TempDir, PluginRegistry) {
    let temp = tempdir().unwrap();
    let candidate = write_package(temp.path(), "acme-tools", skills, mcp_servers);
    let mut registry = PluginRegistry::new(true);
    let generation = registry.begin_scan();
    assert!(registry.apply_scan(generation, resolve_active_packages(vec![candidate])));
    (temp, registry)
}

#[test]
fn a_loaded_package_exposes_its_skills_and_servers() {
    let (_temp, registry) = loaded_registry(&["deploy"], &["github"]);
    assert_eq!(
        registry
            .active_skills()
            .iter()
            .map(|skill| skill.qualified_name())
            .collect::<Vec<_>>(),
        vec!["acme-tools:deploy"]
    );
    assert_eq!(
        registry
            .active_mcp_component_ids()
            .iter()
            .map(|id| id.qualified_name())
            .collect::<Vec<_>>(),
        vec!["acme-tools:github"]
    );
}

/// The Factory runtime ignores the personal interactive preference entirely.
#[test]
fn the_factory_policy_ignores_the_interactive_preference() {
    assert!(!PluginDiscoveryPolicy::InteractivePreference.is_enabled(false));
    assert!(PluginDiscoveryPolicy::InteractivePreference.is_enabled(true));
    assert!(PluginDiscoveryPolicy::RequiredByFactory.is_enabled(false));
    assert!(PluginDiscoveryPolicy::RequiredByFactory.is_enabled(true));
}

/// The ordering the product spec fixes: reject lookups, stop watchers, withdraw skills, cancel
/// in-flight calls, then unregister servers.
#[test]
fn disabling_discovery_produces_the_teardown_steps_in_order() {
    let (_temp, mut registry) = loaded_registry(&["deploy"], &["github", "registry"]);

    let transition = registry.set_enabled(false);
    assert!(!transition.rescan);

    let mut steps = transition.teardown.into_iter();
    assert_eq!(steps.next(), Some(PluginTeardownStep::StopWatchers));
    assert_eq!(steps.next(), Some(PluginTeardownStep::WithdrawSkills));
    assert_eq!(
        steps.next(),
        Some(PluginTeardownStep::CancelInFlightMcpCalls)
    );
    let Some(PluginTeardownStep::UnregisterMcpInstallations { components }) = steps.next() else {
        panic!("expected the MCP installations to be unregistered last");
    };
    assert_eq!(
        components
            .iter()
            .map(|id| id.qualified_name())
            .collect::<Vec<_>>(),
        vec!["acme-tools:github", "acme-tools:registry"],
        "every plugin-provenance installation must be named so it can be stopped"
    );
    assert!(steps.next().is_none());
}

/// The active set is already empty by the time the teardown steps are handed back, so a lookup
/// racing the teardown cannot resolve a component that is about to be stopped.
#[test]
fn lookups_are_rejected_before_the_teardown_is_handed_back() {
    let (_temp, mut registry) = loaded_registry(&["deploy"], &["github"]);
    assert!(registry.resolve_skill("acme-tools:deploy", &[]).is_ok());

    let transition = registry.set_enabled(false);

    assert!(!registry.is_enabled());
    assert!(registry.active_skills().is_empty());
    assert!(registry.active_mcp_component_ids().is_empty());
    assert!(
        !transition.teardown.is_empty(),
        "the teardown must still describe what to stop"
    );
}

/// An explicit reference to a withdrawn component fails with the specified diagnostic code.
#[test]
fn a_reference_while_disabled_reports_discovery_disabled() {
    let (_temp, mut registry) = loaded_registry(&["deploy"], &[]);
    registry.set_enabled(false);

    for name in ["acme-tools:deploy", "deploy"] {
        let diagnostic = registry.resolve_skill(name, &[]).unwrap_err();
        assert_eq!(diagnostic.code, PluginDiagnosticCode::DiscoveryDisabled);
    }
}

/// A scan that started before the toggle must not resurrect the packages the teardown removed.
#[test]
fn a_scan_in_flight_when_discovery_is_disabled_is_dropped() {
    let temp = tempdir().unwrap();
    let candidate = write_package(temp.path(), "acme-tools", &["deploy"], &[]);
    let mut registry = PluginRegistry::new(true);

    let stale_generation = registry.begin_scan();
    registry.set_enabled(false);
    registry.set_enabled(true);

    let applied = registry.apply_scan(stale_generation, resolve_active_packages(vec![candidate]));
    assert!(!applied, "a superseded generation must be dropped");
    assert!(registry.active_skills().is_empty());
}

/// A scan that lands while discovery is off is dropped even if its generation looks current.
#[test]
fn a_scan_applied_while_disabled_is_dropped() {
    let temp = tempdir().unwrap();
    let candidate = write_package(temp.path(), "acme-tools", &["deploy"], &[]);
    let mut registry = PluginRegistry::new(false);

    let generation = registry.begin_scan();
    assert!(!registry.apply_scan(generation, resolve_active_packages(vec![candidate])));
    assert!(registry.active_skills().is_empty());
}

/// Re-enabling asks for a complete rescan rather than restoring the previous snapshot.
#[test]
fn re_enabling_discovery_requires_a_fresh_rescan() {
    let (_temp, mut registry) = loaded_registry(&["deploy"], &["github"]);
    registry.set_enabled(false);

    let transition = registry.set_enabled(true);
    assert!(transition.rescan);
    assert!(transition.teardown.is_empty());
    assert!(
        registry.active_skills().is_empty(),
        "the stale snapshot must not be revived; only a rescan repopulates the set"
    );
}

#[test]
fn a_rescan_after_re_enabling_restores_the_components() {
    let temp = tempdir().unwrap();
    let candidate = write_package(temp.path(), "acme-tools", &["deploy"], &["github"]);
    let mut registry = PluginRegistry::new(true);
    let generation = registry.begin_scan();
    registry.apply_scan(generation, resolve_active_packages(vec![candidate.clone()]));

    registry.set_enabled(false);
    registry.set_enabled(true);

    let generation = registry.begin_scan();
    assert!(registry.apply_scan(generation, resolve_active_packages(vec![candidate])));
    assert_eq!(registry.active_skills().len(), 1);
    assert_eq!(registry.active_mcp_component_ids().len(), 1);
}

#[test]
fn toggling_to_the_same_state_is_a_no_op() {
    let (_temp, mut registry) = loaded_registry(&["deploy"], &[]);
    assert!(registry.set_enabled(true).is_noop());
    assert_eq!(registry.active_skills().len(), 1, "nothing was torn down");

    registry.set_enabled(false);
    assert!(registry.set_enabled(false).is_noop());
}

/// Disabling discovery must not touch the package on disk. Plugin data is likewise untouched:
/// the registry never creates or deletes it, so there is nothing for a teardown to remove.
#[test]
fn disabling_discovery_preserves_package_files() {
    let temp = tempdir().unwrap();
    let candidate = write_package(temp.path(), "acme-tools", &["deploy"], &["github"]);
    let manifest = candidate.root.join("plugin.json");
    let skill = candidate.root.join("skills/deploy/SKILL.md");
    let mcp = candidate.root.join("mcp.json");

    let mut registry = PluginRegistry::new(true);
    let generation = registry.begin_scan();
    registry.apply_scan(generation, resolve_active_packages(vec![candidate]));
    registry.set_enabled(false);

    assert!(manifest.is_file());
    assert!(skill.is_file());
    assert!(mcp.is_file());
}

#[test]
fn a_qualified_name_resolves_to_exactly_that_plugins_skill() {
    let (_temp, registry) = loaded_registry(&["deploy", "summarize"], &[]);
    let skill = registry.resolve_skill("acme-tools:deploy", &[]).unwrap();
    assert_eq!(skill.id.local_name, "deploy");

    let diagnostic = registry
        .resolve_skill("acme-tools:missing", &[])
        .unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::SkillInvalid);
    assert_eq!(diagnostic.component.as_deref(), Some("missing"));
}

#[test]
fn a_unique_unqualified_name_still_resolves() {
    let (_temp, registry) = loaded_registry(&["deploy"], &[]);
    let skill = registry.resolve_skill("deploy", &[]).unwrap();
    assert_eq!(skill.qualified_name(), "acme-tools:deploy");
}

/// A plugin never silently replaces a flat skill: the collision is an ambiguity that lists both
/// candidates.
#[test]
fn a_flat_skill_of_the_same_name_makes_the_unqualified_name_ambiguous() {
    let (_temp, registry) = loaded_registry(&["deploy"], &[]);

    let diagnostic = registry
        .resolve_skill("deploy", &["deploy".to_owned()])
        .unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ComponentAmbiguous);
    assert!(diagnostic.reason.contains("acme-tools:deploy"));

    // The qualified form still works, which is the way out of the ambiguity.
    assert!(
        registry
            .resolve_skill("acme-tools:deploy", &["deploy".to_owned()])
            .is_ok()
    );
}

#[test]
fn two_plugins_providing_the_same_skill_name_are_ambiguous() {
    let temp = tempdir().unwrap();
    let first = write_package(temp.path(), "acme-tools", &["deploy"], &[]);
    let second = write_package(temp.path(), "other-tools", &["deploy"], &[]);

    let mut registry = PluginRegistry::new(true);
    let generation = registry.begin_scan();
    registry.apply_scan(generation, resolve_active_packages(vec![first, second]));

    let diagnostic = registry.resolve_skill("deploy", &[]).unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ComponentAmbiguous);
    assert!(diagnostic.reason.contains("acme-tools:deploy"));
    assert!(diagnostic.reason.contains("other-tools:deploy"));

    assert!(registry.resolve_skill("other-tools:deploy", &[]).is_ok());
}

#[test]
fn an_unknown_name_is_reported_as_missing_rather_than_ambiguous() {
    let (_temp, registry) = loaded_registry(&["deploy"], &[]);
    let diagnostic = registry.resolve_skill("nonexistent", &[]).unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::SkillInvalid);
}

/// Every teardown step except stopping the watchers has an event the rest of the client acts on.
#[test]
fn teardown_steps_map_onto_client_events() {
    use crate::ai::plugins::plugin_manager::{PluginManagerEvent, teardown_event};

    assert!(teardown_event(PluginTeardownStep::StopWatchers).is_none());
    assert!(matches!(
        teardown_event(PluginTeardownStep::WithdrawSkills),
        Some(PluginManagerEvent::WithdrawSkills)
    ));
    assert!(matches!(
        teardown_event(PluginTeardownStep::CancelInFlightMcpCalls),
        Some(PluginManagerEvent::CancelInFlightMcpCalls)
    ));
    assert!(matches!(
        teardown_event(PluginTeardownStep::UnregisterMcpInstallations { components: vec![] }),
        Some(PluginManagerEvent::UnregisterMcpInstallations { .. })
    ));
}
