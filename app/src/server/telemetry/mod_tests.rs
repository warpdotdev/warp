use rudder_message::Track;
use virtual_fs::VirtualFS;

use super::*;
use crate::workspaces::workspace::{OrganizationTelemetryPolicy, TelemetryEnablementSetting};

// Tests that events with UGC are not persisted to desk.
#[test]
fn test_persist_events_doesnt_include_ugc_events() {
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

#[test]
fn blocked_disk_persistence_drains_queue_and_deletes_existing_backlog() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let telemetry_api = TelemetryApi::new();

    VirtualFS::test(
        "blocked_disk_persistence_drains_queue_and_deletes_existing_backlog",
        |dirs, _sandbox| {
            let path = dirs.root().join("rudderstack");
            std::fs::write(&path, b"existing backlog").expect("should create backlog");
            warpui::telemetry::record_event(
                Some("user".into()),
                "anonymous_id".to_owned(),
                "blocked event".into(),
                None,
                false,
                warpui::time::get_current_time(),
            );

            telemetry_api
                .flush_and_persist_events_at_path(
                    10,
                    PrivacySettingsSnapshot::mock_with_organization_policy(
                        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
                    ),
                    &path,
                )
                .expect("blocked persistence should drop events");

            assert!(!path.exists());
            assert!(warpui::telemetry::flush_events().is_empty());
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
