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
fn rejects_conflicting_instance_selectors() {
    let err = ControlArgs::try_parse_from([
        "warpctrl",
        "tab",
        "create",
        "--instance",
        "inst_123",
        "--pid",
        "123",
    ])
    .expect_err("instance and pid conflict");
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}
#[test]
fn parses_pid_instance_selector() {
    let args = ControlArgs::try_parse_from(["warpctrl", "app", "ping", "--pid", "123"])
        .expect("pid selector parses");
    let ControlCommand::App(AppCommand::Ping(target)) = args.command else {
        panic!("expected app ping command");
    };
    assert_eq!(target.pid, Some(123));
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
fn every_implemented_catalog_action_has_a_parser_route() {
    let routes = [
        (
            local_control::protocol::ActionKind::InstanceList,
            vec!["warpctrl", "instance", "list"],
        ),
        (
            local_control::protocol::ActionKind::AppPing,
            vec!["warpctrl", "app", "ping"],
        ),
        (
            local_control::protocol::ActionKind::AppVersion,
            vec!["warpctrl", "app", "version"],
        ),
        (
            local_control::protocol::ActionKind::TabCreate,
            vec!["warpctrl", "tab", "create"],
        ),
    ];
    let implemented = local_control::protocol::ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        implemented,
        routes.iter().map(|(action, _)| *action).collect::<Vec<_>>()
    );
    for (_, args) in routes {
        ControlArgs::try_parse_from(args).expect("implemented action parser route exists");
    }
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
fn rejects_future_catalog_commands_not_in_first_slice() {
    assert!(ControlArgs::try_parse_from(["warpctrl", "window", "list"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "tab", "list"]).is_err());
    assert!(ControlArgs::try_parse_from(["warpctrl", "setting", "list"]).is_err());
}

#[test]
fn generated_bash_completions_include_first_slice_commands() {
    let completions =
        generate_completion_string(Shell::Bash).expect("bash completions render to UTF-8");
    assert!(completions.contains("instance"));
    assert!(completions.contains("tab"));
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
fn renders_human_readable_tab_create_output() {
    let rendered = render_human_readable_for_test(
        local_control::protocol::ActionKind::TabCreate,
        &json!({
            "tab": {
                "id": "tab_123",
                "active_index": 2,
                "count": 3
            },
            "window": {
                "id": "window_123"
            }
        }),
    );
    assert_eq!(
        rendered,
        "Created tab tab_123 in window window_123 (active index 2, tab count 3)"
    );
}

#[test]
#[serial]
fn instance_list_without_discovery_records_succeeds() {
    let dir = std::env::temp_dir().join(format!(
        "warpctrl-empty-discovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("temp discovery dir is created");
    let previous = set_discovery_dir(&dir);
    let args = ControlArgs::try_parse_from(["warpctrl", "instance", "list"])
        .expect("instance list parses");
    let result = run_inner(args);
    restore_discovery_dir(previous);
    result.expect("empty instance list succeeds");
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
