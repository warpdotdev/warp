use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ai::skills::{parse_bundled_skill, ParsedSkill, SkillPathOrigin, SkillReference};
use futures::TryStreamExt;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::safe_warn;
use warp_core::ui::icons::Icon;
use warp_errors::report_error;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warpui::{AppContext, SingletonEntity};

use super::SkillDescriptor;
use crate::ai::mcp::{McpIntegration, TemplatableMCPServerManager};
use crate::keyboard::keybinding_file_path;
use crate::settings::user_preferences_toml_file_path;

/// Activation condition for a bundled skill.
#[derive(Debug, Clone)]
pub enum BundledSkillActivation {
    /// Always active.
    Always,
    /// Active only when a specific Warp feature is enabled.
    RequiresFeature(FeatureFlag),
    /// Active only when a specific MCP server is running.
    RequiresMcp(McpIntegration),
    /// Active only when a specific file exists on disk.
    RequiresFile(PathBuf),
}

impl BundledSkillActivation {
    pub fn is_enabled(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Always => true,
            Self::RequiresFeature(feature) => feature.is_enabled(),
            Self::RequiresMcp(integration) => {
                TemplatableMCPServerManager::as_ref(ctx).is_mcp_server_running(*integration)
            }
            Self::RequiresFile(path) => path.exists(),
        }
    }
}

/// Catalogs of bundled skills for the local host and connected remote hosts.
#[derive(Debug, Default)]
pub struct BundledSkills {
    local: BundledSkill,
    remote_by_host: HashMap<HostId, BundledSkill>,
}

impl BundledSkills {
    pub fn set_local(&mut self, bundled_skill: BundledSkill) {
        self.local = bundled_skill;
    }

    pub fn active_descriptors(
        &self,
        path_origin: &SkillPathOrigin,
        ctx: &AppContext,
    ) -> Vec<SkillDescriptor> {
        match path_origin {
            SkillPathOrigin::Local | SkillPathOrigin::RestoredDisplayOnly => {
                self.local.active_descriptors(ctx)
            }
            SkillPathOrigin::Remote { host_id } => self
                .remote(host_id)
                .map(|bundled_skill| bundled_skill.active_path_referenced_descriptors(ctx))
                .unwrap_or_default(),
            SkillPathOrigin::Unavailable => Vec::new(),
        }
    }

    pub fn reference_for_path(&self, path: &LocalOrRemotePath) -> Option<SkillReference> {
        self.local.reference_for_path(path)
    }

    pub fn local_skill(&self, id: &str) -> Option<&ParsedSkill> {
        self.local.skill(id)
    }

    pub fn active_skill(
        &self,
        id: &str,
        path_origin: &SkillPathOrigin,
        ctx: &AppContext,
    ) -> Option<&ParsedSkill> {
        self.for_path_origin(path_origin)?.active_skill(id, ctx)
    }

    /// Installs the catalog for a connected remote host, replacing any
    /// previous catalog from an earlier connection.
    pub fn insert_remote(&mut self, host_id: HostId, bundled_skill: BundledSkill) {
        self.remote_by_host.insert(host_id, bundled_skill);
    }

    /// Removes all catalog state for a disconnected remote host.
    pub fn remove_remote(&mut self, host_id: &HostId) {
        self.remote_by_host.remove(host_id);
    }

    /// Returns the catalog for a connected remote host.
    pub fn remote(&self, host_id: &HostId) -> Option<&BundledSkill> {
        self.remote_by_host.get(host_id)
    }

    /// Returns the remote catalog skill matching `path`, looked up in the
    /// catalog of the host that owns the path. Remote bundled skills are
    /// addressed by path (their paths are real files on the remote host),
    /// unlike local bundled skills which are addressed by
    /// [`SkillReference::BundledSkillId`].
    pub fn remote_skill_by_path(&self, path: &RemotePath) -> Option<&ParsedSkill> {
        self.remote_by_host
            .get(&path.host_id)?
            .skill_by_path(&LocalOrRemotePath::Remote(path.clone()))
    }

    /// Like [`Self::remote_skill_by_path`], but only returns the skill when
    /// its activation condition is met.
    pub fn remote_active_skill_by_path(
        &self,
        path: &RemotePath,
        ctx: &AppContext,
    ) -> Option<&ParsedSkill> {
        self.remote_by_host
            .get(&path.host_id)?
            .active_skill_by_path(&LocalOrRemotePath::Remote(path.clone()), ctx)
    }

