use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ai::skills::{parse_bundled_skill, ParsedSkill, SkillReference};
use futures::TryStreamExt;
use warp_core::channel::ChannelState;
use warp_core::ui::icons::Icon;
use warp_core::{report_error, safe_warn};
use warp_util::local_or_remote_path::LocalOrRemotePath;
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
    /// Active only when a specific MCP server is running.
    RequiresMcp(McpIntegration),
    /// Active only when a specific file exists on disk.
    RequiresFile(PathBuf),
}

impl BundledSkillActivation {
    pub fn is_enabled(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Always => true,
            Self::RequiresMcp(integration) => {
                TemplatableMCPServerManager::as_ref(ctx).is_mcp_server_running(*integration)
            }
            Self::RequiresFile(path) => path.exists(),
        }
    }
}

/// A bundled skill definition with its activation condition and icon.
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
        let (mut definitions, figma_definitions) = futures::join!(
            load_bundled_skill_definitions(),
            load_figma_skill_definitions()
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
async fn load_bundled_skill_definitions() -> HashMap<String, BundledSkillDefinition> {
    let Some(resources_dir) = warp_core::paths::bundled_resources_dir() else {
        return HashMap::new();
    };
    let skills_dir = resources_dir.join("bundled").join("skills");
    read_bundled_skills(&skills_dir)
        .await
        .into_iter()
        .map(|(id, skill)| {
            let icon = icon_for_bundled_skill(&id);
            let activation = activation_for_bundled_skill(&id, &resources_dir);
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
async fn load_figma_skill_definitions() -> HashMap<String, BundledSkillDefinition> {
    let Some(resources_dir) = warp_core::paths::bundled_resources_dir() else {
        return HashMap::new();
    };
    let figma_skills_dir = resources_dir
        .join("bundled")
        .join("mcp_skills")
        .join("figma");
    read_bundled_skills(&figma_skills_dir)
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

/// Read bundled skill definitions from the specified directory.
pub(crate) async fn read_bundled_skills(skills_dir: &Path) -> HashMap<String, ParsedSkill> {
    let mut skills = HashMap::new();
    let context = build_bundled_skill_context();

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
/// - `{{warp_url_scheme}}` - The URL scheme (e.g., `warp`, `warpdev`, `warppreview`)
/// - `{{settings_schema_path}}` - Path to the bundled JSON settings schema
/// - `{{settings_file_path}}` - Path to the user's settings TOML file
/// - `{{keybindings_file_path}}` - Path to the user's keybindings YAML file
pub(crate) fn build_bundled_skill_context() -> HashMap<String, String> {
    let mut context: HashMap<String, String> = [
        (
            "warp_server_url".to_owned(),
            ChannelState::server_root_url().into_owned(),
        ),
        (
            "warp_cli_binary_name".to_owned(),
            ChannelState::channel().cli_command_name().to_owned(),
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
    ]
    .into_iter()
    .collect();

    if let Some(schema_path) =
        warp_core::paths::bundled_resources_dir().map(|dir| dir.join("settings_schema.json"))
    {
        context.insert(
            "settings_schema_path".to_owned(),
            schema_path.display().to_string(),
        );
    }

    context
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
/// Most skills are always active. Skills that depend on a bundled resource
/// file use `RequiresFile` so they only appear when the resource is present.
fn activation_for_bundled_skill(skill_id: &str, resources_dir: &Path) -> BundledSkillActivation {
    match skill_id {
        "modify-settings" => {
            BundledSkillActivation::RequiresFile(resources_dir.join("settings_schema.json"))
        }
        _ => BundledSkillActivation::Always,
    }
}
