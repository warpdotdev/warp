use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use super::*;
use crate::event::ObservedExitStatus;
use crate::local_tty::server::api::{
    AttachInfo, ClientMessage, DaemonEvent, DaemonMessage, DaemonRequest, DaemonResponse,
    PersistentSessionId, SessionMetadata, SessionSummary, SpawnEnvironment,
};
use crate::local_tty::server::test_util::sh_options;

fn sample_id(n: u128) -> PersistentSessionId {
    PersistentSessionId::from(uuid::Uuid::from_u128(n))
}

fn sample_metadata() -> SessionMetadata {
    SessionMetadata {
        hook_session_id: 42,
        shell_type: "bash".to_owned(),
    }
}

fn sample_info() -> AttachInfo {
    AttachInfo {
        session_id: sample_id(1),
        pid: 1234,
        metadata: sample_metadata(),
        replay_len: 10,
        has_bootstrap_preamble: true,
        exit_status: None,
    }
}

fn client_messages() -> Vec<ClientMessage> {
    let requests = vec![
        DaemonRequest::List,
        DaemonRequest::Spawn {
            session_id: sample_id(1),
            metadata: sample_metadata(),
            options: Box::new(sh_options("true")),
            environment: Some(SpawnEnvironment {
                vars: vec![("EXAMPLE_VAR".into(), "example-value".into())],
            }),
        },
        DaemonRequest::Attach {
            session_id: sample_id(2),
        },
        DaemonRequest::Kill {
            session_id: sample_id(3),
        },
        DaemonRequest::Prune {
            keep: vec![sample_id(4), sample_id(5)],
        },
        DaemonRequest::DetachAll,
    ];
    let mut messages = vec![ClientMessage::Hello {
        protocol_version: 1,
        client_pid: 77,
        app_version: Some("v0.example".to_owned()),
    }];
    messages.extend(
        requests
            .into_iter()
            .enumerate()
            .map(|(i, request)| ClientMessage::Request {
                request_id: i as u64,
                request,
            }),
    );
    messages
}

fn daemon_messages() -> Vec<DaemonMessage> {
    vec![
        DaemonMessage::HelloAck {
            protocol_version: 1,
            daemon_pid: 2,
        },
        DaemonMessage::HelloReject {
            protocol_version: 1,
            reason: "nope".to_owned(),
        },
        DaemonMessage::Response {
            request_id: 1,
            response: DaemonResponse::Sessions(vec![SessionSummary {
                session_id: sample_id(1),
                pid: 3,
                metadata: sample_metadata(),
                exit_status: Some(ObservedExitStatus::Signal(9)),
                attached: false,
            }]),
        },
        DaemonMessage::Response {
            request_id: 2,
            response: DaemonResponse::Attached(sample_info()),
        },
        DaemonMessage::Response {
            request_id: 3,
            response: DaemonResponse::Done,
        },
        DaemonMessage::Response {
            request_id: 4,
            response: DaemonResponse::Error("boom".to_owned()),
        },
        DaemonMessage::Event(DaemonEvent::SessionExited {
            session_id: sample_id(1),
            pid: 3,
            status: ObservedExitStatus::Code(0),
        }),
    ]
}

#[test]
fn every_message_round_trips_over_a_socket() {
    let (a, b) = UnixStream::pair().unwrap();
    for message in client_messages() {
        send_frame(a.as_raw_fd(), &message, &[]).unwrap();
        let (received, fds): (ClientMessage, _) = receive_frame(b.as_raw_fd()).unwrap().unwrap();
        assert!(fds.is_empty());
        assert_eq!(format!("{received:?}"), format!("{message:?}"));
    }
    for message in daemon_messages() {
        send_frame(b.as_raw_fd(), &message, &[]).unwrap();
        let (received, _): (DaemonMessage, _) = receive_frame(a.as_raw_fd()).unwrap().unwrap();
        assert_eq!(format!("{received:?}"), format!("{message:?}"));
    }
    drop(b);
    assert!(
        receive_frame::<DaemonMessage>(a.as_raw_fd())
            .unwrap()
            .is_none()
    );
}

