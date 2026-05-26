use std::path::PathBuf;
use std::sync::Arc;

use warpui::r#async::executor::Background;

use super::{display_working_directory, format_session_location, should_render_oz_updates_section};
use crate::ai::blocklist::agent_view::zero_state_block::current_working_directory_for_zero_state;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{Handler, InitShellValue, PrecmdValue, SSHValue};
use crate::terminal::model::session::Session;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::model::TerminalModel;

fn terminal_with_startup_path(startup_path: Option<&str>) -> TerminalModel {
    TerminalModel::new_for_test(
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        startup_path.map(PathBuf::from),
    )
}

fn prebootstrap_terminal_with_startup_path(startup_path: &str) -> TerminalModel {
    let mut terminal = terminal_with_startup_path(Some(startup_path));
    terminal.block_list_mut().reinit_shell();
    terminal
}
#[test]
fn format_session_location_shows_path_only_for_local_sessions() {
    let session = Session::test();
    let formatted = format_session_location(&session, Some("/Users/alice/repo"));
    assert_eq!(formatted, Some("/Users/alice/repo".to_owned()));
}

#[test]
fn format_session_location_shows_user_host_for_remote_sessions() {
    let session = Session::test_remote();
    let formatted =
        format_session_location(&session, Some("/Users/alice/repo")).expect("path exists");
    assert!(formatted.starts_with(&format!("{}@{}:", session.user(), session.hostname())));
    assert!(formatted.ends_with("/Users/alice/repo"));
}

#[test]
fn format_session_location_preserves_windows_style_paths() {
    let session = Session::test_remote();
    let formatted =
        format_session_location(&session, Some(r"C:\Users\alice\repo")).expect("path exists");
    assert!(formatted.starts_with(&format!("{}@{}:", session.user(), session.hostname())));
    assert!(formatted.ends_with(r"C:\Users\alice\repo"));
}

#[test]
fn format_session_location_returns_none_when_path_missing() {
    let session = Session::test_remote();
    let formatted = format_session_location(&session, None);
    assert_eq!(formatted, None);
}

#[test]
fn display_working_directory_abbreviates_home_directory() {
    let display = display_working_directory(Some("/Users/alice"), Some("/Users/alice"));
    assert_eq!(display, Some("~".to_owned()));
}

#[test]
fn display_working_directory_abbreviates_subdirectory_under_home() {
    let display = display_working_directory(Some("/Users/alice/repo"), Some("/Users/alice"));
    assert_eq!(display, Some("~/repo".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_prefers_active_block_pwd() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    terminal.precmd(PrecmdValue {
        pwd: Some("/active/path".to_owned()),
        session_id: Some(0),
        ..Default::default()
    });

    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, Some("/active/path".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_uses_startup_path_before_bootstrap_for_local_session() {
    let terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, Some("/startup/path".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_for_pending_ssh_bootstrap() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    terminal.ssh(SSHValue::default());
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_for_pending_remote_session() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    terminal.init_shell(InitShellValue {
        session_id: 123.into(),
        shell: "zsh".to_owned(),
        hostname: "remote.example.com".to_owned(),
        ..Default::default()
    });

    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_after_bootstrap() {
    let terminal = terminal_with_startup_path(Some("/startup/path"));
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}

#[test]
fn oz_updates_section_renders_when_all_conditions_are_true() {
    assert!(should_render_oz_updates_section(true, true, true));
}

#[test]
fn oz_updates_section_does_not_render_when_setting_is_disabled() {
    assert!(!should_render_oz_updates_section(true, false, true));
}

#[test]
fn oz_updates_section_does_not_render_without_updates() {
    assert!(!should_render_oz_updates_section(true, true, false));
}

#[test]
fn oz_updates_section_does_not_render_when_feature_flag_is_disabled() {
    assert!(!should_render_oz_updates_section(false, true, true));
}
