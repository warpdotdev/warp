use std::path::PathBuf;

use thiserror::Error;
use warp_multi_agent_api as api;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use crate::agent::action_result::{AnyFileContent, FileContext};
use crate::skills::{ParsedSkill, SkillProvider, SkillReference, SkillScope};

#[derive(Error, Debug)]
pub enum SkillConversionError {
    #[error("No descriptor provided")]
    MissingDescriptor,
    #[error("No skill_reference provided")]
    MissingReference,
    #[error("No content provided")]
    MissingContent,
    #[error("Invalid scope")]
    ScopeInvalid,
    #[error("Invalid provider")]
    ProviderInvalid,
    #[error("Invalid content")]
    ContentInvalid,
    #[error("Skill path origin is unavailable")]
    PathOriginUnavailable,
    #[error("Invalid remote skill path")]
    RemotePathInvalid,
}
/// Identifies how a string skill path from an API payload should be interpreted.
///
/// Live agent responses can be decoded from the active session's location. Restored payloads do
/// not carry enough session identity to safely reconstruct path-based skill locations, so callers
/// must use [`SkillPathOrigin::Unavailable`] rather than silently assuming the local filesystem.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SkillPathOrigin {
    Local,
    Remote {
        host_id: HostId,
    },
    /// Path identity could not be restored, but the API payload already carries the skill
    /// descriptor and content needed to render a historical transcript.
    ///
    /// This intentionally uses a local path wrapper only as a display-compatible identity for
    /// restored conversation UI. Live execution paths should use [`SkillPathOrigin::Local`] or
    /// [`SkillPathOrigin::Remote`] so local/remote provenance is preserved.
    RestoredDisplayOnly,
    Unavailable,
}

impl SkillPathOrigin {
    pub fn location_for_path(
        &self,
        path: impl Into<String>,
    ) -> Result<LocalOrRemotePath, SkillConversionError> {
        let path = path.into();
        match self {
            SkillPathOrigin::Local | SkillPathOrigin::RestoredDisplayOnly => {
                // Normalize the path to collapse duplicate separators (e.g. `//workspace/...`
                // → `/workspace/...`) so skill cache lookups match the filesystem-derived keys.
                // We operate on the raw string rather than using `PathBuf::components().collect()`
                // because the latter re-serialises with platform-specific separators (backslashes
                // on Windows) and treats leading `//` as a UNC prefix on Windows.
                let normalized = collapse_slashes(&path);
                Ok(LocalOrRemotePath::Local(PathBuf::from(normalized)))
            }
            SkillPathOrigin::Remote { host_id } => {
                let path = StandardizedPath::try_new(&path)
                    .map_err(|_| SkillConversionError::RemotePathInvalid)?;
                Ok(LocalOrRemotePath::Remote(RemotePath::new(
                    host_id.clone(),
                    path,
                )))
            }
            SkillPathOrigin::Unavailable => Err(SkillConversionError::PathOriginUnavailable),
        }
    }
}

/// Collapse consecutive `/` separators into a single one.
///
/// Skill paths are always forward-slash POSIX-style paths on all platforms, so we normalise
/// at the string level rather than using [`std::path::PathBuf::components`], which would
/// re-serialise with backslashes on Windows and misinterpret `//prefix` as a UNC path.
fn collapse_slashes(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut prev_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if !prev_slash {
                result.push(ch);
            }
            prev_slash = true;
        } else {
            result.push(ch);
            prev_slash = false;
        }
    }
    result
}

fn skill_reference_for_path(
    path: impl Into<String>,
    path_origin: &SkillPathOrigin,
) -> Result<SkillReference, SkillConversionError> {
    path_origin
        .location_for_path(path)
        .map(SkillReference::Path)
}

pub fn skill_reference_from_api_skill_ref(
    skill_ref: api::SkillRef,
    path_origin: &SkillPathOrigin,
) -> Option<SkillReference> {
    match skill_ref.skill_reference {
        Some(api::skill_ref::SkillReference::Path(path)) => {
            skill_reference_for_path(path, path_origin).ok()
        }
        Some(api::skill_ref::SkillReference::BundledSkillId(id)) => {
            Some(SkillReference::BundledSkillId(id))
        }
        None => None,
    }
}

pub fn skill_reference_from_read_skill_ref(
    skill_reference: api::message::tool_call::read_skill::SkillReference,
    path_origin: &SkillPathOrigin,
) -> Result<SkillReference, SkillConversionError> {
    match skill_reference {
        api::message::tool_call::read_skill::SkillReference::SkillPath(path) => {
            skill_reference_for_path(path, path_origin)
        }
        api::message::tool_call::read_skill::SkillReference::BundledSkillId(id) => {
            Ok(SkillReference::BundledSkillId(id))
        }
    }
}
impl From<ParsedSkill> for api::Skill {
    fn from(skill: ParsedSkill) -> Self {
        api::Skill {
            descriptor: Some(api::SkillDescriptor {
                skill_reference: Some(api::skill_descriptor::SkillReference::Path(
                    skill.path.display_path(),
                )),
                name: skill.name,
                description: skill.description,
                scope: Some(skill.scope.into()),
                provider: Some(skill.provider.into()),
            }),
            content: Some(api::FileContent {
                file_path: skill.path.display_path(),
                content: skill.content,
                line_range: skill
                    .line_range
                    .map(|line_range| api::FileContentLineRange {
                        start: line_range.start as u32,
                        end: line_range.end as u32,
                    }),
            }),
        }
    }
}

