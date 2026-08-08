use rudder_message::Track;
use virtual_fs::VirtualFS;

use super::*;

// Tests that events with UGC are not persisted to disk.
//
// Drives `persist_events_at_path` with a locally-owned event list rather than
// the process-global `warpui::telemetry` queue. Any concurrently-running test
// that emits telemetry (e.g. `ExperimentTriggered` from `experiments::mod.rs`)
// pushes onto that global queue, so a flush-based assertion on its exact
// contents is not achievable in a shared process regardless of clearing or
// serialization.
#[test]
fn test_persist_events_doesnt_include_ugc_events() {
    let telemetry_api = TelemetryApi::new();

    VirtualFS::test(
        "test_persist_events_doesnt_include_ugc_events",
        |dirs, _sandbox| {
            let user_id = Some("user".into());
            let anonymous_id = "anonymous_id".to_owned();

            let events = vec![
                warpui::telemetry::create_event(
                    user_id.clone(),
                    anonymous_id.clone(),
                    "non UGC event name".into(),
                    None,  /* payload */
                    false, /* contains_ugc */
                    warpui::time::get_current_time(),
                ),
                warpui::telemetry::create_event(
                    user_id.clone(),
                    anonymous_id.clone(),
                    "UGC event name".into(),
                    None, /* payload */
                    true, /* contains_ugc */
                    warpui::time::get_current_time(),
                ),
            ];

            let file_path = dirs.root().join("rudderstack");
            let file = File::create(&file_path).expect("Should be able to create file");

            telemetry_api
                .persist_events_at_path(&file, 10, events)
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
