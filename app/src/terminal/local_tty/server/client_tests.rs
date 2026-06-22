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

#[test]
fn spawn_pty_ignores_stale_kill_child_response() {
    let (client, server_stream) = client_and_server_stream();
    let server_fd = server_stream.as_raw_fd();
    let response_fd = dup(server_fd).expect("failed to duplicate response fd");

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let request = protocol::receive_message(server_fd).expect("failed to receive request");
            assert!(matches!(
                request,
                Some(api::Message::SpawnShellRequest { .. })
            ));

            protocol::send_message(
                server_fd,
                api::Message::KillChildResponse { error_msg: None },
                Option::<RawFd>::None,
            )
            .expect("failed to send stale kill response");
            protocol::send_message(
                server_fd,
                api::Message::SpawnShellResponse {
                    spawn_result: api::Result::Ok(PtySpawnResult {
                        pid: 42,
                        leader_fd: -1,
                    }),
                },
                Some(response_fd),
            )
            .expect("failed to send spawn response");
        });

        let result = client
            .spawn_pty(PtyOptions {
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
            })
            .expect("spawn should recover from stale response");

        assert_eq!(result.pid, 42);
        assert_ne!(result.leader_fd, -1);
        nix::unistd::close(result.leader_fd).expect("failed to close received fd");
    });
}
