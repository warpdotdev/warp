use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::{TempDir, tempdir};

use super::*;
use crate::plugins::identity::{PluginScopeId, PluginSourceId, PluginSourceKind};
use crate::plugins::manifest::MANIFEST_SCHEMA_1_0_0;
use crate::plugins::mcp::{MCP_SCHEMA_1_0_0, PluginMcpTransport};

/// Builds a plugin package on disk. Every helper writes files only — nothing here can execute
/// package code, which is the property the loader itself must preserve.
struct PackageBuilder {
    temp: TempDir,
    root: std::path::PathBuf,
}

impl PackageBuilder {
    fn new(directory_name: &str) -> Self {
        let temp = tempdir().unwrap();
        let root = temp.path().join(directory_name);
        fs::create_dir_all(&root).unwrap();
        Self { temp, root }
    }

    fn manifest(self, value: serde_json::Value) -> Self {
        fs::write(self.root.join("plugin.json"), value.to_string()).unwrap();
        self
    }

    fn raw_manifest(self, content: &str) -> Self {
        fs::write(self.root.join("plugin.json"), content).unwrap();
        self
    }

    fn skill(self, directory: &str, content: &str) -> Self {
        let skill_dir = self.root.join("skills").join(directory);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        self
    }

    fn file(self, relative: &str, content: &str) -> Self {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
        self
    }

    fn mcp(self, value: serde_json::Value) -> Self {
        fs::write(self.root.join("mcp.json"), value.to_string()).unwrap();
        self
    }

    fn load(&self) -> Result<PluginPackage, PluginDiagnostic> {
        load_plugin_package(
            &self.root,
            PluginScopeId::Repository,
            PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/repo"),
        )
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn temp_path(&self) -> &Path {
        self.temp.path()
    }
}

fn valid_manifest(name: &str) -> serde_json::Value {
    json!({ "$schema": MANIFEST_SCHEMA_1_0_0, "name": name })
}

fn skill_markdown(name: &str) -> String {
    format!("---\nname: {name}\ndescription: A {name} skill\n---\n\n# {name}\n")
}

#[test]
fn a_package_with_both_component_types_loads() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"))
        .skill("summarize", &skill_markdown("summarize"))
        .mcp(json!({
            "$schema": MCP_SCHEMA_1_0_0,
            "mcpServers": { "github": { "type": "stdio", "command": "server" } },
        }))
        .load()
        .unwrap();

    assert_eq!(package.name(), "acme-tools");
    let skills: Vec<String> = package
        .skills
        .iter()
        .map(PluginSkillComponent::qualified_name)
        .collect();
    assert_eq!(skills, vec!["acme-tools:deploy", "acme-tools:summarize"]);
    assert_eq!(package.mcp_servers.len(), 1);
    assert_eq!(
        package.mcp_component_id("github").qualified_name(),
        "acme-tools:github"
    );
    assert!(package.diagnostics.is_empty());
}

/// §6.2: a missing `skills/` directory or `mcp.json` is not an error.
#[test]
fn missing_component_locations_are_not_errors() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .load()
        .unwrap();
    assert!(package.skills.is_empty());
    assert!(package.mcp_servers.is_empty());
    assert!(package.diagnostics.is_empty());
}

/// §22: the source frontmatter `name` is preserved even though the runtime name is qualified.
#[test]
fn the_portable_skill_name_is_preserved_alongside_the_qualified_name() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"))
        .load()
        .unwrap();

    let skill = &package.skills[0];
    assert_eq!(skill.skill.name, "deploy");
    assert_eq!(skill.qualified_name(), "acme-tools:deploy");
}

/// §7.1: only immediate children of `skills/` are skills; deeper descendants are not searched.
#[test]
fn skill_discovery_is_not_recursive() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"))
        .file(
            "skills/deploy/nested/SKILL.md",
            &skill_markdown("nested-skill"),
        )
        .load()
        .unwrap();

    assert_eq!(package.skills.len(), 1);
    assert_eq!(package.skills[0].id.local_name, "deploy");
}

#[test]
fn a_skill_directory_without_a_skill_file_is_skipped_silently() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"))
        .file("skills/notes/README.md", "not a skill")
        .load()
        .unwrap();

    assert_eq!(package.skills.len(), 1);
    assert!(package.diagnostics.is_empty());
}

/// §6.2: a fixed component location of the wrong filesystem kind invalidates only that component
/// type.
#[test]
fn a_skills_path_that_is_not_a_directory_disables_only_skills() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .file("skills", "not a directory")
        .mcp(json!({
            "$schema": MCP_SCHEMA_1_0_0,
            "mcpServers": { "github": { "type": "stdio", "command": "server" } },
        }))
        .load()
        .unwrap();

    assert!(package.skills.is_empty());
    assert_eq!(package.mcp_servers.len(), 1, "MCP must still load");
    assert_eq!(
        package.diagnostics[0].code,
        PluginDiagnosticCode::ComponentWrongFilesystemKind
    );
}

