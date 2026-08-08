//! Loading one plugin package from a directory.
//!
//! Loading reads files and nothing else. Discovering a stdio MCP server produces a validated
//! configuration, never a process; a skill that ships a script is content, and running that
//! script stays on the ordinary shell-command path with its existing permissions.
//!
//! Failure isolation follows Agent Plugins §11.3: a rejected manifest takes the whole package
//! down, an unusable `skills/` or `mcp.json` takes down only that component type, and a bad skill
//! or MCP entry takes down only itself.
use std::fs;
use std::path::{Path, PathBuf};

use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::identity::{PluginComponentId, PluginComponentKind, PluginInstanceId, PluginSourceKind};
use super::manifest::{MANIFEST_FILE_NAME, PluginManifest, parse_manifest};
use super::mcp::{MCP_FILE_NAME, PluginMcpServer, parse_plugin_mcp};
use super::paths::verify_contained;
use crate::skills::{ParsedSkill, SkillProvider, SkillScope, parse_skill_content_at_location};

/// The fixed skills location inside a plugin root.
pub const SKILLS_DIR_NAME: &str = "skills";

/// The file that marks a directory under `skills/` as one skill.
pub const SKILL_FILE_NAME: &str = "SKILL.md";

/// One skill provided by a plugin.
#[derive(Debug, Clone)]
pub struct PluginSkillComponent {
    pub id: PluginComponentId,
    /// The skill parsed by the shared Agent Skills parser. Its frontmatter `name` is preserved.
    pub skill: ParsedSkill,
    pub skill_file: PathBuf,
}

impl PluginSkillComponent {
    /// The `<plugin>:<skill>` name used for explicit invocation and in the model's catalog.
    pub fn qualified_name(&self) -> String {
        self.id.qualified_name()
    }
}

/// A loaded plugin package.
#[derive(Debug, Clone)]
pub struct PluginPackage {
    pub instance: PluginInstanceId,
    /// The filesystem-resolved plugin root, used as `PLUGIN_ROOT` and as the containment root.
    pub root: PathBuf,
    pub manifest: PluginManifest,
    pub skills: Vec<PluginSkillComponent>,
    pub mcp_servers: Vec<PluginMcpServer>,
    /// Non-fatal problems found while loading. Present even for a fully valid package.
    pub diagnostics: Vec<PluginDiagnostic>,
}

impl PluginPackage {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// The component id for one of this package's MCP servers.
    pub fn mcp_component_id(&self, server_name: &str) -> PluginComponentId {
        PluginComponentId::new(
            self.instance.clone(),
            PluginComponentKind::McpServer,
            server_name,
        )
    }
}

/// Loads the package rooted at `candidate_root`.
///
/// `scope` and `source` supply the identity the caller derived from the search root; the plugin's
/// own name comes from its manifest, never from the directory name, so shadowing cannot be
/// influenced by how a directory happens to be spelled.
///
/// `Err` rejects the package: no component of it may be discovered or executed.
pub fn load_plugin_package(
    candidate_root: &Path,
    scope: super::identity::PluginScopeId,
    source: super::identity::PluginSourceId,
) -> Result<PluginPackage, PluginDiagnostic> {
    let directory_name = candidate_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Resolve the root first: everything after this is checked against the resolved path, so a
    // symlinked package directory is followed once and consistently.
    let root = dunce::canonicalize(candidate_root).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestMissing,
            format!("plugin root could not be resolved: {error}"),
        )
        .with_plugin(&directory_name)
        .with_path(candidate_root)
    })?;

    let manifest_path = root.join(MANIFEST_FILE_NAME);
    let resolved_manifest = verify_contained(&root, &manifest_path).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::PathEscapesPluginRoot,
            format!("{MANIFEST_FILE_NAME} does not resolve inside the plugin root: {error}"),
        )
        .with_plugin(&directory_name)
        .with_path(&manifest_path)
    })?;
    if !resolved_manifest.is_file() {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestMissing,
            format!("no regular {MANIFEST_FILE_NAME} at the plugin root"),
        )
        .with_plugin(&directory_name)
        .with_path(&resolved_manifest));
    }

    let content = fs::read_to_string(&resolved_manifest).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::ManifestUnreadable,
            format!("{MANIFEST_FILE_NAME} could not be read: {error}"),
        )
        .with_plugin(&directory_name)
        .with_path(&resolved_manifest)
    })?;
    let parsed = parse_manifest(&content).map_err(|diagnostic| {
        diagnostic
            .with_plugin(&directory_name)
            .with_path(&resolved_manifest)
    })?;

    let manifest = parsed.manifest;
    let instance = PluginInstanceId::new(scope, source, manifest.name.clone());
    let mut diagnostics: Vec<PluginDiagnostic> = parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| {
            diagnostic
                .with_plugin(&manifest.name)
                .with_path(&resolved_manifest)
        })
        .collect();

    let skills = load_skills(&root, &instance, &mut diagnostics);
    let mcp_servers = load_mcp_servers(&root, &manifest, &mut diagnostics);

    Ok(PluginPackage {
        instance,
        root,
        manifest,
        skills,
        mcp_servers,
        diagnostics,
    })
}

