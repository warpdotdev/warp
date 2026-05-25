use std::ffi::OsString;

use clap::Parser as _;
use clap_complete::aot::Shell;
use local_control::protocol::{ControlError, ErrorCode};
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

#[test]
fn parses_first_slice_tab_create() {
    let args = ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--instance", "inst_123"])
        .expect("tab create parses");
    let ControlCommand::Tab(TabCommand::Create(target)) = args.command else {
        panic!("expected tab create command");
    };
    assert_eq!(target.instance.as_deref(), Some("inst_123"));
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
fn parses_settings_and_appearance_metadata_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "theme", "list"])
            .expect("theme list parses")
            .command,
        ControlCommand::Theme(ThemeCommand::List(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "appearance", "get"])
            .expect("appearance get parses")
            .command,
        ControlCommand::Appearance(AppearanceCommand::Get(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "setting", "list"])
            .expect("setting list parses")
            .command,
        ControlCommand::Setting(SettingCommand::List(_))
    ));

    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "setting",
        "get",
        "--instance",
        "inst_123",
        "appearance.themes.theme",
    ])
    .expect("setting get parses");
    let ControlCommand::Setting(SettingCommand::Get(setting)) = args.command else {
        panic!("expected setting get command");
    };
    assert_eq!(setting.target.instance.as_deref(), Some("inst_123"));
    assert_eq!(setting.key, "appearance.themes.theme");
}

#[test]
fn parses_first_slice_app_smoke_metadata_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "ping"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "version"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "active"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "inspect"]).is_ok());
}

#[test]
fn parses_action_metadata_commands() {
    let args = ControlArgs::try_parse_from(["warpctrl", "action", "list", "--pid", "123"])
        .expect("action list parses");
    let ControlCommand::Action(ActionCommand::List(target)) = args.command else {
        panic!("expected action list command");
    };
    assert_eq!(target.pid, Some(123));

    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "action",
        "get",
        "--instance",
        "inst_123",
        "window.list",
    ])
    .expect("action get parses");
    let ControlCommand::Action(ActionCommand::Get(action)) = args.command else {
        panic!("expected action get command");
    };
    assert_eq!(action.target.instance.as_deref(), Some("inst_123"));
    assert_eq!(action.action, "window.list");
}

#[test]
fn parses_structural_metadata_list_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "window", "list"])
            .expect("window list parses")
            .command,
        ControlCommand::Window(WindowCommand::List(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "list"])
            .expect("tab list parses")
            .command,
        ControlCommand::Tab(TabCommand::List(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "list"])
            .expect("pane list parses")
            .command,
        ControlCommand::Pane(PaneCommand::List(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "session", "list"])
            .expect("session list parses")
            .command,
        ControlCommand::Session(SessionCommand::List(_))
    ));
}

#[test]
fn parses_underlying_data_read_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "block", "list", "--limit", "10"])
            .expect("block list parses")
            .command,
        ControlCommand::Block(BlockCommand::List(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "block", "get", "block_123"])
            .expect("block get parses")
            .command,
        ControlCommand::Block(BlockCommand::Get(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "input", "get"])
            .expect("input get parses")
            .command,
        ControlCommand::Input(InputCommand::Get(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "history", "list", "--limit", "20"])
            .expect("history list parses")
            .command,
        ControlCommand::History(HistoryCommand::List(_))
    ));
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
fn rejects_unknown_top_level_subcommands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "notebook", "list"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "run", "my-workflow"]).is_err());
}

#[test]
fn parses_auth_status_command() {
    let args =
        ControlArgs::try_parse_from(["warpctrl", "auth", "status"]).expect("auth status parses");
    assert!(matches!(
        args.command,
        ControlCommand::Auth(AuthCommand::Status(_))
    ));
}

#[test]
fn parses_auth_login_command() {
    let args =
        ControlArgs::try_parse_from(["warpctrl", "auth", "login"]).expect("auth login parses");
    assert!(matches!(
        args.command,
        ControlCommand::Auth(AuthCommand::Login(_))
    ));
}

#[test]
fn parses_auth_api_key_set_with_env_var() {
    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "auth",
        "api-key",
        "set",
        "--key-env",
        "WARPCTRL_API_KEY",
    ])
    .expect("auth api-key set --key-env parses");
    let ControlCommand::Auth(AuthCommand::ApiKey(ApiKeySubcommand::Set(set_args))) = args.command
    else {
        panic!("expected auth api-key set command");
    };
    assert_eq!(set_args.source.key_env.as_deref(), Some("WARPCTRL_API_KEY"));
    assert!(!set_args.source.key_stdin);
}

