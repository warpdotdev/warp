use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use serde_json::json;
use warpui::r#async::executor::Background;

use super::{OzRunTimelineEvent, SetupClientEventReporter};
use crate::server::server_api::ai::MockAIClient;

#[test]
fn agent_started_transports_known_zero_and_omits_unknown_setup() {
    for duration in [Some(Duration::ZERO), None] {
        let mut client = MockAIClient::new();
        client
            .expect_post_agent_run_client_event()
            .times(1)
            .return_once(move |_, request| {
                let value = serde_json::to_value(request).unwrap();
                assert_eq!(value["event_name"], "agent_started");
                if duration.is_some() {
                    assert_eq!(value["payload"]["user_setup"]["version"], 1);
                    assert_eq!(value["payload"]["user_setup"]["duration_us"], 0);
                    assert_eq!(value["payload"]["user_setup"]["had_setup_commands"], false);
                } else {
                    assert!(value.get("payload").is_none());
                }
                Ok(())
            });
        let reporter = SetupClientEventReporter::new(
            "019e3c43-885b-70a7-9d3c-a38ca1e7681d".parse().unwrap(),
            Arc::new(client),
            Arc::new(Background::default()),
        );
        reporter.set_user_setup(duration, false);
        block_on(
            reporter
                .clone()
                .post_timeline_event(OzRunTimelineEvent::AgentStarted),
        );
    }
}

#[test]
fn optional_oz_measurement_uses_legacy_envelope_and_failure_is_nonfatal() {
    let mut client = MockAIClient::new();
    client
        .expect_post_agent_run_client_event()
        .times(1)
        .return_once(|_, request| {
            let value = serde_json::to_value(request).unwrap();
            assert_eq!(value["event_name"], "startup_setup_measurement");
            let payload = &value["payload"];
            assert_eq!(payload["start_ts"], payload["finish_ts"]);
            assert_eq!(payload["latency_ms"], json!(0));
            assert_eq!(payload["is_error"], false);
            assert_eq!(payload["user_setup"]["duration_us"], 1500);
            assert_eq!(payload["user_setup"]["had_setup_commands"], true);
            Err(anyhow::anyhow!("unsupported optional event"))
        });
    let reporter = SetupClientEventReporter::new(
        "019e3c43-885b-70a7-9d3c-a38ca1e7681d".parse().unwrap(),
        Arc::new(client),
        Arc::new(Background::default()),
    );
    reporter.set_user_setup(Some(Duration::from_micros(1500)), true);
    block_on(reporter.post_startup_setup_measurement());
}
