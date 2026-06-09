//! Tests for [`inherit_share_for_local_child`]. These verify the pure
//! branching independent of the PaneGroup dispatch code.

use uuid::Uuid;

use super::*;

fn new_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn user_source(task_id: Option<&str>) -> SharedSessionSource {
    SharedSessionSource::user(task_id.map(str::to_owned))
}

fn ambient_source(task_id: Option<&str>) -> SharedSessionSource {
    SharedSessionSource::ambient_agent(task_id.map(str::to_owned))
}

#[test]
fn inherit_share_returns_no_when_host_is_not_sharing() {
    let result = inherit_share_for_local_child(None, new_task_id());
    assert!(matches!(result, IsSharedSessionCreator::No));
}

#[test]
fn inherit_share_returns_no_when_host_user_share_has_no_task_id() {
    let host = user_source(None);
    let result = inherit_share_for_local_child(Some(&host), new_task_id());
    assert!(
        matches!(result, IsSharedSessionCreator::No),
        "hosts without a stamped task_id must NOT cascade; the viewer cannot enumerate \
         children via REST without a task_id"
    );
}

#[test]
fn inherit_share_returns_no_when_host_ambient_share_has_no_task_id() {
    let host = ambient_source(None);
    let result = inherit_share_for_local_child(Some(&host), new_task_id());
    assert!(matches!(result, IsSharedSessionCreator::No));
}

#[test]
fn inherit_share_cascades_user_source_for_manually_shared_local_orchestrator() {
    let host = user_source(Some("parent-task-id"));
    let child_task_id = new_task_id();
    let expected_child_str = child_task_id.to_string();
    match inherit_share_for_local_child(Some(&host), child_task_id) {
        IsSharedSessionCreator::Yes {
            source:
                SharedSessionSource {
                    source_type: SessionSourceType::User,
                    source_task_id: Some(task_id),
                },
        } => {
            assert_eq!(
                task_id, expected_child_str,
                "the cascaded child must carry its own task_id in the sidecar, not the host's"
            );
        }
        other => panic!(
            "expected IsSharedSessionCreator::Yes with unit User variant carrying child task_id in \
             the sidecar, got {other:?}"
        ),
    }
}

#[test]
fn inherit_share_cascades_ambient_source_for_cloud_orchestrator() {
    let host = ambient_source(Some("parent-task-id"));
    let child_task_id = new_task_id();
    let expected_child_str = child_task_id.to_string();
    match inherit_share_for_local_child(Some(&host), child_task_id) {
        IsSharedSessionCreator::Yes {
            source:
                SharedSessionSource {
                    source_type:
                        SessionSourceType::AmbientAgent {
                            task_id: Some(task_id),
                        },
                    source_task_id,
                },
        } => {
            assert_eq!(task_id, expected_child_str);
            assert_eq!(
                source_task_id.as_deref(),
                Some(expected_child_str.as_str()),
                "the sidecar must mirror the cascaded child's task_id so viewers can read one \
                 field for both `User` and `AmbientAgent` shares"
            );
        }
        other => panic!(
            "expected IsSharedSessionCreator::Yes with AmbientAgent variant carrying child \
             task_id, got {other:?}"
        ),
    }
}
