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
fn parses_tab_create() {
    let args = ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--instance", "inst_123"])
        .expect("tab create parses");
    let ControlCommand::Tab(TabCommand::Create(target)) = args.command else {
        panic!("expected tab create command");
    };
    assert_eq!(target.instance.as_deref(), Some("inst_123"));
}

#[test]
fn parses_instance_list() {
    let args = ControlArgs::try_parse_from(["warpctrl", "instance", "list"])
        .expect("instance list parses");
    assert!(matches!(
        args.command,
        ControlCommand::Instance(InstanceCommand::List)
    ));
}

#[test]
fn parses_app_metadata_commands() {
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
fn rejects_non_metadata_and_future_catalog_commands_not_in_this_shard() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "setting", "list"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "drive", "list"]).is_err());
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
