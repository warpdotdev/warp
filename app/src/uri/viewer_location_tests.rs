use std::collections::HashMap;

use chrono::Utc;

use super::*;
use crate::ai::ambient_agents::AmbientAgentTaskState;

const ROOT: &str = "11111111-1111-1111-1111-111111111111";
const CHILD: &str = "22222222-2222-2222-2222-222222222222";

fn task(id: &str, parent: Option<&str>) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id: id.parse().unwrap(),
        parent_run_id: parent.map(str::to_string),
        title: String::new(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: None,
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        executor: None,
        creator: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
        debug_agent_available: false,
        scope: None,
    }
}

#[test]
fn parses_supported_viewer_state_without_changing_the_root_url() {
    let url = Url::parse(&format!(
        "https://app.warp.dev/conversation/root?view=standalone&foo=bar#child={CHILD}"
    ))
    .unwrap();
    let location = ViewerLocation::parse(&url).unwrap();
    assert!(location.standalone);
    assert_eq!(
        location.child_anchor,
        ChildAnchor::Selected(CHILD.parse().unwrap())
    );
    assert_eq!(
        location.root_url.as_str(),
        "https://app.warp.dev/conversation/root?view=standalone&foo=bar"
    );
}

#[test]
fn standalone_value_and_child_key_are_case_sensitive() {
    let location = ViewerLocation::parse(
        &Url::parse(&format!(
            "https://app.warp.dev/session/root?view=Standalone#Child={CHILD}"
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(!location.standalone);
    assert_eq!(location.child_anchor, ChildAnchor::Root);
}

#[test]
fn empty_malformed_and_duplicate_child_anchors_are_invalid() {
    for fragment in [
        "child=",
        "child=not-a-run-id",
        &format!("child={CHILD}&child={ROOT}"),
    ] {
        let location = ViewerLocation::parse(
            &Url::parse(&format!(
                "https://app.warp.dev/conversation/root#{fragment}"
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(location.child_anchor, ChildAnchor::Invalid);
    }
}

#[test]
fn resolves_to_the_top_level_root_and_rejects_cycles() {
    let root_id = ROOT.parse().unwrap();
    let child_id = CHILD.parse().unwrap();
    let mut tasks = HashMap::from([
        (child_id, task(CHILD, Some(ROOT))),
        (root_id, task(ROOT, None)),
    ]);
    let resolved = futures::executor::block_on(resolve_root_task(child_id, |id| {
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }))
    .unwrap()
    .unwrap();
    assert_eq!(resolved.root_task.task_id, root_id);
    assert_eq!(resolved.entry_task_id, child_id);

    let mut cycle = HashMap::from([
        (child_id, task(CHILD, Some(ROOT))),
        (root_id, task(ROOT, Some(CHILD))),
    ]);
    let result = futures::executor::block_on(resolve_root_task(child_id, |id| {
        futures::future::ready(cycle.remove(&id).ok_or_else(|| anyhow!("missing")))
    }));
    assert!(result.is_err());
}

#[test]
fn deep_walk_returns_only_the_top_level_root() {
    let root_id = ROOT.parse().unwrap();
    let child_id = CHILD.parse().unwrap();
    let middle = "33333333-3333-3333-3333-333333333333";
    let middle_id = middle.parse().unwrap();
    let mut tasks = HashMap::from([
        (child_id, task(CHILD, Some(middle))),
        (middle_id, task(middle, Some(ROOT))),
        (root_id, task(ROOT, None)),
    ]);
    let resolved = futures::executor::block_on(resolve_root_task(child_id, |id| {
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }))
    .unwrap()
    .unwrap();
    assert_eq!(resolved.root_task.task_id, root_id);
    assert_eq!(resolved.entry_task_id, child_id);
}

#[test]
fn top_level_entry_is_not_canonicalized_as_a_child() {
    let root_id = ROOT.parse().unwrap();
    let resolved = futures::executor::block_on(resolve_root_task(root_id, |_| {
        futures::future::ready(Ok(task(ROOT, None)))
    }))
    .unwrap();
    assert!(resolved.is_none());
}

#[test]
fn root_session_wins_over_conversation_and_query_is_not_copied() {
    let mut root = task(ROOT, None);
    root.state = AmbientAgentTaskState::InProgress;
    root.is_sandbox_running = true;
    root.session_id = Some("33333333-3333-3333-3333-333333333333".to_string());
    root.conversation_id = Some("root-conversation".to_string());
    let resolution = RootResolution {
        entry_task_id: CHILD.parse().unwrap(),
        root_task: root,
    };
    let url = canonical_root_url(
        &Url::parse("https://app.warp.dev/conversation/child?credential=nope").unwrap(),
        &resolution,
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://app.warp.dev/session/33333333-3333-3333-3333-333333333333#child=22222222-2222-2222-2222-222222222222"
    );
}

#[test]
fn unreachable_active_session_falls_back_to_the_root_conversation() {
    let mut root = task(ROOT, None);
    root.is_sandbox_running = true;
    root.session_id = Some("not-a-session-id".to_string());
    root.conversation_id = Some("root-conversation".to_string());
    let url = canonical_root_url(
        &Url::parse("https://app.warp.dev/session/child").unwrap(),
        &RootResolution {
            entry_task_id: CHILD.parse().unwrap(),
            root_task: root,
        },
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://app.warp.dev/conversation/root-conversation#child=22222222-2222-2222-2222-222222222222"
    );
}

#[test]
fn root_without_a_reachable_route_stays_standalone() {
    let resolution = RootResolution {
        entry_task_id: CHILD.parse().unwrap(),
        root_task: task(ROOT, None),
    };
    assert!(
        canonical_root_url(
            &Url::parse("https://app.warp.dev/conversation/child").unwrap(),
            &resolution,
        )
        .is_none()
    );
}

#[test]
fn anchor_waits_for_registration_after_seed_and_clears_unmatched_values() {
    let child_id = CHILD.parse().unwrap();
    let seeded = HashSet::from([child_id]);
    assert_eq!(
        hydrated_anchor_action(ChildAnchor::Selected(child_id), &seeded, &HashSet::new()),
        HydratedAnchorAction::Wait
    );
    assert_eq!(
        hydrated_anchor_action(ChildAnchor::Selected(child_id), &seeded, &seeded),
        HydratedAnchorAction::Select(child_id)
    );
    assert_eq!(
        hydrated_anchor_action(
            ChildAnchor::Selected(ROOT.parse().unwrap()),
            &seeded,
            &seeded
        ),
        HydratedAnchorAction::Clear
    );
    assert_eq!(
        hydrated_anchor_action(ChildAnchor::Invalid, &seeded, &seeded),
        HydratedAnchorAction::Clear
    );
}

#[test]
fn resolver_rejects_response_mismatches_fetch_failures_and_depth_overflow() {
    let child_id = CHILD.parse().unwrap();
    let mismatched = futures::executor::block_on(resolve_root_task(child_id, |_| {
        futures::future::ready(Ok(task(ROOT, None)))
    }));
    assert!(mismatched.is_err());

    let failed = futures::executor::block_on(resolve_root_task(child_id, |_| {
        futures::future::ready(Err(anyhow!("unauthorized")))
    }));
    assert!(failed.is_err());

    let ids = (1..=MAX_PARENT_EDGES + 2)
        .map(|value| {
            uuid::Uuid::from_u128(value as u128)
                .to_string()
                .parse::<AmbientAgentTaskId>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mut tasks = HashMap::new();
    for (index, task_id) in ids.iter().enumerate() {
        let parent = ids.get(index + 1).map(ToString::to_string);
        tasks.insert(*task_id, task(&task_id.to_string(), parent.as_deref()));
    }
    let overflow = futures::executor::block_on(resolve_root_task(ids[0], |id| {
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }));
    assert!(overflow.is_err());
}
