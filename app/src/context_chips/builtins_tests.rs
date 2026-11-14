use std::sync::Arc;

use crate::context_chips::context_chip::GeneratorContext;
use crate::context_chips::display_chip::OperatingSystemLogo;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{BootstrapSessionType, HostInfo, Session, SessionInfo};

#[test]
fn test_working_directory() {
    let session = Session::test();
    // SessionInfo forces the home directory in tests.
    let home_dir = session.home_dir().expect("Home dir is set in tests");

    let block_in_cwd = BlockMetadata::new(Some(session.id()), Some(format!("{home_dir}/projects")));

    assert_eq!(
        super::working_directory(&GeneratorContext {
            active_block_metadata: &block_in_cwd,
            active_session: Some(&session),
            current_environment: &Default::default(),
        })
        .as_ref()
        .and_then(|v| v.as_text()),
        Some("~/projects")
    );

    let block_outside_cwd = BlockMetadata::new(Some(session.id()), Some("/etc".to_string()));

    assert_eq!(
        super::working_directory(&GeneratorContext {
            active_block_metadata: &block_outside_cwd,
            active_session: Some(&session),
            current_environment: &Default::default(),
        })
        .as_ref()
        .and_then(|v| v.as_text()),
        Some("/etc")
    );
}

#[test]
fn test_remote_sessions() {
    let local_session = Session::test();
    let remote_session = Session::new(
        SessionInfo::new_for_test()
            .with_session_type(BootstrapSessionType::WarpifiedRemote)
            .with_hostname("remote-host".to_string())
            .with_user("remote-user".to_string()),
        Arc::new(TestCommandExecutor {}),
    );

    let local_ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(local_session.id()), None),
        active_session: Some(&local_session),
        current_environment: &Default::default(),
    };

    let remote_ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(remote_session.id()), None),
        active_session: Some(&remote_session),
        current_environment: &Default::default(),
    };

    // The Username and Hostname chips are always present.
    assert_eq!(
        super::username(&local_ctx)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("local:user")
    );
    assert_eq!(
        super::username(&remote_ctx)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("remote-user")
    );
    assert_eq!(
        super::hostname(&local_ctx)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("local:host")
    );
    assert_eq!(
        super::hostname(&remote_ctx)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("remote-host")
    );

    // The SSH chip is only shown for remote sessions.
    assert_eq!(super::ssh_session(&local_ctx), None);
    assert_eq!(
        super::ssh_session(&remote_ctx)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("remote-user@remote-host")
    );
}

#[test]
fn test_operating_system_prefers_linux_distribution() {
    let session = Session::new(
        SessionInfo::new_for_test().with_host_info(HostInfo {
            os_category: Some("Linux".to_string()),
            linux_distribution: Some("Fedora Linux".to_string()),
        }),
        Arc::new(TestCommandExecutor {}),
    );
    let ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(session.id()), None),
        active_session: Some(&session),
        current_environment: &Default::default(),
    };

    let Some(crate::context_chips::ChipValue::OperatingSystem(info)) =
        super::operating_system(&ctx)
    else {
        panic!("expected operating system chip value");
    };
    assert_eq!(info.name(), "Fedora Linux");
    assert_eq!(info.logo(), OperatingSystemLogo::Fedora);
}

#[test]
fn test_operating_system_uses_linux_fallback_for_unsupported_distro() {
    let session = Session::new(
        SessionInfo::new_for_test().with_host_info(HostInfo {
            os_category: Some("Linux".to_string()),
            linux_distribution: Some("Red Hat Enterprise Linux".to_string()),
        }),
        Arc::new(TestCommandExecutor {}),
    );
    let ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(session.id()), None),
        active_session: Some(&session),
        current_environment: &Default::default(),
    };

    let Some(crate::context_chips::ChipValue::OperatingSystem(info)) =
        super::operating_system(&ctx)
    else {
        panic!("expected operating system chip value");
    };
    assert_eq!(info.name(), "Red Hat Enterprise Linux");
    assert_eq!(info.logo(), OperatingSystemLogo::Linux);
}

#[test]
fn test_operating_system_uses_os_category_when_distribution_is_absent() {
    let session = Session::new(
        SessionInfo::new_for_test().with_host_info(HostInfo {
            os_category: Some("MacOS".to_string()),
            linux_distribution: None,
        }),
        Arc::new(TestCommandExecutor {}),
    );
    let ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(session.id()), None),
        active_session: Some(&session),
        current_environment: &Default::default(),
    };

    let Some(crate::context_chips::ChipValue::OperatingSystem(info)) =
        super::operating_system(&ctx)
    else {
        panic!("expected operating system chip value");
    };
    assert_eq!(info.name(), "macOS");
    assert_eq!(info.logo(), OperatingSystemLogo::MacOS);
}

#[test]
fn test_operating_system_uses_unknown_fallback_when_unidentified() {
    let session = Session::test();
    let ctx = GeneratorContext {
        active_block_metadata: &BlockMetadata::new(Some(session.id()), None),
        active_session: Some(&session),
        current_environment: &Default::default(),
    };

    let Some(crate::context_chips::ChipValue::OperatingSystem(info)) =
        super::operating_system(&ctx)
    else {
        panic!("expected operating system chip value");
    };
    assert_eq!(info.name(), "Unknown OS");
    assert_eq!(info.logo(), super::OperatingSystemLogo::Unknown);
}

#[test]
fn test_node_version() {
    use crate::context_chips::context_chip::Environment;
    use crate::terminal::model::block::BlockMetadata;
    use crate::terminal::model::session::Session;

    let session = Session::test();
    let block_metadata = BlockMetadata::new(Some(session.id()), None);

    // Test with no node version
    let environment_no_node = Environment::default();
    let ctx_no_node = GeneratorContext {
        active_block_metadata: &block_metadata,
        active_session: Some(&session),
        current_environment: &environment_no_node,
    };
    assert_eq!(super::node_version(&ctx_no_node), None);

    // Test with node version - create environment with node version
    let environment_with_node = Environment::new(
        None,                        // virtual_env
        None,                        // conda_env
        Some("v18.0.0".to_string()), // node_version
    );
    let ctx_with_node = GeneratorContext {
        active_block_metadata: &block_metadata,
        active_session: Some(&session),
        current_environment: &environment_with_node,
    };
    assert_eq!(
        super::node_version(&ctx_with_node)
            .as_ref()
            .and_then(|v| v.as_text()),
        Some("v18.0.0")
    );
}
