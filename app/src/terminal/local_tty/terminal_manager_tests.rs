use super::*;

/// Windows refuses planned shell replacement.
#[cfg(unix)]
#[test]
fn planned_environment_becomes_the_next_recovery_baseline() {
    use crate::terminal::local_tty::shell::DirectShellStarter;
    use crate::terminal::shell::ShellType;

    warpui::App::test((), |mut app| async move {
        let init = app.update(TerminalSurfaceInit::new_for_test);
        let mut resources = ShellRecoveryResources {
            starter: ShellStarter::Direct(DirectShellStarter::new_for_test(
                ShellType::Bash,
                "/bin/bash".into(),
                Vec::new(),
            )),
            original_env: HashMap::from([("WARP_API_KEY".into(), "standby-key".into())]),
            channel_event_proxy: ChannelEventListener::new_for_test(),
            model_events: init.model_events,
        };
        let run_env = HashMap::from([
            ("WARP_API_KEY".into(), "run-key".into()),
            ("CUSTOM_SETUP_VALUE".into(), "run-value".into()),
        ]);
        assert_eq!(resources.adopt_environment(run_env.clone()), run_env);
        let recovered = resources.recovery_environment(Some(HashMap::from([(
            "KUBECONFIG".into(),
            "/run/kubeconfig".into(),
        )])));
        assert_eq!(
            recovered.get(&OsString::from("WARP_API_KEY")).unwrap(),
            "run-key"
        );
        assert_eq!(
            recovered
                .get(&OsString::from("CUSTOM_SETUP_VALUE"))
                .unwrap(),
            "run-value"
        );
        assert_eq!(
            recovered.get(&OsString::from("KUBECONFIG")).unwrap(),
            "/run/kubeconfig"
        );
        let next_env = HashMap::from([("WARP_API_KEY".into(), "next-run-key".into())]);
        resources.adopt_environment(next_env.clone());
        assert_eq!(resources.recovery_environment(None), next_env);
    });
}

#[test]
fn replacement_join_returns_without_waiting_for_a_stuck_loop() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        release_rx.recv().unwrap();
        mio_channel::channel().1
    });
    let started = Instant::now();
    let result = warpui::r#async::block_on(join_event_loop(handle, Duration::ZERO));
    assert!(result.err().unwrap().to_string().contains("timeout"));
    assert!(started.elapsed() < Duration::from_secs(1));
    release_tx.send(()).unwrap();
}

#[test]
fn replacement_join_returns_the_original_channel() {
    let (sender, receiver) = mio_channel::channel();
    let handle = std::thread::spawn(move || receiver);
    while !handle.is_finished() {
        std::thread::yield_now();
    }
    let receiver = warpui::r#async::block_on(join_event_loop(handle, Duration::ZERO)).unwrap();
    sender.send(Message::ShutdownForReplacement).unwrap();
    assert!(matches!(
        receiver.try_recv(),
        Ok(Message::ShutdownForReplacement)
    ));
}
