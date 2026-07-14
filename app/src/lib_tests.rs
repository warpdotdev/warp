use super::*;

#[test]
fn tui_uses_distinct_secure_storage_service_name() {
    let launch_mode = LaunchMode::Tui {
        mount: Box::new(|_| {}),
        api_key: None,
    };

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev.tui"
    );
}

#[test]
fn app_keeps_default_secure_storage_service_name() {
    let launch_mode = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev"
    );
}

#[test]
fn gui_modes_start_local_http_server() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    assert!(app.should_start_local_http_server());
    assert!(LaunchMode::new_for_unit_test().should_start_local_http_server());
}

#[test]
fn headless_modes_do_not_start_local_http_server() {
    // Regression: the fixed-port local HTTP server must not start in headless
    // modes. Several headless Warp processes (e.g. the remote server daemon and
    // CLI/SDK runs) commonly run on the same host, so all but the first would
    // fail to bind the fixed port and log a spurious `EADDRINUSE` error.
    let daemon = LaunchMode::RemoteServerDaemon {
        identity_key: "test-identity".to_string(),
    };
    assert!(!daemon.should_start_local_http_server());
    assert!(!LaunchMode::RemoteServerProxy.should_start_local_http_server());

    let tui = LaunchMode::Tui {
        mount: Box::new(|_| {}),
        api_key: None,
    };
    assert!(!tui.should_start_local_http_server());
}
