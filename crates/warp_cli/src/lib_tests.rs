use std::ffi::OsString;

use clap::Parser;

use super::*;
use crate::agent::{AgentCommand, Harness, OutputFormat};
use crate::artifact::ArtifactCommand;
use crate::environment::{EnvironmentCommand, ImageCommand};
use crate::harness_support::{HarnessSupportCommand, TaskStatus};
use crate::integration::IntegrationCommand;
use crate::schedule::ScheduleSubcommand;
use crate::secret::{CodexMethod, CreateProvider, SecretCommand};
use crate::task::{MessageCommand, TaskCommand};

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
fn agent_run_accepts_model() {
    let args = Args::try_parse_from([
        "warp", "agent", "run", "--prompt", "hello", "--model", "gpt-4o",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.model.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn agent_run_accepts_hidden_bedrock_inference_role_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-inference-role",
        "arn:aws:iam::123456789012:role/test",
        "--bedrock-role-region",
        "us-east-1",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.bedrock_inference_role.as_deref(),
        Some("arn:aws:iam::123456789012:role/test")
    );
    assert_eq!(run_args.bedrock_role_region.as_deref(), Some("us-east-1"));
}

#[test]
fn agent_run_rejects_bedrock_inference_role_without_region() {
    let err = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-inference-role",
        "arn:aws:iam::123456789012:role/test",
    ])
    .expect_err("--bedrock-inference-role must require --bedrock-role-region");
    assert!(
        err.to_string().contains("--bedrock-role-region"),
        "expected error to reference --bedrock-role-region, got: {err}"
    );
}

#[test]
fn agent_run_rejects_bedrock_role_region_without_role() {
    let err = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-role-region",
        "us-east-1",
    ])
    .expect_err("--bedrock-role-region must require --bedrock-inference-role");
    assert!(
        err.to_string().contains("--bedrock-inference-role"),
        "expected error to reference --bedrock-inference-role, got: {err}"
    );
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
fn agent_run_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--file",
        "config.yaml",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.config_file.file.as_ref().and_then(|p| p.to_str()),
        Some("config.yaml")
    );
}

#[test]
fn agent_run_accepts_idle_on_complete_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--idle-on-complete",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.idle_on_complete,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            45 * 60
        )))
    );
}

#[test]
fn agent_run_accepts_idle_on_complete_duration() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--idle-on-complete",
        "10m",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.idle_on_complete,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            10 * 60
        )))
    );
}

#[test]
fn agent_run_accepts_snapshot_flags() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--no-snapshot",
        "--snapshot-upload-timeout",
        "90s",
        "--snapshot-script-timeout",
        "45s",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.snapshot.no_snapshot);
    assert_eq!(
        run_args.snapshot.snapshot_upload_timeout,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            90
        )))
    );
    assert_eq!(
        run_args.snapshot.snapshot_script_timeout,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            45
        )))
    );
}
#[test]
fn agent_run_cloud_accepts_file_short_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "-f",
        "config.json",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(
        run_args.config_file.file.as_ref().and_then(|p| p.to_str()),
        Some("config.json")
    );
}

#[test]
fn agent_run_cloud_accepts_model() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--model",
        "gpt-4o",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.model.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn agent_run_cloud_accepts_agent_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--agent",
        "agent_123",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.agent_uid.as_deref(), Some("agent_123"));
}

#[test]
fn agent_run_cloud_accepts_mcp() {
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--mcp",
        "550e8400-e29b-41d4-a716-446655440000",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert!(matches!(
        run_args.mcp_specs.as_slice(),
        [crate::mcp::MCPSpec::Uuid(parsed_uuid)] if *parsed_uuid == uuid
    ));
}

#[test]
fn agent_run_cloud_accepts_run_ambient_alias() {
    // Ensure backwards compatibility: run-ambient should still work as an alias
    let args = Args::try_parse_from(["warp", "agent", "run-ambient", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-ambient` (alias) command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(_)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-ambient` to parse as RunCloud");
    };
}

#[test]
fn agent_update_rejects_conflicting_remove_flags() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "update",
        "agent_123",
        "--description",
        "new",
        "--remove-description",
    ]);

    assert!(result.is_err());
}

