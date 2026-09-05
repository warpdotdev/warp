use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "local_tty")]
use warpui::SingletonEntity;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewContext};

use super::command_executor::testing::TestCommandExecutor;
use super::{
    BootstrapSessionType, Session, SessionId, SessionInfo, SessionType, Sessions, SessionsEvent,
    get_local_hostname,
};

struct TestView {
    events: Vec<SessionsEvent>,
}

impl Entity for TestView {
    type Event = usize;
}

impl View for TestView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

impl TestView {
    fn new(model: ModelHandle<Sessions>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&model, |me, _, event, _| {
            me.events.push(event.to_owned());
        });
        Self { events: Vec::new() }
    }
}

#[test]
fn test_set_env_var_emits_event() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
            let expected_session_id = session_id;
            let event = view.events.first().expect("checked length already");
            if let SessionsEvent::EnvironmentVariablesUpdated { session_id } = event {
                assert_eq!(*session_id, expected_session_id);
            } else {
                assert!(matches!(
                    event,
                    SessionsEvent::EnvironmentVariablesUpdated { .. }
                ));
            }
        });
    });
}

#[test]
fn test_set_env_var_emits_no_event_when_no_change() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });

        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });
    });
}

#[test]
fn test_malicious_histfile_path_does_not_execute_injected_commands() {
    App::test((), |_app| async move {
        // If escaping is missing, `touch /tmp/warp_injection_test` would execute
        // as a side effect of reading history.
        let marker = "/tmp/warp_injection_test";
        // Clean up in case a previous broken run left the marker.
        let _ = std::fs::remove_file(marker);

        let malicious_histfile = format!("/tmp/x'; touch {marker}; echo '");

        let session_info = SessionInfo::new_for_test()
            .with_session_type(BootstrapSessionType::WarpifiedRemote)
            .with_histfile(Some(malicious_histfile));
        let session = Session::new(session_info, Arc::new(TestCommandExecutor::default()));

        // read_history for a WarpifiedRemote session calls read_history_from_file,
        // which builds `cat '{escaped_path}'` and executes it via TestCommandExecutor
        let _ = session.read_history(false).await;

        assert!(
            !std::path::Path::new(marker).exists(),
            "Injected command executed — escaping regression!"
        );
    });
}

#[cfg(not(windows))]
#[test]
fn can_resolve_cwd_to_native_path_accepts_posix_path() {
    let session = Session::test();
    assert!(session.can_resolve_cwd_to_native_path("/Users/foo/bar"));
}

#[cfg(windows)]
#[test]
fn can_resolve_cwd_to_native_path_accepts_windows_drive_path() {
    let session = Session::test();
    assert!(session.can_resolve_cwd_to_native_path(r"E:\CLAUDE-BASE"));
}

#[cfg(windows)]
#[test]
fn can_resolve_cwd_to_native_path_rejects_unix_encoded_path_on_windows() {
    let session_info =
        SessionInfo::new_for_test().with_shell_type(crate::terminal::shell::ShellType::Bash);
    let session = Session::new(session_info, Arc::new(TestCommandExecutor::default()));
    assert!(!session.can_resolve_cwd_to_native_path("/E:/CLAUDE-BASE"));
}

fn dev_container_launch_data(session_id: SessionId) -> crate::terminal::ShellLaunchData {
    crate::terminal::ShellLaunchData::DevContainer {
        workspace_folder: PathBuf::from("/host/project"),
        docker_path: PathBuf::from("/usr/bin/docker"),
        container_id: "abc123".to_owned(),
        remote_user: Some("vscode".to_owned()),
        remote_workspace_folder: "/workspaces/project".to_owned(),
        sandbox_id: "sandbox".to_owned(),
        session_id,
    }
}

fn init_shell(hostname: &str) -> crate::terminal::model::ansi::InitShellValue {
    crate::terminal::model::ansi::InitShellValue {
        session_id: SessionId::from(7),
        shell: "bash".to_owned(),
        is_subshell: false,
        user: "vscode".to_owned(),
        hostname: hostname.to_owned(),
        wsl_name: None,
    }
}

