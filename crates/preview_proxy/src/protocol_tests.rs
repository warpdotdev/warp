use super::*;

fn snapshot(sequence: i64) -> Snapshot {
    Snapshot {
        session_id: "session".into(),
        revision: 1,
        lease_sequence: sequence,
        lease_seconds: 30,
        routes: vec![DesiredRoute {
            id: "route".into(),
            generation: 1,
            app_port: 3000,
            token: "a-bearer-token-of-at-least-32-characters".into(),
            valid_for_seconds: 300,
        }],
    }
}

fn reconciler() -> Reconciler {
    Reconciler {
        proxy: Proxy::default(),
        session: "session".into(),
        revision: -1,
        lease_sequence: -1,
        sandbox_deadline: None,
    }
}

#[tokio::test(start_paused = true)]
async fn delayed_snapshot_cannot_extend_lease_past_request_start() {
    let mut manager = reconciler();
    let started = Instant::now();
    tokio::time::advance(Duration::from_secs(29)).await;
    let ack = manager.apply(snapshot(1), started).await.unwrap();
    assert!(ack.routes[0].proxy_port.is_some());
    assert_eq!(
        *manager.proxy.listeners["route"].deadline.borrow(),
        started + MAX_LEASE
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(manager.apply(snapshot(2), started).await.is_none());
    manager.proxy.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn rejects_stale_duplicate_wrong_session_and_lower_revision_snapshots() {
    let mut manager = reconciler();
    let started = Instant::now();
    let port = manager.apply(snapshot(2), started).await.unwrap().routes[0].proxy_port;
    tokio::time::advance(Duration::from_secs(10)).await;
    for mut invalid in [snapshot(1), snapshot(2), snapshot(3), snapshot(4)] {
        if invalid.lease_sequence == 3 {
            invalid.session_id = "fenced".into();
        }
        if invalid.lease_sequence == 4 {
            invalid.revision = 0;
        }
        assert!(manager.apply(invalid, Instant::now()).await.is_none());
        assert_eq!(
            *manager.proxy.listeners["route"].deadline.borrow(),
            started + MAX_LEASE
        );
    }
    assert_eq!(
        manager
            .apply(snapshot(5), Instant::now())
            .await
            .unwrap()
            .routes[0]
            .proxy_port,
        port
    );
    assert_eq!(
        *manager.proxy.listeners["route"].deadline.borrow(),
        started + Duration::from_secs(40)
    );
    manager.proxy.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn authoritative_removal_and_sandbox_deadline_limit_all_routes() {
    let mut manager = reconciler();
    let started = Instant::now();
    manager.sandbox_deadline = Some(started + Duration::from_secs(15));
    manager.apply(snapshot(1), started).await.unwrap();
    assert_eq!(
        *manager.proxy.listeners["route"].deadline.borrow(),
        started + Duration::from_secs(15)
    );
    let mut removed = snapshot(2);
    removed.revision = 2;
    removed.routes.clear();
    manager.apply(removed, started).await.unwrap();
    assert!(manager.proxy.listeners.is_empty());
}
