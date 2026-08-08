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

/// A restored pane must not persist a hole for a field whose live answer is
/// not available yet.
///
/// `snapshot()` reads `cwd` and `shell_launch_data` from the live view, and
/// those only exist once the shell has started and reported in. Quitting during
/// that window — seconds long when many tabs restore at once — would otherwise
/// save `None` and permanently lose the pane's directory and shell: the next
/// restore opens somewhere else, and saves that as the new truth.
///
/// `FeatureFlag::LazyShellStartup` turns that window from "seconds" into
/// "forever" for a tab the user never opens, so the first case below is the
/// one every deferred pane takes on every save.
#[test]
fn a_field_the_shell_has_not_answered_yet_keeps_its_restored_value() {
    // Shell has not reported: fall back to what we were restored with.
    assert_eq!(
        preserved_on_save(None, Some("/dev/tools/warp".to_owned())),
        Some("/dev/tools/warp".to_owned())
    );
    // Shell has reported: the live answer always wins, including after the
    // user has `cd`-ed somewhere new.
    assert_eq!(
        preserved_on_save(
            Some("/dev/tools/orbit".to_owned()),
            Some("/dev/tools/warp".to_owned())
        ),
        Some("/dev/tools/orbit".to_owned())
    );
    // Nothing known from either source stays unknown rather than inventing one.
    assert_eq!(preserved_on_save::<String>(None, None), None);
    // A pane that was never restored (a brand-new tab) has no fallback.
    assert_eq!(
        preserved_on_save(Some("/dev/new".to_owned()), None),
        Some("/dev/new".to_owned())
    );
}
