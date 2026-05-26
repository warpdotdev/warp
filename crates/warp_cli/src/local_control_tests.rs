use std::ffi::OsString;

use clap::Parser as _;
use clap_complete::aot::Shell;
use local_control::protocol::{ControlError, ErrorCode, PaneTarget, TabTarget, WindowTarget};
use local_control::{ActionImplementationStatus, ActionKind};
use serde_json::json;
use serial_test::serial;

use super::*;

const DISCOVERY_DIR_ENV: &str = "WARP_LOCAL_CONTROL_DISCOVERY_DIR";

fn set_discovery_dir(path: &std::path::Path) -> Option<OsString> {
    let previous = std::env::var_os(DISCOVERY_DIR_ENV);
    unsafe { std::env::set_var(DISCOVERY_DIR_ENV, path) };
    previous
}

fn restore_discovery_dir(previous: Option<OsString>) {
    match previous {
        Some(value) => unsafe { std::env::set_var(DISCOVERY_DIR_ENV, value) },
        None => unsafe { std::env::remove_var(DISCOVERY_DIR_ENV) },
    }
}

fn test_instance_record() -> local_control::discovery::InstanceRecord {
    local_control::discovery::InstanceRecord {
        protocol_version: local_control::PROTOCOL_VERSION,
        instance_id: local_control::discovery::InstanceId("inst_test".to_owned()),
        pid: std::process::id(),
        channel: "dev".to_owned(),
        app_id: "dev.warp.Warp".to_owned(),
        app_version: Some("test".to_owned()),
        started_at: chrono::Utc::now(),
        executable_path: None,
        endpoint: Some(local_control::discovery::ControlEndpoint::localhost(1)),
        credential_broker: Some(local_control::discovery::CredentialBrokerReference {
            endpoint: local_control::discovery::ControlEndpoint::localhost(1),
        }),
        outside_warp_control_enabled: true,
        actions: local_control::ActionKind::implemented_metadata(),
    }
}

fn write_test_instance_record(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("temp discovery dir is created");
    let record = test_instance_record();
    let path = dir.join("inst_test.json");
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&record).expect("record serializes"),
    )
    .expect("record is written");
}
#[test]
fn parses_first_slice_tab_create() {
    let args = ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--instance", "inst_123"])
        .expect("tab create parses");
    let ControlCommand::Tab(TabCommand::Create(target)) = args.command else {
        panic!("expected tab create command");
    };
    assert_eq!(target.target.instance.as_deref(), Some("inst_123"));
    assert_eq!(target.tab_type, TabType::Terminal);
}

#[test]
fn parses_first_slice_instance_list() {
    let args = ControlArgs::try_parse_from(["warpctrl", "instance", "list"])
        .expect("instance list parses");
    assert!(matches!(
        args.command,
        ControlCommand::Instance(InstanceCommand::List)
    ));
}

#[test]
fn parses_first_slice_app_smoke_metadata_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "ping"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "version"]).is_ok());
}
#[test]
fn parses_auth_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "status"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "login"]).is_ok());
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "auth",
            "api-key",
            "set",
            "--key-env",
            "WARPCTRL_TEST_KEY",
        ])
        .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "status"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "revoke"]).is_ok());
}

#[test]
fn rejects_multiple_api_key_sources() {
    let err = ControlArgs::try_parse_from([
        "warpctrl",
        "auth",
        "api-key",
        "set",
        "--key-env",
        "WARPCTRL_TEST_KEY",
        "--key-stdin",
    ])
    .expect_err("multiple key sources are rejected");
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn parses_completion_generation_command() {
    let args = ControlArgs::try_parse_from(["warpctrl", "completions", "bash"])
        .expect("completions parses");
    assert!(matches!(
        args.command,
        ControlCommand::Completions {
            shell: Some(Shell::Bash)
        }
    ));
}

#[test]
fn parses_shared_selector_aliases() {
    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "pane",
        "split",
        "--direction",
        "right",
        "--instance",
        "inst_123",
        "--window-id",
        "win_123",
        "--tab-index",
        "2",
        "--pane",
        "active",
        "--session-id",
        "sess_123",
        "--block-index",
        "4",
    ])
    .expect("pane split parses");
    let ControlCommand::Pane(PaneCommand::Split(args)) = args.command else {
        panic!("expected pane split command");
    };
    assert_eq!(args.target.instance.as_deref(), Some("inst_123"));
    assert_eq!(args.target.window_id.as_deref(), Some("win_123"));
    assert_eq!(args.target.tab_index, Some(2));
    assert_eq!(args.target.pane.as_deref(), Some("active"));
    assert_eq!(args.target.session_id.as_deref(), Some("sess_123"));
    assert_eq!(args.target.block_index, Some(4));
}