#[test]
fn agent_update_rejects_remove_all_secret_deltas() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "update",
        "agent_123",
        "--add-secret",
        "GITHUB_TOKEN",
        "--remove-all-secrets",
    ]);

    assert!(result.is_err());
}

#[test]
fn agent_run_rejects_prompt_and_task_id() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--task-id",
        "d1b9b002-a8e1-422a-9016-e62490cb6a59",
    ]);
    assert!(result.is_err());
}

#[test]
fn agent_run_rejects_without_prompt_or_task_id() {
    let result = Args::try_parse_from(["warp", "agent", "run", "--model", "gpt-4o"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("prompt_group") || err_str.contains("required"));
}

#[test]
fn agent_run_accepts_prompt_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.prompt.as_deref(), Some("hello"));
    assert!(run_args.prompt_arg.saved_prompt.is_none());
    assert!(run_args.skill.is_empty());
    assert!(run_args.task_id.is_none());
}

#[test]
fn agent_run_accepts_saved_prompt_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--saved-prompt", "sp-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert_eq!(run_args.prompt_arg.saved_prompt.as_deref(), Some("sp-123"));
    assert!(run_args.skill.is_empty());
    assert!(run_args.task_id.is_none());
}

#[test]
fn agent_run_accepts_skill_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--skill", "my-skill"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert_eq!(run_args.skill.len(), 1);
    assert_eq!(
        run_args.invoked_skill().unwrap().skill_identifier,
        "my-skill"
    );
    assert!(run_args.task_id.is_none());
}

#[test]
fn agent_run_accepts_multiple_skills() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--skill",
        "primary",
        "--skill",
        "repo:helper",
        "--prompt",
        "do stuff",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.skill.len(), 2);
    assert_eq!(run_args.skill[0].skill_identifier, "primary");
    assert_eq!(run_args.skill[1].repo.as_deref(), Some("repo"));
    assert_eq!(run_args.skill[1].skill_identifier, "helper");
    assert_eq!(
        run_args.invoked_skill().unwrap().skill_identifier,
        "primary"
    );
}

#[test]
fn agent_run_accepts_task_id_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--task-id", "tid-456"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert!(run_args.skill.is_empty());
    assert_eq!(run_args.task_id.as_deref(), Some("tid-456"));
}

#[test]
fn agent_run_accepts_prompt_and_skill() {
    let args = Args::try_parse_from([
        "warp", "agent", "run", "--prompt", "do stuff", "--skill", "my-skill",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.prompt.as_deref(), Some("do stuff"));
    assert_eq!(run_args.skill.len(), 1);
}

#[test]
fn agent_run_accepts_saved_prompt_and_skill() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--saved-prompt",
        "sp-1",
        "--skill",
        "my-skill",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.saved_prompt.as_deref(), Some("sp-1"));
    assert_eq!(run_args.skill.len(), 1);
}

#[test]
fn agent_run_rejects_saved_prompt_and_task_id() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--saved-prompt",
        "sp-1",
        "--task-id",
        "tid-1",
    ]);
    assert!(result.is_err());
}

#[test]
fn agent_run_rejects_file_and_task_id() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--task-id",
        "tid-1",
        "--file",
        "config.yaml",
    ]);
    assert!(result.is_err());
}

#[test]
fn agent_run_accepts_skill_and_task_id() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--skill",
        "my-skill",
        "--task-id",
        "tid-1",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert_eq!(run_args.skill.len(), 1);
    assert_eq!(run_args.task_id.as_deref(), Some("tid-1"));
}

#[test]
fn agent_run_rejects_prompt_and_saved_prompt() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--saved-prompt",
        "sp-1",
    ]);
    assert!(result.is_err());
}