impl From<SkillScope> for api::skill_descriptor::Scope {
    fn from(scope: SkillScope) -> Self {
        let scope_type: api::skill_descriptor::scope::Type = match scope {
            SkillScope::Home => api::skill_descriptor::scope::Type::Home(()),
            SkillScope::Project => api::skill_descriptor::scope::Type::Project(()),
            SkillScope::Bundled => api::skill_descriptor::scope::Type::Bundled(()),
        };

        api::skill_descriptor::Scope {
            r#type: Some(scope_type),
        }
    }
}

impl From<SkillProvider> for api::skill_descriptor::Provider {
    fn from(scope: SkillProvider) -> Self {
        let provider_type: api::skill_descriptor::provider::Type = match scope {
            SkillProvider::Warp => api::skill_descriptor::provider::Type::Warp(()),
            SkillProvider::Agents => api::skill_descriptor::provider::Type::Agents(()),
            SkillProvider::Claude => api::skill_descriptor::provider::Type::Claude(()),
            SkillProvider::Codex => api::skill_descriptor::provider::Type::Codex(()),
            SkillProvider::Cursor => api::skill_descriptor::provider::Type::Cursor(()),
            SkillProvider::Gemini => api::skill_descriptor::provider::Type::Gemini(()),
            SkillProvider::Copilot => api::skill_descriptor::provider::Type::Copilot(()),
            SkillProvider::Droid => api::skill_descriptor::provider::Type::Droid(()),
            SkillProvider::Github => api::skill_descriptor::provider::Type::Github(()),
            SkillProvider::OpenCode => api::skill_descriptor::provider::Type::OpenCode(()),
        };

        api::skill_descriptor::Provider {
            r#type: Some(provider_type),
        }
    }
}

impl TryFrom<api::Skill> for ParsedSkill {
    type Error = SkillConversionError;

    fn try_from(api_skill: api::Skill) -> Result<Self, Self::Error> {
        Self::try_from_api_with_origin(api_skill, &SkillPathOrigin::Unavailable)
    }
}

impl ParsedSkill {
    pub fn try_from_api_with_origin(
        api_skill: api::Skill,
        path_origin: &SkillPathOrigin,
    ) -> Result<Self, SkillConversionError> {
        let Some(descriptor) = api_skill.descriptor else {
            return Err(SkillConversionError::MissingDescriptor);
        };
        let Some(file_content) = api_skill.content else {
            return Err(SkillConversionError::MissingContent);
        };
        let Some(skill_reference) = descriptor.skill_reference else {
            return Err(SkillConversionError::MissingReference);
        };
        // TODO(pei): Once we refactor ParsedSkill to use SkillDescriptor,
        // we can pass forward the reference directly to ParsedSkill
        let path = match skill_reference {
            api::skill_descriptor::SkillReference::Path(path) => path,
            _ => "".to_string(), // This is ok only because we don't use the path
        };

        let Some(Ok(scope)) = descriptor.scope.map(convert_scope) else {
            return Err(SkillConversionError::ScopeInvalid);
        };

        let Some(Ok(provider)) = descriptor.provider.map(convert_provider) else {
            return Err(SkillConversionError::ProviderInvalid);
        };

        let context: FileContext = file_content.into();
        let AnyFileContent::StringContent(content) = context.content else {
            return Err(SkillConversionError::ContentInvalid);
        };

        let line_range = context.line_range.as_ref();

        Ok(ParsedSkill {
            path: path_origin.location_for_path(path)?,
            name: descriptor.name,
            description: descriptor.description,
            content,
            line_range: line_range.cloned(),
            scope,
            provider,
        })
    }
}

fn convert_scope(scope: api::skill_descriptor::Scope) -> Result<SkillScope, SkillConversionError> {
    let Some(scope_type) = scope.r#type else {
        return Err(SkillConversionError::ScopeInvalid);
    };

    match scope_type {
        api::skill_descriptor::scope::Type::Home(_) => Ok(SkillScope::Home),
        api::skill_descriptor::scope::Type::Project(_) => Ok(SkillScope::Project),
        api::skill_descriptor::scope::Type::Bundled(_) => Ok(SkillScope::Bundled),
    }
}

fn convert_provider(
    provider: api::skill_descriptor::Provider,
) -> Result<SkillProvider, SkillConversionError> {
    let Some(provider_type) = provider.r#type else {
        return Err(SkillConversionError::ProviderInvalid);
    };

    match provider_type {
        api::skill_descriptor::provider::Type::Warp(_) => Ok(SkillProvider::Warp),
        api::skill_descriptor::provider::Type::Agents(_) => Ok(SkillProvider::Agents),
        api::skill_descriptor::provider::Type::Claude(_) => Ok(SkillProvider::Claude),
        api::skill_descriptor::provider::Type::Codex(_) => Ok(SkillProvider::Codex),
        api::skill_descriptor::provider::Type::Cursor(_) => Ok(SkillProvider::Cursor),
        api::skill_descriptor::provider::Type::Gemini(_) => Ok(SkillProvider::Gemini),
        api::skill_descriptor::provider::Type::Copilot(_) => Ok(SkillProvider::Copilot),
        api::skill_descriptor::provider::Type::Droid(_) => Ok(SkillProvider::Droid),
        api::skill_descriptor::provider::Type::Github(_) => Ok(SkillProvider::Github),
        api::skill_descriptor::provider::Type::OpenCode(_) => Ok(SkillProvider::OpenCode),
    }
}

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod conversion_tests;
