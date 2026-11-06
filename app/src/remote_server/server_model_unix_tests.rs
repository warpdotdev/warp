use std::fs;
use std::time::Duration;

use async_io::Timer;
use instant::Instant;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use warp_core::SessionId;

use super::*;
use crate::remote_server::proto::{
    Abort, ClientMessage, RunCommandRequest, SessionBootstrapped, notification,
    session_scoped_request,
};

async fn wait_for_command_pid(path: &std::path::Path) -> Pid {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(pid) = contents.parse()
        {
            return Pid::from_raw(pid);
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for command PID in {}",
            path.display()
        );
        Timer::after(Duration::from_millis(10)).await;
    }
}

async fn wait_for_process_exit(pid: Pid) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => return,
            Ok(()) | Err(nix::errno::Errno::EPERM) => {}
            Err(error) => panic!("failed to inspect remote command process {pid}: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for remote command process {pid} to exit"
        );
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[test]
fn aborting_remote_run_command_kills_process_and_clears_in_progress_request() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let ready_file = temp_dir.path().join("ready");
        let session_id = SessionId::from(42u64);
        let connection_id = uuid::Uuid::new_v4();
        let request_id = RequestId::new();
        let model = test_model_handle(&mut app);

        model.update(&mut app, |model, ctx| {
            model.handle_message(
                connection_id,
                ClientMessage::notification(notification::Message::SessionBootstrapped(
                    SessionBootstrapped {
                        session_id: session_id.as_u64(),
                        shell_type: "bash".to_string(),
                        shell_path: Some("/bin/bash".to_string()),
                    },
                )),
                ctx,
            );
            model.handle_message(
                connection_id,
                ClientMessage::session_scoped(
                    request_id.to_string(),
                    session_scoped_request::Message::RunCommand(RunCommandRequest {
                        command: "printf %s \"$$\" > \"$READY_FILE\"; sleep 60".to_string(),
                        working_directory: None,
                        environment_variables: HashMap::from([(
                            "READY_FILE".to_string(),
                            ready_file.to_string_lossy().into_owned(),
                        )]),
                        session_id: session_id.as_u64(),
                    }),
                ),
                ctx,
            );
            assert!(model.in_progress.contains_key(&request_id));
        });

        let command_pid = wait_for_command_pid(&ready_file).await;
        model.update(&mut app, |model, ctx| {
            model.handle_message(
                connection_id,
                ClientMessage::notification(notification::Message::Abort(Abort {
                    request_id_to_abort: request_id.to_string(),
                })),
                ctx,
            );
        });

        wait_for_process_exit(command_pid).await;
        assert!(model.read(&app, |model, _| model.in_progress.is_empty()));
    });
}
