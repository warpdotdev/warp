use super::*;

#[test]
fn interpret_dev_container_up_output_ready_to_attach_on_full_success() {
    let stdout = r#"Some progress line
{"outcome":"success","containerId":"abc123","remoteUser":"vscode","remoteWorkspaceFolder":"/workspaces/project"}"#;
    let outcome = interpret_dev_container_up_output(true, stdout.as_bytes(), b"");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::ReadyToAttach {
            container_id: "abc123".to_owned(),
            remote_user: Some("vscode".to_owned()),
            remote_workspace_folder: "/workspaces/project".to_owned(),
        }
    );
}

#[test]
fn interpret_dev_container_up_output_ready_to_attach_without_remote_user() {
    let stdout = r#"{"outcome":"success","containerId":"abc123","remoteWorkspaceFolder":"/workspaces/project"}"#;
    let outcome = interpret_dev_container_up_output(true, stdout.as_bytes(), b"");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::ReadyToAttach {
            container_id: "abc123".to_owned(),
            remote_user: None,
            remote_workspace_folder: "/workspaces/project".to_owned(),
        }
    );
}

#[test]
fn interpret_dev_container_up_output_errors_on_missing_container_id() {
    let stdout = r#"{"outcome":"success","remoteWorkspaceFolder":"/workspaces/project"}"#;
    let outcome = interpret_dev_container_up_output(true, stdout.as_bytes(), b"");

    assert!(
        matches!(outcome, DevContainerUpOutcome::Error(message) if message.contains("didn't report"))
    );
}

#[test]
fn interpret_dev_container_up_output_errors_on_missing_remote_workspace_folder() {
    let stdout = r#"{"outcome":"success","containerId":"abc123"}"#;
    let outcome = interpret_dev_container_up_output(true, stdout.as_bytes(), b"");

    assert!(
        matches!(outcome, DevContainerUpOutcome::Error(message) if message.contains("didn't report"))
    );
}

#[test]
fn interpret_dev_container_up_output_uses_structured_message_on_failure_exit_status() {
    let stdout = r#"{"outcome":"error","message":"Command failed: docker pull nope:latest"}"#;
    let outcome = interpret_dev_container_up_output(false, stdout.as_bytes(), b"some stderr noise");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error(
            "Dev container failed to start: Command failed: docker pull nope:latest".to_owned()
        )
    );
}

#[test]
fn interpret_dev_container_up_output_falls_back_to_stderr_tail_when_unparseable() {
    let outcome = interpret_dev_container_up_output(false, b"not json at all", b"boom");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error("Dev container failed to start:\nboom".to_owned())
    );
}

#[test]
fn interpret_dev_container_up_output_errors_when_success_exit_but_outcome_is_error() {
    // A process can exit 0 while `devcontainer up`'s own JSON still reports
    // an error outcome; make sure that's not misread as ready-to-attach.
    let stdout = r#"{"outcome":"error","message":"something went wrong"}"#;
    let outcome = interpret_dev_container_up_output(true, stdout.as_bytes(), b"");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error(
            "Dev container failed to start: something went wrong".to_owned()
        )
    );
}

#[test]
fn dev_container_up_failure_message_prefers_structured_description_over_stderr() {
    let stdout = r#"{"outcome":"error","description":"no space left on device"}"#;
    let message = dev_container_up_failure_message(stdout.as_bytes(), b"irrelevant stderr");

    assert_eq!(
        message,
        "Dev container failed to start: no space left on device"
    );
}

#[test]
fn dev_container_up_failure_message_falls_back_to_stderr_tail_and_trims_blank_lines() {
    let stderr = "line one\n\nline two\n";
    let message = dev_container_up_failure_message(b"not json", stderr.as_bytes());

    assert_eq!(
        message,
        "Dev container failed to start:\nline one\nline two"
    );
}

#[test]
fn dev_container_preflight_args_without_remote_user() {
    let args = dev_container_preflight_args("abc123", None, "/workspaces/project");

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-w"),
            std::ffi::OsString::from("/workspaces/project"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("sh"),
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from("command -v bash"),
        ]
    );
}

#[test]
fn dev_container_preflight_args_with_remote_user() {
    let args = dev_container_preflight_args("abc123", Some("vscode"), "/workspaces/project");

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("vscode"),
            std::ffi::OsString::from("-w"),
            std::ffi::OsString::from("/workspaces/project"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("sh"),
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from("command -v bash"),
        ]
    );
}

#[test]
fn dev_container_preflight_args_checks_bash() {
    let args = dev_container_preflight_args("abc123", None, "/workspaces/project");
    let check = args.last().expect("args should be non-empty");

    assert_eq!(check, "command -v bash");
}

