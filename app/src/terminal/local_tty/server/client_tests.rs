use std::{
    collections::HashSet,
    os::unix::{io::OwnedFd, net::UnixStream, prelude::AsRawFd, prelude::RawFd},
    sync::Arc,
};

use nix::unistd::dup;
use parking_lot::Mutex;

use super::super::{api, protocol};
use super::TerminalServerClient;
use crate::terminal::{
    local_tty::{
        shell::{DirectShellStarter, ShellStarter},
        PtyOptions, PtySpawnResult,
    },
    shell::ShellType,
    SizeInfo,
};

fn client_and_server_stream() -> (TerminalServerClient, UnixStream) {
    let (client_stream, server_stream) = UnixStream::pair().expect("failed to create socket pair");
    let client_fd: OwnedFd = client_stream.into();
    let client = TerminalServerClient::new(client_fd, Arc::new(Mutex::new(HashSet::new())));
    (client, server_stream)
}

fn pty_options() -> PtyOptions {
    PtyOptions {
        size: SizeInfo::new_without_font_metrics(24, 80),
        window_id: None,
        shell_starter: ShellStarter::Direct(DirectShellStarter::new_for_test(
            ShellType::Zsh,
            "/bin/zsh".into(),
            Vec::new(),
        )),
        start_dir: None,
        env_vars: Default::default(),
        enable_ssh_wrapper: false,
        shell_debug_mode: false,
        honor_ps1: false,
        close_fds: true,
    }
}

fn send_spawn_response(socket_fd: RawFd, request_id: api::RequestId, pid: u32) {
    let response_fd = dup(socket_fd).expect("failed to duplicate response fd");
    protocol::send_message(
        socket_fd,
        api::Message::SpawnShellResponse {
            request_id,
            spawn_result: api::Result::Ok(PtySpawnResult { pid, leader_fd: -1 }),
        },
        Some(response_fd),
    )
    .expect("failed to send spawn response");
    nix::unistd::close(response_fd).expect("failed to close local response fd");
}

#[test]
fn spawn_pty_ignores_stale_kill_child_response() {
    let (client, server_stream) = client_and_server_stream();
    let server_fd = server_stream.as_raw_fd();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let request = protocol::receive_message(server_fd).expect("failed to receive request");
            let request_id = match request {
                Some(api::Message::SpawnShellRequest { request_id, .. }) => request_id,
                _ => panic!("expected spawn request"),
            };

            protocol::send_message(
                server_fd,
                api::Message::KillChildResponse {
                    request_id: 0,
                    error_msg: None,
                },
                Option::<RawFd>::None,
            )
            .expect("failed to send stale kill response");
            send_spawn_response(server_fd, request_id, 42);
        });

        let result = client
            .spawn_pty(pty_options())
            .expect("spawn should recover from stale response");

        assert_eq!(result.pid, 42);
        assert_ne!(result.leader_fd, -1);
        nix::unistd::close(result.leader_fd).expect("failed to close received fd");
    });
}

#[test]
fn spawn_pty_ignores_stale_spawn_shell_response() {
    let (client, server_stream) = client_and_server_stream();
    let server_fd = server_stream.as_raw_fd();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            send_spawn_response(server_fd, 0, 13);

            let request = protocol::receive_message(server_fd).expect("failed to receive request");
            let request_id = match request {
                Some(api::Message::SpawnShellRequest { request_id, .. }) => request_id,
                _ => panic!("expected spawn request"),
            };

            send_spawn_response(server_fd, request_id, 42);
        });

        let result = client
            .spawn_pty(pty_options())
            .expect("spawn should ignore stale spawn response");

        assert_eq!(result.pid, 42);
        assert_ne!(result.leader_fd, -1);
        nix::unistd::close(result.leader_fd).expect("failed to close received fd");
    });
}

#[test]
fn spawn_pty_handles_children_terminated_request_while_waiting_for_response() {
    let (client, server_stream) = client_and_server_stream();
    let server_fd = server_stream.as_raw_fd();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let request = protocol::receive_message(server_fd).expect("failed to receive request");
            let request_id = match request {
                Some(api::Message::SpawnShellRequest { request_id, .. }) => request_id,
                _ => panic!("expected spawn request"),
            };

            protocol::send_message(
                server_fd,
                api::Message::ChildrenTerminatedRequest { pids: vec![7] },
                Option::<RawFd>::None,
            )
            .expect("failed to send children terminated request");
            send_spawn_response(server_fd, request_id, 42);
        });

        let result = client
            .spawn_pty(pty_options())
            .expect("spawn should tolerate terminal server notifications");

        assert_eq!(result.pid, 42);
        assert!(client.has_child_terminated(7));
        assert_ne!(result.leader_fd, -1);
        nix::unistd::close(result.leader_fd).expect("failed to close received fd");
    });
}

#[test]
fn kill_child_ignores_stale_spawn_shell_response() {
    let (client, server_stream) = client_and_server_stream();
    let server_fd = server_stream.as_raw_fd();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let request = protocol::receive_message(server_fd).expect("failed to receive request");
            let request_id = match request {
                Some(api::Message::KillChildRequest { request_id, pid }) => {
                    assert_eq!(pid, 9);
                    request_id
                }
                _ => panic!("expected kill child request"),
            };

            send_spawn_response(server_fd, 0, 13);
            protocol::send_message(
                server_fd,
                api::Message::KillChildResponse {
                    request_id,
                    error_msg: None,
                },
                Option::<RawFd>::None,
            )
            .expect("failed to send kill child response");
        });

        client
            .kill_child(9)
            .expect("kill should ignore stale spawn response");
    });
}
