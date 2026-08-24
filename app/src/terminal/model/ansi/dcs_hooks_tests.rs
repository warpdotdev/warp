use std::collections::HashSet;

use super::*;

#[test]
fn bootstrapped_hook_parses_full_payload() {
    let json = r#"{
        "hook": "Bootstrapped",
        "value": {
            "session_id": 42,
            "histfile": "/Users/me/.zsh_history",
            "shell": "zsh\u0000",
            "home_dir": "/Users/me",
            "path": "/usr/bin:/bin",
            "cdpath": "",
            "editor": "vim",
            "aliases": "",
            "abbreviations": "",
            "function_names": "foo bar",
            "env_var_names": "PATH HOME",
            "builtins": "cd echo",
            "keywords": "if then",
            "shell_version": "5.9",
            "shell_options": "autocd extendedglob",
            "rcfiles_start_time": "100.5",
            "rcfiles_end_time": "101.25",
            "shell_plugins": "ohmyzsh",
            "vi_mode_enabled": "",
            "os_category": "MacOS",
            "linux_distribution": "",
            "wsl_name": "",
            "shell_path": "/bin/zsh"
        }
    }"#;
    let DProtoHook::Bootstrapped { value } = serde_json::from_str::<DProtoHook>(json).unwrap()
    else {
        panic!("expected a Bootstrapped hook");
    };
    let expected = BootstrappedValue {
        session_id: Some(42),
        histfile: Some("/Users/me/.zsh_history".to_string()),
        shell: "zsh".to_string(),
        home_dir: Some("/Users/me".to_string()),
        path: Some("/usr/bin:/bin".to_string()),
        cdpath: None,
        editor: Some("vim".to_string()),
        aliases: None,
        abbreviations: None,
        function_names: Some("foo bar".to_string()),
        env_var_names: Some("PATH HOME".to_string()),
        builtins: Some("cd echo".to_string()),
        keywords: Some("if then".to_string()),
        shell_version: Some("5.9".to_string()),
        shell_options: Some(HashSet::from([
            "autocd".to_string(),
            "extendedglob".to_string(),
        ])),
        rcfiles_start_time: Some(100.5.into()),
        rcfiles_end_time: Some(101.25.into()),
        shell_plugins: Some(HashSet::from(["ohmyzsh".to_string()])),
        vi_mode_enabled: None,
        os_category: Some("MacOS".to_string()),
        linux_distribution: None,
        wsl_name: None,
        shell_path: Some("/bin/zsh".to_string()),
    };
    assert_eq!(*value, expected);
}

#[test]
fn bootstrapped_hook_defaults_optional_fields() {
    let json = r#"{
        "hook": "Bootstrapped",
        "value": {
            "histfile": "",
            "home_dir": "/Users/me",
            "path": "",
            "aliases": "",
            "abbreviations": "",
            "function_names": "",
            "env_var_names": "",
            "builtins": "",
            "keywords": "",
            "shell_version": ""
        }
    }"#;
    let DProtoHook::Bootstrapped { value } = serde_json::from_str::<DProtoHook>(json).unwrap()
    else {
        panic!("expected a Bootstrapped hook");
    };
    let expected = BootstrappedValue {
        home_dir: Some("/Users/me".to_string()),
        ..Default::default()
    };
    assert_eq!(*value, expected);
}

#[test]
fn bootstrapped_hook_requires_required_fields() {
    // The histfile field has no default, so its absence is an error.
    let json = r#"{"hook": "Bootstrapped", "value": {"home_dir": "/Users/me"}}"#;
    assert!(serde_json::from_str::<DProtoHook>(json).is_err());
}

#[test]
fn bootstrapped_hook_rejects_malformed_rcfiles_time() {
    let json = r#"{
        "hook": "Bootstrapped",
        "value": {
            "histfile": "",
            "home_dir": "",
            "path": "",
            "aliases": "",
            "abbreviations": "",
            "function_names": "",
            "env_var_names": "",
            "builtins": "",
            "keywords": "",
            "shell_version": "",
            "rcfiles_start_time": "not-a-float"
        }
    }"#;
    assert!(serde_json::from_str::<DProtoHook>(json).is_err());
}