#[test]
fn an_mcp_path_that_is_not_a_regular_file_disables_only_mcp() {
    let builder = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"));
    fs::create_dir_all(builder.path().join("mcp.json")).unwrap();

    let package = builder.load().unwrap();
    assert_eq!(package.skills.len(), 1, "skills must still load");
    assert!(package.mcp_servers.is_empty());
    assert_eq!(
        package.diagnostics[0].code,
        PluginDiagnosticCode::ComponentWrongFilesystemKind
    );
}

/// §11.3: a malformed `mcp.json` disables MCP for the plugin without touching its skills.
#[test]
fn a_malformed_mcp_file_leaves_skills_loaded() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("deploy", &skill_markdown("deploy"))
        .file("mcp.json", "{ not json")
        .load()
        .unwrap();

    assert_eq!(package.skills.len(), 1);
    assert!(package.mcp_servers.is_empty());
    assert_eq!(
        package.diagnostics[0].code,
        PluginDiagnosticCode::McpInvalidJson
    );
    assert_eq!(package.diagnostics[0].plugin.as_deref(), Some("acme-tools"));
}

/// §11.3(2): any fatal manifest violation rejects the package, so no component is discovered.
#[test]
fn a_rejected_manifest_discovers_no_components() {
    let builder = PackageBuilder::new("acme")
        .raw_manifest("{ not json")
        .skill("deploy", &skill_markdown("deploy"));

    let diagnostic = builder.load().unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ManifestInvalidJson);
    // The directory name is the only identity available before the manifest parses.
    assert_eq!(diagnostic.plugin.as_deref(), Some("acme"));
}

#[test]
fn a_directory_without_a_manifest_is_not_a_plugin() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("not-a-plugin");
    fs::create_dir_all(&root).unwrap();

    let diagnostic = load_plugin_package(
        &root,
        PluginScopeId::User,
        PluginSourceId::new(PluginSourceKind::AgentsDirectory, "/home/.agents"),
    )
    .unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ManifestMissing);
}

/// The package name comes from the manifest, never from the directory it happens to sit in.
#[test]
fn the_manifest_name_wins_over_the_directory_name() {
    let package = PackageBuilder::new("some-directory")
        .manifest(valid_manifest("acme-tools"))
        .load()
        .unwrap();
    assert_eq!(package.name(), "acme-tools");
    assert_eq!(package.instance.manifest_name, "acme-tools");
}

/// §4.1(3): a `SKILL.md` that resolves outside the plugin root is skipped, and the rest of the
/// package still loads.
#[cfg(unix)]
#[test]
fn a_skill_symlinked_out_of_the_plugin_root_is_skipped() {
    let builder = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .skill("legit", &skill_markdown("legit"));

    let outside = builder.temp_path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), skill_markdown("stolen")).unwrap();
    let escaping = builder.path().join("skills").join("escaping");
    fs::create_dir_all(&escaping).unwrap();
    std::os::unix::fs::symlink(outside.join("SKILL.md"), escaping.join("SKILL.md")).unwrap();

    let package = builder.load().unwrap();
    let names: Vec<&str> = package
        .skills
        .iter()
        .map(|skill| skill.id.local_name.as_str())
        .collect();
    assert_eq!(names, vec!["legit"]);
    assert_eq!(
        package.diagnostics[0].code,
        PluginDiagnosticCode::PathEscapesPluginRoot
    );
    assert_eq!(
        package.diagnostics[0].component.as_deref(),
        Some("escaping")
    );
}

/// A non-fatal manifest diagnostic is attributed to the manifest name once it is known.
#[test]
fn non_fatal_manifest_diagnostics_carry_the_plugin_name_and_path() {
    let builder = PackageBuilder::new("acme").manifest(json!({
        "$schema": MANIFEST_SCHEMA_1_0_0,
        "name": "acme-tools",
        "hooks": {},
    }));

    let package = builder.load().unwrap();
    let diagnostic = &package.diagnostics[0];
    assert_eq!(diagnostic.code, PluginDiagnosticCode::ManifestUnknownField);
    assert_eq!(diagnostic.plugin.as_deref(), Some("acme-tools"));
    assert!(diagnostic.path.as_ref().unwrap().ends_with("plugin.json"));
}

/// The parsed configuration is inert: a stdio entry becomes data, never a running process.
#[test]
fn loading_a_stdio_server_produces_configuration_only() {
    let package = PackageBuilder::new("acme")
        .manifest(valid_manifest("acme-tools"))
        .mcp(json!({
            "$schema": MCP_SCHEMA_1_0_0,
            "mcpServers": {
                "validator": {
                    "type": "stdio",
                    "command": "./bin/validator",
                    "args": ["${PLUGIN_DATA}/db"],
                },
            },
        }))
        .load()
        .unwrap();

    let PluginMcpTransport::Stdio { command, args, .. } = &package.mcp_servers[0].transport else {
        panic!("expected a stdio transport");
    };
    assert_eq!(command, "./bin/validator");
    // Placeholders are still literal here: expansion happens only when a launch is planned.
    assert_eq!(args, &["${PLUGIN_DATA}/db".to_owned()]);
}
