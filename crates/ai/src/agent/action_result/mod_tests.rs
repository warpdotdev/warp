use super::{
    AIAgentActionResultType, RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind,
    RunAgentsLaunchedExecutionMode, RunAgentsResult, StartAgentResult, StartAgentVersion,
};

fn launched_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Launched {
            agent_id: format!("{name}-id"),
        },
    }
}

fn failed_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Failed {
            error: "launch failed".to_string(),
        },
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
fn deserializes_legacy_start_agent_success_without_version_as_v1() {
    let result: StartAgentResult =
        serde_json::from_value(serde_json::json!({ "Success": { "agent_id": "agent-1" } }))
            .expect("legacy start-agent success should deserialize");

    assert_eq!(
        result,
        StartAgentResult::Success {
            agent_id: "agent-1".to_string(),
            version: StartAgentVersion::V1,
        }
    );
}

#[test]
fn deserializes_legacy_start_agent_error_without_version_as_v1() {
    let result: StartAgentResult =
        serde_json::from_value(serde_json::json!({ "Error": { "error": "boom" } }))
            .expect("legacy start-agent error should deserialize");

    assert_eq!(
        result,
        StartAgentResult::Error {
            error: "boom".to_string(),
            version: StartAgentVersion::V1,
        }
    );
}

#[test]
fn deserializes_legacy_start_agent_cancelled_without_version_as_v1() {
    let result: StartAgentResult = serde_json::from_value(serde_json::json!({ "Cancelled": {} }))
        .expect("legacy start-agent cancellation should deserialize");

    assert_eq!(
        result,
        StartAgentResult::Cancelled {
            version: StartAgentVersion::V1,
        }
    );
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
