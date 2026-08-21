use super::{
    AIAgentActionResultType, FetchConversationResult, RunAgentsAgentOutcome,
    RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode, RunAgentsResult,
};

fn launched_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Launched {
            agent_id: format!("{name}-id"),
        },
        resolved_model_id: String::new(),
    }
}

fn failed_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Failed {
            error: "launch failed".to_string(),
        },
        resolved_model_id: String::new(),
    }
}

fn run_agents_result(agents: Vec<RunAgentsAgentOutcome>) -> AIAgentActionResultType {
    AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents,
    })
}

#[test]
fn run_agents_is_successful_when_all_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), launched_agent("second")]);

    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_successful_when_some_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), failed_agent("second")]);
    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_failed_when_no_agents_launch() {
    let result = run_agents_result(vec![failed_agent("first"), failed_agent("second")]);

    assert!(!result.is_successful());
    assert!(result.is_failed());
}

#[test]
fn cancelled_fetch_conversation_does_not_unconditionally_trigger_a_follow_up_request() {
    // `Cancelled` covers both a deliberate terminal cancellation (Stop, pane close,
    // delete) and collateral same-conversation cleanup, and `AIAgentActionResultType`
    // alone can't distinguish the two (that context lives in the `CancellationReason`
    // passed alongside the `FinishedAction` event, not in the result itself). So this
    // must behave like every other cancelled result here and NOT unconditionally
    // trigger a follow-up -- doing so would revive a conversation the user genuinely
    // stopped. The collateral case is instead reported through the request that
    // already owns it (see `crates/ai/.../convert.rs`'s explicit-error serialization
    // and the app-level `BlocklistAIController` tests).
    let result = AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled);

    assert!(result.is_cancelled());
    assert!(!result.should_trigger_request_upon_completion());
}
