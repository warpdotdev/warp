use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::tempdir;
use warp_cli::agent::Harness;

use super::{MANIFEST_FILE, discover, read_manifest, resolve_sidecar_program};

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

#[test]
fn native_manifests_resolve_paths_after_relocation_without_rewriting_literal_args() {
    let root = tempdir().unwrap();
    let original = root.path().join("original");
    let relocated = root.path().join("relocated package with spaces");
    let sidecar = &original;
    let node = format!("runtime/{PROGRAM}");
    write_executable(&sidecar.join(&node));
    fs::write(sidecar.join("adapter.js"), "").unwrap();
    fs::write(
        sidecar.join("warp-harness.pi.json"),
        json!({
            "schema_version": 1,
            "harness": "pi",
            "acp": {
                "program": format!("${{SIDECAR_ROOT}}/{node}"),
                "args": ["${SIDECAR_ROOT}/adapter.js", "--flag with spaces", "relative/value"]
            }
        })
        .to_string(),
    )
    .unwrap();
    fs::rename(&original, &relocated).unwrap();
    let selected = discover(
        Harness::Pi,
        None,
        &root.path().join("mnt"),
        std::slice::from_ref(&relocated),
    )
    .unwrap();
    assert_eq!(
        Path::new(&selected.launch.program),
        fs::canonicalize(relocated.join(node)).unwrap()
    );
    assert_eq!(
        selected.launch.args,
        [
            fs::canonicalize(relocated.join("adapter.js"))
                .unwrap()
                .to_str()
                .unwrap(),
            "--flag with spaces",
            "relative/value"
        ]
    );
}

#[test]
fn native_discovery_preserves_override_mount_and_path_order_and_fails_closed() {
    let root = tempdir().unwrap();
    let mounts = root.path().join("mnt");
    let override_dir = root.path().join("override");
    let path_dirs = [root.path().join("first"), root.path().join("second")];
    let mount = mounts.join("pi-cli-sidecar");
    for (dir, name, arg) in [
        (&override_dir, MANIFEST_FILE, "override"),
        (&mount, MANIFEST_FILE, "mount"),
        (&path_dirs[0], "warp-harness.pi.json", "first"),
        (&path_dirs[1], "warp-harness.pi.json", "second"),
    ] {
        write_manifest(&dir.join(name), &[arg]);
        write_executable(&dir.join("usr/local/bin").join(PROGRAM));
    }
    assert_eq!(
        discover(Harness::Pi, Some(&override_dir), &mounts, &path_dirs)
            .unwrap()
            .launch
            .args,
        ["override"]
    );
    assert_eq!(
        discover(Harness::Pi, None, &mounts, &path_dirs)
            .unwrap()
            .launch
            .args,
        ["mount"]
    );
    fs::remove_file(mount.join(MANIFEST_FILE)).unwrap();
    assert_eq!(
        discover(Harness::Pi, None, &mounts, &path_dirs)
            .unwrap()
            .launch
            .args,
        ["first"]
    );
    fs::write(path_dirs[0].join("warp-harness.pi.json"), "invalid").unwrap();
    assert!(discover(Harness::Pi, None, &mounts, &path_dirs).is_err());
}

#[test]
fn explicit_sidecar_paths_reject_traversal_absolute_paths_and_missing_files() {
    let root = tempdir().unwrap();
    for relative in [
        "",
        "/absolute",
        "../escape",
        "nested/../escape",
        "./file",
        r"C:\file.exe",
        "C:file.exe",
        r"nested\file",
        "missing.js",
    ] {
        assert!(
            super::resolve_sidecar_path(root.path(), &format!("${{SIDECAR_ROOT}}/{relative}"))
                .is_err(),
            "{relative}"
        );
    }
    #[cfg(unix)]
    {
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("adapter.js"), "").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("adapter.js"),
            root.path().join("escape"),
        )
        .unwrap();
        assert!(super::resolve_sidecar_path(root.path(), "${SIDECAR_ROOT}/escape").is_err());
    }
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

    let selected = discover(Harness::Pi, Some(&override_dir), &mounts, &[]).unwrap();
    assert_eq!(selected.launch.args, ["override"]);
    assert_eq!(
        Path::new(&selected.launch.program),
        override_dir.join("usr/local/bin").join(PROGRAM)
    );
    assert!(selected.is_sidecar);
    let selected = discover(Harness::Pi, None, &mounts, &[]).unwrap();
    assert_eq!(selected.launch.args, ["mount"]);
    fs::remove_file(mount_manifest).unwrap();
    let selected = discover(Harness::Pi, None, &mounts, &[]).unwrap();
    assert_eq!(selected.launch.program, "pi-acp");
    assert!(selected.launch.args.is_empty());
    assert!(!selected.is_sidecar);
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

    let error = discover(Harness::Pi, Some(&override_dir), root.path(), &[])
        .err()
        .unwrap();
    assert!(error.to_string().contains("expected pi"));
    fs::remove_file(&path).unwrap();
    let error = discover(Harness::Pi, Some(&override_dir), root.path(), &[])
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