#[test]
fn frame_carries_two_fds_in_order() {
    let (a, b) = UnixStream::pair().unwrap();
    let (mut first_local, first_remote) = UnixStream::pair().unwrap();
    let (mut second_local, second_remote) = UnixStream::pair().unwrap();
    let message = DaemonMessage::Response {
        request_id: 9,
        response: DaemonResponse::Attached(sample_info()),
    };
    send_frame(
        a.as_raw_fd(),
        &message,
        &[first_remote.as_raw_fd(), second_remote.as_raw_fd()],
    )
    .unwrap();
    drop((first_remote, second_remote));

    let (received, fds): (DaemonMessage, Vec<OwnedFd>) =
        receive_frame(b.as_raw_fd()).unwrap().unwrap();
    assert_eq!(format!("{received:?}"), format!("{message:?}"));
    assert_eq!(fds.len(), 2);
    let flags = nix::fcntl::fcntl(fds[0].as_raw_fd(), nix::fcntl::F_GETFD).unwrap();
    assert!(nix::fcntl::FdFlag::from_bits_truncate(flags).contains(nix::fcntl::FdFlag::FD_CLOEXEC));

    let mut streams = fds.into_iter().map(UnixStream::from);
    let mut first = streams.next().unwrap();
    let mut second = streams.next().unwrap();
    first.write_all(b"one").unwrap();
    second.write_all(b"two").unwrap();
    let mut buf = [0u8; 3];
    first_local.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"one");
    second_local.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"two");
}

fn large_message() -> DaemonMessage {
    DaemonMessage::Response {
        request_id: 5,
        response: DaemonResponse::Error("x".repeat(3 * 1024 * 1024)),
    }
}

#[test]
fn blocking_reader_reassembles_payloads_larger_than_the_socket_buffer() {
    let (a, b) = UnixStream::pair().unwrap();
    let (fd_local, fd_remote) = UnixStream::pair().unwrap();
    let sender = std::thread::spawn(move || {
        send_frame(a.as_raw_fd(), &large_message(), &[fd_remote.as_raw_fd()]).unwrap();
        send_frame(a.as_raw_fd(), &large_message(), &[]).unwrap();
    });
    for expected_fds in [1, 0] {
        let (received, fds): (DaemonMessage, _) = receive_frame(b.as_raw_fd()).unwrap().unwrap();
        assert_eq!(fds.len(), expected_fds);
        match received {
            DaemonMessage::Response {
                response: DaemonResponse::Error(text),
                ..
            } => assert_eq!(text.len(), 3 * 1024 * 1024),
            other => panic!("unexpected message: {other:?}"),
        }
    }
    sender.join().unwrap();
    drop(fd_local);
}

#[test]
fn nonblocking_reader_reassembles_split_frames_and_attributes_fds() {
    let (a, b) = UnixStream::pair().unwrap();
    b.set_nonblocking(true).unwrap();
    let (_fd_local, fd_remote) = UnixStream::pair().unwrap();
    let fd_remote = OwnedFd::from(fd_remote);
    let sender = std::thread::spawn(move || {
        send_frame(a.as_raw_fd(), &large_message(), &[]).unwrap();
        send_frame(
            a.as_raw_fd(),
            &DaemonMessage::Response {
                request_id: 6,
                response: DaemonResponse::Done,
            },
            &[fd_remote.as_raw_fd()],
        )
        .unwrap();
    });

    let mut reader = FrameReader::new();
    let mut frames: Vec<(DaemonMessage, Vec<OwnedFd>)> = Vec::new();
    loop {
        let fill = reader.fill(b.as_raw_fd()).unwrap();
        while let Some(frame) = reader.next_frame().unwrap() {
            frames.push(frame);
        }
        if fill == FillResult::Closed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    sender.join().unwrap();
    assert_eq!(frames.len(), 2);
    assert!(frames[0].1.is_empty());
    assert!(matches!(
        frames[1],
        (
            DaemonMessage::Response {
                request_id: 6,
                response: DaemonResponse::Done
            },
            ref fds
        ) if fds.len() == 1
    ));
}

#[test]
fn legacy_messages_still_round_trip() {
    let (a, b) = UnixStream::pair().unwrap();
    let (_local, remote) = UnixStream::pair().unwrap();
    send_message(
        a.as_raw_fd(),
        api::Message::SpawnShellResponse {
            spawn_result: api::Result::Ok(PtySpawnResult {
                pid: 5,
                leader_fd: -1,
            }),
        },
        Some(remote.as_raw_fd()),
    )
    .unwrap();
    match receive_message(b.as_raw_fd()).unwrap() {
        Some(api::Message::SpawnShellResponse {
            spawn_result: api::Result::Ok(PtySpawnResult { pid: 5, leader_fd }),
        }) => {
            assert!(leader_fd >= 0);
            // SAFETY: the fd was just received and is owned by this test.
            drop(unsafe { OwnedFd::from_raw_fd(leader_fd) });
        }
        _ => panic!("unexpected legacy message"),
    }
}
