use ai::agent::action_result::{AnyFileContent, FileContext};
use futures::future::{BoxFuture, FutureExt};
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, ReadSkillRequest, ReadSkillResult};
use crate::ai::blocklist::SessionContext;
use crate::ai::skills::{SkillManager, SkillTelemetryEvent};
use crate::send_telemetry_from_ctx;
use crate::terminal::model::session::active_session::ActiveSession;

pub struct ReadSkillExecutor {
    active_session: ModelHandle<ActiveSession>,
}

impl ReadSkillExecutor {
    pub fn new(active_session: ModelHandle<ActiveSession>) -> Self {
        Self { active_session }
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

        // Resolve from the catalog selected by the active session's host, so
        // remote sessions read the host-rendered bundled skill.
        let path_origin =
            SessionContext::from_session(self.active_session.as_ref(ctx), ctx).skill_path_origin();

        match SkillManager::as_ref(ctx).active_skill_by_reference_with_origin(
            skill_ref,
            &path_origin,
            ctx,
        ) {
            Ok(skill) => {
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
                    skill.path.display_path(),
                    AnyFileContent::StringContent(skill.content.clone()),
                    skill.line_range.clone(),
                    None,
                );
                ActionExecution::Sync(ReadSkillResult::Success { content }.into())
            }
            Err(error) => {
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
                ActionExecution::Sync(ReadSkillResult::Error(error.to_string()).into())
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
