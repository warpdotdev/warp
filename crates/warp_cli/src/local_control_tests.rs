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
fn parses_first_slice_app_smoke_metadata_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "ping"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "version"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "active"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "app", "focus"]).is_ok());
}

#[test]
fn parses_catalog_metadata_commands() {
    let args =
        ControlArgs::try_parse_from(["warpctrl", "action", "inspect", "surface.settings.open"])
            .expect("action inspect parses");
    let ControlCommand::Action(ActionCatalogCommand::Inspect { action }) = args.command else {
        panic!("expected action inspect command");
    };
    assert_eq!(action, "surface.settings.open");
    assert!(ControlArgs::try_parse_from(["warpctrl", "action", "list", "--stubs-only"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "capability", "list", "--implemented-only",])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "capability", "inspect", "tab.create"]).is_ok()
    );
}
#[test]
fn parses_control_mode_args_after_hidden_flag() {
    let args = ControlArgs::try_parse_control_mode_from([
        "warp",
        "--warpctrl",
        "tab",
        "create",
        "--instance",
        "inst_123",
    ])
    .expect("control mode flag is present")
    .expect("control mode args parse");
    let ControlCommand::Tab(TabCommand::Create(target)) = args.command else {
        panic!("expected tab create command");
    };
    assert_eq!(target.instance.as_deref(), Some("inst_123"));
}

#[test]
fn ignores_args_without_control_mode_flag() {
    assert!(ControlArgs::try_parse_control_mode_from(["warp", "tab", "create"]).is_none());
}

#[test]
fn parses_execution_underlying_commands() {
    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "input",
        "run",
        "cargo check",
        "--instance",
        "inst_123",
    ])
    .expect("input run parses");
    let ControlCommand::Input(InputCommand::Run(input_args)) = args.command else {
        panic!("expected input run command");
    };
    assert_eq!(input_args.text, "cargo check");
    assert_eq!(input_args.target.instance.as_deref(), Some("inst_123"));

    let args = ControlArgs::try_parse_from([
        "warpctrl",
        "drive",
        "workflow",
        "run",
        "workflow_123",
        "--arg",
        "name=value",
    ])
    .expect("drive workflow run parses");
    let ControlCommand::Drive(DriveCommand::Workflow(DriveWorkflowCommand::Run(workflow_args))) =
        args.command
    else {
        panic!("expected drive workflow run command");
    };
    assert_eq!(workflow_args.id, "workflow_123");
    assert_eq!(workflow_args.args[0].name, "name");
    assert_eq!(workflow_args.args[0].value, "value");
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
fn parses_readonly_capability_and_target_commands() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "instance", "inspect"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "capability", "list"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "capability", "inspect", "tab.create"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "action", "inspect", "drive.inspect"]).is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "window", "list"]).is_ok());
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "window", "inspect", "--window", "active"])
            .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "tab", "inspect", "--tab-index", "0"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "pane", "inspect", "--pane", "active"]).is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["warpctrl", "session", "inspect", "--session", "active"])
            .is_ok()
    );
    assert!(ControlArgs::try_parse_from(["warpctrl", "block", "output", "block_1"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "input", "get"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "history", "list", "--limit", "5"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "theme", "get"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "keybinding", "get", "copy"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "file", "list"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "project", "active"]).is_ok());
    assert!(ControlArgs::try_parse_from(["warpctrl", "drive", "inspect", "drive_1"]).is_ok());
}

#[test]
fn excluded_actions_are_not_allowlisted_catalog_entries() {
    let args = ControlArgs::try_parse_from(["warpctrl", "action", "inspect", "auth.api_key.set"])
        .expect("action inspect parses arbitrary action name");
    let error = run_inner(args).expect_err("excluded auth api-key action is not allowlisted");
    assert_eq!(error.code, ErrorCode::NotAllowlisted);
    let args = ControlArgs::try_parse_from(["warpctrl", "action", "inspect", "file.write"])
        .expect("action inspect parses arbitrary action name");
    let error = run_inner(args).expect_err("excluded file mutation action is not allowlisted");
    assert_eq!(error.code, ErrorCode::NotAllowlisted);
}

#[test]
fn generated_bash_completions_include_readonly_commands() {
    let completions =
        generate_completion_string(Shell::Bash).expect("bash completions render to UTF-8");
    assert!(completions.contains("instance"));
    assert!(completions.contains("action"));
    assert!(completions.contains("capability"));
    assert!(completions.contains("stubs-only"));
    assert!(completions.contains("window"));
    assert!(completions.contains("block"));
    assert!(completions.contains("input"));
    assert!(completions.contains("drive"));
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
