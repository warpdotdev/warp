use chrono::{Duration, Utc};

use crate::ai::ambient_agents::task::{RequestUsage, TaskPrincipalInfo, TaskStatusMessage};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::format_credits;
use crate::util::time_format::human_readable_precise_duration;

use super::TombstoneDisplayData;

const RUN_DURATION_SECONDS: i64 = 90;
const INFERENCE_COST: f64 = 1.5;
const COMPUTE_COST: f64 = 3.0;
const PLATFORM_COST: f64 = 2.5;

fn task_with_run_time_and_credits() -> AmbientAgentTask {
    let started_at = Utc::now();
    let updated_at = started_at + Duration::seconds(RUN_DURATION_SECONDS);
    AmbientAgentTask {
        task_id: "550e8400-e29b-41d4-a716-000000005000".parse().unwrap(),
        parent_run_id: None,
        title: "Task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: started_at,
        started_at: Some(started_at),
        updated_at,
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: "user-1".to_string(),
            display_name: Some("User 1".to_string()),
        }),
        executor: None,
        conversation_id: None,
        request_usage: Some(RequestUsage {
            inference_cost: Some(INFERENCE_COST),
            compute_cost: Some(COMPUTE_COST),
            platform_cost: Some(PLATFORM_COST),
        }),
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence: None,
        children: vec![],
    }
}

fn task_without_run_time_or_credits() -> AmbientAgentTask {
    let mut task = task_with_run_time_and_credits();
    task.started_at = None;
    task.request_usage = None;
    task
}

fn data_with_conversation_values() -> TombstoneDisplayData {
    TombstoneDisplayData {
        run_time: Some("conv run time".to_string()),
        credits: Some("conv credits".to_string()),
        ..Default::default()
    }
}

#[test]
fn task_failure_status_message_overrides_conversation_error() {
    let mut task = task_with_run_time_and_credits();
    task.state = AmbientAgentTaskState::Failed;
    task.status_message = Some(TaskStatusMessage {
        message: "task failed".to_string(),
        error_code: None,
    });
    let mut data = TombstoneDisplayData {
        is_error: true,
        error_message: Some("setup failed".to_string()),
        ..Default::default()
    };

    data.enrich_from_task(task);

    assert!(data.is_error);
    assert_eq!(data.error_message.as_deref(), Some("task failed"));
}

fn pr_artifact(branch: &str) -> Artifact {
    Artifact::PullRequest {
        url: format!("https://github.com/example/repo/pull/{branch}"),
        branch: branch.to_string(),
        repo: Some("example/repo".to_string()),
        number: None,
    }
}

#[test]
fn task_overrides_run_time_and_credits_when_present() {
    let task = task_with_run_time_and_credits();
    let mut data = data_with_conversation_values();

    data.enrich_from_task(task);

    let expected_run_time =
        human_readable_precise_duration(Duration::seconds(RUN_DURATION_SECONDS));
    let expected_credits = format_credits((INFERENCE_COST + COMPUTE_COST + PLATFORM_COST) as f32);
    assert_eq!(data.run_time, Some(expected_run_time));
    assert_eq!(data.credits, Some(expected_credits));
}

#[test]
fn conversation_values_preserved_when_task_lacks_run_time_and_credits() {
    let task = task_without_run_time_or_credits();
    let mut data = data_with_conversation_values();

    data.enrich_from_task(task);

    assert_eq!(data.run_time.as_deref(), Some("conv run time"));
    assert_eq!(data.credits.as_deref(), Some("conv credits"));
}

#[test]
fn empty_defaults_populated_from_task_for_non_oz() {
    let task = task_with_run_time_and_credits();
    let mut data = TombstoneDisplayData::default();

    data.enrich_from_task(task);

    let expected_run_time =
        human_readable_precise_duration(Duration::seconds(RUN_DURATION_SECONDS));
    let expected_credits = format_credits((INFERENCE_COST + COMPUTE_COST + PLATFORM_COST) as f32);
    assert_eq!(data.run_time, Some(expected_run_time));
    assert_eq!(data.credits, Some(expected_credits));
}

#[test]
fn task_artifacts_populate_empty_defaults() {
    let mut task = task_with_run_time_and_credits();
    task.artifacts = vec![pr_artifact("feature/foo")];
    let expected_artifacts = task.artifacts.clone();
    let mut data = TombstoneDisplayData::default();

    data.enrich_from_task(task);

    assert_eq!(data.artifacts, expected_artifacts);
}

#[test]
fn task_artifacts_override_conversation_artifacts() {
    let mut task = task_with_run_time_and_credits();
    task.artifacts = vec![pr_artifact("task-branch")];
    let expected_artifacts = task.artifacts.clone();
    let mut data = TombstoneDisplayData {
        artifacts: vec![pr_artifact("conv-branch")],
        ..Default::default()
    };

    data.enrich_from_task(task);

    assert_eq!(data.artifacts, expected_artifacts);
}

#[test]
fn empty_task_artifacts_preserve_conversation_artifacts() {
    let task = task_with_run_time_and_credits();
    assert!(task.artifacts.is_empty());
    let conversation_artifacts = vec![pr_artifact("conv-branch")];
    let mut data = TombstoneDisplayData {
        artifacts: conversation_artifacts.clone(),
        ..Default::default()
    };

    data.enrich_from_task(task);

    assert_eq!(data.artifacts, conversation_artifacts);
}