#[test]
fn unknown_hook_name_is_an_error() {
    let json = r#"{"hook": "NotARealHook", "value": {}}"#;
    let error = serde_json::from_str::<DProtoHook>(json).unwrap_err();
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn missing_value_field_is_an_error() {
    let json = r#"{"hook": "Clear"}"#;
    assert!(serde_json::from_str::<DProtoHook>(json).is_err());
}

#[test]
fn every_hook_tag_dispatches_to_the_matching_variant() {
    let cases = [
        (
            "CommandFinished",
            serde_json::json!({"exit_code": 0, "next_block_id": "block-1"}),
        ),
        ("Precmd", serde_json::json!({})),
        ("Preexec", serde_json::json!({"command": "echo hi"})),
        (
            "Bootstrapped",
            serde_json::json!({
                "histfile": "",
                "home_dir": "",
                "path": "",
                "aliases": "",
                "abbreviations": "",
                "function_names": "",
                "env_var_names": "",
                "builtins": "",
                "keywords": "",
                "shell_version": "",
            }),
        ),
        ("PreInteractiveSSHSession", serde_json::json!({})),
        (
            "SSH",
            serde_json::json!({"socket_path": "/tmp/warp.sock", "remote_shell": "bash"}),
        ),
        (
            "InitShell",
            serde_json::json!({"session_id": 1, "shell": "zsh"}),
        ),
        ("InputBuffer", serde_json::json!({"buffer": "echo hi"})),
        ("Clear", serde_json::json!({})),
        (
            "InitSubshell",
            serde_json::json!({"shell": "zsh", "uname": "Darwin"}),
        ),
        (
            "SourcedRcFileForWarp",
            serde_json::json!({"shell": "zsh", "uname": "Darwin"}),
        ),
        ("FinishUpdate", serde_json::json!({"update_id": "update-1"})),
        ("ExitShell", serde_json::json!({"session_id": 1})),
    ];
    let covered_variants = cases.each_ref().map(|(name, _)| *name);
    assert_eq!(covered_variants.as_slice(), DPROTO_HOOK_VARIANTS);

    for (expected_name, value) in cases {
        let hook: DProtoHook = serde_json::from_value(serde_json::json!({
            "hook": expected_name,
            "value": value,
        }))
        .unwrap();
        assert_eq!(hook.name(), expected_name);

        let serialized = serde_json::to_value(hook).unwrap();
        assert_eq!(serialized["hook"], expected_name);
    }
}

#[test]
fn precmd_hook_with_completion_metadata_classifies() {
    let json = r#"{
        "hook": "Precmd",
        "value": {
            "pwd": "/Users/me",
            "ps1": "",
            "exit_code": 0,
            "next_block_id": "block-1",
            "session_id": 7
        }
    }"#;
    let DProtoHook::Precmd { value } = serde_json::from_str::<DProtoHook>(json).unwrap() else {
        panic!("expected a Precmd hook");
    };
    let PrecmdHookValue::WithCompletionMetadata(value) = value else {
        panic!("expected completion metadata");
    };
    assert_eq!(value.prompt_metadata.pwd.as_deref(), Some("/Users/me"));
    assert_eq!(value.prompt_metadata.session_id, Some(7));
}

#[test]
fn precmd_hook_without_completion_metadata_is_prompt_only() {
    let json = r#"{"hook": "Precmd", "value": {"pwd": "/Users/me", "session_id": 7}}"#;
    let DProtoHook::Precmd { value } = serde_json::from_str::<DProtoHook>(json).unwrap() else {
        panic!("expected a Precmd hook");
    };
    assert!(matches!(value, PrecmdHookValue::PromptOnly(_)));
}

#[test]
fn sourced_rc_file_hook_parses_frozen_snippet_format() {
    // This literal payload shape ships inside user RC files, so it must keep
    // parsing forever.
    let json = r#"{"hook": "SourcedRcFileForWarp", "value": {"shell": "zsh", "uname": "Darwin"}}"#;
    let DProtoHook::SourcedRcFileForWarp { value } =
        serde_json::from_str::<DProtoHook>(json).unwrap()
    else {
        panic!("expected a SourcedRcFileForWarp hook");
    };
    assert_eq!(value.shell, "zsh");
    assert_eq!(value.uname.as_deref(), Some("Darwin"));
}

#[test]
fn sourced_rc_file_hook_ignores_legacy_tmux_field() {
    let json = r#"{"hook": "SourcedRcFileForWarp", "value": {"shell": "zsh", "uname": "Darwin", "tmux": false}}"#;
    assert!(serde_json::from_str::<DProtoHook>(json).is_ok());
}

#[test]
fn ssh_hook_round_trips_through_serialization() {
    let hook = DProtoHook::SSH {
        value: SSHValue {
            socket_path: "/tmp/warp.sock".into(),
            remote_shell: "bash".to_string(),
            session_id: Some(3),
            remote_session_id: Some(4),
            external_control_master: true,
        },
    };
    let json = serde_json::to_string(&hook).unwrap();
    let DProtoHook::SSH { value } = serde_json::from_str::<DProtoHook>(&json).unwrap() else {
        panic!("expected an SSH hook");
    };
    assert_eq!(value.socket_path, PathBuf::from("/tmp/warp.sock"));
    assert_eq!(value.remote_shell, "bash");
    assert_eq!(value.session_id, Some(3));
    assert_eq!(value.remote_session_id, Some(4));
    assert!(value.external_control_master);
}

#[test]
fn init_shell_hook_round_trips_through_serialization() {
    // Note that `wsl_name` must be `Some` here: the field's deserializer
    // requires a string, and `None` serializes to `null`. Real hook payloads
    // always send a string, with the empty string standing in for `None`.
    let hook = DProtoHook::InitShell {
        value: InitShellValue {
            session_id: 9.into(),
            shell: "zsh".to_string(),
            is_subshell: false,
            user: "me".to_string(),
            hostname: "host".to_string(),
            wsl_name: Some("Ubuntu".to_string()),
        },
    };
    let json = serde_json::to_string(&hook).unwrap();
    let DProtoHook::InitShell { value } = serde_json::from_str::<DProtoHook>(&json).unwrap() else {
        panic!("expected an InitShell hook");
    };
    assert_eq!(value.session_id, 9.into());
    assert_eq!(value.shell, "zsh");
    assert_eq!(value.user, "me");
    assert_eq!(value.hostname, "host");
    assert_eq!(value.wsl_name.as_deref(), Some("Ubuntu"));
}