/// Scans `skills/` for immediate child directories containing a regular `SKILL.md`.
fn load_skills(
    root: &Path,
    instance: &PluginInstanceId,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<PluginSkillComponent> {
    let skills_dir = root.join(SKILLS_DIR_NAME);
    if !skills_dir.exists() {
        return Vec::new();
    }
    let resolved_skills_dir = match verify_contained(root, &skills_dir) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::PathEscapesPluginRoot,
                    format!("'{SKILLS_DIR_NAME}' does not resolve inside the plugin root: {error}"),
                )
                .with_plugin(&instance.manifest_name)
                .with_path(&skills_dir),
            );
            return Vec::new();
        }
    };
    if !resolved_skills_dir.is_dir() {
        diagnostics.push(
            PluginDiagnostic::new(
                PluginDiagnosticCode::ComponentWrongFilesystemKind,
                format!("'{SKILLS_DIR_NAME}' exists but is not a directory, so no skills load"),
            )
            .with_plugin(&instance.manifest_name)
            .with_path(&resolved_skills_dir),
        );
        return Vec::new();
    }

    let entries = match fs::read_dir(&resolved_skills_dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::ComponentWrongFilesystemKind,
                    format!("'{SKILLS_DIR_NAME}' could not be read: {error}"),
                )
                .with_plugin(&instance.manifest_name)
                .with_path(&resolved_skills_dir),
            );
            return Vec::new();
        }
    };

    let mut skills = Vec::new();
    let mut skill_dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Directory order is not defined by the filesystem, and both the model catalog and the
    // diagnostics read better when it is stable.
    skill_dirs.sort();

    for skill_dir in skill_dirs {
        let local_name = skill_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let skill_file = skill_dir.join(SKILL_FILE_NAME);
        if !skill_file.exists() {
            continue;
        }
        match load_skill(root, instance, &local_name, &skill_file) {
            Ok(component) => skills.push(component),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    skills
}

fn load_skill(
    root: &Path,
    instance: &PluginInstanceId,
    local_name: &str,
    skill_file: &Path,
) -> Result<PluginSkillComponent, PluginDiagnostic> {
    let skill_invalid = |code, reason: String| {
        PluginDiagnostic::new(code, reason)
            .with_plugin(&instance.manifest_name)
            .with_component(local_name)
            .with_path(skill_file)
    };

    let resolved = verify_contained(root, skill_file).map_err(|error| {
        skill_invalid(
            PluginDiagnosticCode::PathEscapesPluginRoot,
            format!("{SKILL_FILE_NAME} does not resolve inside the plugin root: {error}"),
        )
    })?;
    if !resolved.is_file() {
        return Err(skill_invalid(
            PluginDiagnosticCode::SkillInvalid,
            format!("{SKILL_FILE_NAME} is not a regular file"),
        ));
    }
    let content = fs::read_to_string(&resolved).map_err(|error| {
        skill_invalid(
            PluginDiagnosticCode::SkillInvalid,
            format!("{SKILL_FILE_NAME} could not be read: {error}"),
        )
    })?;

    // Provider and scope are supplied explicitly rather than derived from the path: a plugin
    // skill does not live under a flat provider directory, so path-based classification would
    // silently mislabel it.
    let skill = parse_skill_content_at_location(
        LocalOrRemotePath::Local(resolved.clone()),
        &content,
        skill_provider_for(instance),
        skill_scope_for(instance),
    )
    .map_err(|error| {
        skill_invalid(
            PluginDiagnosticCode::SkillInvalid,
            format!("{SKILL_FILE_NAME} is not a valid Agent Skill: {error}"),
        )
    })?;

    Ok(PluginSkillComponent {
        id: PluginComponentId::new(instance.clone(), PluginComponentKind::Skill, local_name),
        skill,
        skill_file: resolved,
    })
}

fn skill_provider_for(instance: &PluginInstanceId) -> SkillProvider {
    match instance.source.kind {
        PluginSourceKind::AgentsDirectory => SkillProvider::Agents,
        PluginSourceKind::WarpDirectory | PluginSourceKind::FactoryRepository => {
            SkillProvider::Warp
        }
    }
}

fn skill_scope_for(instance: &PluginInstanceId) -> SkillScope {
    match instance.scope {
        super::identity::PluginScopeId::User => SkillScope::Home,
        _ => SkillScope::Project,
    }
}

/// Reads root `mcp.json`, if present. A failure here disables MCP for the plugin only.
fn load_mcp_servers(
    root: &Path,
    manifest: &PluginManifest,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<PluginMcpServer> {
    let mcp_path = root.join(MCP_FILE_NAME);
    if !mcp_path.exists() {
        return Vec::new();
    }
    let mcp_invalid = |code, reason: String, path: &Path| {
        PluginDiagnostic::new(code, reason)
            .with_plugin(&manifest.name)
            .with_path(path)
    };

    let resolved = match verify_contained(root, &mcp_path) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(mcp_invalid(
                PluginDiagnosticCode::PathEscapesPluginRoot,
                format!("{MCP_FILE_NAME} does not resolve inside the plugin root: {error}"),
                &mcp_path,
            ));
            return Vec::new();
        }
    };
    if !resolved.is_file() {
        diagnostics.push(mcp_invalid(
            PluginDiagnosticCode::ComponentWrongFilesystemKind,
            format!("{MCP_FILE_NAME} exists but is not a regular file, so MCP is disabled"),
            &resolved,
        ));
        return Vec::new();
    }
    let content = match fs::read_to_string(&resolved) {
        Ok(content) => content,
        Err(error) => {
            diagnostics.push(mcp_invalid(
                PluginDiagnosticCode::McpInvalidJson,
                format!("{MCP_FILE_NAME} could not be read: {error}"),
                &resolved,
            ));
            return Vec::new();
        }
    };

    match parse_plugin_mcp(&content, &manifest.agent_plugins_version) {
        Ok(parsed) => {
            diagnostics.extend(
                parsed
                    .diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.or_plugin(&manifest.name).with_path(&resolved)),
            );
            parsed.servers
        }
        Err(diagnostic) => {
            diagnostics.push(diagnostic.or_plugin(&manifest.name).with_path(&resolved));
            Vec::new()
        }
    }
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