#[cfg(all(not(feature = "remote_tty"), feature = "local_tty"))]
#[test]
fn create_pending_classifies_dev_container_as_remote_when_hostnames_match() {
    let hostname = get_local_hostname().unwrap_or_else(|_| "testhost".to_owned());
    let info = SessionInfo::create_pending(
        crate::terminal::shell::ShellType::Bash,
        init_shell(&hostname),
        None,
        Some(dev_container_launch_data(SessionId::from(7))),
        None,
        None,
    );
    assert_eq!(info.session_type, BootstrapSessionType::WarpifiedRemote);
    assert!(!matches!(
        info.is_ssh_wrapper_session,
        super::IsSSHWrapperSession::Yes { .. }
    ));
}

#[cfg(all(not(feature = "remote_tty"), feature = "local_tty"))]
#[test]
fn create_pending_classifies_dev_container_as_remote_when_hostnames_differ() {
    let info = SessionInfo::create_pending(
        crate::terminal::shell::ShellType::Bash,
        init_shell("container-host"),
        None,
        Some(dev_container_launch_data(SessionId::from(7))),
        None,
        None,
    );
    assert_eq!(info.session_type, BootstrapSessionType::WarpifiedRemote);
}

#[cfg(all(not(feature = "remote_tty"), feature = "local_tty"))]
#[test]
fn create_pending_preserves_local_classification_for_matching_hostnames() {
    let Ok(hostname) = get_local_hostname() else {
        return;
    };
    let info = SessionInfo::create_pending(
        crate::terminal::shell::ShellType::Bash,
        init_shell(&hostname),
        None,
        Some(crate::terminal::ShellLaunchData::Executable {
            executable_path: PathBuf::from("/bin/bash"),
            shell_type: crate::terminal::shell::ShellType::Bash,
        }),
        None,
        None,
    );
    assert_eq!(info.session_type, BootstrapSessionType::Local);
}

#[cfg(all(not(feature = "remote_tty"), feature = "local_tty"))]
#[test]
fn create_pending_preserves_ssh_classification_for_matching_hostnames() {
    let hostname = get_local_hostname().unwrap_or_else(|_| "testhost".to_owned());
    let info = SessionInfo::create_pending(
        crate::terminal::shell::ShellType::Bash,
        init_shell(&hostname),
        None,
        None,
        Some(crate::terminal::model::ansi::SSHValue {
            socket_path: PathBuf::from("/tmp/ssh.sock"),
            remote_shell: "bash".to_owned(),
            session_id: Default::default(),
            remote_session_id: Default::default(),
            external_control_master: false,
        }),
        None,
    );
    assert_eq!(info.session_type, BootstrapSessionType::WarpifiedRemote);
}

#[cfg(feature = "local_tty")]
#[test]
fn session_origin_uses_remote_server_for_dev_container_when_flag_enabled() {
    let _flag = crate::features::FeatureFlag::LocalDevContainer.override_enabled(true);
    let info =
        SessionInfo::new_for_test().with_launch_data(dev_container_launch_data(SessionId::from(0)));
    assert!(super::session_origin_uses_remote_server(&info));
}

#[test]
fn remote_host_id_attaches_and_clears_without_local_fallback() {
    let info = SessionInfo::new_for_test()
        .with_session_type(BootstrapSessionType::WarpifiedRemote)
        .with_launch_data(dev_container_launch_data(SessionId::from(0)));
    let session = Session::new(info, Arc::new(TestCommandExecutor::default()));
    assert!(matches!(
        session.session_type(),
        SessionType::WarpifiedRemote { host_id: None }
    ));

    session.set_remote_host_id(Some(warp_core::HostId::new("container-host".to_owned())));
    match session.session_type() {
        SessionType::WarpifiedRemote { host_id: Some(id) } => {
            assert_eq!(id.as_str(), "container-host");
        }
        other => panic!("expected connected remote session, got {other:?}"),
    }

    session.set_remote_host_id(None);
    assert!(matches!(
        session.session_type(),
        SessionType::WarpifiedRemote { host_id: None }
    ));
}

