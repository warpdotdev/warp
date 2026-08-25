use std::time::Duration;

use warp_multi_agent_api as api;

use super::{
    AIAgentActionResultType, LrcActivity, LrcProcessActivity, LrcProcessState,
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
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

fn populated_activity() -> LrcActivity {
    LrcActivity {
        since_last_activity: Some(Duration::from_millis(1500)),
        process: Some(LrcProcessActivity {
            cpu_time_delta: Duration::from_millis(2750),
            state: LrcProcessState::DiskWait,
            live_process_count: 3,
            io_write_bytes_delta: 4096,
        }),
    }
}

#[test]
fn activity_survives_a_round_trip_through_the_api_type() {
    let activity = populated_activity();

    let wire = api::LongRunningShellCommandActivity::from(activity.clone());
    assert_eq!(LrcActivity::from(&wire), activity);
}

#[test]
fn activity_converts_durations_to_proto_durations_on_the_wire() {
    let wire = api::LongRunningShellCommandActivity::from(populated_activity());

    assert_eq!(
        wire.since_last_activity,
        Some(prost_types::Duration {
            seconds: 1,
            nanos: 500_000_000,
        })
    );
    assert_eq!(wire.process.expect("process tier").cpu_time_delta_ms, 2750);
}

/// A clock that never ticked is absent on the wire rather than fabricated as a
/// zero reading, and stays absent when read back.
#[test]
fn an_empty_activity_survives_the_round_trip_without_inventing_readings() {
    let wire = api::LongRunningShellCommandActivity::from(LrcActivity::default());

    assert!(wire.since_last_activity.is_none());
    assert!(wire.process.is_none());
    assert_eq!(LrcActivity::from(&wire), LrcActivity::default());
}

/// Every scalar in the process submessage encodes with implicit presence, so
/// the server can only tell "inspected and idle" from "not inspected" by whether
/// the submessage itself is there. An all-zero reading must therefore convert to
/// a present submessage rather than being collapsed away.
#[test]
fn a_fully_quiet_process_tier_still_converts_to_a_present_submessage() {
    let activity = LrcActivity {
        process: Some(LrcProcessActivity::default()),
        ..Default::default()
    };

    let wire = api::LongRunningShellCommandActivity::from(activity);

    let process = wire
        .process
        .expect("an all-zero reading is still a reading");
    assert_eq!(process.cpu_time_delta_ms, 0);
    assert_eq!(process.live_process_count, 0);
    assert_eq!(process.io_write_bytes_delta, 0);
}

/// The mirror of the case above: a tier that genuinely was not collected must
/// convert to an absent submessage, so the two cannot be confused.
#[test]
fn an_uncollected_process_tier_converts_to_an_absent_submessage() {
    let activity = LrcActivity {
        process: None,
        ..Default::default()
    };

    let wire = api::LongRunningShellCommandActivity::from(activity);

    assert!(wire.process.is_none());
}

#[test]
fn an_unrecognized_process_state_reads_back_as_unknown() {
    let wire = api::long_running_shell_command_activity::ProcessActivity {
        state: 999,
        ..Default::default()
    };

    assert_eq!(
        LrcProcessActivity::from(&wire).state,
        LrcProcessState::Unknown
    );
}

/// `Unknown` must serialize as the explicit `STATE_UNKNOWN` value rather than
/// the `STATE_UNSPECIFIED` zero value: the proto3-rewritten bindings omit
/// zero-valued enums from the wire, and "the client looked and could not
/// classify" must not read back as "never populated".
#[test]
fn an_unknown_state_is_sent_as_the_explicit_unknown_value() {
    let wire =
        api::long_running_shell_command_activity::ProcessActivity::from(LrcProcessActivity {
            state: LrcProcessState::Unknown,
            ..Default::default()
        });

    assert_eq!(
        wire.state,
        api::long_running_shell_command_activity::process_activity::State::Unknown as i32
    );
}