#[test]
fn parse_dev_container_up_stdout_reads_the_last_line_only() {
    let stdout = "some progress\nmore progress\n{\"outcome\":\"success\"}";
    let result = parse_dev_container_up_stdout(stdout.as_bytes()).expect("should parse");
    assert_eq!(result.outcome, "success");
}

#[test]
fn parse_dev_container_up_stdout_returns_none_for_empty_or_malformed_input() {
    assert!(parse_dev_container_up_stdout(b"").is_none());
    assert!(parse_dev_container_up_stdout(b"not json").is_none());
}

#[test]
fn discover_dev_container_configs_finds_nothing_when_no_devcontainer_json_exists() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    assert!(discover_dev_container_configs(workspace.path()).is_empty());
}

#[test]
fn discover_dev_container_configs_finds_the_top_level_config() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let devcontainer_dir = workspace.path().join(".devcontainer");
    std::fs::create_dir_all(&devcontainer_dir).expect("create .devcontainer");
    let config_path = devcontainer_dir.join("devcontainer.json");
    std::fs::write(&config_path, "{}").expect("write devcontainer.json");

    assert_eq!(
        discover_dev_container_configs(workspace.path()),
        vec![config_path]
    );
}

#[test]
fn discover_dev_container_configs_finds_the_root_level_config() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let config_path = workspace.path().join(".devcontainer.json");
    std::fs::write(&config_path, "{}").expect("write .devcontainer.json");

    assert_eq!(
        discover_dev_container_configs(workspace.path()),
        vec![config_path]
    );
}

#[test]
fn discover_dev_container_configs_finds_nested_configs_sorted_by_folder_name() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let devcontainer_dir = workspace.path().join(".devcontainer");
    for folder in ["web", "api"] {
        let nested_dir = devcontainer_dir.join(folder);
        std::fs::create_dir_all(&nested_dir).expect("create nested devcontainer folder");
        std::fs::write(nested_dir.join("devcontainer.json"), "{}")
            .expect("write nested devcontainer.json");
    }

    assert_eq!(
        discover_dev_container_configs(workspace.path()),
        vec![
            devcontainer_dir.join("api").join("devcontainer.json"),
            devcontainer_dir.join("web").join("devcontainer.json"),
        ]
    );
}

#[test]
fn discover_dev_container_configs_finds_all_three_shapes_together_in_spec_order() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let devcontainer_dir = workspace.path().join(".devcontainer");
    std::fs::create_dir_all(&devcontainer_dir).expect("create .devcontainer");
    std::fs::write(devcontainer_dir.join("devcontainer.json"), "{}")
        .expect("write top-level devcontainer.json");
    let nested_dir = devcontainer_dir.join("web");
    std::fs::create_dir_all(&nested_dir).expect("create nested devcontainer folder");
    std::fs::write(nested_dir.join("devcontainer.json"), "{}")
        .expect("write nested devcontainer.json");
    std::fs::write(workspace.path().join(".devcontainer.json"), "{}")
        .expect("write root-level .devcontainer.json");

    assert_eq!(
        discover_dev_container_configs(workspace.path()),
        vec![
            devcontainer_dir.join("devcontainer.json"),
            workspace.path().join(".devcontainer.json"),
            nested_dir.join("devcontainer.json"),
        ]
    );
}

#[test]
fn discover_dev_container_configs_puts_root_ahead_of_nested_when_top_level_is_absent() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let nested_dir = workspace.path().join(".devcontainer").join("web");
    std::fs::create_dir_all(&nested_dir).expect("create nested devcontainer folder");
    std::fs::write(nested_dir.join("devcontainer.json"), "{}")
        .expect("write nested devcontainer.json");
    let root_config = workspace.path().join(".devcontainer.json");
    std::fs::write(&root_config, "{}").expect("write root-level .devcontainer.json");

    assert_eq!(
        discover_dev_container_configs(workspace.path()),
        vec![root_config, nested_dir.join("devcontainer.json")]
    );
}

#[test]
fn discover_dev_container_configs_ignores_non_directory_entries_in_devcontainer_dir() {
    let workspace = tempfile::tempdir().expect("create temp workspace");
    let devcontainer_dir = workspace.path().join(".devcontainer");
    std::fs::create_dir_all(&devcontainer_dir).expect("create .devcontainer");
    // A stray file (e.g. a Dockerfile referenced by the top-level config) should not be
    // mistaken for a nested config folder.
    std::fs::write(devcontainer_dir.join("Dockerfile"), "FROM scratch").expect("write stray file");

    assert!(discover_dev_container_configs(workspace.path()).is_empty());
}