#[test]
fn rejects_conflicting_shared_selectors() {
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "tab",
            "list",
            "--tab-id",
            "tab_123",
            "--tab-index",
            "1",
        ])
        .is_err()
    );
}

#[test]
fn converts_protocol_target_selectors() {
    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "tab",
        "create",
        "--window",
        "title:Build logs",
        "--tab",
        "index:3",
        "--pane-id",
        "pane_123",
    ])
    .expect("tab create parses");
    let ControlCommand::Tab(TabCommand::Create(args)) = args.command else {
        panic!("expected tab create command");
    };
    let target = selectors::target_selector(args.target).expect("selectors convert");
    assert!(matches!(
        target.window,
        Some(WindowTarget::Title { ref title }) if title == "Build logs"
    ));
    assert!(matches!(target.tab, Some(TabTarget::Index { index: 3 })));
    assert!(matches!(
        target.pane,
        Some(PaneTarget::Id { ref id }) if id.0 == "pane_123"
    ));
}

#[test]
fn parses_read_only_command_surface() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "instance", "inspect"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "active"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "capability", "list"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "window", "list"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "tab", "inspect", "--tab", "active"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "pane", "list", "--tab", "active"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "session", "inspect", "--session-id", "sess_1"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "block",
            "output",
            "--block-id",
            "block_1",
            "--plain"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "input", "get", "--session", "active"]).is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "history", "list", "--limit", "5"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "theme", "list"]).is_ok());
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "setting",
            "get",
            "appearance.themes.system_theme"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "keybinding", "get", "open_command_palette"])
            .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "action", "inspect", "tab.create"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "list"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "project", "active"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "list", "--type", "workflow"]).is_ok()
    );
}

#[test]
fn parses_mutating_command_surface_without_execution_submit() {
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "window", "create", "--shell", "zsh"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "window", "close", "--window-title", "Scratch"])
            .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--type", "agent"]).is_ok());
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "tab",
            "rename",
            "Build logs",
            "--tab-id",
            "tab_1"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "color", "set", "blue", "--tab", "active"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "navigate", "--direction", "previous"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "input",
            "insert",
            "cargo check",
            "--session-id",
            "sess_1"
        ])
        .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "input", "replace", "cargo check"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "input", "clear"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "input", "run", "cargo check"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "input", "mode", "set", "agent"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "theme", "system", "set", "true"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "appearance", "font-size", "increase"]).is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "appearance", "zoom", "reset"]).is_ok());
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "setting",
            "toggle",
            "editor.syntax_highlighting"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "surface",
            "settings",
            "open",
            "--page",
            "scripting"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "surface",
            "command-palette",
            "open",
            "--query",
            "Settings"
        ])
        .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "surface", "warp-drive", "toggle"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "file", "open", "src/main.rs", "--line", "10"])
            .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "project", "open", "/tmp/project"]).is_ok());
}

#[test]
fn parses_drive_share_surface_and_native_team_share_only() {
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "drive",
            "object",
            "create",
            "--type",
            "notebook",
            "--content",
            "hello"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "drive",
            "object",
            "update",
            "obj_1",
            "--content-file",
            "/tmp/notebook.json"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "object", "delete", "obj_1"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "object", "insert", "obj_1"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "object", "share", "open", "obj_1"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "object", "share-to-team", "obj_1"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "object", "share", "external", "obj_1"])
            .is_err()
    );
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "drive",
            "object",
            "public-link",
            "create",
            "obj_1"
        ])
        .is_err()
    );
}

#[test]
fn excludes_local_file_content_crud_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "read", "src/main.rs"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "create", "src/main.rs"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "write", "src/main.rs"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "append", "src/main.rs"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "delete", "src/main.rs"]).is_err());
}

#[test]
fn excludes_command_and_agent_prompt_submission() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "agent", "prompt", "hello"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "command", "accept"]).is_err());
}

#[test]
fn parses_auth_surface_stubs() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "status"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "login"]).is_ok());
    assert!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "auth",
            "api-key",
            "set",
            "--key-env",
            "WARP_SCRIPTING_API_KEY"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "set", "--key-stdin"]).is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "status"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "revoke"]).is_ok());
}

