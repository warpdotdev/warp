use warp_multi_agent_api as api;

use super::external_query_metadata;

fn slack_message(channel_name: &str) -> api::ExternalMessage {
    api::ExternalMessage {
        platform: Some(api::external_message::Platform::Slack(
            api::external_message::Slack {
                channel_name: channel_name.to_owned(),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

#[test]
fn external_query_metadata_joins_platform_and_container() {
    assert_eq!(
        external_query_metadata(&slack_message("eng")),
        "Slack • #eng"
    );
    assert_eq!(external_query_metadata(&slack_message("")), "Slack");
}

#[test]
fn external_query_metadata_appends_relative_time_when_timestamped() {
    let mut message = slack_message("eng");
    message.platform_timestamp = Some(prost_types::Timestamp {
        seconds: 0,
        nanos: 0,
    });
    let metadata = external_query_metadata(&message);
    assert!(
        metadata.starts_with("Slack • #eng • "),
        "unexpected metadata: {metadata}"
    );
    assert!(metadata.len() > "Slack • #eng • ".len());
}