    /// Returns the bundled catalog selected by the execution path origin.
    fn for_path_origin(&self, path_origin: &SkillPathOrigin) -> Option<&BundledSkill> {
        match path_origin {
            SkillPathOrigin::Local | SkillPathOrigin::RestoredDisplayOnly => Some(&self.local),
            SkillPathOrigin::Remote { host_id } => self.remote(host_id),
            SkillPathOrigin::Unavailable => None,
        }
    }

    #[cfg(test)]
    pub fn insert_local_for_testing(
        &mut self,
        id: impl Into<String>,
        skill: ParsedSkill,
        activation: BundledSkillActivation,
    ) {
        self.local.insert_for_testing(id, skill, activation);
    }

    #[cfg(test)]
    pub fn insert_remote_for_testing(
        &mut self,
        host_id: HostId,
        id: impl Into<String>,
        skill: ParsedSkill,
        activation: BundledSkillActivation,
    ) {
        self.remote_by_host
            .entry(host_id)
            .or_default()
            .insert_for_testing(id, skill, activation);
    }
}

/// One bundled skill definition with its activation condition and icon.
#[derive(Debug, Clone)]
struct BundledSkillDefinition {
    skill: ParsedSkill,
    activation: BundledSkillActivation,
    icon: Icon,
}

/// Skills bundled with Warp for a single host.
#[derive(Debug, Default)]
pub struct BundledSkill {
    definitions: HashMap<String, BundledSkillDefinition>,
}

impl BundledSkill {
    /// Detect all skill definitions bundled with Warp for the local host.
    pub async fn detect() -> Self {
        let Some(resources_dir) = warp_core::paths::bundled_resources_dir() else {
            return Self::default();
        };
        Self::detect_in_resources_dir(resources_dir).await
    }

    /// Detect all skill definitions under the given resources root on the
    /// local filesystem, rendering skill content against this host.
    ///
    /// Called directly by the remote-server daemon, whose resources live at
    /// the global install location rather than inside an app bundle (which
    /// is what [`warp_core::paths::bundled_resources_dir`] resolves).
    pub(crate) async fn detect_in_resources_dir(resources_dir: PathBuf) -> Self {
        let (mut definitions, figma_definitions) = futures::join!(
            load_bundled_skill_definitions(&resources_dir),
            load_figma_skill_definitions(&resources_dir)
        );
        definitions.extend(figma_definitions);
        Self { definitions }
    }

    /// Returns descriptors for bundled skills whose activation conditions are met.
    pub fn active_descriptors(&self, ctx: &AppContext) -> Vec<SkillDescriptor> {
        self.definitions
            .iter()
            .filter(|(_, definition)| definition.activation.is_enabled(ctx))
            .map(|(id, definition)| {
                SkillDescriptor::new_bundled(id.clone(), definition.skill.clone(), definition.icon)
            })
            .collect()
    }

    /// Returns descriptors for bundled skills whose activation conditions are
    /// met, referenced by their `SKILL.md` paths instead of
    /// [`SkillReference::BundledSkillId`].
    ///
    /// Used for remote-host catalogs: a `BundledSkillId` reference resolves
    /// against the local catalog, so descriptors listed from a remote catalog
    /// must carry the skill's real remote path — which resolves back to this
    /// catalog through the path lookups — or invoking a listed skill would
    /// serve the local client's content.
    pub fn active_path_referenced_descriptors(&self, ctx: &AppContext) -> Vec<SkillDescriptor> {
        self.definitions
            .values()
            .filter(|definition| definition.activation.is_enabled(ctx))
            .map(|definition| {
                let mut descriptor = SkillDescriptor::from(definition.skill.clone());
                descriptor.icon_override = Some(definition.icon);
                descriptor
            })
            .collect()
    }

    /// Returns a bundled skill reference when the path belongs to a bundled skill.
    pub fn reference_for_path(&self, path: &LocalOrRemotePath) -> Option<SkillReference> {
        self.definitions
            .iter()
            .find(|(_, definition)| definition.skill.path == *path)
            .map(|(id, _)| SkillReference::BundledSkillId(id.clone()))
    }

    /// Returns a bundled skill definition by ID.
    pub fn skill(&self, id: &str) -> Option<&ParsedSkill> {
        self.definitions.get(id).map(|definition| &definition.skill)
    }

    /// Returns a bundled skill by ID only if its activation condition is met.
    pub fn active_skill(&self, id: &str, ctx: &AppContext) -> Option<&ParsedSkill> {
        let definition = self.definitions.get(id)?;
        definition
            .activation
            .is_enabled(ctx)
            .then_some(&definition.skill)
    }