#[test]
fn parses_global_output_formats() {
    let args =
        ControlArgs::try_parse_from(["warpctrl", "--output-format", "ndjson", "instance", "list"])
            .expect("ndjson output parses");
    assert_eq!(args.output_format, crate::agent::OutputFormat::Ndjson);
}

#[test]
fn generated_bash_completions_include_expanded_commands() {
    let completions =
        generate_completion_string(Shell::Bash).expect("bash completions render to UTF-8");
    assert!(completions.contains("instance"));
    assert!(completions.contains("window"));
    assert!(completions.contains("pane"));
    assert!(completions.contains("drive"));
    assert!(completions.contains("auth"));
    assert!(completions.contains("completions"));
}

#[test]
fn structured_error_output_uses_stable_error_code() {
    let error = ControlError::new(ErrorCode::NoInstance, "no local Warp control instances");
    let value = serde_json::to_value(ErrorSummary {
        ok: false,
        error: &error,
    })
    .expect("error summary serializes");
    assert_eq!(value["ok"], json!(false));
    assert_eq!(value["error"]["code"], json!("no_instance"));
    assert_eq!(
        value["error"]["message"],
        json!("no local Warp control instances")
    );
}

#[test]
fn operator_readme_tracks_action_catalog_status() {
    let readme = include_str!("../../../specs/warp-control-cli/README.md");
    for action in ActionKind::ALL {
        let metadata = action.metadata();
        let status = match metadata.implementation_status {
            ActionImplementationStatus::Implemented => "status: implemented",
            ActionImplementationStatus::Stub => "status: stub",
        };
        assert!(
            readme.contains(&format!("action: `{}`, {status}", metadata.name)),
            "README must document implementation status for {}",
            metadata.name
        );
    }
}

#[test]
fn dogfood_warpctrl_skill_documents_current_boundaries() {
    let skill = include_str!("../../../resources/channel-gated-skills/dogfood/warpctrl/SKILL.md");
    for command in [
        "warpctrl --output-format json instance list",
        "warpctrl --output-format json action list",
        "warpctrl --output-format json tab create",
    ] {
        assert!(skill.contains(command), "skill must mention {command}");
    }
    for boundary in [
        "file read",
        "file write",
        "file append",
        "file delete",
        "accepted-command submission",
        "agent-prompt submission",
        "arbitrary internal dispatch",
        "public links",
    ] {
        assert!(
            skill.contains(boundary),
            "skill must document excluded boundary {boundary}"
        );
    }
    for sharing_path in ["drive object share open", "drive object share-to-team"] {
        assert!(
            skill.contains(sharing_path),
            "skill must document Drive sharing path {sharing_path}"
        );
    }
}

#[test]
fn parses_app_focus_command() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "app", "focus"])
            .expect("app focus parses")
            .command,
        ControlCommand::App(AppCommand::Focus(_))
    ));
}

#[test]
fn parses_window_mutation_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "window", "create"])
            .expect("window create parses")
            .command,
        ControlCommand::Window(WindowCommand::Create(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "window", "focus"])
            .expect("window focus parses")
            .command,
        ControlCommand::Window(WindowCommand::Focus(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "window", "close"])
            .expect("window close parses")
            .command,
        ControlCommand::Window(WindowCommand::Close(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "window", "close", "--window-title", "Scratch"])
            .expect("window close with selector parses")
            .command,
        ControlCommand::Window(WindowCommand::Close(_))
    ));
}

#[test]
fn parses_tab_mutation_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "activate"])
            .expect("tab activate parses")
            .command,
        ControlCommand::Tab(TabCommand::Activate(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "activate", "--previous"])
            .expect("tab activate previous parses")
            .command,
        ControlCommand::Tab(TabCommand::Activate(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "activate", "--next"])
            .expect("tab activate next parses")
            .command,
        ControlCommand::Tab(TabCommand::Activate(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "activate", "--last"])
            .expect("tab activate last parses")
            .command,
        ControlCommand::Tab(TabCommand::Activate(_))
    ));

    let args = ControlArgs::try_parse_from(["warpctrl", "tab", "move", "--direction", "right"])
        .expect("tab move parses");
    assert!(matches!(
        args.command,
        ControlCommand::Tab(TabCommand::Move(_))
    ));
    assert!(ControlArgs::try_parse_from(["warpctrl", "tab", "move"]).is_err());

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "close"])
            .expect("tab close parses")
            .command,
        ControlCommand::Tab(TabCommand::Close(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "close", "--others"])
            .expect("tab close with others parses")
            .command,
        ControlCommand::Tab(TabCommand::Close(_))
    ));
}

