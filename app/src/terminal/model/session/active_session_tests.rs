use warp_core::HostId;

use super::*;

#[test]
fn disconnected_remote_session_does_not_fall_back_to_host_path() {
    assert_eq!(
        location_for_session_type(
            Some(SessionType::WarpifiedRemote { host_id: None }),
            "/tmp/secret-on-host",
        ),
        None
    );
}

#[test]
fn connected_remote_session_keeps_container_path() {
    let host_id = HostId::new("container-host".to_owned());
    let location = location_for_session_type(
        Some(SessionType::WarpifiedRemote {
            host_id: Some(host_id.clone()),
        }),
        "/workspaces/project/src/main.rs",
    );
    match location {
        Some(LocalOrRemotePath::Remote(remote)) => {
            assert_eq!(remote.host_id, host_id);
            assert_eq!(remote.path.as_str(), "/workspaces/project/src/main.rs");
        }
        other => panic!("expected remote path, got {other:?}"),
    }
}
