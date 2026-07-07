use super::*;

fn worker(worker_host: &str) -> ConnectedSelfHostedWorker {
    ConnectedSelfHostedWorker {
        worker_host: worker_host.to_string(),
        connection_count: 1,
        connected_at: "2026-05-18T19:00:00Z".to_string(),
        last_seen_at: "2026-05-18T19:05:00Z".to_string(),
    }
}

#[test]
fn worker_hosts_excluding_sorts_dedups_and_filters_empty_and_warp_hosts() {
    let model = ConnectedSelfHostedWorkersModel {
        workers: vec![
            worker("worker-2"),
            worker(""),
            worker("warp"),
            worker("WARP"),
            worker("worker-1"),
            worker("worker-2"),
        ],
    };

    assert_eq!(
        model.worker_hosts_excluding(None),
        vec!["worker-1".to_string(), "worker-2".to_string()]
    );
}

#[test]
fn worker_hosts_excluding_filters_excluded_host() {
    let model = ConnectedSelfHostedWorkersModel {
        workers: vec![
            worker("default-host"),
            worker("worker-1"),
            worker("worker-2"),
        ],
    };

    assert_eq!(
        model.worker_hosts_excluding(Some("default-host")),
        vec!["worker-1".to_string(), "worker-2".to_string()]
    );
}

#[test]
fn clear_worker_cache_removes_cached_hosts() {
    let mut model = ConnectedSelfHostedWorkersModel {
        workers: vec![worker("private-host")],
    };

    assert!(model.clear_worker_cache());
    assert!(model.worker_hosts_excluding(None).is_empty());
}

#[test]
fn clear_worker_cache_is_noop_when_empty() {
    let mut model = ConnectedSelfHostedWorkersModel {
        workers: Vec::new(),
    };

    assert!(!model.clear_worker_cache());
}
