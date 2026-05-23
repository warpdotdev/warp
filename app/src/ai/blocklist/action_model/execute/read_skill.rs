use ai::agent::action_result::{AnyFileContent, FileContext};
use futures::future::{BoxFuture, FutureExt};
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, ReadSkillRequest, ReadSkillResult};
use crate::ai::skills::{SkillManager, SkillTelemetryEvent};
use crate::send_telemetry_from_ctx;

pub struct ReadSkillExecutor;

impl ReadSkillExecutor {
    pub fn new() -> Self {
        Self
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        // User-created skills are readable on demand.
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::ReadSkill(ReadSkillRequest { skill: skill_ref }) = &action.action
        else {
            return ActionExecution::<ReadSkillResult>::InvalidAction;
        };

        match SkillManager::as_ref(ctx).active_skill_by_reference(skill_ref, ctx) {
            Some(skill) => {
                send_telemetry_from_ctx!(
                    SkillTelemetryEvent::Read {
                        reference: skill_ref.clone(),
                        name: Some(skill.name.clone()),
                        scope: Some(skill.scope),
                        provider: Some(skill.provider),
                        error: false,
                    },
                    ctx
                );
                let content = FileContext::new(
                    skill.path.to_string_lossy().into_owned(),
                    AnyFileContent::StringContent(skill.content.clone()),
                    skill.line_range.clone(),
                    None,
                );
                ActionExecution::Sync(ReadSkillResult::Success { content }.into())
            }
            None => {
                send_telemetry_from_ctx!(
                    SkillTelemetryEvent::Read {
                        reference: skill_ref.clone(),
                        name: None,
                        scope: None,
                        provider: None,
                        error: true,
                    },
                    ctx
                );
                ActionExecution::Sync(
                    ReadSkillResult::Error(format!("Skill not found: {:?}", skill_ref)).into(),
                )
            }
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for ReadSkillExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "read_skill_tests.rs"]
mod tests;
