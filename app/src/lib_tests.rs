use super::*;

#[test]
fn app_and_tui_use_distinct_api_key_initialization_policies() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: Some("app-api-key".to_owned()),
    };
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: Some("tui-api-key".to_owned()),
        },
    };

    assert_eq!(app.api_key().as_deref(), Some("app-api-key"));
    assert_eq!(tui.api_key().as_deref(), Some("tui-api-key"));
    assert!(app.should_initialize_api_key_eagerly());
    assert!(!tui.should_initialize_api_key_eagerly());
}

#[test]
fn tui_uses_distinct_secure_storage_service_name() {
    let launch_mode = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    assert!(matches!(
        &launch_mode,
        LaunchMode::Tui {
            entrypoint: TuiEntryPoint::Interactive { .. }
        }
    ));

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
fn launch_modes_select_expected_logging_frontend() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    let test = LaunchMode::Test {
        driver: Box::new(None),
        is_integration_test: false,
    };

    assert_eq!(tui.log_frontend(), LogFrontend::Tui);
    assert_eq!(app.log_frontend(), LogFrontend::Gui);
    assert_eq!(test.log_frontend(), LogFrontend::Gui);
    assert_eq!(
        LaunchMode::RemoteServerProxy.log_frontend(),
        LogFrontend::Cli
    );
    assert_eq!(
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        }
        .log_frontend(),
        LogFrontend::Cli
    );
}

/// Characterization test — this behavior predates APP-2946 and this test also
/// passes on `master`. It pins the two facts the Dock fix is built on, so a
/// future change that reclassifies the CLI has to do so deliberately:
/// `is_headless()` is what `startup_steps` keys every Dock/app-bundle guard
/// off, and `ExecutionMode::Sdk` is what keeps autoupdate work off the CLI.
/// The behavior-level coverage for the fix itself lives in
/// `startup_steps_tests.rs`.
#[test]
fn command_line_launch_mode_is_headless_and_runs_as_the_sdk() {
    let cli = LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: Default::default(),
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    };

    assert!(cli.is_headless());
    assert_eq!(cli.execution_mode(), ExecutionMode::Sdk);
}
