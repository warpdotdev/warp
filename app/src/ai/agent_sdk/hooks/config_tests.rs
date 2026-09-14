use std::fs;

use serde_json::json;

use super::*;
use crate::ai::agent_sdk::hooks::trust::{DenyProjectHookTrust, ExactHookTrustStore};

fn write_config(directory: &Path, value: serde_json::Value) -> PathBuf {
    let path = directory.join("hooks.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

fn valid_config() -> serde_json::Value {
    json!({
        "schema_version": CONFIG_SCHEMA_VERSION,
        "hooks": {
            "PreToolUse": [{
                "matcher": "^(run_shell_command|apply_patch)$",
                "hooks": [{
                    "type": "command",
                    "command": "check",
                    "timeout": 10,
                    "on_failure": "deny"
                }]
            }]
        }
    })
}

#[test]
fn oz_hooks_config_parses_strict_valid_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_config(temp.path(), valid_config());

    let snapshot = load_hook_config(Some(&path), None, &DenyProjectHookTrust);

    assert_eq!(snapshot.handlers().len(), 1);
    assert!(snapshot.diagnostics.is_empty());
    assert!(snapshot.handlers()[0].matches(Some("apply_patch")));
    assert!(!snapshot.handlers()[0].matches(Some("read_files")));
}

#[test]
fn oz_hooks_config_rejects_unknown_fields_events_and_schema_versions() {
    for value in [
        json!({"schema_version": "future", "hooks": {}}),
        json!({"schema_version": CONFIG_SCHEMA_VERSION, "hooks": {}, "unknown": true}),
        json!({"schema_version": CONFIG_SCHEMA_VERSION, "hooks": {"Future": []}}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = write_config(temp.path(), value);

        let snapshot = load_hook_config(Some(&path), None, &DenyProjectHookTrust);

        assert!(snapshot.handlers().is_empty());
        assert_eq!(snapshot.diagnostics.len(), 1);
        assert_eq!(
            snapshot.diagnostics[0].kind,
            HookConfigDiagnosticKind::Invalid
        );
    }
}

#[test]
fn oz_hooks_config_rejects_invalid_regex_timeout_and_failure_mode() {
    let cases = [
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"PreToolUse": [{"matcher": "(", "hooks": [{"type": "command", "command": "x"}]}]}
        }),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "x", "timeout": 121}]}]}
        }),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "x", "timeout": 4}]}]}
        }),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x", "on_failure": "deny"}]}]}
        }),
    ];
    for value in cases {
        let temp = tempfile::tempdir().unwrap();
        let path = write_config(temp.path(), value);

        let snapshot = load_hook_config(Some(&path), None, &DenyProjectHookTrust);

        assert!(snapshot.handlers().is_empty());
        assert_eq!(snapshot.diagnostics.len(), 1);
    }
}

#[test]
fn oz_hooks_config_enforces_file_and_handler_limits() {
    let temp = tempfile::tempdir().unwrap();
    let oversized = temp.path().join("oversized.json");
    fs::write(&oversized, vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
    let handlers = (0..=MAX_HANDLERS_PER_FILE)
        .map(|_| json!({"type": "command", "command": "x"}))
        .collect::<Vec<_>>();
    let too_many = write_config(
        temp.path(),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"Stop": [{"hooks": handlers}]}
        }),
    );

    let oversized_snapshot = load_hook_config(Some(&oversized), None, &DenyProjectHookTrust);
    let handler_snapshot = load_hook_config(Some(&too_many), None, &DenyProjectHookTrust);

    assert_eq!(oversized_snapshot.diagnostics.len(), 1);
    assert_eq!(handler_snapshot.diagnostics.len(), 1);
}

#[test]
fn oz_hooks_ordering_preserves_user_then_project_declarations() {
    let user_dir = tempfile::tempdir().unwrap();
    let project_dir = tempfile::tempdir().unwrap();
    let user_path = write_config(
        user_dir.path(),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"Stop": [{"hooks": [
                {"type": "command", "command": "user-one"},
                {"type": "command", "command": "user-two"}
            ]}]}
        }),
    );
    let project_path = write_config(
        project_dir.path(),
        json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "hooks": {"Stop": [{"hooks": [
                {"type": "command", "command": "project"}
            ]}]}
        }),
    );
    let bytes = fs::read(&project_path).unwrap();
    let trust = ExactHookTrustStore::default();
    trust.trust(HookTrustKey {
        git_root: fs::canonicalize(project_dir.path()).unwrap(),
        config_path: fs::canonicalize(&project_path).unwrap(),
        definition_hash: hex::encode(Sha256::digest(bytes)),
    });

    let snapshot = load_hook_config(
        Some(&user_path),
        Some(&ProjectConfig {
            path: project_path,
            git_root: project_dir.path().to_path_buf(),
        }),
        &trust,
    );

    assert_eq!(
        snapshot
            .handlers()
            .iter()
            .map(|handler| handler.command.as_str())
            .collect::<Vec<_>>(),
        ["user-one", "user-two", "project"]
    );
}

#[test]
fn oz_hooks_trust_requires_exact_bytes_and_supports_revocation() {
    let project_dir = tempfile::tempdir().unwrap();
    let path = write_config(project_dir.path(), valid_config());
    let project = ProjectConfig {
        path: path.clone(),
        git_root: project_dir.path().to_path_buf(),
    };
    let trust = ExactHookTrustStore::default();
    let key = HookTrustKey {
        git_root: fs::canonicalize(project_dir.path()).unwrap(),
        config_path: fs::canonicalize(&path).unwrap(),
        definition_hash: hex::encode(Sha256::digest(fs::read(&path).unwrap())),
    };

    assert!(
        load_hook_config(None, Some(&project), &trust)
            .handlers()
            .is_empty()
    );
    trust.trust(key.clone());
    assert_eq!(
        load_hook_config(None, Some(&project), &trust)
            .handlers()
            .len(),
        1
    );
    fs::write(&path, [fs::read(&path).unwrap(), b"\n".to_vec()].concat()).unwrap();
    assert!(
        load_hook_config(None, Some(&project), &trust)
            .handlers()
            .is_empty()
    );
    fs::write(&path, serde_json::to_vec(&valid_config()).unwrap()).unwrap();
    trust.revoke(&key);
    assert!(
        load_hook_config(None, Some(&project), &trust)
            .handlers()
            .is_empty()
    );
}

#[test]
fn oz_hooks_config_matcher_subject_rules_cover_all_events() {
    let temp = tempfile::tempdir().unwrap();
    let hooks = HookEventName::ALL
        .into_iter()
        .map(|event| {
            (
                event.as_str().to_owned(),
                json!([{"matcher": "^wanted$", "hooks": [{"type": "command", "command": "x"}]}]),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let path = write_config(
        temp.path(),
        json!({"schema_version": CONFIG_SCHEMA_VERSION, "hooks": hooks}),
    );
    let snapshot = load_hook_config(Some(&path), None, &DenyProjectHookTrust);

    for event in HookEventName::ALL {
        let matched = snapshot.matching_handlers(event, Some("other")).count();
        if event.ignores_matcher() {
            assert_eq!(matched, 1, "{event}");
        } else {
            assert_eq!(matched, 0, "{event}");
            assert_eq!(
                snapshot.matching_handlers(event, Some("wanted")).count(),
                1,
                "{event}"
            );
        }
    }
}