    /// Returns a bundled skill by its `SKILL.md` path.
    pub fn skill_by_path(&self, path: &LocalOrRemotePath) -> Option<&ParsedSkill> {
        self.definitions
            .values()
            .map(|definition| &definition.skill)
            .find(|skill| skill.path == *path)
    }

    /// Returns a bundled skill by its `SKILL.md` path only if its activation
    /// condition is met.
    pub fn active_skill_by_path(
        &self,
        path: &LocalOrRemotePath,
        ctx: &AppContext,
    ) -> Option<&ParsedSkill> {
        self.definitions
            .values()
            .find(|definition| definition.skill.path == *path)
            .filter(|definition| definition.activation.is_enabled(ctx))
            .map(|definition| &definition.skill)
    }

    /// Builds a catalog from pre-parsed definitions. Used for catalogs
    /// received from a remote host's daemon, which parses and renders the
    /// skills against its own filesystem.
    pub(crate) fn from_definitions(
        definitions: impl IntoIterator<Item = (String, ParsedSkill, BundledSkillActivation)>,
    ) -> Self {
        let definitions = definitions
            .into_iter()
            .map(|(id, skill, activation)| {
                // MCP-gated skills carry their integration's brand icon, like
                // the local figma catalog loaded from `mcp_skills/figma`.
                let icon = match &activation {
                    BundledSkillActivation::RequiresMcp(McpIntegration::Figma) => Icon::Figma,
                    BundledSkillActivation::Always
                    | BundledSkillActivation::RequiresFeature(_)
                    | BundledSkillActivation::RequiresFile(_) => icon_for_bundled_skill(&id),
                };
                (
                    id,
                    BundledSkillDefinition {
                        skill,
                        activation,
                        icon,
                    },
                )
            })
            .collect();
        Self { definitions }
    }

    /// Iterates the catalog's definitions as `(id, skill, activation)`.
    /// Used by the daemon to serialize its catalog for the
    /// aggregate remote Agent Mode context snapshot.
    pub(crate) fn iter_definitions(
        &self,
    ) -> impl Iterator<Item = (&str, &ParsedSkill, &BundledSkillActivation)> {
        self.definitions
            .iter()
            .map(|(id, definition)| (id.as_str(), &definition.skill, &definition.activation))
    }

    #[cfg(test)]
    pub fn insert_for_testing(
        &mut self,
        id: impl Into<String>,
        skill: ParsedSkill,
        activation: BundledSkillActivation,
    ) {
        let id = id.into();
        self.definitions.insert(
            id.clone(),
            BundledSkillDefinition {
                skill,
                activation,
                icon: icon_for_bundled_skill(&id),
            },
        );
    }
}

/// Load skill definitions bundled with Warp.
async fn load_bundled_skill_definitions(
    resources_dir: &Path,
) -> HashMap<String, BundledSkillDefinition> {
    let skills_dir = resources_dir.join("bundled").join("skills");
    read_bundled_skills(&skills_dir, resources_dir)
        .await
        .into_iter()
        .map(|(id, skill)| {
            let icon = icon_for_bundled_skill(&id);
            let activation = activation_for_bundled_skill(&id, resources_dir);
            let bundled = BundledSkillDefinition {
                skill,
                activation,
                icon,
            };
            (id, bundled)
        })
        .collect()
}

/// Load Figma-specific bundled skills from the `figma/` subdirectory.
async fn load_figma_skill_definitions(
    resources_dir: &Path,
) -> HashMap<String, BundledSkillDefinition> {
    let figma_skills_dir = resources_dir
        .join("bundled")
        .join("mcp_skills")
        .join("figma");
    read_bundled_skills(&figma_skills_dir, resources_dir)
        .await
        .into_iter()
        .map(|(id, skill)| {
            let bundled = BundledSkillDefinition {
                skill,
                activation: BundledSkillActivation::RequiresMcp(McpIntegration::Figma),
                icon: Icon::Figma,
            };
            (id, bundled)
        })
        .collect()
}

