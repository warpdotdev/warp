use self::parse_url_paths::{get_item_data_from_warp_link, WarpWebLink};
use super::*;
use crate::launch_configs::launch_config::make_mock_single_window_launch_config;
use crate::linear::{LinearAction, LinearIssueWork};
use crate::ChannelState;

#[test]
fn test_find_matching_config() {
    let mut configs: Vec<LaunchConfig> = vec![];
    for i in 0..5 {
        add_mock_config_with_name(
            (String::from("config") + i.to_string().as_str()).as_str(),
            &mut configs,
        );
    }

    let with_extension = "config1.yaml";
    assert_eq!(
        find_matching_config(with_extension, &configs),
        Some(&configs[1])
    );

    let no_extension = "config4";
    assert_eq!(
        find_matching_config(no_extension, &configs),
        Some(&configs[4])
    );

    let caps_insensitive = "ConFig3";
    assert_eq!(
        find_matching_config(caps_insensitive, &configs),
        Some(&configs[3])
    );

    let missing_config = "missing";
    assert_eq!(find_matching_config(missing_config, &configs), None);
}

#[test]
fn test_find_matching_config_with_spaces() {
    let mut configs: Vec<LaunchConfig> = vec![];
    for i in 0..3 {
        add_mock_config_with_name(
            (String::from("config") + i.to_string().as_str()).as_str(),
            &mut configs,
        );
    }

    let with_space = "config 3.yaml";
    add_mock_config_with_name(with_space, &mut configs);
    assert_eq!(
        find_matching_config(with_space, &configs),
        Some(&configs[3])
    );

    let more_space = " a ";
    add_mock_config_with_name(more_space, &mut configs);
    assert_eq!(
        find_matching_config(more_space, &configs),
        Some(&configs[4])
    );
}

#[test]
fn test_find_matching_configs_special_chars() {
    let mut configs: Vec<LaunchConfig> = vec![];
    for i in 0..3 {
        add_mock_config_with_name(
            (String::from("config") + i.to_string().as_str()).as_str(),
            &mut configs,
        );
    }

    // test special characters
    let special_ascii = "yes! this_works,too-even[braces}and(parens'.";
    add_mock_config_with_name(special_ascii, &mut configs);
    assert_eq!(
        find_matching_config(special_ascii, &configs),
        Some(&configs[3])
    );

    // test emojis
    let bread = "🍞";
    add_mock_config_with_name(bread, &mut configs);
    assert_eq!(find_matching_config(bread, &configs), Some(&configs[4]));
}

fn add_mock_config_with_name(name: &str, configs: &mut Vec<LaunchConfig>) {
    let mut new_config = make_mock_single_window_launch_config();
    new_config.name = name.to_string();
    new_config.windows[0].tabs[0].title = Some(String::from("First tab from config ") + name);
    configs.push(new_config);
}

#[test]
fn test_find_matching_tab_config() {
    let configs = vec![
        make_mock_tab_config("my tab", Some("/tab_configs/my_tab.toml")),
        make_mock_tab_config("Deploy", Some("/tab_configs/Deploy.yaml")),
        make_mock_tab_config("dotted", Some("/tab_configs/foo.bar.toml")),
        make_mock_tab_config("orphan", None),
    ];

    // Stem match without extension.
    assert_eq!(
        find_matching_tab_config("my_tab", configs.clone()).map(|c| c.name),
        Some(String::from("my tab")),
    );

    // Stem match with extension.
    assert_eq!(
        find_matching_tab_config("my_tab.toml", configs.clone()).map(|c| c.name),
        Some(String::from("my tab")),
    );

    // Case-insensitive match.
    assert_eq!(
        find_matching_tab_config("deploy", configs.clone()).map(|c| c.name),
        Some(String::from("Deploy")),
    );

    // Dotted stem resolves both with and without `.toml`.
    assert_eq!(
        find_matching_tab_config("foo.bar", configs.clone()).map(|c| c.name),
        Some(String::from("dotted")),
    );
    assert_eq!(
        find_matching_tab_config("foo.bar.toml", configs.clone()).map(|c| c.name),
        Some(String::from("dotted")),
    );

    // Miss returns None.
    assert!(find_matching_tab_config("unknown", configs.clone()).is_none());

    // Configs without a `source_path` never match.
    assert!(find_matching_tab_config("orphan", configs).is_none());
}

