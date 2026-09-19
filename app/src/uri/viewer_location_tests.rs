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
        "{}/conversation/root?view=standalone&foo=bar#child={CHILD}",
        crate::ChannelState::server_root_url()
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
        format!(
            "{}/conversation/root?view=standalone&foo=bar",
            crate::ChannelState::server_root_url()
        )
    );
}

#[test]
fn standalone_value_and_child_key_are_case_sensitive() {
    let location = ViewerLocation::parse(
        &Url::parse(&format!(
            "{}/session/{ROOT}?view=Standalone#Child={CHILD}",
            crate::ChannelState::server_root_url()
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(!location.standalone);
    assert_eq!(location.child_anchor, ChildAnchor::Root);
}

#[test]
fn rejects_viewer_like_paths_with_extra_segments() {
    for path in [
        "/conversation/root/extra",
        "/session/33333333-3333-3333-3333-333333333333/extra",
    ] {
        assert!(
            ViewerLocation::parse(
                &Url::parse(&format!("{}{path}", crate::ChannelState::server_root_url())).unwrap()
            )
            .is_none()
        );
    }
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
                "{}/conversation/root#{fragment}",
                crate::ChannelState::server_root_url()
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
    let entry_task = tasks.remove(&child_id).unwrap();
    let mut fetched_task_ids = Vec::new();
    let resolved = futures::executor::block_on(resolve_root_task(entry_task, |id| {
        fetched_task_ids.push(id);
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }))
    .unwrap()
    .unwrap();
    assert_eq!(resolved.root_task.task_id, root_id);
    assert_eq!(resolved.entry_task_id, child_id);
    assert_eq!(fetched_task_ids, vec![root_id]);

    let mut cycle = HashMap::from([
        (child_id, task(CHILD, Some(ROOT))),
        (root_id, task(ROOT, Some(CHILD))),
    ]);
    let entry_task = cycle.remove(&child_id).unwrap();
    let result = futures::executor::block_on(resolve_root_task(entry_task, |id| {
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
    let entry_task = tasks.remove(&child_id).unwrap();
    let resolved = futures::executor::block_on(resolve_root_task(entry_task, |id| {
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }))
    .unwrap()
    .unwrap();
    assert_eq!(resolved.root_task.task_id, root_id);
    assert_eq!(resolved.entry_task_id, child_id);
}

#[test]
fn top_level_entry_is_not_canonicalized_as_a_child() {
    let resolved = futures::executor::block_on(resolve_root_task(task(ROOT, None), |_| {
        futures::future::ready(Err(anyhow!("root entries do not fetch a parent")))
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
fn anchor_waits_for_registration_and_verifies_unseeded_values() {
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
        HydratedAnchorAction::FetchAndVerify(ROOT.parse().unwrap())
    );
    assert_eq!(
        hydrated_anchor_action(ChildAnchor::Invalid, &seeded, &seeded),
        HydratedAnchorAction::Clear
    );
}

#[test]
fn anchor_omitted_from_first_hundred_requires_verified_direct_child_fetch() {
    let parent_task_id = ROOT.parse().unwrap();
    let child_task_id = CHILD.parse().unwrap();
    let first_page = (3..=102)
        .map(|value| {
            uuid::Uuid::from_u128(value)
                .to_string()
                .parse::<AmbientAgentTaskId>()
                .unwrap()
        })
        .collect::<HashSet<_>>();
    assert_eq!(first_page.len(), 100);
    assert_eq!(
        hydrated_anchor_action(
            ChildAnchor::Selected(child_task_id),
            &first_page,
            &HashSet::new()
        ),
        HydratedAnchorAction::FetchAndVerify(child_task_id)
    );
    assert_eq!(
        hydrated_anchor_action(
            ChildAnchor::Selected(child_task_id),
            &first_page,
            &HashSet::from([child_task_id])
        ),
        HydratedAnchorAction::Select(child_task_id)
    );

    assert!(is_expected_direct_child(
        &task(CHILD, Some(ROOT)),
        child_task_id,
        parent_task_id
    ));
    assert!(!is_expected_direct_child(
        &task(CHILD, Some("33333333-3333-3333-3333-333333333333")),
        child_task_id,
        parent_task_id
    ));
    assert!(!is_expected_direct_child(
        &task(CHILD, None),
        child_task_id,
        parent_task_id
    ));
    assert!(!is_expected_direct_child(
        &task(CHILD, Some("not-a-run-id")),
        child_task_id,
        parent_task_id
    ));
    assert!(!is_expected_direct_child(
        &task(ROOT, Some(ROOT)),
        child_task_id,
        parent_task_id
    ));
}

#[test]
fn resolver_rejects_response_mismatches_fetch_failures_and_depth_overflow() {
    let mismatched =
        futures::executor::block_on(resolve_root_task(task(CHILD, Some(ROOT)), |_| {
            futures::future::ready(Ok(task(CHILD, None)))
        }));
    assert!(mismatched.is_err());

    let failed = futures::executor::block_on(resolve_root_task(task(CHILD, Some(ROOT)), |_| {
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
    let entry_task = tasks.remove(&ids[0]).unwrap();
    let overflow = futures::executor::block_on(resolve_root_task(entry_task, |id| {
        futures::future::ready(tasks.remove(&id).ok_or_else(|| anyhow!("missing")))
    }));
    assert!(overflow.is_err());
}
