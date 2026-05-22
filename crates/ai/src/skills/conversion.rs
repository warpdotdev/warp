use std::path::PathBuf;

use thiserror::Error;
use warp_multi_agent_api as api;

use crate::agent::action_result::{AnyFileContent, FileContext};
use crate::skills::{ParsedSkill, SkillProvider, SkillScope};

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
}

impl From<ParsedSkill> for api::Skill {
    fn from(skill: ParsedSkill) -> Self {
        api::Skill {
            descriptor: Some(api::SkillDescriptor {
                skill_reference: Some(api::skill_descriptor::SkillReference::Path(
                    skill.path.to_string_lossy().to_string(),
                )),
                name: skill.name,
                description: skill.description,
                scope: Some(skill.scope.into()),
                provider: Some(skill.provider.into()),
            }),
            content: Some(api::FileContent {
                file_path: skill.path.to_string_lossy().to_string(),
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
            path: PathBuf::from(&path),
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