#[test]
fn schedule_create_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "create",
        "--name",
        "test",
        "--cron",
        "0 9 * * 1",
        "--prompt",
        "hello",
        "--file",
        "schedule.yml",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule create` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule create` command");
    };

    let Some(ScheduleSubcommand::Create(create_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule create` subcommand");
    };

    assert_eq!(
        create_args
            .config_file
            .file
            .as_ref()
            .and_then(|p| p.to_str()),
        Some("schedule.yml")
    );
}

#[test]
fn schedule_resume_alias_parses_as_unpause() {
    let args = Args::try_parse_from(["warp", "schedule", "resume", "schedule-id"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule resume` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule resume` command");
    };

    let Some(ScheduleSubcommand::Unpause(unpause_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule resume` to parse as `unpause`");
    };

    assert_eq!(unpause_args.schedule_id, "schedule-id");
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
fn run_help_hides_message_when_orchestration_v2_disabled() {
    warp_core::features::mark_initialized();

    let mut command = Args::clap_command();
    command.build();

    let run = command
        .find_subcommand("run")
        .expect("run subcommand should exist");
    let message = run
        .find_subcommand("message")
        .expect("message subcommand should exist");

    assert!(message.is_hide_set());

    let visible_subcommands: Vec<_> = run
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| subcommand.get_name())
        .collect();

    assert!(!visible_subcommands.contains(&"message"));
}

#[test]
fn raw_command_keeps_message_visible_before_runtime_help_customization() {
    let mut command = <Args as clap::CommandFactory>::command();
    command.build();

    let run = command
        .find_subcommand("run")
        .expect("run subcommand should exist");
    let message = run
        .find_subcommand("message")
        .expect("message subcommand should exist");

    assert!(!message.is_hide_set());

    let visible_subcommands: Vec<_> = run
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| subcommand.get_name())
        .collect();

    assert!(visible_subcommands.contains(&"message"));
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
fn schedule_create_accepts_mcp_json() {
    let json = r#"{"my-server":{"command":"echo"}}"#;

    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "create",
        "--name",
        "test",
        "--cron",
        "0 9 * * 1",
        "--prompt",
        "hello",
        "--mcp",
        json,
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule create` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule create` command");
    };

    let Some(ScheduleSubcommand::Create(create_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule create` subcommand");
    };

    assert!(matches!(
        create_args.mcp_specs.as_slice(),
        [crate::mcp::MCPSpec::Json(parsed_json)] if parsed_json == json
    ));
}

#[test]
fn schedule_create_accepts_team_scope() {
    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "create",
        "--name",
        "test",
        "--cron",
        "0 9 * * 1",
        "--prompt",
        "hello",
        "--team",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule create` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule create` command");
    };

    let Some(ScheduleSubcommand::Create(create_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule create` subcommand");
    };

    assert!(create_args.scope.team);
    assert!(!create_args.scope.personal);
}

#[test]
fn schedule_create_accepts_personal_scope() {
    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "create",
        "--name",
        "test",
        "--cron",
        "0 9 * * 1",
        "--prompt",
        "hello",
        "--personal",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule create` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule create` command");
    };

    let Some(ScheduleSubcommand::Create(create_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule create` subcommand");
    };

    assert!(!create_args.scope.team);
    assert!(create_args.scope.personal);
}

#[test]
fn schedule_create_rejects_multiple_scopes() {
    assert!(
        Args::try_parse_from([
            "warp",
            "schedule",
            "create",
            "--name",
            "test",
            "--cron",
            "0 9 * * 1",
            "--prompt",
            "hello",
            "--team",
            "--personal",
        ])
        .is_err()
    );
}

#[test]
fn schedule_update_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "update",
        "schedule-id",
        "--file",
        "schedule.json",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule update` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule update` command");
    };

    let Some(ScheduleSubcommand::Update(update_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule update` subcommand");
    };

    assert_eq!(
        update_args
            .config_file
            .file
            .as_ref()
            .and_then(|p| p.to_str()),
        Some("schedule.json")
    );
}

#[test]
fn schedule_update_accepts_mcp_json_and_remove_mcp() {
    let json = r#"{"my-server":{"command":"echo"}}"#;

    let args = Args::try_parse_from([
        "warp",
        "schedule",
        "update",
        "schedule-id",
        "--mcp",
        json,
        "--remove-mcp",
        "existing",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp schedule update` command");
    };
    let CliCommand::Schedule(schedule_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp schedule update` command");
    };

    let Some(ScheduleSubcommand::Update(update_args)) = schedule_cmd.subcommand() else {
        panic!("Expected `warp schedule update` subcommand");
    };

    assert!(matches!(
        update_args.mcp_specs.as_slice(),
        [crate::mcp::MCPSpec::Json(parsed_json)] if parsed_json == json
    ));
    assert_eq!(update_args.remove_mcp, vec!["existing".to_string()]);
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
fn agent_run_accepts_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(true));
}

