use super::*;

#[test]
fn interpret_dev_container_up_output_does_not_attach_when_ordinary_tail_follows_outcome() {
    let stdout = r#"{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}
ordinary-tail"#;
    let drain = futures_lite::future::block_on(stream::drain_dev_container_pipes(
        futures_lite::io::Cursor::new(stdout.as_bytes().to_vec()),
        futures_lite::io::Cursor::new(Vec::<u8>::new()),
        |_| {},
    ))
    .expect("drain");
    let outcome = interpret_dev_container_up_output(true, &drain.stdout.bytes, &drain.stderr_tail);
    assert!(
        matches!(outcome, DevContainerUpOutcome::Error(_)),
        "stale outcome before ordinary tail must not attach, got {outcome:?}"
    );
}

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
    let outcome = interpret_dev_container_up_output(false, stdout.as_bytes(), b"");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error(
            "Dev container failed to start:\nCommand failed: docker pull nope:latest".to_owned()
        )
    );
}

#[test]
fn interpret_dev_container_up_output_falls_back_to_stderr_tail_when_unparseable() {
    let outcome = interpret_dev_container_up_output(false, b"not json at all", b"boom");

    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error(
            "Dev container failed to start:\nnot json at all\nboom".to_owned()
        )
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
            "Dev container failed to start:\nsomething went wrong".to_owned()
        )
    );
}

#[test]
fn dev_container_up_failure_message_prefers_structured_description_over_stderr() {
    let stdout = r#"{"outcome":"error","description":"no space left on device"}"#;
    let message = dev_container_up_failure_message(stdout.as_bytes(), b"");

    assert_eq!(
        message,
        "Dev container failed to start:\nno space left on device"
    );
}

