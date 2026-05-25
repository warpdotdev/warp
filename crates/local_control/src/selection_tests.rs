use chrono::Utc;

use super::*;
use crate::discovery::{ControlEndpoint, CredentialBrokerReference};
use crate::protocol::{ActionKind, PROTOCOL_VERSION};

fn record(id: &str, pid: u32) -> InstanceRecord {
    InstanceRecord {
        protocol_version: PROTOCOL_VERSION,
        instance_id: InstanceId(id.to_owned()),
        pid,
        channel: "local".to_owned(),
        app_id: "dev.warp.WarpLocal".to_owned(),
        app_version: None,
        started_at: Utc::now(),
        executable_path: None,
        endpoint: Some(ControlEndpoint::localhost(4000)),
        credential_broker: Some(CredentialBrokerReference {
            endpoint: ControlEndpoint::localhost(4000),
        }),
        outside_warp_control_enabled: true,
        actions: vec![ActionKind::TabCreate.metadata()],
    }
}

#[test]
fn selects_instance_by_id() {
    let records = vec![record("one", 1), record("two", 2)];
    let selected = select_instance(&records, &InstanceSelector::Id(InstanceId("two".into())))
        .expect("selected");
    assert_eq!(selected.pid, 2);
}

#[test]
fn active_selector_rejects_ambiguity() {
    let records = vec![record("one", 1), record("two", 2)];
    let err = select_instance(&records, &InstanceSelector::Active).expect_err("ambiguous");
    assert_eq!(err.code, ErrorCode::AmbiguousInstance);
}

#[test]
fn active_selector_rejects_no_instances() {
    let err = select_instance(&[], &InstanceSelector::Active).expect_err("no instance");
    assert_eq!(err.code, ErrorCode::NoInstance);
}
