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

/// Builds a `LaunchMode::CommandLine` equivalent to running the bundled
/// `oz` / `oz-<channel>` wrapper for a headless command.
fn cli_launch_mode() -> LaunchMode {
    LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: Default::default(),
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    }
}

fn tui_launch_mode() -> LaunchMode {
    LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    }
}

/// The bundled CLI wrapper `exec`s the GUI executable from inside `Warp.app`,
/// so the CLI process must opt out of the GUI bundle's dockable identity or
/// macOS shows a Warp Dock tile that bounces for the whole command (APP-2946).
#[test]
fn headless_launch_modes_run_as_background_processes() {
    for launch_mode in [
        cli_launch_mode(),
        tui_launch_mode(),
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ] {
        assert!(
            launch_mode.is_headless(),
            "{} should be headless",
            launch_mode.as_str_for_tracing()
        );
        assert!(
            launch_mode.should_run_as_background_process(),
            "{} must be marked background-only so macOS gives it no Dock tile",
            launch_mode.as_str_for_tracing()
        );
    }
}

/// The GUI app keeps its Dock presence; only headless launches are suppressed.
#[test]
fn gui_launch_modes_keep_their_dock_presence() {
    for launch_mode in [
        LaunchMode::App {
            args: Default::default(),
            api_key: None,
        },
        LaunchMode::Test {
            driver: Box::new(None),
            is_integration_test: false,
        },
    ] {
        assert!(!launch_mode.is_headless());
        assert!(
            !launch_mode.should_run_as_background_process(),
            "{} must keep the regular foreground process type",
            launch_mode.as_str_for_tracing()
        );
        assert!(
            launch_mode.should_configure_dock_and_menus(),
            "{} must still configure its Dock icon, Dock menu, and menu bar",
            launch_mode.as_str_for_tracing()
        );
    }
}

/// Headless startup must not perform any Dock-visible setup (Dock icon, Dock
/// menu, menu bar).
#[test]
fn headless_launch_modes_skip_dock_and_menu_setup() {
    for launch_mode in [
        cli_launch_mode(),
        tui_launch_mode(),
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ] {
        assert!(
            !launch_mode.should_configure_dock_and_menus(),
            "{} must not perform Dock-visible setup",
            launch_mode.as_str_for_tracing()
        );
    }
}

/// `autoupdate::remove_old_executable` deletes `Contents/MacOS/old` inside the
/// installed app bundle. The bundled CLI shares that bundle with the GUI app,
/// so it must never mutate it (APP-2946).
#[test]
fn headless_launch_modes_do_not_mutate_the_app_bundle() {
    for launch_mode in [
        cli_launch_mode(),
        tui_launch_mode(),
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ] {
        assert!(
            !launch_mode.should_clean_up_old_executable(),
            "{} must not remove the old executable from the app bundle",
            launch_mode.as_str_for_tracing()
        );
    }

    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    assert!(
        app.should_clean_up_old_executable(),
        "the GUI app still cleans up after its own autoupdate relaunch"
    );
}

/// The CLI runs under the SDK execution mode, which cannot autoupdate — the
/// gate that keeps autoupdate work (polling and bundle mutation) off the CLI.
#[test]
fn command_line_launch_mode_maps_to_sdk_execution_mode() {
    assert_eq!(cli_launch_mode().execution_mode(), ExecutionMode::Sdk);
}
