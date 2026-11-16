use super::*;

#[test]
fn enabled_support_uses_remote_server_for_ssh_wrapper_when_feature_is_enabled() {
    assert!(SshRemoteServerSupport::Enabled.should_use_remote_server(true, true,));
}

#[test]
fn disabled_support_skips_remote_server_for_ssh_wrapper() {
    assert!(!SshRemoteServerSupport::Disabled.should_use_remote_server(true, true,));
}

#[test]
fn enabled_support_skips_remote_server_when_feature_is_disabled() {
    assert!(!SshRemoteServerSupport::Enabled.should_use_remote_server(false, true,));
}

#[test]
fn enabled_support_skips_remote_server_for_non_ssh_session() {
    assert!(!SshRemoteServerSupport::Enabled.should_use_remote_server(true, false,));
}