#[test]
fn agent_run_accepts_no_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--no-computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(false));
}

#[test]
fn agent_run_rejects_both_computer_use_flags() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--computer-use",
        "--no-computer-use",
    ]);

    assert!(result.is_err());
}

#[test]
fn agent_run_defaults_to_no_computer_use_override() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), None);
}
#[test]
fn agent_run_cloud_accepts_snapshot_flags() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--no-snapshot",
        "--snapshot-upload-timeout",
        "2m",
        "--snapshot-script-timeout",
        "1m",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert!(run_args.snapshot.no_snapshot);
    assert_eq!(
        run_args.snapshot.snapshot_upload_timeout,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            120
        )))
    );
    assert_eq!(
        run_args.snapshot.snapshot_script_timeout,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            60
        )))
    );
}

#[test]
fn agent_run_accepts_task_id_with_conversation_for_worker_followups() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--task-id",
        "task-123",
        "--conversation",
        "conv-123",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.task_id.as_deref(), Some("task-123"));
    assert_eq!(run_args.conversation.as_deref(), Some("conv-123"));
}

#[test]
fn agent_run_cloud_accepts_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert!(run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(true));
}

#[test]
fn agent_run_cloud_accepts_no_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--no-computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(false));
}

#[test]
fn agent_run_cloud_rejects_both_computer_use_flags() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--computer-use",
        "--no-computer-use",
    ]);

    assert!(result.is_err());
}

#[test]
fn agent_run_cloud_defaults_to_no_computer_use_override() {
    let args = Args::try_parse_from(["warp", "agent", "run-cloud", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), None);
}

#[test]
fn agent_run_cloud_accepts_harness_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--harness",
        "claude",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.harness, Harness::Claude);
}

#[test]
fn agent_run_cloud_defaults_harness_to_oz() {
    let args = Args::try_parse_from(["warp", "agent", "run-cloud", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.harness, Harness::Oz);
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
fn agent_run_cloud_accepts_claude_auth_secret_with_harness() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--harness",
        "claude",
        "--claude-auth-secret",
        "my-key",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.harness, Harness::Claude);
    assert_eq!(run_args.claude_auth_secret.as_deref(), Some("my-key"));
}

#[test]
fn agent_run_cloud_claude_auth_secret_without_harness_parses() {
    // Clap parsing succeeds; runtime validation (in mod.rs) rejects this combination.
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run-cloud",
        "--prompt",
        "hello",
        "--claude-auth-secret",
        "my-key",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run-cloud` command");
    };
    let CliCommand::Agent(AgentCommand::RunCloud(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run-cloud` command");
    };

    assert_eq!(run_args.harness, Harness::Oz);
    assert_eq!(run_args.claude_auth_secret.as_deref(), Some("my-key"));
}

