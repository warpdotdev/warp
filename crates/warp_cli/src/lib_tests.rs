use std::ffi::OsString;

use clap::Parser;

use super::*;
use crate::agent::Harness;
use crate::artifact::ArtifactCommand;
use crate::environment::{EnvironmentCommand, ImageCommand};
use crate::integration::IntegrationCommand;
use crate::secret::{CodexMethod, CreateProvider, SecretCommand};

fn set_env_var(name: &str, value: &str) -> Option<OsString> {
    let previous = std::env::var_os(name);
    // Safety: tests that mutate process environment are marked `serial` so we
    // do not race with other environment readers/writers in this crate.
    unsafe { std::env::set_var(name, value) };
    previous
}

fn restore_env_var(name: &str, previous: Option<OsString>) {
    match previous {
        // Safety: tests that mutate process environment are marked `serial` so
        // we do not race with other environment readers/writers in this crate.
        Some(value) => unsafe { std::env::set_var(name, value) },
        // Safety: tests that mutate process environment are marked `serial` so
        // we do not race with other environment readers/writers in this crate.
        None => unsafe { std::env::remove_var(name) },
    }
}

#[test]
fn model_list_parses() {
    let args = Args::try_parse_from(["warp", "model", "list"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp model list` command");
    };
    let CliCommand::Model(model_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp model` command");
    };

    assert!(matches!(model_cmd, crate::model::ModelCommand::List));
}

#[test]
fn login_parses() {
    let args = Args::try_parse_from(["warp", "login"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp login` command");
    };

    assert!(matches!(boxed_cmd.as_ref(), CliCommand::Login));
}

#[test]
fn logout_parses() {
    let args = Args::try_parse_from(["warp", "logout"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp logout` command");
    };

    assert!(matches!(boxed_cmd.as_ref(), CliCommand::Logout));
}

#[test]
fn cli_metadata_uses_zerp_cli_branding() {
    warp_core::features::mark_initialized();

    let command = Args::clap_command();

    assert_eq!(command.get_name(), "zerp-cli");
    assert_eq!(command.get_display_name(), Some("Zerp CLI"));
    assert!(
        command
            .get_about()
            .is_some_and(|about| about.to_string().contains("Zerp command-line tools"))
    );
    assert!(
        command
            .get_about()
            .is_none_or(|about| !about.to_string().contains("Oz"))
    );
}

#[test]
fn zerp_cli_binary_names_enter_cli_mode_without_matching_app_binaries() {
    assert!(is_cli_binary_name("zerp-cli"));
    assert!(is_cli_binary_name("zerp-cli-dev"));
    assert!(is_cli_binary_name("zerp-cli-preview"));
    assert!(is_cli_binary_name("zerp-cli-local"));
    assert!(is_cli_binary_name("zerp-cli-integration"));
    assert!(is_cli_binary_name("zerp-cli-oss"));

    assert!(!is_cli_binary_name("zerp"));
    assert!(!is_cli_binary_name("zerp-oss"));
    assert!(!is_cli_binary_name("oz"));
}

#[test]
fn artifact_upload_accepts_run_id() {
    let args = Args::try_parse_from([
        "warp",
        "artifact",
        "upload",
        "path/to/file.json",
        "--run-id",
        "run-123",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact upload` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Upload(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact upload` command");
    };

    assert_eq!(args.path.to_str(), Some("path/to/file.json"));
    assert_eq!(args.run_id.as_deref(), Some("run-123"));
    assert_eq!(args.conversation_id, None);
}

#[test]
fn artifact_help_hides_upload_but_keeps_download_visible() {
    warp_core::features::mark_initialized();

    let mut command = Args::clap_command();
    command.build();

    let artifact = command
        .find_subcommand("artifact")
        .expect("artifact subcommand should exist");
    let upload = artifact
        .find_subcommand("upload")
        .expect("upload subcommand should exist");
    let download = artifact
        .find_subcommand("download")
        .expect("download subcommand should exist");
    let get = artifact
        .find_subcommand("get")
        .expect("get subcommand should exist");

    assert!(upload.is_hide_set());
    assert!(!get.is_hide_set());
    assert!(!download.is_hide_set());

    let visible_subcommands: Vec<_> = artifact
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| subcommand.get_name())
        .collect();
    assert!(visible_subcommands.contains(&"get"));

    assert!(visible_subcommands.contains(&"download"));
    assert!(!visible_subcommands.contains(&"upload"));
}

#[test]
fn artifact_upload_accepts_run_id_and_description() {
    let args = Args::try_parse_from([
        "warp",
        "artifact",
        "upload",
        "path/to/file.json",
        "--run-id",
        "run-123",
        "--description",
        "Test artifact",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact upload` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Upload(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact upload` command");
    };

    assert_eq!(args.run_id.as_deref(), Some("run-123"));
    assert_eq!(args.conversation_id, None);
    assert_eq!(args.description.as_deref(), Some("Test artifact"));
}

#[test]
fn artifact_upload_accepts_conversation_id_and_description() {
    let args = Args::try_parse_from([
        "warp",
        "artifact",
        "upload",
        "path/to/file.json",
        "--conversation-id",
        "conversation-123",
        "--description",
        "Test artifact",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact upload` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Upload(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact upload` command");
    };

    assert_eq!(args.path.to_str(), Some("path/to/file.json"));
    assert_eq!(args.run_id, None);
    assert_eq!(args.conversation_id.as_deref(), Some("conversation-123"));
    assert_eq!(args.description.as_deref(), Some("Test artifact"));
}

#[test]
fn artifact_upload_accepts_missing_association_target_for_env_fallback() {
    let args = Args::try_parse_from(["warp", "artifact", "upload", "path/to/file.json"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact upload` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Upload(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact upload` command");
    };

    assert_eq!(args.path.to_str(), Some("path/to/file.json"));
    assert_eq!(args.run_id, None);
    assert_eq!(args.conversation_id, None);
}

#[test]
fn artifact_upload_rejects_both_association_targets() {
    let err = Args::try_parse_from([
        "warp",
        "artifact",
        "upload",
        "path/to/file.json",
        "--run-id",
        "run-123",
        "--conversation-id",
        "conversation-123",
    ])
    .unwrap_err();
    let err = err.to_string();

    assert!(err.contains("--run-id"));
    assert!(err.contains("--conversation-id"));
}

#[test]
fn artifact_download_parses_artifact_id_and_out() {
    let args = Args::try_parse_from([
        "warp",
        "artifact",
        "download",
        "artifact-123",
        "--out",
        "downloads/file.json",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact download` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Download(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact download` command");
    };

    assert_eq!(args.artifact_uid, "artifact-123");
    assert_eq!(
        args.out.as_ref().and_then(|path| path.to_str()),
        Some("downloads/file.json")
    );
}
#[test]
fn artifact_get_parses_artifact_uid() {
    let args = Args::try_parse_from(["warp", "artifact", "get", "artifact-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp artifact get` command");
    };
    let CliCommand::Artifact(ArtifactCommand::Get(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp artifact get` command");
    };

    assert_eq!(args.artifact_uid, "artifact-123");
}

#[test]
fn integration_create_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "integration",
        "create",
        "slack",
        "--file",
        "integration.yml",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration create` command");
    };
    let CliCommand::Integration(IntegrationCommand::Create(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration create` command");
    };

    assert_eq!(
        args.config_file.file.as_ref().and_then(|p| p.to_str()),
        Some("integration.yml")
    );
}

#[test]
fn integration_create_accepts_model() {
    let args = Args::try_parse_from([
        "warp",
        "integration",
        "create",
        "slack",
        "--model",
        "gpt-4o",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration create` command");
    };
    let CliCommand::Integration(IntegrationCommand::Create(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration create` command");
    };

    assert_eq!(args.model.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn integration_update_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "integration",
        "update",
        "slack",
        "--file",
        "integration.json",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration update` command");
    };
    let CliCommand::Integration(IntegrationCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration update` command");
    };

    assert_eq!(
        args.config_file.file.as_ref().and_then(|p| p.to_str()),
        Some("integration.json")
    );
}

#[test]
fn integration_update_accepts_model() {
    let args = Args::try_parse_from([
        "warp",
        "integration",
        "update",
        "slack",
        "--model",
        "gpt-4o",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration update` command");
    };
    let CliCommand::Integration(IntegrationCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration update` command");
    };

    assert_eq!(args.model.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn integration_create_accepts_mcp_json() {
    let json = r#"{"my-server":{"command":"echo"}}"#;

    let args =
        Args::try_parse_from(["warp", "integration", "create", "slack", "--mcp", json]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration create` command");
    };
    let CliCommand::Integration(IntegrationCommand::Create(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration create` command");
    };

    assert!(matches!(
        args.mcp_specs.as_slice(),
        [crate::mcp::MCPSpec::Json(parsed_json)] if parsed_json == json
    ));
}

#[test]
fn integration_update_accepts_mcp_json_and_remove_mcp() {
    let json = r#"{"my-server":{"command":"echo"}}"#;

    let args = Args::try_parse_from([
        "warp",
        "integration",
        "update",
        "slack",
        "--mcp",
        json,
        "--remove-mcp",
        "existing",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp integration update` command");
    };
    let CliCommand::Integration(IntegrationCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp integration update` command");
    };

    assert!(matches!(
        args.mcp_specs.as_slice(),
        [crate::mcp::MCPSpec::Json(parsed_json)] if parsed_json == json
    ));
    assert_eq!(args.remove_mcp, vec!["existing".to_string()]);
}

#[test]
fn schedule_command_is_removed() {
    let result = Args::try_parse_from(["warp", "schedule", "create", "--name", "test"]);
    assert!(result.is_err());
}

#[test]
fn environment_image_list_parses() {
    let args = Args::try_parse_from(["warp", "environment", "image", "list"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp environment image list` command");
    };
    let CliCommand::Environment(EnvironmentCommand::Image(image_cmd)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp environment image` command");
    };

    assert!(matches!(image_cmd, ImageCommand::List));
}

#[test]
fn environment_create_accepts_description() {
    let args = Args::try_parse_from([
        "warp",
        "environment",
        "create",
        "--name",
        "test-env",
        "--description",
        "A test environment",
        "--docker-image",
        "ubuntu:latest",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp environment create` command");
    };
    let CliCommand::Environment(EnvironmentCommand::Create {
        name,
        description,
        docker_image,
        ..
    }) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp environment create` command");
    };

    assert_eq!(name, "test-env");
    assert_eq!(description.as_deref(), Some("A test environment"));
    assert_eq!(docker_image.as_deref(), Some("ubuntu:latest"));
}

#[test]
fn environment_create_description_max_length() {
    // 240 characters should be accepted
    let valid_description = "a".repeat(240);
    let args = Args::try_parse_from([
        "warp",
        "environment",
        "create",
        "--name",
        "test-env",
        "--description",
        &valid_description,
        "--docker-image",
        "ubuntu:latest",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp environment create` command");
    };
    let CliCommand::Environment(EnvironmentCommand::Create { description, .. }) =
        boxed_cmd.as_ref()
    else {
        panic!("Expected `warp environment create` command");
    };

    assert_eq!(description.as_deref(), Some(valid_description.as_str()));

    // 241 characters should be rejected
    let invalid_description = "a".repeat(241);
    assert!(
        Args::try_parse_from([
            "warp",
            "environment",
            "create",
            "--name",
            "test-env",
            "--description",
            &invalid_description,
            "--docker-image",
            "ubuntu:latest",
        ])
        .is_err()
    );
}

#[test]
fn environment_update_accepts_description() {
    let args = Args::try_parse_from([
        "warp",
        "environment",
        "update",
        "env-id",
        "--description",
        "Updated description",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp environment update` command");
    };
    let CliCommand::Environment(EnvironmentCommand::Update {
        id,
        description,
        remove_description,
        ..
    }) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp environment update` command");
    };

    assert_eq!(id, "env-id");
    assert_eq!(description.as_deref(), Some("Updated description"));
    assert!(!remove_description);
}

#[test]
fn environment_update_accepts_remove_description() {
    let args = Args::try_parse_from([
        "warp",
        "environment",
        "update",
        "env-id",
        "--remove-description",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp environment update` command");
    };
    let CliCommand::Environment(EnvironmentCommand::Update {
        id,
        description,
        remove_description,
        ..
    }) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp environment update` command");
    };

    assert_eq!(id, "env-id");
    assert!(description.is_none());
    assert!(remove_description);
}

#[test]
fn agent_run_command_is_removed() {
    let result = Args::try_parse_from(["warp", "agent", "run", "--prompt", "hello"]);
    assert!(result.is_err());
}

#[test]
fn agent_run_cloud_command_is_removed() {
    let result = Args::try_parse_from(["warp", "agent", "run-cloud", "--prompt", "hello"]);
    assert!(result.is_err());
}

#[test]
fn agent_run_ambient_alias_is_removed() {
    let result = Args::try_parse_from(["warp", "agent", "run-ambient", "--prompt", "hello"]);
    assert!(result.is_err());
}

#[test]
fn harness_parse_orchestration_harness_accepts_aliases() {
    assert_eq!(
        Harness::parse_orchestration_harness("claude-code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        Harness::parse_orchestration_harness("open_code"),
        Some(Harness::OpenCode)
    );
}

#[test]
fn harness_parse_orchestration_harness_rejects_warp_native_harness() {
    assert_eq!(Harness::parse_orchestration_harness("oz"), None);
}

#[test]
fn harness_parse_local_child_harness_rejects_oz() {
    assert_eq!(Harness::parse_local_child_harness("oz"), None);
    assert_eq!(
        Harness::parse_local_child_harness("opencode"),
        Some(Harness::OpenCode)
    );
}

#[test]
fn harness_parse_orchestration_harness_accepts_codex() {
    assert_eq!(
        Harness::parse_orchestration_harness("codex"),
        Some(Harness::Codex)
    );
}

#[test]
fn harness_parse_local_child_harness_accepts_codex() {
    assert_eq!(
        Harness::parse_local_child_harness("codex"),
        Some(Harness::Codex)
    );
}

#[test]
fn run_command_is_removed() {
    let result = Args::try_parse_from(["warp", "run", "list"]);
    assert!(result.is_err());
}

#[test]
fn run_message_command_is_removed() {
    let result = Args::try_parse_from([
        "warp",
        "run",
        "message",
        "send",
        "--to",
        "run-1",
        "--to",
        "run-2",
        "--subject",
        "Build update",
        "--body",
        "Done",
        "--sender-run-id",
        "sender-1",
    ]);
    assert!(result.is_err());
}

#[test]
#[serial_test::serial]
fn hidden_server_overrides_parse_from_env() {
    let previous_server_root = set_env_var(SERVER_ROOT_URL_OVERRIDE_ENV, "http://localhost:8080");
    let previous_ws = set_env_var(WS_SERVER_URL_OVERRIDE_ENV, "ws://localhost:8082/graphql/v2");
    let previous_session_sharing = set_env_var(
        SESSION_SHARING_SERVER_URL_OVERRIDE_ENV,
        "ws://127.0.0.1:8081",
    );

    let args = Args::try_parse_from(["warp", "whoami"]).unwrap();

    restore_env_var(SERVER_ROOT_URL_OVERRIDE_ENV, previous_server_root);
    restore_env_var(WS_SERVER_URL_OVERRIDE_ENV, previous_ws);
    restore_env_var(
        SESSION_SHARING_SERVER_URL_OVERRIDE_ENV,
        previous_session_sharing,
    );

    assert_eq!(args.server_root_url(), Some("http://localhost:8080"));
    assert_eq!(args.ws_server_url(), Some("ws://localhost:8082/graphql/v2"));
    assert_eq!(
        args.session_sharing_server_url(),
        Some("ws://127.0.0.1:8081")
    );
}

#[test]
fn harness_support_command_is_removed() {
    let result = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "finish-task",
        "--status",
        "success",
        "--summary",
        "all good",
    ]);
    assert!(result.is_err());
}

#[test]
fn secret_create_codex_api_key_parses_minimal() {
    warp_core::features::mark_initialized();

    let args = Args::try_parse_from([
        "warp",
        "secret",
        "create",
        "codex",
        "api-key",
        "my-openai-key",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp secret create codex api-key` command");
    };
    let CliCommand::Secret(SecretCommand::Create(create_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp secret create` command");
    };
    let Some(CreateProvider::Codex(codex)) = &create_args.provider else {
        panic!("Expected `codex` provider subcommand");
    };
    let CodexMethod::ApiKey(api_key_args) = &codex.method;

    assert_eq!(api_key_args.common.name, "my-openai-key");
    assert!(api_key_args.common.description.is_none());
    assert!(api_key_args.value.value_file.is_none());
    assert!(api_key_args.base_url.is_none());
}

#[test]
fn secret_create_codex_api_key_accepts_base_url_and_value_file() {
    warp_core::features::mark_initialized();

    let args = Args::try_parse_from([
        "warp",
        "secret",
        "create",
        "codex",
        "api-key",
        "my-openai-key",
        "--value-file",
        "key.txt",
        "--base-url",
        "https://us.api.openai.com/v1",
        "--description",
        "OpenAI key for Codex",
        "--team",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp secret create codex api-key` command");
    };
    let CliCommand::Secret(SecretCommand::Create(create_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp secret create` command");
    };
    let Some(CreateProvider::Codex(codex)) = &create_args.provider else {
        panic!("Expected `codex` provider subcommand");
    };
    let CodexMethod::ApiKey(api_key_args) = &codex.method;

    assert_eq!(api_key_args.common.name, "my-openai-key");
    assert_eq!(
        api_key_args.common.description.as_deref(),
        Some("OpenAI key for Codex")
    );
    assert!(api_key_args.common.scope.team);
    assert!(!api_key_args.common.scope.personal);
    assert_eq!(
        api_key_args
            .value
            .value_file
            .as_ref()
            .and_then(|p| p.to_str()),
        Some("key.txt")
    );
    assert_eq!(
        api_key_args.base_url.as_deref(),
        Some("https://us.api.openai.com/v1")
    );
}

#[test]
fn secret_create_codex_api_key_requires_name() {
    warp_core::features::mark_initialized();

    let result = Args::try_parse_from(["warp", "secret", "create", "codex", "api-key"]);
    assert!(result.is_err());
}
