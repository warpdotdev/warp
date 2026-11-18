use std::path::{Path, PathBuf};

use ai::hooks::{
    DiagnosticSeverity, HookDiagnostic, HookFormat, HookNotice, HookNoticeKind, HookSourceId,
    HookSourceKind, HookSourceLocation, HookSourcePolicy,
};
use ai_types::{AIAgentActionId, AIConversationId};
use cloud_object_models::CodeForge;
use serde_json::Value;
use warp_cli::agent::OutputFormat;

use super::*;

fn denial(conversation_id: AIConversationId) -> HookNotice {
    HookNotice {
        conversation_id,
        action_id: Some(AIAgentActionId::from("action-1".to_owned())),
        kind: HookNoticeKind::Denied,
        text: "Blocked by policy\nsecond line".to_owned(),
    }
}

fn malformed_source() -> HookDiagnostic {
    HookDiagnostic {
        source: HookSourceId {
            kind: HookSourceKind::FactoryWide,
            location: HookSourceLocation::CanonicalPath(PathBuf::from("/hooks/hooks.json")),
        },
        severity: DiagnosticSeverity::Error,
        message: "expected value at line 1 column 1".to_owned(),
    }
}

fn written(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
    let mut buf = Vec::new();
    write(&mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn notices_and_diagnostics_are_one_typed_json_record_each() {
    let conversation_id = AIConversationId::new();
    for format in [OutputFormat::Json, OutputFormat::Ndjson] {
        let notice = written(|buf| {
            write_notice(
                &denial(conversation_id),
                Some("server-conversation"),
                format,
                buf,
            )
        });
        assert_eq!(notice.lines().count(), 1, "{notice:?}");
        let record: Value = serde_json::from_str(&notice).unwrap();
        assert_eq!(record["type"], "hook_notice");
        assert_eq!(record["kind"], "denied");
        assert_eq!(
            record["conversation_id"], "server-conversation",
            "the same id conversation_started reports"
        );
        assert_eq!(record["local_conversation_id"], conversation_id.to_string());
        assert_eq!(record["action_id"], "action-1");
        assert_eq!(record["text"], "Blocked by policy\nsecond line");

        let diagnostic = written(|buf| write_diagnostic(&malformed_source(), format, buf));
        assert_eq!(diagnostic.lines().count(), 1, "{diagnostic:?}");
        let record: Value = serde_json::from_str(&diagnostic).unwrap();
        assert_eq!(record["type"], "hook_diagnostic");
        assert_eq!(record["source"], "/hooks/hooks.json");
        assert_eq!(record["severity"], "error");
        assert_eq!(record["message"], "expected value at line 1 column 1");
    }
}

#[test]
fn a_notice_before_the_server_token_or_without_an_action_has_null_fields() {
    let mut notice = denial(AIConversationId::new());
    notice.action_id = None;
    notice.kind = HookNoticeKind::Warning;
    let record: Value = serde_json::from_str(&written(|buf| {
        write_notice(&notice, None, OutputFormat::Json, buf)
    }))
    .unwrap();
    assert_eq!(record["kind"], "warning");
    assert!(record["action_id"].is_null());
    assert!(record["conversation_id"].is_null());
    assert_eq!(
        record["local_conversation_id"],
        notice.conversation_id.to_string()
    );
}

#[test]
fn notices_and_diagnostics_are_one_hook_line_each_in_text_output() {
    for format in [OutputFormat::Text, OutputFormat::Pretty] {
        let notice = written(|buf| {
            write_notice(&denial(AIConversationId::new()), Some("token"), format, buf)
        });
        assert_eq!(notice, "hook: denied: Blocked by policy second line\n");

        let diagnostic = written(|buf| write_diagnostic(&malformed_source(), format, buf));
        assert_eq!(
            diagnostic,
            "hook: error: /hooks/hooks.json: expected value at line 1 column 1\n"
        );
    }
}

#[test]
fn only_a_dispatched_task_is_an_oz_run() {
    assert_eq!(run_surface_for(true), RunSurface::Oz);
    assert_eq!(run_surface_for(false), RunSurface::CommandLine);
}

fn source(location: ConfiguredHookLocation, strict: bool) -> ConfiguredHookSource {
    ConfiguredHookSource {
        format: HookFormat::Cursor,
        location,
        kind: HookSourceKind::FactoryAgent,
        policy: HookSourcePolicy {
            strict,
            ..Default::default()
        },
    }
}

fn repo(owner: &str, name: &str) -> SourceRepo {
    SourceRepo::new(CodeForge::GitHub, owner.to_owned(), name.to_owned())
}

#[test]
fn a_repository_source_is_found_in_its_clone_by_forge_owner_and_name() {
    let (sources, failures) = resolve_sources(
        &[source(
            ConfiguredHookLocation::Repository {
                repo: repo("Acme", "API"),
                path: ".cursor".to_owned(),
            },
            false,
        )],
        Path::new("/work"),
        &[repo("acme", "web"), repo("acme", "api")],
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].config_file,
        PathBuf::from("/work/api/.cursor/hooks.json")
    );
    assert_eq!(sources[0].working_directory, PathBuf::from("/work/api"));
    assert_eq!(sources[0].project_dir, Some(PathBuf::from("/work/api")));
}

#[test]
fn the_repository_root_itself_is_a_hook_directory() {
    let (sources, failures) = resolve_sources(
        &[source(
            ConfiguredHookLocation::Repository {
                repo: repo("acme", "api"),
                path: ".".to_owned(),
            },
            false,
        )],
        Path::new("/work"),
        &[repo("acme", "api")],
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(sources[0].working_directory, PathBuf::from("/work/api"));
    assert!(sources[0].config_file.ends_with("hooks.json"));
}

#[test]
fn a_repository_on_another_forge_is_not_the_same_repository() {
    let (sources, failures) = resolve_sources(
        &[source(
            ConfiguredHookLocation::Repository {
                repo: repo("acme", "api"),
                path: ".cursor".to_owned(),
            },
            true,
        )],
        Path::new("/work"),
        &[SourceRepo::new(
            CodeForge::GitLab,
            "acme".to_owned(),
            "api".to_owned(),
        )],
    );
    assert!(sources.is_empty());
    assert_eq!(failures.len(), 1);
    assert!(failures[0].strict);
    assert!(
        failures[0].diagnostic.message.contains("not checked out"),
        "{:?}",
        failures[0]
    );
}

#[test]
fn a_worker_source_must_be_absolute() {
    let absolute_path = std::env::current_dir()
        .unwrap()
        .join("acme")
        .join("hooks")
        .to_string_lossy()
        .into_owned();
    let (sources, failures) = resolve_sources(
        &[
            source(
                ConfiguredHookLocation::Worker {
                    path: absolute_path.clone(),
                },
                false,
            ),
            source(
                ConfiguredHookLocation::Worker {
                    path: "etc/acme/hooks".to_owned(),
                },
                false,
            ),
        ],
        Path::new("/work"),
        &[],
    );
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].working_directory, PathBuf::from(absolute_path));
    assert_eq!(sources[0].project_dir, None);
    assert_eq!(failures.len(), 1);
    assert!(!failures[0].strict);
}