#[cfg(feature = "local_tty")]
#[test]
fn disconnect_clears_host_id_until_reconnect_handshake() {
    let _dc = crate::features::FeatureFlag::LocalDevContainer.override_enabled(true);
    let _ssh = crate::features::FeatureFlag::SshRemoteServer.override_enabled(false);
    App::test((), |mut app| async move {
        app.add_singleton_model(crate::remote_server::manager::RemoteServerManager::new);
        let (tx, _rx) = async_channel::unbounded();
        let sessions = app.add_model(|ctx| Sessions::new(tx, ctx));
        let session_id = SessionId::from(11);
        let info = SessionInfo::new_for_test()
            .with_id(session_id)
            .with_session_type(BootstrapSessionType::WarpifiedRemote)
            .with_launch_data(dev_container_launch_data(session_id));
        sessions.update(&mut app, |sessions, _ctx| {
            sessions.register_session_for_test(info);
        });

        let connected = warp_core::HostId::new("container-a".to_owned());
        crate::remote_server::manager::RemoteServerManager::handle(&app).update(
            &mut app,
            |_mgr, ctx| {
                ctx.emit(
                    crate::remote_server::manager::RemoteServerManagerEvent::SessionConnected {
                        session_id,
                        host_id: connected.clone(),
                    },
                );
            },
        );
        sessions.read(&app, |sessions, _ctx| {
            let session = sessions.get(session_id).expect("session registered");
            match session.session_type() {
                SessionType::WarpifiedRemote {
                    host_id: Some(host_id),
                } => assert_eq!(host_id.as_str(), "container-a"),
                other => panic!("expected connected host id, got {other:?}"),
            }
        });

        crate::remote_server::manager::RemoteServerManager::handle(&app).update(
            &mut app,
            |_mgr, ctx| {
                ctx.emit(
                    crate::remote_server::manager::RemoteServerManagerEvent::SessionDisconnected {
                        session_id,
                        host_id: connected.clone(),
                        exit_status: None,
                        was_reconnect_attempt: false,
                    },
                );
            },
        );
        sessions.read(&app, |sessions, _ctx| {
            let session = sessions.get(session_id).expect("session registered");
            assert!(
                matches!(
                    session.session_type(),
                    SessionType::WarpifiedRemote { host_id: None }
                ),
                "host id must be cleared during the reconnect window"
            );
        });

        crate::remote_server::manager::RemoteServerManager::handle(&app).update(
            &mut app,
            |_mgr, ctx| {
                ctx.emit(
                    crate::remote_server::manager::RemoteServerManagerEvent::SessionConnected {
                        session_id,
                        host_id: warp_core::HostId::new("container-b".to_owned()),
                    },
                );
            },
        );
        sessions.read(&app, |sessions, _ctx| {
            let session = sessions.get(session_id).expect("session registered");
            match session.session_type() {
                SessionType::WarpifiedRemote {
                    host_id: Some(host_id),
                } => assert_eq!(host_id.as_str(), "container-b"),
                other => panic!("expected handshake host id, got {other:?}"),
            }
        });
    });
}

#[cfg(feature = "local_tty")]
#[test]
fn session_origin_does_not_use_remote_server_for_dev_container_when_flag_disabled() {
    let _flag = crate::features::FeatureFlag::LocalDevContainer.override_enabled(false);
    let _ssh = crate::features::FeatureFlag::SshRemoteServer.override_enabled(false);
    let info =
        SessionInfo::new_for_test().with_launch_data(dev_container_launch_data(SessionId::from(0)));
    assert!(!super::session_origin_uses_remote_server(&info));
}

#[cfg(windows)]
#[test]
fn powershell_read_command_embeds_escaped_path_without_args() {
    use std::ffi::{OsStr, OsString};

    use super::powershell_read_all_text_command;

    // The path is embedded directly inside a single-quoted PowerShell literal.
    let raw = r"C:\Users\dev\AppData\Roaming\Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt";
    let command = powershell_read_all_text_command(OsStr::new(raw));
    assert_eq!(
        command,
        OsString::from(format!("[System.IO.File]::ReadAllText('{raw}')"))
    );

    // A single quote in the path is doubled so it can't terminate the literal.
    let command = powershell_read_all_text_command(OsStr::new(r"C:\o'brien\history.txt"));
    assert_eq!(
        command,
        OsString::from(r"[System.IO.File]::ReadAllText('C:\o''brien\history.txt')")
    );
}