#[test]
fn dev_container_up_failure_message_falls_back_to_stderr_tail_and_trims_blank_lines() {
    let stderr = "line one\n\nline two\n";
    let message = dev_container_up_failure_message(b"not json", stderr.as_bytes());

    assert_eq!(
        message,
        "Dev container failed to start:\nnot json\nline one\nline two"
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
    assert!(parse_dev_container_up_stdout(b"{\"outcome\":").is_none());
}

#[test]
fn interpret_dev_container_up_output_errors_on_missing_or_incomplete_stdout() {
    assert!(matches!(
        interpret_dev_container_up_output(true, b"", b""),
        DevContainerUpOutcome::Error(_)
    ));
    assert!(matches!(
        interpret_dev_container_up_output(true, b"{\"outcome\":", b""),
        DevContainerUpOutcome::Error(_)
    ));
}

#[test]
fn interpret_docker_ps_probe_failure_keeps_command_and_underlying_stderr() {
    let stdout = r#"{"outcome":"error","message":"Command failed: docker ps -q --filter label=devcontainer.local_folder=/private/tmp/ha-core --filter label=devcontainer.config_file=/private/tmp/ha-core/.devcontainer/devcontainer.json"}"#;
    let stderr = "[2026-09-02T00:43:15.960Z] @devcontainers/cli 0.89.0. Node.js v24.17.0. darwin 25.6.0 arm64.\nCannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?\n";
    let outcome = interpret_dev_container_up_output(false, stdout.as_bytes(), stderr.as_bytes());
    let DevContainerUpOutcome::Error(message) = outcome else {
        panic!("expected a failed outcome");
    };
    assert!(
        message.contains("Command failed: docker ps"),
        "structured CLI wrapper must be kept, got {message:?}"
    );
    assert!(
        !message.contains("@devcontainers/cli"),
        "CLI version banner is not the failure cause, got {message:?}"
    );
    assert!(
        !message.contains("\u{1b}"),
        "failure text must not include raw ANSI, got {message:?}"
    );
    assert!(
        message.contains("Cannot connect to the Docker daemon"),
        "unique useful stderr must accompany structured text, got {message:?}"
    );
}

#[test]
fn failure_message_keeps_novel_stderr_after_repeated_structured_prefix() {
    let stdout = r#"{"outcome":"error","message":"Command failed: docker pull nope:latest"}"#;
    let stderr = "Command failed: docker pull nope:latest\n\
Cannot connect to the Docker daemon\n\
Cannot connect to the Docker daemon\n\
Command failed: docker pull nope:latest: manifest unknown\n";
    let message = dev_container_up_failure_message(stdout.as_bytes(), stderr.as_bytes());
    assert_eq!(
        message
            .matches("Command failed: docker pull nope:latest")
            .count(),
        2,
        "exact prefix line is dropped, enriched cause line is kept, got {message:?}"
    );
    assert_eq!(
        message
            .matches("Cannot connect to the Docker daemon")
            .count(),
        1,
        "duplicate stderr diagnostic must appear once, got {message:?}"
    );
    assert_eq!(
        message.matches("manifest unknown").count(),
        1,
        "novel registry/daemon cause must be kept once, got {message:?}"
    );
}

#[test]
fn interpret_dev_container_up_output_uses_bounded_stderr_tail_fallback() {
    let stderr = "keep-me\nthis-is-the-tail\n";
    let outcome = interpret_dev_container_up_output(false, b"", stderr.as_bytes());
    assert_eq!(
        outcome,
        DevContainerUpOutcome::Error(
            "Dev container failed to start:\nkeep-me\nthis-is-the-tail".to_owned()
        )
    );
}

#[test]
fn leftover_stdout_strips_ansi_from_failure_text() {
    let stdout = "\u{1b}[31mContainer started\u{1b}[0m\n\u{1b}]0;title\u{7}not a result\n";
    let message = dev_container_up_failure_message(stdout.as_bytes(), b"");
    assert!(
        message.contains("Container started"),
        "plain leftover stdout must remain, got {message:?}"
    );
    assert!(
        !message.contains('\u{1b}'),
        "CSI/OSC must not leak into failure text, got {message:?}"
    );
}

#[test]
fn interpret_dev_container_up_output_strips_tty_redraw_from_stderr_fallback() {
    let stderr = "\u{1b}[1A\u{1b}[K#15 extracting 1MB\r\u{1b}[1A\u{1b}[KCannot connect to the Docker daemon\n";
    let outcome = interpret_dev_container_up_output(false, b"", stderr.as_bytes());
    let DevContainerUpOutcome::Error(message) = outcome else {
        panic!("expected a failed outcome");
    };
    assert!(
        message.contains("Cannot connect to the Docker daemon"),
        "sanitized diagnostic must remain, got {message:?}"
    );
    assert!(
        !message.contains('\u{1b}'),
        "cursor-up/erase must not leak into failure text, got {message:?}"
    );
}

#[test]
fn interpret_dev_container_up_output_extracts_text_from_jsonl_stderr() {
    let banner = serde_json::json!({
        "type": "text",
        "level": 3,
        "timestamp": 1,
        "text": "[cli] @devcontainers/cli 0.89.0",
    });
    let error = serde_json::json!({
        "type": "text",
        "level": 5,
        "timestamp": 2,
        "text": "Cannot connect to the Docker daemon",
    });
    let stderr = format!("{banner}\n{error}\n");
    let outcome = interpret_dev_container_up_output(false, b"", stderr.as_bytes());
    let DevContainerUpOutcome::Error(message) = outcome else {
        panic!("expected a failed outcome");
    };
    assert!(
        message.contains("Cannot connect to the Docker daemon"),
        "JSONL text events must surface in the fallback, got {message:?}"
    );
    assert!(
        !message.contains("@devcontainers/cli"),
        "CLI version banner is not the failure cause, got {message:?}"
    );
    assert!(
        !message.contains("\"type\":"),
        "raw JSONL envelopes must not appear in the fallback, got {message:?}"
    );
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
