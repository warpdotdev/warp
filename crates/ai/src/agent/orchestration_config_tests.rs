use super::*;
use crate::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};

fn make_config(model: &str, harness: &str, remote: bool) -> OrchestrationConfig {
    OrchestrationConfig {
        model_id: model.to_string(),
        harness_type: harness.to_string(),
        execution_mode: if remote {
            OrchestrationExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
            }
        } else {
            OrchestrationExecutionMode::Local
        },
    }
}

fn make_request(model: &str, harness: &str, remote: bool) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "test".to_string(),
        base_prompt: "prompt".to_string(),
        skills: vec![],
        model_id: model.to_string(),
        harness_type: harness.to_string(),
        execution_mode: if remote {
            RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
            }
        } else {
            RunAgentsExecutionMode::Local
        },
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            name: "a".to_string(),
            prompt: String::new(),
            title: String::new(),
        }],
        plan_id: String::new(),
        harness_auth_secret_name: None,
    }
}

#[test]
fn exact_match_local() {
    let config = make_config("auto", "oz", false);
    let request = make_request("auto", "oz", false);
    assert!(matches_active_config(&request, &config));
}

#[test]
fn exact_match_remote() {
    let config = make_config("auto", "oz", true);
    let request = make_request("auto", "oz", true);
    assert!(matches_active_config(&request, &config));
}

#[test]
fn empty_model_inherits_and_matches() {
    let config = make_config("auto", "oz", false);
    let request = make_request("", "oz", false);
    assert!(matches_active_config(&request, &config));
}

#[test]
fn empty_harness_inherits_and_matches() {
    let config = make_config("auto", "oz", false);
    let request = make_request("auto", "", false);
    assert!(matches_active_config(&request, &config));
}

#[test]
fn different_model_mismatches() {
    let config = make_config("auto", "oz", false);
    let request = make_request("claude-4-6-opus-high", "oz", false);
    assert!(!matches_active_config(&request, &config));
}

#[test]
fn different_harness_mismatches() {
    let config = make_config("auto", "oz", false);
    let request = make_request("auto", "claude", false);
    assert!(!matches_active_config(&request, &config));
}

#[test]
fn execution_mode_variant_mismatch() {
    let config = make_config("auto", "oz", false);
    let request = make_request("auto", "oz", true);
    assert!(!matches_active_config(&request, &config));
}

#[test]
fn remote_different_environment_mismatches() {
    let config = make_config("auto", "oz", true);
    let mut request = make_request("auto", "oz", true);
    if let RunAgentsExecutionMode::Remote {
        ref mut environment_id,
        ..
    } = request.execution_mode
    {
        *environment_id = "env-other".to_string();
    }
    assert!(!matches_active_config(&request, &config));
}

#[test]
fn remote_empty_env_inherits_and_matches() {
    let config = make_config("auto", "oz", true);
    let mut request = make_request("auto", "oz", true);
    if let RunAgentsExecutionMode::Remote {
        ref mut environment_id,
        ..
    } = request.execution_mode
    {
        *environment_id = String::new();
    }
    assert!(matches_active_config(&request, &config));
}

#[test]
fn computer_use_not_in_match_check() {
    let config = make_config("auto", "oz", true);
    let mut request = make_request("auto", "oz", true);
    if let RunAgentsExecutionMode::Remote {
        ref mut computer_use_enabled,
        ..
    } = request.execution_mode
    {
        *computer_use_enabled = true;
    }
    // computer_use_enabled differs but should still match
    assert!(matches_active_config(&request, &config));
}

#[test]
fn status_default_is_none() {
    assert_eq!(
        OrchestrationConfigStatus::default(),
        OrchestrationConfigStatus::None
    );
}

#[test]
fn status_predicates() {
    assert!(OrchestrationConfigStatus::Approved.is_approved());
    assert!(!OrchestrationConfigStatus::Approved.is_disapproved());
    assert!(OrchestrationConfigStatus::Disapproved.is_disapproved());
    assert!(!OrchestrationConfigStatus::None.is_approved());
}

#[test]
fn proto_round_trip_config_local() {
    let config = make_config("auto", "oz", false);
    let proto = config.to_proto();
    let round_tripped = OrchestrationConfig::from_proto(&proto);
    assert_eq!(config, round_tripped);
}

#[test]
fn proto_round_trip_config_remote() {
    let config = make_config("auto", "claude", true);
    let proto = config.to_proto();
    let round_tripped = OrchestrationConfig::from_proto(&proto);
    assert_eq!(config, round_tripped);
}

#[test]
fn proto_round_trip_status() {
    for status in [
        OrchestrationConfigStatus::None,
        OrchestrationConfigStatus::Approved,
        OrchestrationConfigStatus::Disapproved,
    ] {
        let proto = status.to_proto();
        let round_tripped = OrchestrationConfigStatus::from_proto(proto.as_ref());
        assert_eq!(status, round_tripped);
    }
}