fn make_mock_tab_config(name: &str, source_path: Option<&str>) -> TabConfig {
    TabConfig {
        name: name.to_string(),
        title: None,
        color: None,
        panes: vec![],
        params: HashMap::new(),
        source_path: source_path.map(PathBuf::from),
    }
}

#[test]
fn test_get_launch_config_path() {
    assert_eq!(
        get_launch_config_path("/path/to/a/config"),
        Some(String::from("path/to/a/config")),
    );
    assert_eq!(
        get_launch_config_path("/hello%20world.yaml"),
        Some(String::from("hello world.yaml")),
    );
    assert_eq!(
        get_launch_config_path("/%3Bhello%20%23world!"),
        Some(String::from(";hello #world!")),
    );
    assert_eq!(
        get_launch_config_path("/yes%21%20this_works%2Ctoo-even%5Bbraces%7Dand%28parens%27."),
        Some(String::from("yes! this_works,too-even[braces}and(parens'."))
    );
    assert_eq!(
        get_launch_config_path("/%F0%9F%8D%9E"),
        Some(String::from("🍞"))
    );
    assert_eq!(
        get_launch_config_path("/..filename_.with_dots.."),
        Some(String::from("..filename_.with_dots.."))
    );
}

#[test]
fn test_get_launch_config_path_invalid() {
    assert_eq!(get_launch_config_path(""), None);
    assert_eq!(get_launch_config_path("/"), None);
    assert_eq!(get_launch_config_path("%2F"), None);
    assert_eq!(get_launch_config_path("/../outside"), None);
    assert_eq!(get_launch_config_path("/..%2Foutside"), None);
    assert_eq!(get_launch_config_path("/A/.."), None);
    assert_eq!(get_launch_config_path("/A/../B"), None);
    assert_eq!(get_launch_config_path("//absolute"), None);
    assert_eq!(get_launch_config_path("/%2Fabsolute sneaky"), None);
    assert_eq!(get_launch_config_path("//../very_bad/.."), None);
}

#[test]
fn test_remove_extension() {
    assert_eq!(remove_extension(""), None);
    assert_eq!(remove_extension(".yaml"), Some(""));
    assert_eq!(remove_extension(" .yaml"), Some(" "));
    assert_eq!(remove_extension("config.yaml"), Some("config"));
    assert_eq!(remove_extension("..yaml"), Some("."));
    assert_eq!(remove_extension("config"), None);
    assert_eq!(remove_extension("🍞.yaml"), Some("🍞"));
}

#[test]
fn test_warp_web_link_notebook() {
    assert_eq!(
        get_item_data_from_warp_link(
            &Url::parse(&format!(
                "{}/drive/notebook/Performance-Analysis-LkDlnAe34vfYD2JXsAkssc?focused_folder_id=test_uid00000000000123&invitee_email=test@example.com",
                ChannelState::server_root_url()
            ))
            .unwrap()
        ),
        Some(WarpWebLink::DriveObject(Box::new(OpenWarpDriveObjectArgs {
            object_type: ObjectType::Notebook,
server_id: ServerId::from_string_lossy("LkDlnAe34vfYD2JXsAkssc"),
            settings: OpenWarpDriveObjectSettings {
                focused_folder_id: Some(ServerId::from(123)),
                invitee_email: Some(String::from("test@example.com")),
            },
        })))
    );
}

#[test]
fn test_warp_web_link_session() {
    assert_eq!(
        get_item_data_from_warp_link(
            &Url::parse(&format!(
                "{}/session/317d0686-7a0b-4b67-806b-aaa3e9df501b?
                pwd=6f727249-af9f-4025-a240-59df40a4c64b",
                ChannelState::server_root_url()
            ))
            .unwrap()
        ),
        Some(WarpWebLink::Session)
    );
}

