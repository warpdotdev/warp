use super::*;

#[test]
fn app_and_tui_accept_api_keys() {
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

    assert_eq!(
        api_key_from_launch_mode(&app).as_deref(),
        Some("app-api-key")
    );
    assert_eq!(
        api_key_from_launch_mode(&tui).as_deref(),
        Some("tui-api-key")
    );
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
fn launch_intent_url_classification() {
    let scheme = ChannelState::url_scheme();
    let parse = |s: &str| Url::parse(s).expect("valid test URL");

    let intents = [
        format!("{scheme}://tab_config/my_tab"),
        format!("{scheme}://tab_config/my_tab.toml"),
        format!("{scheme}://action/new_window?path=~"),
        format!("{scheme}://action/new_cloud_agent_conversation?source=web_home"),
    ];
    for s in intents {
        assert!(
            is_launch_intent_url(&parse(&s)),
            "expected launch intent: {s}"
        );
    }

    let not_intents = [
        format!("{scheme}://action/new_cloud_agent_conversation"),
        format!("{scheme}://action/new_cloud_agent_conversation?source=other"),
        format!("{scheme}://action/something_else"),
        "https://example.com/action/new_window".to_string(),
    ];
    for s in not_intents {
        assert!(
            !is_launch_intent_url(&parse(&s)),
            "expected no launch intent: {s}"
        );
    }
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