#[test]
fn run_message_send_parses() {
    let args = Args::try_parse_from([
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
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message send` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::Send(send_args))) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message send` command");
    };

    assert_eq!(send_args.to, vec!["run-1".to_string(), "run-2".to_string()]);
    assert_eq!(send_args.subject, "Build update");
    assert_eq!(send_args.body, "Done");
    assert_eq!(send_args.sender_run_id, "sender-1");
}

#[test]
fn run_message_list_parses_filters() {
    let args = Args::try_parse_from([
        "warp",
        "run",
        "message",
        "list",
        "run-123",
        "--unread",
        "--since",
        "2026-04-09T20:00:00Z",
        "--limit",
        "25",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message list` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::List(list_args))) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message list` command");
    };

    assert_eq!(list_args.run_id, "run-123");
    assert!(list_args.unread);
    assert_eq!(list_args.since.as_deref(), Some("2026-04-09T20:00:00Z"));
    assert_eq!(list_args.limit, 25);
}

#[test]
fn run_message_list_rejects_non_positive_limit() {
    assert!(
        Args::try_parse_from(["warp", "run", "message", "list", "run-123", "--limit", "0",])
            .is_err()
    );
}

#[test]
fn run_message_watch_parses() {
    let args = Args::try_parse_from([
        "warp",
        "run",
        "message",
        "watch",
        "--output-format",
        "ndjson",
        "run-123",
        "--since-sequence",
        "7",
    ])
    .unwrap();

    assert_eq!(args.global_options.output_format, OutputFormat::Ndjson);

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message watch` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::Watch(watch_args))) =
        boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message watch` command");
    };

    assert_eq!(watch_args.run_id, "run-123");
    assert_eq!(watch_args.since_sequence, 7);
}

#[test]
fn run_message_read_parses() {
    let args = Args::try_parse_from(["warp", "run", "message", "read", "message-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message read` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::Read(read_args))) = boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message read` command");
    };

    assert_eq!(read_args.message_id, "message-123");
}

#[test]
fn run_message_mark_delivered_parses() {
    let args =
        Args::try_parse_from(["warp", "run", "message", "mark-delivered", "message-456"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message mark-delivered` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::MarkDelivered(delivered_args))) =
        boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message mark-delivered` command");
    };

    assert_eq!(delivered_args.message_id, "message-456");
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
fn run_message_delivered_alias_parses() {
    let args =
        Args::try_parse_from(["warp", "run", "message", "delivered", "message-456"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp run message delivered` command");
    };
    let CliCommand::Run(TaskCommand::Message(MessageCommand::MarkDelivered(delivered_args))) =
        boxed_cmd.as_ref()
    else {
        panic!("Expected `warp run message delivered` command");
    };

    assert_eq!(delivered_args.message_id, "message-456");
}

#[test]
fn finish_task_accepts_status_success() {
    let args = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "finish-task",
        "--status",
        "success",
        "--summary",
        "all good",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected harness-support command");
    };
    let CliCommand::HarnessSupport(hs_args) = boxed_cmd.as_ref() else {
        panic!("Expected harness-support command");
    };
    let HarnessSupportCommand::FinishTask(finish_args) = &hs_args.command else {
        panic!("Expected finish-task subcommand");
    };

    assert_eq!(finish_args.status, TaskStatus::Success);
    assert_eq!(finish_args.summary, "all good");
}

#[test]
fn finish_task_accepts_status_failure() {
    let args = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "finish-task",
        "--status",
        "failure",
        "--summary",
        "something broke",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected harness-support command");
    };
    let CliCommand::HarnessSupport(hs_args) = boxed_cmd.as_ref() else {
        panic!("Expected harness-support command");
    };
    let HarnessSupportCommand::FinishTask(finish_args) = &hs_args.command else {
        panic!("Expected finish-task subcommand");
    };

    assert_eq!(finish_args.status, TaskStatus::Failure);
    assert_eq!(finish_args.summary, "something broke");
}

#[test]
fn finish_task_rejects_invalid_status() {
    let result = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "finish-task",
        "--status",
        "maybe",
        "--summary",
        "who knows",
    ]);
    assert!(result.is_err());
}

#[test]
fn finish_task_rejects_missing_status() {
    let result = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "finish-task",
        "--summary",
        "no status",
    ]);
    assert!(result.is_err());
}

#[test]
fn report_shutdown_clean_parses() {
    let args = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "report-shutdown",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected harness-support command");
    };
    let CliCommand::HarnessSupport(hs_args) = boxed_cmd.as_ref() else {
        panic!("Expected harness-support command");
    };
    let HarnessSupportCommand::ReportShutdown(shutdown_args) = &hs_args.command else {
        panic!("Expected report-shutdown subcommand");
    };

    assert!(shutdown_args.error_category.is_none());
    assert!(shutdown_args.error_message.is_none());
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

#[test]
fn report_shutdown_abnormal_parses() {
    let args = Args::try_parse_from([
        "warp",
        "harness-support",
        "--run-id",
        "run-1",
        "report-shutdown",
        "--error-category",
        "oom",
        "--error-message",
        "out of memory",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected harness-support command");
    };
    let CliCommand::HarnessSupport(hs_args) = boxed_cmd.as_ref() else {
        panic!("Expected harness-support command");
    };
    let HarnessSupportCommand::ReportShutdown(shutdown_args) = &hs_args.command else {
        panic!("Expected report-shutdown subcommand");
    };

    assert_eq!(shutdown_args.error_category.as_deref(), Some("oom"));
    assert_eq!(
        shutdown_args.error_message.as_deref(),
        Some("out of memory")
    );
}