#[test]
fn test_warp_web_link_workflow() {
    assert_eq!(
        get_item_data_from_warp_link(
            &Url::parse(&format!(
                "{}/drive/workflow/Remove-all-stopped-docker-container-image-and-volumes-ZCJSkai2gpwTqpBFs5HOfZ",
                ChannelState::server_root_url()
            ))
            .unwrap()
        ),
        Some(WarpWebLink::DriveObject(Box::new(OpenWarpDriveObjectArgs {
            object_type: ObjectType::Workflow,
server_id: ServerId::from_string_lossy("ZCJSkai2gpwTqpBFs5HOfZ"),
            settings: OpenWarpDriveObjectSettings::default(),
        })))
    );
}

#[test]
fn test_warp_web_link_failure() {
    assert_eq!(
        get_item_data_from_warp_link(&Url::parse("https://google.com").unwrap()),
        None
    );
}
#[test]
fn test_app_web_link_rewrites_to_new_cloud_agent_conversation() {
    let url = Url::parse(&format!("{}/app", ChannelState::server_root_url())).unwrap();
    let intent = web_intent_parser::maybe_rewrite_web_url_to_intent(&url).unwrap();

    assert_eq!(
        intent.as_str(),
        format!(
            "{}://action/new_cloud_agent_conversation?source=web_home",
            ChannelState::url_scheme()
        )
    );
}

