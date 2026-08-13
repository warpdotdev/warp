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
        signals_unavailable: false,
    }
}

#[test]
fn activity_survives_a_round_trip_through_the_api_type() {
    let activity = populated_activity();

    let wire = api::LongRunningCommandActivity::from(activity.clone());
    assert_eq!(LrcActivity::from(&wire), activity);
}

#[test]
fn activity_converts_durations_to_seconds_on_the_wire() {
    let wire = api::LongRunningCommandActivity::from(populated_activity());

    assert_eq!(wire.seconds_since_last_activity, 1.5);
    assert_eq!(wire.process.expect("process tier").cpu_time_delta_ms, 2750);
}

#[test]
fn an_activity_clock_that_never_ticked_is_sent_as_zero_seconds() {
    let wire = api::LongRunningCommandActivity::from(LrcActivity::default());

    assert_eq!(wire.seconds_since_last_activity, 0.0);
    assert!(wire.process.is_none());
}

#[test]
fn unavailable_signals_survive_the_round_trip() {
    let activity = LrcActivity {
        signals_unavailable: true,
        ..Default::default()
    };

    let wire = api::LongRunningCommandActivity::from(activity);

    assert!(wire.signals_unavailable);
    assert!(LrcActivity::from(&wire).signals_unavailable);
}

/// The wire type has no way to say "never", so a clock that never ticked is
/// indistinguishable from one that ticked just now. The process submessage and
/// `signals_unavailable` alongside it are what tell the two apart.
#[test]
fn a_never_ticked_clock_reads_back_as_zero_rather_than_never() {
    let wire = api::LongRunningCommandActivity::from(LrcActivity::default());

    assert_eq!(
        LrcActivity::from(&wire).since_last_activity,
        Some(Duration::ZERO)
    );
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

    let wire = api::LongRunningCommandActivity::from(activity);

    let process = wire
        .process
        .expect("an all-zero reading is still a reading");
    assert_eq!(process.cpu_time_delta_ms, 0);
    assert_eq!(process.live_process_count, 0);
    assert_eq!(process.io_write_bytes_delta, 0);
    assert!(!wire.signals_unavailable);
}

/// The mirror of the case above: a tier that genuinely was not collected must
/// convert to an absent submessage, so the two cannot be confused.
#[test]
fn an_uncollected_process_tier_converts_to_an_absent_submessage() {
    let activity = LrcActivity {
        process: None,
        signals_unavailable: true,
        ..Default::default()
    };

    let wire = api::LongRunningCommandActivity::from(activity);

    assert!(wire.process.is_none());
    assert!(wire.signals_unavailable);
}

#[test]
fn an_unrecognized_process_state_reads_back_as_unknown() {
    let wire = api::long_running_command_activity::ProcessActivity {
        state: "something-new".to_string(),
        ..Default::default()
    };

    assert_eq!(
        LrcProcessActivity::from(&wire).state,
        LrcProcessState::Unknown
    );
}
