use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::tempdir;
use warp_cli::agent::Harness;

use super::{
    MANIFEST_FILE, default_adapter, default_manifest, discover, read_manifest,
    resolve_sidecar_program,
};

#[cfg(windows)]
const PROGRAM: &str = "adapter.exe";
#[cfg(not(windows))]
const PROGRAM: &str = "adapter";

fn write_manifest(path: &Path, args: &[&str]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        json!({
            "schema_version": 1,
            "harness": "pi",
            "acp": {"program": PROGRAM, "args": args}
        })
        .to_string(),
    )
    .unwrap();
}

fn write_executable(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "").unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn discovery_selects_override_then_mount_then_defaults_without_merging() {
    let root = tempdir().unwrap();
    let override_dir = root.path().join("override");
    let mounts = root.path().join("mnt");
    let mount_dir = mounts.join("pi-cli-sidecar");
    let mount_manifest = mount_dir.join(MANIFEST_FILE);
    write_manifest(&override_dir.join(MANIFEST_FILE), &["override"]);
    write_manifest(&mount_manifest, &["mount"]);
    for dir in [&override_dir, &mount_dir] {
        write_executable(&dir.join("usr/local/bin").join(PROGRAM));
    }

    let selected = discover(Harness::Pi, Some(&override_dir), &mounts).unwrap();
    assert_eq!(selected.launch.args, ["override"]);
    assert_eq!(
        Path::new(&selected.launch.program),
        override_dir.join("usr/local/bin").join(PROGRAM)
    );
    assert!(selected.default.is_none());
    assert_eq!(
        selected.program_candidates(),
        [selected.launch.program.as_str()]
    );
    let selected = discover(Harness::Pi, None, &mounts).unwrap();
    assert_eq!(selected.launch.args, ["mount"]);
    fs::remove_file(mount_manifest).unwrap();
    let selected = discover(Harness::Pi, None, &mounts).unwrap();
    assert_eq!(selected.launch.program, "pi-acp");
    assert!(selected.launch.args.is_empty());
    assert!(selected.default.is_some());
}

#[test]
fn override_errors_do_not_fall_through_to_defaults() {
    let root = tempdir().unwrap();
    let override_dir = root.path().join("override");
    let path = override_dir.join(MANIFEST_FILE);
    write_manifest(&path, &[]);
    let mut wrong: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    wrong["harness"] = json!("codex");
    fs::write(&path, wrong.to_string()).unwrap();

    let error = discover(Harness::Pi, Some(&override_dir), root.path())
        .err()
        .unwrap();
    assert!(error.to_string().contains("expected pi"));
    fs::remove_file(&path).unwrap();
    let error = discover(Harness::Pi, Some(&override_dir), root.path())
        .err()
        .unwrap();
    assert!(error.to_string().contains(&path.display().to_string()));
}

#[test]
fn sidecar_program_requires_an_executable_under_usr_local_bin() {
    let root = tempdir().unwrap();
    write_executable(&root.path().join("bin").join(PROGRAM));
    assert!(resolve_sidecar_program(root.path(), PROGRAM).is_err());
    let executable = root.path().join("usr/local/bin").join(PROGRAM);
    write_executable(&executable);
    #[cfg(unix)]
    {
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            resolve_sidecar_program(root.path(), PROGRAM)
                .unwrap_err()
                .to_string()
                .contains("not executable")
        );
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(
        Path::new(&resolve_sidecar_program(root.path(), PROGRAM).unwrap()),
        executable
    );
    for program in ["/adapter", "../adapter", r"bin\adapter", "C:adapter.exe"] {
        assert!(
            resolve_sidecar_program(root.path(), program)
                .unwrap_err()
                .to_string()
                .contains("bare filename")
        );
    }
}

#[test]
fn unsupported_versions_and_unknown_section_fields_are_rejected() {
    let root = tempdir().unwrap();
    let path = root.path().join(MANIFEST_FILE);
    write_manifest(&path, &[]);
    let valid: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut unsupported = valid.clone();
    unsupported["schema_version"] = json!(2);
    fs::write(&path, unsupported.to_string()).unwrap();
    assert!(
        read_manifest(&path, Harness::Pi)
            .unwrap_err()
            .to_string()
            .contains("unsupported schema_version")
    );
    let mut future_section = valid.clone();
    future_section["tui"] = json!({"program": "pi"});
    fs::write(&path, future_section.to_string()).unwrap();
    assert!(read_manifest(&path, Harness::Pi).is_ok());
    let mut acp = valid.clone();
    acp["acp"]["unknown"] = json!(true);
    let mut auth = valid;
    auth["auth"] = json!({"unknown": true});
    for invalid in [acp, auth] {
        fs::write(&path, invalid.to_string()).unwrap();
        assert!(
            format!("{:#}", read_manifest(&path, Harness::Pi).unwrap_err())
                .contains("unknown field")
        );
    }
}

#[test]
fn claude_default_lists_the_package_launcher_before_the_npm_bin() {
    let manifest = default_manifest(Harness::Claude).unwrap();
    assert_eq!(manifest.launch.program, "claude-code-acp");
    assert!(manifest.launch.args.is_empty());
    assert_eq!(
        manifest.program_candidates(),
        ["claude-code-acp", "claude-agent-acp"]
    );
    let adapter = manifest.default.unwrap();
    assert_eq!(adapter.wraps_cli, Some("claude"));
    assert_eq!(
        adapter.package,
        Some("@agentclientprotocol/claude-agent-acp")
    );
    // The terminal transport's auth knowledge comes along.
    assert_eq!(
        manifest.auth_check_command.as_deref(),
        Some("claude auth status --json")
    );

    // The other defaults are single names that wrap nothing.
    for harness in [Harness::Codex, Harness::Gemini, Harness::Pi] {
        let (adapter, _) = default_adapter(harness).unwrap();
        assert!(adapter.alternates.is_empty(), "{harness}");
        assert_eq!(adapter.wraps_cli, None, "{harness}");
    }
    for harness in [Harness::Oz, Harness::OpenCode, Harness::Unknown] {
        assert!(default_adapter(harness).is_none(), "{harness}");
    }
}

#[test]
fn missing_adapter_reason_names_every_program_the_package_and_the_wrapped_cli() {
    let (claude, _) = default_adapter(Harness::Claude).unwrap();
    let reason = claude.missing_reason(Some("https://docs.example"));
    for expected in [
        "'claude-code-acp'",
        "'claude-agent-acp'",
        "npm install -g @agentclientprotocol/claude-agent-acp",
        "oz process",
        "shell",
        "'claude' CLI must be installed too: https://docs.example",
    ] {
        assert!(reason.contains(expected), "{reason:?} lacks {expected:?}");
    }
    assert!(!claude.missing_reason(None).contains("https://"));

    let (codex, _) = default_adapter(Harness::Codex).unwrap();
    let reason = codex.missing_reason(Some("https://docs.example"));
    assert!(
        reason.starts_with("ACP adapter 'codex-acp' is not on PATH."),
        "{reason:?}"
    );
    assert!(!reason.contains("npm install"), "{reason:?}");
    assert!(
        reason.ends_with("Install docs: https://docs.example"),
        "{reason:?}"
    );
}
