use rudder_message::Track;
use serial_test::serial;
use virtual_fs::VirtualFS;

use super::*;

// Tests that events with UGC are not persisted to desk.
//
// `warpui::telemetry`'s event queue is a process-global, append-only cache
// (1024-cap deque). Other tests that emit telemetry (e.g. `ExperimentTriggered`
// from `experiments::mod.rs`) leave events queued for the rest of the process,
// so this test must clear the queue before recording its own events, and must
// run `#[serial]` so a concurrently-running event emitter can't repopulate it
// mid-test.
#[test]
#[serial]
fn test_persist_events_doesnt_include_ugc_events() {
    clear_event_queue();

    let telemetry_api = TelemetryApi::new();

    VirtualFS::test(
        "test_persist_events_doesnt_include_ugc_events",
        |dirs, _sandbox| {
            // Add one event without UGC
            let user_id = Some("user".into());
            let anonymous_id = "anonymous_id".to_owned();

            warpui::telemetry::record_event(
                user_id.clone(),
                anonymous_id.clone(),
                "non UGC event name".into(),
                None,  /* payload */
                false, /* contains_ugc  */
                warpui::time::get_current_time(),
            );

            warpui::telemetry::record_event(
                user_id.clone(),
                anonymous_id.clone(),
                "UGC event name".into(),
                None, /* payload */
                true, /* contains_ugc  */
                warpui::time::get_current_time(),
            );

            let file_path = dirs.root().join("rudderstack");

            telemetry_api
                .flush_and_persist_events_at_path(10, PrivacySettingsSnapshot::mock(), &file_path)
                .expect("Should be able to persist events");

            let file_content: Vec<RudderBatchMessage> =
                serde_json::from_reader(File::open(file_path).expect("Failed to open file"))
                    .expect("Failed to parse file");

            assert_eq!(file_content.len(), 1);

            let track = file_content[0].unwrap_track();
            assert_eq!(track.event, "non UGC event name");
        },
    );
}

impl RudderBatchMessage {
    fn unwrap_track(&self) -> &Track {
        match self {
            RudderBatchMessage::Track(track) => track,
            _ => panic!("Expected a track event"),
        }
    }
}