#[test]
fn parses_auth_api_key_set_with_stdin() {
    let args = ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "set", "--key-stdin"])
        .expect("auth api-key set --key-stdin parses");
    let ControlCommand::Auth(AuthCommand::ApiKey(ApiKeySubcommand::Set(set_args))) = args.command
    else {
        panic!("expected auth api-key set command");
    };
    assert!(set_args.source.key_stdin);
    assert!(set_args.source.key_env.is_none());
}

#[test]
fn rejects_auth_api_key_set_without_key_source() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "set"]).is_err());
}

#[test]
fn parses_auth_api_key_status_command() {
    let args = ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "status"])
        .expect("auth api-key status parses");
    assert!(matches!(
        args.command,
        ControlCommand::Auth(AuthCommand::ApiKey(ApiKeySubcommand::Status(_)))
    ));
}

#[test]
fn parses_auth_api_key_revoke_command() {
    let args = ControlArgs::try_parse_from(["warpctrl", "auth", "api-key", "revoke"])
        .expect("auth api-key revoke parses");
    assert!(matches!(
        args.command,
        ControlCommand::Auth(AuthCommand::ApiKey(ApiKeySubcommand::Revoke(_)))
    ));
}

#[test]
fn generated_bash_completions_include_metadata_commands() {
    let completions =
        generate_completion_string(Shell::Bash).expect("bash completions render to UTF-8");
    assert!(completions.contains("instance"));
    assert!(completions.contains("app"));
    assert!(completions.contains("action"));
    assert!(completions.contains("window"));
    assert!(completions.contains("tab"));
    assert!(completions.contains("pane"));
    assert!(completions.contains("session"));
    assert!(completions.contains("block"));
    assert!(completions.contains("input"));
    assert!(completions.contains("history"));
    assert!(completions.contains("theme"));
    assert!(completions.contains("appearance"));
    assert!(completions.contains("setting"));
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

    let args = ControlArgs::try_parse_from(["warpctrl", "window", "close", "--force"])
        .expect("window close --force parses");
    let ControlCommand::Window(WindowCommand::Close(close)) = args.command else {
        panic!("expected window close command");
    };
    assert!(close.force);
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
        ControlArgs::try_parse_from(["warpctrl", "tab", "previous"])
            .expect("tab previous parses")
            .command,
        ControlCommand::Tab(TabCommand::Previous(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "next"])
            .expect("tab next parses")
            .command,
        ControlCommand::Tab(TabCommand::Next(_))
    ));
    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "last"])
            .expect("tab last parses")
            .command,
        ControlCommand::Tab(TabCommand::Last(_))
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
        ControlArgs::try_parse_from(["warpctrl", "tab", "close", "--scope", "others"])
            .expect("tab close with scope parses")
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
        ControlArgs::try_parse_from(["warpctrl", "pane", "maximize", "--enabled", "true"])
            .expect("pane maximize parses with enabled")
            .command,
        ControlCommand::Pane(PaneCommand::Maximize(_))
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
fn parses_file_mutation_commands() {
    let args =
        ControlArgs::try_parse_from(["warpctrl", "file", "write", "/tmp/test.txt", "hello world"])
            .expect("file write parses");
    let ControlCommand::File(FileCommand::Write(write_args)) = args.command else {
        panic!("expected file write command");
    };
    assert_eq!(write_args.path, "/tmp/test.txt");
    assert_eq!(write_args.contents, "hello world");

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "file", "delete", "/tmp/test.txt"])
            .expect("file delete parses")
            .command,
        ControlCommand::File(FileCommand::Delete(_))
    ));

    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "write"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "delete"]).is_err());
}

#[test]
fn parses_drive_mutation_commands() {
    assert!(matches!(
        ControlArgs::try_parse_from([
            "warpctrl",
            "drive",
            "create",
            "--type",
            "workflow",
            "build",
            "{\"command\":\"cargo check\"}",
        ])
        .expect("drive create parses")
        .command,
        ControlCommand::Drive(DriveCommand::Create(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from([
            "warpctrl", "drive", "delete", "--type", "workflow", "abc123"
        ])
        .expect("drive delete parses")
        .command,
        ControlCommand::Drive(DriveCommand::Delete(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from(["warpctrl", "drive", "run", "--type", "workflow", "abc123"])
            .expect("drive run parses")
            .command,
        ControlCommand::Drive(DriveCommand::Run(_))
    ));

    assert!(matches!(
        ControlArgs::try_parse_from([
            "warpctrl", "drive", "insert", "--type", "notebook", "abc123"
        ])
        .expect("drive insert parses")
        .command,
        ControlCommand::Drive(DriveCommand::Insert(_))
    ));

    assert!(ControlArgs::try_parse_from(["warpctrl", "drive", "create"]).is_err());
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
