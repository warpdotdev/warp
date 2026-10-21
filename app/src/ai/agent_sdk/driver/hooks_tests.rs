use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ai::hooks::{
    DiagnosticSeverity, ExplicitHookSource, ExplicitSourceScope, HookDiagnostic, HookFormat,
    HookNotice, HookNoticeKind, HookSourceId, HookSourceKind, HookSourceLocation,
};
use ai_types::{AIAgentActionId, AIConversationId};
use serde_json::Value;
use warp_cli::agent::{HooksOnAsk, OutputFormat};

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
            kind: HookSourceKind::RunDelivered,
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

#[test]
fn on_ask_maps_to_the_headless_ask_behavior() {
    assert_eq!(
        headless_ask_behavior(HooksOnAsk::Deny),
        HeadlessAskBehavior::DenyWithReason
    );
    assert_eq!(
        headless_ask_behavior(HooksOnAsk::FailRun),
        HeadlessAskBehavior::FailRun
    );
}

#[test]
fn hook_dir_flags_are_added_after_the_environment_entries() {
    assert_eq!(
        hook_dirs(Some(" /a , ,b ,"), &[PathBuf::from("c")]),
        vec![PathBuf::from("/a"), PathBuf::from("b"), PathBuf::from("c")]
    );
    assert_eq!(hook_dirs(None, &[]), Vec::<PathBuf>::new());
}

#[test]
fn hook_directories_resolve_against_the_working_directory_and_config_sources_follow() {
    let configured = ExplicitHookSource {
        path: PathBuf::from("/config/hooks/hooks.json"),
        format: HookFormat::Cursor,
        scope: ExplicitSourceScope::FactoryAgent,
    };
    let sources = explicit_sources(
        Path::new("/work"),
        &[PathBuf::from("/abs"), PathBuf::from("rel")],
        std::slice::from_ref(&configured),
    );
    assert_eq!(
        sources,
        vec![
            ExplicitHookSource {
                path: PathBuf::from("/abs/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            },
            ExplicitHookSource {
                path: PathBuf::from("/work/rel/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            },
            configured,
        ]
    );
}

#[test]
fn the_sessions_hook_dirs_value_comes_before_the_process_environment() {
    let session = HashMap::from([(
        OsString::from("WARP_HOOK_DIRS"),
        OsString::from("/from/session"),
    )]);
    assert_eq!(
        hook_dirs_env_value(&session).as_deref(),
        Some("/from/session")
    );
}