#[test]
fn parses_pane_mutation_commands() {
    let args = ControlArgs::try_parse_from(["warpctrl", "pane", "split", "--direction", "right"])
        .expect("pane split parses");
    assert!(matches!(
        args.command,
        ControlCommand::Pane(PaneCommand::Split(_))
    ));
    assert!(ControlArgs::try_parse_from(["warpctrl", "pane", "split"]).is_err());

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "focus"])
            .expect("pane focus parses")
            .command,
        ControlCommand::Pane(PaneCommand::Focus(_))
    ));

    let args = ControlArgs::try_parse_from(["warpctrl", "pane", "navigate", "--direction", "left"])
        .expect("pane navigate parses");
    assert!(matches!(
        args.command,
        ControlCommand::Pane(PaneCommand::Navigate(_))
    ));
    assert!(ControlArgs::try_parse_from(["warpctrl", "pane", "navigate"]).is_err());

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "close"])
            .expect("pane close parses")
            .command,
        ControlCommand::Pane(PaneCommand::Close(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "maximize"])
            .expect("pane maximize parses without enabled")
            .command,
        ControlCommand::Pane(PaneCommand::Maximize(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "unmaximize"])
            .expect("pane unmaximize parses")
            .command,
        ControlCommand::Pane(PaneCommand::Unmaximize(_))
    ));

    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "pane",
        "resize",
        "--direction",
        "up",
        "--amount",
        "3",
    ])
    .expect("pane resize parses");
    assert!(matches!(
        args.command,
        ControlCommand::Pane(PaneCommand::Resize(_))
    ));
    assert!(ControlArgs::try_parse_from(["warpctrl", "pane", "resize"]).is_err());
}

#[test]
#[serial]
fn tab_create_without_discovery_records_reports_no_instance() {
    let dir = std::env::temp_dir().join(format!(
        "warpctrl-empty-discovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("temp discovery dir is created");
    let previous = set_discovery_dir(&dir);
    let args =
        ControlArgs::try_parse_from(["warpctrl", "--output-format", "json", "tab", "create"])
            .expect("tab create parses");
    let error = run_inner(args).expect_err("missing instance is rejected");
    restore_discovery_dir(previous);
    assert_eq!(error.code, ErrorCode::NoInstance);
}

#[test]
fn tab_create_options_are_parser_only_until_handler_contract_lands() {
    let args = ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--type", "agent"])
        .expect("agent tab create parses");
    let error = run_inner(args).expect_err("agent tab create is a parser-only stub");
    assert_eq!(error.code, ErrorCode::UnsupportedAction);
}

#[test]
#[serial]
fn auth_login_reports_unsupported_until_app_sign_in_action_exists() {
    let discovery_dir = std::env::temp_dir().join(format!(
        "warpctrl-auth-login-discovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    write_test_instance_record(&discovery_dir);
    let previous_discovery = set_discovery_dir(&discovery_dir);
    let args =
        ControlArgs::try_parse_from(["warpctrl", "auth", "login"]).expect("auth login parses");
    let error = run_inner(args).expect_err("login broker action is not implemented");
    restore_discovery_dir(previous_discovery);
    assert_eq!(error.code, ErrorCode::UnsupportedAction);
}

#[test]
#[serial]
fn auth_status_without_discovery_records_reports_no_instance() {
    let dir = std::env::temp_dir().join(format!(
        "warpctrl-empty-auth-discovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("temp discovery dir is created");
    let previous = set_discovery_dir(&dir);
    let args =
        ControlArgs::try_parse_from(["warpctrl", "auth", "status"]).expect("auth status parses");
    let error = run_inner(args).expect_err("missing instance is rejected");
    restore_discovery_dir(previous);
    assert_eq!(error.code, ErrorCode::NoInstance);
}

#[test]
#[serial]
fn auth_api_key_set_rejects_missing_env_var() {
    let discovery_dir = std::env::temp_dir().join(format!(
        "warpctrl-auth-discovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    write_test_instance_record(&discovery_dir);
    let previous_discovery = set_discovery_dir(&discovery_dir);
    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "auth",
        "api-key",
        "set",
        "--key-env",
        "WARPCTRL_MISSING_TEST_KEY",
    ])
    .expect("api-key set parses");
    let error = run_inner(args).expect_err("missing env var is rejected");
    restore_discovery_dir(previous_discovery);
    assert_eq!(error.code, ErrorCode::InvalidParams);
}
