use super::*;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

fn worker(worker_host: &str) -> ConnectedSelfHostedWorker {
    ConnectedSelfHostedWorker {
        worker_host: worker_host.to_string(),
        connection_count: 1,
        connected_at: "2026-05-18T19:00:00Z".to_string(),
        last_seen_at: "2026-05-18T19:05:00Z".to_string(),
    }
}

fn scope(team_uid: i64) -> TeamContextForOperation {
    TeamContextForOperation::new_for_test(ServerId::from(team_uid))
}

#[test]
fn worker_hosts_excluding_sorts_dedups_and_filters_empty_and_warp_hosts() {
    let scope = scope(1);
    let model = ConnectedSelfHostedWorkersModel {
        workers_by_team: HashMap::from([(
            scope.team_uid().unwrap(),
            vec![
                worker("worker-2"),
                worker(""),
                worker("warp"),
                worker("WARP"),
                worker("worker-1"),
                worker("worker-2"),
            ],
        )]),
    };

    assert_eq!(
        model.worker_hosts_excluding(&scope, None),
        vec!["worker-1".to_string(), "worker-2".to_string()]
    );
}

#[test]
fn worker_hosts_excluding_filters_excluded_host() {
    let scope = scope(2);
    let model = ConnectedSelfHostedWorkersModel {
        workers_by_team: HashMap::from([(
            scope.team_uid().unwrap(),
            vec![
                worker("default-host"),
                worker("worker-1"),
                worker("worker-2"),
            ],
        )]),
    };

    assert_eq!(
        model.worker_hosts_excluding(&scope, Some("default-host")),
        vec!["worker-1".to_string(), "worker-2".to_string()]
    );
}

#[test]
fn worker_hosts_are_isolated_by_team_and_empty_for_teamless_scope() {
    let team_a = scope(3);
    let team_b = scope(4);
    let model = ConnectedSelfHostedWorkersModel {
        workers_by_team: HashMap::from([
            (team_a.team_uid().unwrap(), vec![worker("team-a-host")]),
            (team_b.team_uid().unwrap(), vec![worker("team-b-host")]),
        ]),
    };

    assert_eq!(
        model.worker_hosts_excluding(&team_a, None),
        vec!["team-a-host".to_string()]
    );
    assert_eq!(
        model.worker_hosts_excluding(&team_b, None),
        vec!["team-b-host".to_string()]
    );
    assert!(
        model
            .worker_hosts_excluding(&TeamlessScopeForTest, None)
            .is_empty()
    );
}

#[test]
fn clear_worker_cache_removes_all_team_hosts() {
    let team_a = scope(5);
    let team_b = scope(6);
    let mut model = ConnectedSelfHostedWorkersModel {
        workers_by_team: HashMap::from([
            (team_a.team_uid().unwrap(), vec![worker("team-a-host")]),
            (team_b.team_uid().unwrap(), vec![worker("team-b-host")]),
        ]),
    };

    assert!(model.clear_worker_cache());
    assert!(model.worker_hosts_excluding(&team_a, None).is_empty());
    assert!(model.worker_hosts_excluding(&team_b, None).is_empty());
}

#[test]
fn clear_worker_cache_is_noop_when_empty() {
    let mut model = ConnectedSelfHostedWorkersModel {
        workers_by_team: HashMap::new(),
    };

    assert!(!model.clear_worker_cache());
}
