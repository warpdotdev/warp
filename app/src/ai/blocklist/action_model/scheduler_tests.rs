use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{App, AppContext, Entity, ModelContext};

use super::super::execute::{ParallelExecutionPolicy, RunningActionPhase, TryExecuteResult};
use super::{AgentToolActionModel, AgentToolScheduleHost, AgentToolScheduler};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};

/// A minimal mock host for deterministic scheduler tests.
///
/// `try_execute` returns `ExecutedAsync` but never calls `finish_action`, so admitted
/// actions stay "running" forever — making the post-queue state stable to assert on.
/// `spawn_after_preprocess` invokes its callback synchronously, so the entire
/// queue_actions flow completes within the same `app.update()` call.
struct MockHost {
    tools: AgentToolActionModel,
}

impl Entity for MockHost {
    type Event = ();
}

impl AgentToolScheduleHost for MockHost {
    type Context<'a> = ModelContext<'a, Self>;

    fn app_context<'a, 'b>(ctx: &'a Self::Context<'b>) -> &'a AppContext {
        ctx
    }

    fn tools(&mut self) -> &mut AgentToolActionModel {
        &mut self.tools
    }

    fn tools_ref(&self) -> &AgentToolActionModel {
        &self.tools
    }

    fn preprocess(
        &mut self,
        _action: &AIAgentAction,
        _conversation_id: AIConversationId,
        _ctx: &mut Self::Context<'_>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    fn try_execute(
        &mut self,
        _action: AIAgentAction,
        _conversation_id: AIConversationId,
        _is_user_initiated: bool,
        _ctx: &mut Self::Context<'_>,
    ) -> TryExecuteResult {
        // Never calls finish_action — leaves the action "running" so state is stable.
        TryExecuteResult::ExecutedAsync
    }

    fn can_autoexecute(
        &mut self,
        _action: &AIAgentAction,
        _conversation_id: AIConversationId,
        _ctx: &mut Self::Context<'_>,
    ) -> bool {
        true
    }

    fn action_phase(&self, action: &AIAgentAction, _ctx: &AppContext) -> RunningActionPhase {
        match &action.action {
            AIAgentActionType::ReadFiles(_) => {
                RunningActionPhase::Parallel(ParallelExecutionPolicy::ReadOnlyLocalContext)
            }
            _ => RunningActionPhase::Serial,
        }
    }

    fn spawn_after_preprocess(
        &mut self,
        _futures: Vec<BoxFuture<'static, ()>>,
        ctx: &mut Self::Context<'_>,
        then: impl FnOnce(&mut Self, &mut Self::Context<'_>) + 'static,
    ) {
        // Run synchronously so queue_actions completes within one update call.
        then(self, ctx);
    }
}

/// Returns a minimal `AIAgentAction` with the given discriminant.
fn make_action(id: &str, action_type: AIAgentActionType) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        action: action_type,
        requires_result: true,
    }
}

fn make_read_files(id: &str) -> AIAgentAction {
    use ai::agent::action::ReadFilesRequest;

    use crate::ai::agent::FileLocations;
    make_action(
        id,
        AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: vec![FileLocations {
                name: "/dev/null".to_string(),
                lines: vec![],
            }],
        }),
    )
}

fn make_file_edits(id: &str) -> AIAgentAction {
    make_action(
        id,
        AIAgentActionType::RequestFileEdits {
            file_edits: vec![],
            title: None,
        },
    )
}

fn make_command(id: &str) -> AIAgentAction {
    make_action(
        id,
        AIAgentActionType::RequestCommandOutput {
            command: "echo hi".to_string(),
            is_read_only: None,
            is_risky: None,
            wait_until_completion: true,
            uses_pager: None,
            rationale: None,
            citations: vec![],
        },
    )
}

#[test]
fn scheduler_admits_parallel_read_phase() {
    App::test((), |mut app| async move {
        let handle = app.add_model(|_| MockHost {
            tools: AgentToolActionModel::new(),
        });
        let conversation_id = AIConversationId::new();

        handle.update(&mut app, |host, ctx| {
            AgentToolScheduler::queue_actions(
                host,
                vec![make_read_files("r1"), make_read_files("r2")],
                conversation_id,
                ctx,
            );
        });

        handle.read(&app, |host, _| {
            assert_eq!(
                host.tools.running_action_count(conversation_id),
                2,
                "both ReadFiles should be running in parallel"
            );
            assert_eq!(
                host.tools.pending_action_count(conversation_id),
                0,
                "no actions should be left pending"
            );
        });
    });
}

#[test]
fn scheduler_serial_barrier_holds_second_action() {
    App::test((), |mut app| async move {
        let handle = app.add_model(|_| MockHost {
            tools: AgentToolActionModel::new(),
        });
        let conversation_id = AIConversationId::new();

        handle.update(&mut app, |host, ctx| {
            AgentToolScheduler::queue_actions(
                host,
                vec![make_file_edits("e1"), make_command("c1")],
                conversation_id,
                ctx,
            );
        });

        handle.read(&app, |host, _| {
            assert_eq!(
                host.tools.running_action_count(conversation_id),
                1,
                "only the first serial action should be running"
            );
            assert_eq!(
                host.tools.pending_action_count(conversation_id),
                1,
                "the second action should be blocked behind the serial barrier"
            );
        });
    });
}
