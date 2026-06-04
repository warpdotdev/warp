use crate::context_chips::available_chips;
use crate::context_chips::ContextChipKind;
use crate::ui_components::icons::Icon;
use std::sync::Arc;

use crate::context_chips::context_chip::GeneratorContext;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{BootstrapSessionType, Session, SessionInfo};

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

#[test]
fn test_jj_bookmark_shell_command() {
    let generator = super::jj_bookmark();
    assert_eq!(generator.dependencies(), &["jj".to_owned()]);
}

#[test]
fn test_jj_dirty_items_shell_command() {
    let generator = super::jj_dirty_items();
    assert_eq!(generator.dependencies(), &["jj".to_owned()]);
}

#[test]
fn test_available_chips_includes_jj_variants() {
    let chips = available_chips();
    assert!(chips.contains(&ContextChipKind::JjBookmark));
    assert!(chips.contains(&ContextChipKind::JjDirtyItems));
}

#[test]
fn test_jj_chips_udi_icon() {
    assert_eq!(
        ContextChipKind::JjBookmark.udi_icon(),
        Some(Icon::GitBranch)
    );
    assert_eq!(ContextChipKind::JjDirtyItems.udi_icon(), Some(Icon::File));
}

#[test]
fn test_jj_bookmark_placeholder() {
    assert_eq!(
        ContextChipKind::JjBookmark
            .placeholder_value()
            .as_text()
            .unwrap(),
        "jj-feature-bookmark"
    );
}

#[test]
fn test_jj_dirty_items_placeholder() {
    assert_eq!(
        ContextChipKind::JjDirtyItems
            .placeholder_value()
            .as_text()
            .unwrap(),
        "3"
    );
}

#[test]
fn test_jj_bookmark_display_value() {
    assert_eq!(
        ContextChipKind::JjBookmark.display_value(&"main".to_string().into()),
        "jj:(main)"
    );
}

#[test]
fn test_jj_dirty_items_display_value() {
    assert_eq!(
        ContextChipKind::JjDirtyItems.display_value(&"3".to_string().into()),
        "±3"
    );
}
