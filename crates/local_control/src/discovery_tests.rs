use std::fs;
use std::path::Path;

#[cfg(unix)]
use command::blocking::Command;

use super::*;

#[test]
fn control_endpoint_composes_loopback_control_route() {
    assert_eq!(
        ControlEndpoint::localhost(4000).url(),
        "http://127.0.0.1:4000/v1/control"
    );
}
#[test]
fn broker_socket_reference_is_bound_to_instance_identity() {
    let record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );

    assert_eq!(
        record
            .credential_broker
            .expect("credential broker")
            .socket_path,
        PathBuf::from(format!("{}.broker.sock", record.instance_id.0))
    );
}

#[test]
fn registered_instance_round_trips_discovery_record() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    let _registered = RegisteredInstance::register_in_dir_for_test(record.clone(), dir.path())
        .expect("registered");
    let records = list_instances_from_dir(dir.path());
    assert_eq!(records, vec![record]);
}

#[test]
fn incompatible_protocol_record_is_ignored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    record.protocol_version = PROTOCOL_VERSION + 1;
    let _registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");

    assert!(list_instances_from_dir(dir.path()).is_empty());
}
#[cfg(unix)]
#[test]
fn stale_process_record_is_pruned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut child = Command::new("true")
        .spawn()
        .expect("short-lived process starts");
    let pid = child.id();
    child.wait().expect("short-lived process exits");
    let mut record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    record.pid = pid;
    let registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");

    assert!(list_instances_from_dir(dir.path()).is_empty());
    assert!(!registered.path.exists());
}
#[cfg(unix)]
#[test]
fn multiple_live_process_records_are_discovered() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut first_process = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("first process starts");
    let mut second_process = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("second process starts");
    let mut first_record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    first_record.pid = first_process.id();
    let mut second_record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4001)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    second_record.pid = second_process.id();
    let first_id = first_record.instance_id.clone();
    let second_id = second_record.instance_id.clone();
    let _first = RegisteredInstance::register_in_dir_for_test(first_record, dir.path())
        .expect("first registered");
    let _second = RegisteredInstance::register_in_dir_for_test(second_record, dir.path())
        .expect("second registered");

    let ids = list_instances_from_dir(dir.path())
        .into_iter()
        .map(|record| record.instance_id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));

    first_process.kill().expect("first process stops");
    first_process.wait().expect("first process reaped");
    second_process.kill().expect("second process stops");
    second_process.wait().expect("second process reaped");
}

#[test]
fn serialized_discovery_record_does_not_contain_raw_credential_material() {
    let raw_secret = "raw-secret-token-material";
    let record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    let serialized = serde_json::to_string_pretty(&record).expect("serialize");
    assert!(!serialized.contains(raw_secret));
    assert!(!serialized.contains("auth_token"));
    assert!(!serialized.contains("bearer_token"));
}

#[test]
fn disabled_outside_warp_record_does_not_expose_actionable_authority() {
    let record = InstanceRecord::for_current_process(
        None,
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    assert!(!record.outside_warp_control_enabled);
    assert!(record.endpoint.is_none());
    assert!(record.credential_broker.is_none());
}

#[test]
fn rejects_unsafe_or_divergent_discovery_authority() {
    let mut record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    record
        .validate_local_control_authority()
        .expect("matching 127.0.0.1 endpoints are accepted");

    record.endpoint.as_mut().expect("endpoint").host = "localhost".to_owned();
    let err = record
        .validate_local_control_authority()
        .expect_err("localhost alias is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);

    record.endpoint = Some(ControlEndpoint::localhost(4000));
    record
        .credential_broker
        .as_mut()
        .expect("credential broker")
        .socket_path = "different.broker.sock".into();
    let err = record
        .validate_local_control_authority()
        .expect_err("divergent broker socket is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[cfg(unix)]
#[test]
fn discovery_directory_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    let _registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");
    let mode = fs::metadata(dir.path())
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[cfg(unix)]
#[test]
fn discovery_record_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let record = InstanceRecord::for_current_process(
        Some(ControlEndpoint::localhost(4000)),
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    );
    let registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");
    let mode = fs::metadata(&registered.path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

impl RegisteredInstance {
    fn register_in_dir_for_test(record: InstanceRecord, dir: &Path) -> Result<Self, ControlError> {
        fs::create_dir_all(dir).expect("create dir");
        set_private_dir_permissions(dir)?;
        let path = record_path(dir, &record.instance_id);
        write_record(&path, &record)?;
        Ok(Self {
            record,
            path,
            broker_socket_path: None,
        })
    }
}