#[test]
fn test_action_create_environment_parse() {
    let url = Url::parse(&format!(
        "{}://action/create_environment?repo=foo&repo=bar",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::CreateEnvironment { repos } => {
            assert_eq!(repos, vec!["foo".to_owned(), "bar".to_owned()]);
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_focus_cloud_mode_parse() {
    let url = Url::parse(&format!(
        "{}://action/focus_cloud_mode",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(action, Action::FocusCloudMode));
}

#[test]
fn test_action_create_environment_parse_no_repos() {
    let url = Url::parse(&format!(
        "{}://action/create_environment",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::CreateEnvironment { repos } => {
            assert!(repos.is_empty());
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

fn open_file_editor_test_path(file_name: &str) -> (String, PathBuf) {
    #[cfg(windows)]
    let path = format!("C:/tmp/{file_name}");
    #[cfg(not(windows))]
    let path = format!("/tmp/{file_name}");

    (path.clone(), PathBuf::from(path))
}

#[test]
fn test_action_open_file_editor_parse_with_path_only() {
    let (path_param, expected_path) = open_file_editor_test_path("test.rs");
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path={path_param}",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::OpenFileEditor { path, line_col } => {
            assert_eq!(path, expected_path);
            assert_eq!(line_col, None);
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_open_file_editor_parse_with_line_only() {
    let (path_param, expected_path) = open_file_editor_test_path("test.rs");
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path={path_param}&line=120",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::OpenFileEditor { path, line_col } => {
            assert_eq!(path, expected_path);
            assert_eq!(
                line_col,
                Some(LineAndColumnArg {
                    line_num: 120,
                    column_num: None,
                })
            );
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_open_file_editor_parse_with_line_and_column() {
    let (path_param, expected_path) = open_file_editor_test_path("test.rs");
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path={path_param}&line=120&column=8",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::OpenFileEditor { path, line_col } => {
            assert_eq!(path, expected_path);
            assert_eq!(
                line_col,
                Some(LineAndColumnArg {
                    line_num: 120,
                    column_num: Some(8),
                })
            );
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_open_file_editor_parse_decodes_percent_encoded_path() {
    let (path_param, _) = open_file_editor_test_path("hello%20world.rs");
    let (_, expected_path) = open_file_editor_test_path("hello world.rs");
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path={path_param}&line=1",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::OpenFileEditor { path, line_col } => {
            assert_eq!(path, expected_path);
            assert_eq!(
                line_col,
                Some(LineAndColumnArg {
                    line_num: 1,
                    column_num: None,
                })
            );
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_open_file_editor_parse_expands_home_dir() {
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path=~/tmp/test.rs&line=1",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    match action {
        Action::OpenFileEditor { path, line_col } => {
            assert_eq!(
                path,
                PathBuf::from(shellexpand::tilde("~/tmp/test.rs").into_owned())
            );
            assert_eq!(
                line_col,
                Some(LineAndColumnArg {
                    line_num: 1,
                    column_num: None,
                })
            );
        }
        _ => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn test_action_open_file_editor_parse_requires_path() {
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?line=1",
        ChannelState::url_scheme()
    ))
    .unwrap();

    assert!(Action::parse(&url).is_err());
}

#[test]
fn test_action_open_file_editor_parse_rejects_relative_path() {
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path=src/main.rs&line=1",
        ChannelState::url_scheme()
    ))
    .unwrap();

    assert!(Action::parse(&url).is_err());
}

#[test]
fn test_action_open_file_editor_parse_rejects_column_without_line() {
    let url = Url::parse(&format!(
        "{}://action/open_file_editor?path=/tmp/test.rs&column=8",
        ChannelState::url_scheme()
    ))
    .unwrap();

    assert!(Action::parse(&url).is_err());
}

#[test]
fn test_action_open_file_editor_parse_rejects_invalid_line_or_column() {
    let invalid_line = Url::parse(&format!(
        "{}://action/open_file_editor?path=/tmp/test.rs&line=abc",
        ChannelState::url_scheme()
    ))
    .unwrap();
    assert!(Action::parse(&invalid_line).is_err());

    let zero_line = Url::parse(&format!(
        "{}://action/open_file_editor?path=/tmp/test.rs&line=0",
        ChannelState::url_scheme()
    ))
    .unwrap();
    assert!(Action::parse(&zero_line).is_err());

    let invalid_column = Url::parse(&format!(
        "{}://action/open_file_editor?path=/tmp/test.rs&line=1&column=0",
        ChannelState::url_scheme()
    ))
    .unwrap();
    assert!(Action::parse(&invalid_column).is_err());
}

#[test]
fn test_action_cloud_agent_setup_parse() {
    let url = Url::parse(&format!(
        "{}://action/cloud_agent_setup",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(action, Action::CloudAgentSetup));
}
#[test]
fn test_action_auto_handoff_to_cloud_parse_default_trigger() {
    let url = Url::parse(&format!(
        "{}://action/auto_handoff_to_cloud",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(
        action,
        Action::AutoHandoffToCloud {
            trigger: AutoCloudHandoffTrigger::Uri,
        }
    ));
}

#[test]
fn test_action_auto_handoff_to_cloud_parse_alias_path() {
    let url = Url::parse(&format!(
        "{}://action/auto-handoff-to-cloud",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(
        action,
        Action::AutoHandoffToCloud {
            trigger: AutoCloudHandoffTrigger::Uri,
        }
    ));
}

#[test]
fn test_action_auto_handoff_to_cloud_parse_sleep_trigger() {
    let url = Url::parse(&format!(
        "{}://action/auto_handoff_to_cloud?trigger=sleep",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(
        action,
        Action::AutoHandoffToCloud {
            trigger: AutoCloudHandoffTrigger::MacOsSleep,
        }
    ));
}

#[test]
fn test_action_new_cloud_agent_conversation_parse() {
    let url = Url::parse(&format!(
        "{}://action/new_cloud_agent_conversation",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(action, Action::NewCloudAgentConversation));
}

#[test]
fn test_action_new_agent_conversation_parse() {
    let url = Url::parse(&format!(
        "{}://action/new_agent_conversation",
        ChannelState::url_scheme()
    ))
    .unwrap();

    let action = Action::parse(&url).unwrap();
    assert!(matches!(action, Action::NewAgentConversation));
}

#[test]
fn test_validate_custom_uri_linear() {
    let url = Url::parse(&format!(
        "{}://linear/work?prompt=hello",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let host = validate_custom_uri(&url).unwrap();
    assert!(matches!(host, UriHost::Linear));
}

#[test]
fn test_linear_action_parse_work() {
    let url = Url::parse(&format!(
        "{}://linear/work?prompt=hello",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let action = LinearAction::parse(&url).unwrap();
    assert_eq!(action, LinearAction::WorkOnIssue);
}

#[test]
fn test_linear_action_parse_unknown_path() {
    let url = Url::parse(&format!("{}://linear/unknown", ChannelState::url_scheme())).unwrap();
    assert!(LinearAction::parse(&url).is_err());
}

#[test]
fn test_linear_issue_work_with_prompt() {
    let url = Url::parse(&format!(
        "{}://linear/work?prompt=fix+the+bug",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let args = LinearIssueWork::from_url(&url);
    assert_eq!(args.prompt.as_deref(), Some("fix the bug"));
}

#[test]
fn test_linear_issue_work_without_prompt() {
    let url = Url::parse(&format!("{}://linear/work", ChannelState::url_scheme())).unwrap();
    let args = LinearIssueWork::from_url(&url);
    assert!(args.prompt.is_none());
}

#[test]
fn test_linear_issue_work_empty_prompt() {
    let url = Url::parse(&format!(
        "{}://linear/work?prompt=",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let args = LinearIssueWork::from_url(&url);
    assert!(args.prompt.is_none());
}

// -- handle_incoming_uri validation errors -----------------------------------

/// `validate_custom_uri` returns `anyhow::Error`s whose messages feed the
/// non-dogfood `log::warn!("Custom URI is invalid: {e:?}")` fallback in
/// `handle_incoming_uri`. Those messages must never embed the full URL, its
/// query string, or its fragment — otherwise the fallback warn line becomes
/// a second secret leak.
#[test]
fn validate_custom_uri_errors_do_not_leak_query_string() {
    // Unexpected scheme.
    let url = Url::parse("https://auth/desktop_redirect?refresh_token=LEAKED").unwrap();
    let err = validate_custom_uri(&url).unwrap_err();
    let msg = format!("{err:?}");
    assert!(!msg.contains("refresh_token"), "{msg}");
    assert!(!msg.contains("LEAKED"), "{msg}");

    // Unexpected host.
    let url = Url::parse(&format!(
        "{}://unknown_host/desktop_redirect?refresh_token=LEAKED",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let err = validate_custom_uri(&url).unwrap_err();
    let msg = format!("{err:?}");
    assert!(!msg.contains("refresh_token"), "{msg}");
    assert!(!msg.contains("LEAKED"), "{msg}");

    // Unexpected path for a host that doesn't allow arbitrary paths.
    let url = Url::parse(&format!(
        "{}://auth/not_the_redirect?refresh_token=LEAKED",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let err = validate_custom_uri(&url).unwrap_err();
    let msg = format!("{err:?}");
    assert!(!msg.contains("refresh_token"), "{msg}");
    assert!(!msg.contains("LEAKED"), "{msg}");
}

#[test]
fn test_parse_tab_path_expands_tilde() {
    let url = Url::parse("warp://action/new_tab?path=~/Projects").unwrap();
    let home = dirs::home_dir().expect("HOME must be set for this test");
    assert_eq!(parse_tab_path(&url), Some(home.join("Projects")));
}

#[test]
fn test_parse_tab_path_expands_url_encoded_tilde() {
    // `%7E` and `%2F` are URL-encoded `~` and `/`.
    let url = Url::parse("warp://action/new_tab?path=%7E%2FProjects").unwrap();
    let home = dirs::home_dir().expect("HOME must be set for this test");
    assert_eq!(parse_tab_path(&url), Some(home.join("Projects")));
}

#[test]
fn test_parse_tab_path_absolute_path_unchanged() {
    let url = Url::parse("warp://action/new_tab?path=/tmp/foo").unwrap();
    assert_eq!(parse_tab_path(&url), Some(PathBuf::from("/tmp/foo")));
}

#[test]
fn test_parse_tab_path_relative_path_unchanged() {
    let url = Url::parse("warp://action/new_tab?path=relative/dir").unwrap();
    assert_eq!(parse_tab_path(&url), Some(PathBuf::from("relative/dir")));
}

#[test]
fn test_parse_tab_path_missing_returns_none() {
    let url = Url::parse("warp://action/new_tab").unwrap();
    assert_eq!(parse_tab_path(&url), None);
}

#[test]
fn test_parse_tab_path_bare_tilde() {
    let url = Url::parse("warp://action/new_tab?path=~").unwrap();
    let home = dirs::home_dir().expect("HOME must be set for this test");
    assert_eq!(parse_tab_path(&url), Some(home));
}

// Regression coverage for issue #9005: shell scripts opened via `file://` should run,
// not open in the editor. Exercised through the pure routing helper to avoid standing
// up a full `AppContext`.

#[test]
#[cfg(unix)]
fn test_open_file_executable_sh_routes_to_execute() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("run.sh");
    std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    let action = classify_open_file_action(&p);
    assert_eq!(action, OpenFileAction::ExecuteInSession);
}

#[test]
#[cfg(unix)]
fn test_open_file_non_executable_sh_routes_to_editor() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("view.sh");
    std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(classify_open_file_action(&p), OpenFileAction::Editor);
}

#[test]
#[cfg(unix)]
fn test_open_file_executable_bash_zsh_fish_route_to_execute() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for name in ["run.bash", "run.zsh", "run.fish", "run.command"] {
        let p = dir.path().join(name);
        std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            classify_open_file_action(&p),
            OpenFileAction::ExecuteInSession,
            "{name} should route to ExecuteInSession",
        );
    }
}

#[test]
fn test_open_file_markdown_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("README.md");
    std::fs::write(&p, b"# hi\n").unwrap();
    assert_eq!(classify_open_file_action(&p), OpenFileAction::Notebook);
}

#[test]
#[cfg(feature = "local_fs")]
fn test_open_file_rust_source_still_opens_in_editor() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(&p, b"fn main() {}\n").unwrap();
    assert_eq!(classify_open_file_action(&p), OpenFileAction::Editor);
}

#[test]
#[cfg(unix)]
fn test_open_file_editor_executable_sh_opens_in_editor() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("run.sh");
    std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(can_open_file_editor_path(&p));
}

#[test]
#[cfg(feature = "local_fs")]
fn test_open_file_editor_rust_source_opens_in_editor() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(&p, b"fn main() {}\n").unwrap();
    assert!(can_open_file_editor_path(&p));
}

#[test]
#[cfg(feature = "local_fs")]
fn test_open_file_editor_binary_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("image.png");
    std::fs::write(&p, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    assert!(!can_open_file_editor_path(&p));
}

#[test]
fn test_open_file_directory_routes_to_session() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        classify_open_file_action(dir.path()),
        OpenFileAction::ExecuteInSession
    );
}

#[test]
#[cfg(unix)]
fn test_open_file_non_runnable_shebang_routes_to_editor() {
    // Extensionless `#!/bin/sh` file without the user-execute bit. Without the
    // shebang fall-through this would hit `ExecuteInSession` and the shell would
    // refuse to run it; the editor is the right place to view it.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("noext");
    std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(classify_open_file_action(&p), OpenFileAction::Editor);
}

#[test]
fn test_session_uri_host_parsing() {
    let result = UriHost::from_str("session");
    assert!(matches!(result, Ok(UriHost::Session)));
}

#[test]
fn test_session_uri_validation() {
    let url = Url::parse(&format!(
        "{}://session/A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let host = validate_custom_uri(&url).unwrap();
    assert!(matches!(host, UriHost::Session));
}

#[test]
fn test_session_uri_empty_path_does_not_panic() {
    let url = Url::parse(&format!("{}://session/", ChannelState::url_scheme())).unwrap();
    let host = validate_custom_uri(&url).unwrap();
    assert!(matches!(host, UriHost::Session));
}

#[test]
fn test_session_uri_invalid_hex_does_not_panic() {
    let url = Url::parse(&format!(
        "{}://session/ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        ChannelState::url_scheme()
    ))
    .unwrap();
    let host = validate_custom_uri(&url).unwrap();
    assert!(matches!(host, UriHost::Session));
}

#[test]
fn test_session_uri_case_insensitive_hex() {
    let upper = "A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4";
    let lower = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4";
    let upper_bytes = super::decode_uuid_hex(upper).expect("upper hex should decode");
    let lower_bytes = super::decode_uuid_hex(lower).expect("lower hex should decode");
    assert_eq!(upper_bytes, lower_bytes);
    assert_eq!(upper_bytes.len(), 16);
}

#[test]
fn test_decode_uuid_hex_rejects_wrong_length() {
    assert!(super::decode_uuid_hex("ABCD").is_none());
    assert!(super::decode_uuid_hex("").is_none());
    assert!(super::decode_uuid_hex("A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4FF").is_none());
}

#[test]
fn test_decode_uuid_hex_rejects_invalid_chars() {
    assert!(super::decode_uuid_hex("ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_none());
}
