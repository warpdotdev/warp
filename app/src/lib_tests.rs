use clap::{Args as _, FromArgMatches as _};
use warp_cli::agent::RunAgentArgs;

use super::*;

fn agent_run_args(argv: &[&str]) -> RunAgentArgs {
    let matches = RunAgentArgs::augment_args(clap::Command::new("run"))
        .try_get_matches_from(argv)
        .expect("agent run args should parse");
    RunAgentArgs::from_arg_matches(&matches).expect("agent run args should convert")
}

#[test]
fn app_api_key_requires_validation() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: Some("app-api-key".to_owned()),
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "app-api-key"
    ));
}

#[test]
fn tui_api_key_requires_validation() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: Some("tui-api-key".to_owned()),
        },
    };

    assert!(matches!(
        tui.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "tui-api-key"
    ));
}

#[test]
fn command_line_api_key_requires_validation() {
    let command_line = LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: GlobalOptions {
            api_key: Some("cli-api-key".to_owned()),
            ..Default::default()
        },
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    };

    assert!(matches!(
        command_line.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "cli-api-key"
    ));
}

#[test]
fn startup_without_api_key_loads_persisted_auth() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::Persisted
    ));
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
fn only_the_primary_desktop_instance_owns_the_desktop_application_service() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    let test = LaunchMode::Test {
        driver: Box::new(None),
        is_integration_test: false,
    };
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    let daemon = LaunchMode::RemoteServerDaemon {
        identity_key: "test".to_owned(),
    };

    assert!(app.owns_desktop_application_service());
    assert!(test.owns_desktop_application_service());
    assert!(!tui.owns_desktop_application_service());
    assert!(!daemon.owns_desktop_application_service());
    assert!(!LaunchMode::RemoteServerProxy.owns_desktop_application_service());
}

/// `agent run --gui` is the one non-headless mode kept out of the desktop
/// application service, so the exclusion is asserted rather than left to be
/// "corrected" by a reader who notices the divergence from `is_headless`.
#[test]
fn command_line_never_owns_the_desktop_application_service() {
    let command_line = |command| LaunchMode::CommandLine {
        command,
        global_options: GlobalOptions::default(),
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    };

    let headless_cli = command_line(CliCommand::Whoami);
    let gui_agent_run = command_line(CliCommand::Agent(AgentCommand::Run(agent_run_args(&[
        "run",
        "--prompt",
        "do something",
        "--gui",
    ]))));

    assert!(!headless_cli.owns_desktop_application_service());
    assert!(!gui_agent_run.owns_desktop_application_service());
    assert!(
        !gui_agent_run.is_headless(),
        "`agent run --gui` renders windows, so this exclusion is deliberate rather than a \
         restatement of `is_headless`"
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
