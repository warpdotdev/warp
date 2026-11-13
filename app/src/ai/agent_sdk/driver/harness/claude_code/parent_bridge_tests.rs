use super::*;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::MockAIClient;

fn waited_on_event(run_id: &str, waiter: Option<&str>) -> AgentRunEvent {
    serde_json::from_value(serde_json::json!({
        "event_type": "run_waited_on",
        "run_id": run_id,
        "ref_id": waiter,
        "occurred_at": "2026-11-12T00:00:00Z",
        "sequence": 42
    }))
    .unwrap()
}

#[test]
fn waited_on_is_surfaced_read_only_without_mailbox_reads_or_acknowledgments() {
    let server_api = ServerApiProvider::new_for_test().get();
    futures::executor::block_on(async move {
        let state = tempfile::tempdir().unwrap();
        ensure_parent_bridge_state_dir(state.path()).unwrap();
        let mut consumer = MessageBridgeEventConsumer {
            run_id: "root-run".to_string(),
            state_dir: state.path().to_path_buf(),
            server_api,
        };
        consumer
            .on_event(waited_on_event("root-run", Some("child-run")))
            .await
            .unwrap();
        consumer
            .on_event(waited_on_event("root-run", Some("child-run")))
            .await
            .unwrap();
        let staged =
            parent_bridge_message_records(&parent_bridge_staged_dir(state.path())).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].1.sender_run_id, "child-run");
        assert_eq!(staged[0].1.event_type.as_deref(), Some("run_waited_on"));

        let mut client = MockAIClient::new();
        client.expect_read_agent_message().times(0);
        client.expect_mark_message_delivered().times(0);
        let hydrator = MessageHydrator::new(Arc::new(client) as Arc<dyn AIClient>);
        prepare_parent_bridge_hook_output(&hydrator, state.path(), 6000)
            .await
            .unwrap();
        let output: MessageBridgeHookOutput = serde_json::from_slice(
            &fs::read(parent_bridge_hook_output_file(state.path())).unwrap(),
        )
        .unwrap();
        assert_eq!(output.surfaced_count, 1);
        assert!(
            output
                .additional_context
                .contains("Read-only lifecycle notification\n\nRun child-run is waiting for you")
        );
        assert!(
            !output
                .additional_context
                .contains("Lead agent mailbox message")
        );

        fs::write(parent_bridge_hook_output_ack_file(state.path()), "").unwrap();
        acknowledge_parent_bridge_hook_output(&hydrator, state.path())
            .await
            .unwrap();
        assert!(
            parent_bridge_message_records(&parent_bridge_surfaced_dir(state.path()))
                .unwrap()
                .is_empty()
        );
        assert!(!parent_bridge_hook_output_file(state.path()).exists());
        assert!(!parent_bridge_hook_output_ack_file(state.path()).exists());
    });
}

#[test]
fn waited_on_for_another_run_or_without_sender_is_not_staged() {
    let server_api = ServerApiProvider::new_for_test().get();
    futures::executor::block_on(async move {
        let state = tempfile::tempdir().unwrap();
        let mut consumer = MessageBridgeEventConsumer {
            run_id: "self-run".to_string(),
            state_dir: state.path().to_path_buf(),
            server_api,
        };
        consumer
            .on_event(waited_on_event("other-run", Some("waiter")))
            .await
            .unwrap();
        consumer
            .on_event(waited_on_event("self-run", None))
            .await
            .unwrap();
        consumer
            .on_event(waited_on_event("self-run", Some("")))
            .await
            .unwrap();
        assert!(
            parent_bridge_message_records(&parent_bridge_staged_dir(state.path()))
                .unwrap()
                .is_empty()
        );
    });
}