/// Read bundled skill definitions from the specified directory, rendering
/// handlebars variables against this host's filesystem (`resources_dir` is
/// the resources root the skills belong to).
///
/// Only ever runs against the calling process's own filesystem: the local
/// app reads its bundled resources, and the remote daemon reads its global
/// install location before pushing the rendered content over the wire.
/// Clients never call this for files on another host, so local `Path`
/// semantics (this OS's encoding) are correct here.
pub(crate) async fn read_bundled_skills(
    skills_dir: &Path,
    resources_dir: &Path,
) -> HashMap<String, ParsedSkill> {
    let mut skills = HashMap::new();

    let Ok(mut entries) = async_fs::read_dir(skills_dir).await else {
        return skills;
    };

    while let Ok(Some(entry)) = entries.try_next().await {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let skill_file_path = entry_path.join("SKILL.md");
        let mut skill = match parse_bundled_skill(&skill_file_path) {
            Ok(skill) => skill,
            Err(err) => {
                report_error!(err.context(format!(
                    "Failed to parse bundled skill at {}",
                    skill_file_path.display()
                )));
                continue;
            }
        };

        // We use the directory name as the skill ID (guaranteed unique within bundled skills).
        let Some(skill_id) = entry_path.file_name().and_then(|s| s.to_str()) else {
            safe_warn!(
                safe: ("Could not resolve bundled skill ID, skipping skill"),
                full: ("Could not resolve bundled skill ID from {}, skipping skill", skill.path.display_path())
            );
            continue;
        };
        let context = build_bundled_skill_context(resources_dir, &entry_path);

        // Apply variable substitution to the skill content.
        skill.content = handlebars::render_template(&skill.content, &context);
        skills.insert(skill_id.to_owned(), skill);
    }

    log::info!("Read {} bundled skills", skills.len());

    skills
}

/// Builds the context map for bundled skill variable substitution.
///
/// Supported variables:
/// - `{{warp_server_url}}` - The server root URL (e.g., `https://api.warp.dev`)
/// - `{{warp_cli_binary_name}}` - The CLI binary name (e.g., `warp` or `warp-cli`)
/// - `{{warpctrl_binary_name}}` - The channel-specific Warp Control command name
/// - `{{warpctrl_wrapper_path}}` - Path to the bundled Warp Control wrapper
/// - `{{warp_url_scheme}}` - The URL scheme (e.g., `warp`, `warpdev`, `warppreview`)
/// - `{{settings_schema_path}}` - Path to the bundled JSON settings schema
/// - `{{skill_dir}}` - Path to the bundled skill's directory
/// - `{{settings_file_path}}` - Path to the user's settings TOML file
/// - `{{keybindings_file_path}}` - Path to the user's keybindings YAML file
pub(crate) fn build_bundled_skill_context(
    resources_dir: &Path,
    skill_dir: &Path,
) -> HashMap<String, String> {
    [
        (
            "warp_server_url".to_owned(),
            ChannelState::server_root_url().into_owned(),
        ),
        (
            "warp_cli_binary_name".to_owned(),
            ChannelState::channel().cli_command_name().to_owned(),
        ),
        (
            "warpctrl_binary_name".to_owned(),
            ChannelState::channel().warpctrl_command_name().to_owned(),
        ),
        (
            "warpctrl_wrapper_path".to_owned(),
            resources_dir
                .join("bin")
                .join(ChannelState::channel().warpctrl_command_name())
                .display()
                .to_string(),
        ),
        (
            "warp_url_scheme".to_owned(),
            ChannelState::url_scheme().to_owned(),
        ),
        (
            "settings_file_path".to_owned(),
            user_preferences_toml_file_path().display().to_string(),
        ),
        (
            "keybindings_file_path".to_owned(),
            keybinding_file_path().display().to_string(),
        ),
        (
            "settings_schema_path".to_owned(),
            resources_dir
                .join("settings_schema.json")
                .display()
                .to_string(),
        ),
        ("skill_dir".to_owned(), skill_dir.display().to_string()),
    ]
    .into_iter()
    .collect()
}

/// Returns the icon for a bundled skill, given its directory-based ID.
/// Skills with a known brand (e.g. `pr-comments` → GitHub) get a
/// branded icon; everything else falls back to the Warp logo.
pub(crate) fn icon_for_bundled_skill(skill_id: &str) -> Icon {
    match skill_id {
        "pr-comments" => Icon::Github,
        _ => Icon::WarpLogoLight,
    }
}

/// Returns the activation condition for a bundled skill.
///
/// Most skills are always active. Other skills appear only when their required
/// feature, integration, or bundled resource is available.
pub(crate) fn activation_for_bundled_skill(
    skill_id: &str,
    resources_dir: &Path,
) -> BundledSkillActivation {
    match skill_id {
        "modify-settings" => {
            BundledSkillActivation::RequiresFile(resources_dir.join("settings_schema.json"))
        }
        "warpctrl" => BundledSkillActivation::RequiresFeature(FeatureFlag::WarpControlCli),
        _ => BundledSkillActivation::Always,
    }
}

#[cfg(test)]
#[path = "bundled_tests.rs"]
mod tests;
