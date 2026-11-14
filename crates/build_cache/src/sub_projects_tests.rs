use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use command::r#async::Command;
use futures::executor::block_on;

use super::{CargoWorkspace, ModeLocality, classified_mode_locality, mode_locality, needs_dry_run};
use crate::CacheSetupError;

/// Modes provided by spacectl 0.12.2, the version the cloud agent image installs
/// (`SPACECTL_VERSION` in warpdotdev/warp-agent-docker's `lib/build-tools.sh`).
const SPACECTL_MODES: &[&str] = &[
    "apt",
    "brew",
    "bun",
    "cocoapods",
    "composer",
    "deno",
    "go",
    "golangci-lint",
    "gradle",
    "kotlin-native",
    "maven",
    "mise",
    "nix",
    "npm",
    "playwright",
    "pnpm",
    "poetry",
    "python",
    "ruby",
    "rust",
    "swiftpm",
    "tuist",
    "uv",
    "xcode",
    "yarn",
];

fn modes(names: &[&str]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

fn cargo_metadata(manifests: &[PathBuf]) -> Vec<u8> {
    let packages = manifests
        .iter()
        .map(|manifest| serde_json::json!({"name": "crate", "manifest_path": manifest}))
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({"packages": packages, "workspace_root": "/ignored"}))
        .unwrap()
}

fn workspace_with_member(root: &Path, member: &str) -> PathBuf {
    let member = root.join(member);
    fs::create_dir_all(&member).unwrap();
    fs::write(root.join("Cargo.toml"), "").unwrap();
    fs::write(member.join("Cargo.toml"), "").unwrap();
    member
}

fn decide(
    detected: &[&str],
    cwd: &Path,
    repository_root: &Path,
    workspace: &CargoWorkspace,
    metadata: Result<Vec<u8>, CacheSetupError>,
) -> bool {
    block_on(needs_dry_run(
        &modes(detected),
        cwd,
        repository_root,
        workspace,
        &|_: Command| futures::future::ready(metadata.clone()),
    ))
}

#[test]
fn every_spacectl_mode_is_classified() {
    let unclassified = SPACECTL_MODES
        .iter()
        .filter(|mode| classified_mode_locality(mode).is_none())
        .collect::<Vec<_>>();
    assert!(
        unclassified.is_empty(),
        "classify {unclassified:?} in `classified_mode_locality` by reading their providers in \
         spacectl's internal/cache/mode/providers.go"
    );
}

/// Runs wherever spacectl is installed, such as cloud agent sandboxes, so that upgrading spacectl
/// cannot silently add modes that this table treats as unknown.
#[test]
fn installed_spacectl_modes_are_classified() {
    let Ok(output) = command::blocking::Command::new("spacectl")
        .args(["cache", "modes", "-o", "json"])
        .output()
    else {
        return;
    };
    assert!(output.status.success(), "spacectl cache modes failed");
    let response = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    let unclassified = response["modes"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|mode| classified_mode_locality(mode).is_none())
        .collect::<BTreeSet<_>>();
    assert!(
        unclassified.is_empty(),
        "the installed spacectl provides cache modes {unclassified:?} that are not classified; \
         classify them in `classified_mode_locality` and update SPACECTL_MODES and its version \
         if the agent image installs this spacectl"
    );
}

#[test]
fn modes_that_only_mount_outside_the_repository_are_global() {
    for mode in [
        "apt",
        "brew",
        "composer",
        "deno",
        "go",
        "golangci-lint",
        "gradle",
        "kotlin-native",
        "maven",
        "mise",
        "nix",
        "playwright",
        "python",
        "tuist",
        "xcode",
    ] {
        assert_eq!(mode_locality(mode), ModeLocality::Global, "{mode}");
    }
}

#[test]
fn project_local_and_unknown_modes_require_a_dry_run() {
    for mode in ["cocoapods", "ruby", "swiftpm", "some-future-mode", ""] {
        assert_eq!(mode_locality(mode), ModeLocality::Repository, "{mode}");
    }
    assert_eq!(mode_locality("rust"), ModeLocality::CargoTarget);
    for mode in ["bun", "npm", "pnpm", "poetry", "uv", "yarn"] {
        assert!(
            matches!(mode_locality(mode), ModeLocality::ProjectConfigurable(_)),
            "{mode}"
        );
    }
}

#[test]
fn global_modes_skip_the_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("tools/linter");
    fs::create_dir_all(&child).unwrap();
    let workspace = CargoWorkspace::new(temp.path().to_owned());

    assert!(!decide(
        &["apt", "go", "golangci-lint"],
        &child,
        temp.path(),
        &workspace,
        Err(CacheSetupError::SpawnFailed),
    ));
}

#[test]
fn unknown_or_project_local_modes_need_a_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = CargoWorkspace::new(temp.path().to_owned());

    for detected in [&["apt", "some-future-mode"][..], &["go", "ruby"][..]] {
        assert!(decide(
            detected,
            temp.path(),
            temp.path(),
            &workspace,
            Err(CacheSetupError::SpawnFailed),
        ));
    }
}

#[test]
fn project_configuration_between_sub_project_and_root_needs_a_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let child = root.join("web/app");
    fs::create_dir_all(&child).unwrap();
    let workspace = CargoWorkspace::new(root.clone());
    let no_metadata = || Err(CacheSetupError::SpawnFailed);

    assert!(!decide(
        &["npm", "yarn"],
        &child,
        &root,
        &workspace,
        no_metadata()
    ));

    fs::write(temp.path().join(".npmrc"), "cache=./npm-cache").unwrap();
    assert!(
        !decide(&["npm"], &child, &root, &workspace, no_metadata()),
        "configuration outside the repository applies to the root as well"
    );

    fs::write(root.join("web/.npmrc"), "cache=./npm-cache").unwrap();
    assert!(decide(&["npm"], &child, &root, &workspace, no_metadata()));

    fs::create_dir_all(child.join(".yarn")).unwrap();
    assert!(decide(&["yarn"], &child, &root, &workspace, no_metadata()));
}

#[test]
fn cargo_workspace_members_skip_the_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let member = workspace_with_member(temp.path(), "crates/member");
    let workspace = CargoWorkspace::new(temp.path().to_owned());

    assert!(!decide(
        &["apt", "rust"],
        &member,
        temp.path(),
        &workspace,
        Ok(cargo_metadata(&[
            temp.path().join("Cargo.toml"),
            member.join("Cargo.toml"),
        ])),
    ));
}

#[test]
fn rust_sub_projects_outside_the_root_workspace_need_a_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let member = workspace_with_member(temp.path(), "crates/member");
    let excluded = workspace_with_member(temp.path(), "fuzz");
    let metadata = cargo_metadata(&[temp.path().join("Cargo.toml"), member.join("Cargo.toml")]);

    assert!(decide(
        &["rust"],
        &excluded,
        temp.path(),
        &CargoWorkspace::new(temp.path().to_owned()),
        Ok(metadata),
    ));
    assert!(
        decide(
            &["rust"],
            &member,
            temp.path(),
            &CargoWorkspace::new(temp.path().to_owned()),
            Err(CacheSetupError::NonzeroExit {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "toolchain is not installed".to_owned(),
            }),
        ),
        "members cannot be identified without cargo metadata"
    );
}

#[test]
fn cargo_configuration_below_the_root_needs_a_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let member = workspace_with_member(temp.path(), "crates/member");
    let metadata = cargo_metadata(&[temp.path().join("Cargo.toml"), member.join("Cargo.toml")]);

    fs::create_dir_all(temp.path().join(".cargo")).unwrap();
    fs::write(temp.path().join(".cargo/config.toml"), "").unwrap();
    assert!(
        !decide(
            &["rust"],
            &member,
            temp.path(),
            &CargoWorkspace::new(temp.path().to_owned()),
            Ok(metadata.clone()),
        ),
        "root configuration applies to the root and its members alike"
    );

    fs::create_dir_all(temp.path().join("crates/.cargo")).unwrap();
    fs::write(temp.path().join("crates/.cargo/config.toml"), "").unwrap();
    assert!(decide(
        &["rust"],
        &member,
        temp.path(),
        &CargoWorkspace::new(temp.path().to_owned()),
        Ok(metadata),
    ));
}

#[test]
fn cargo_metadata_runs_once_per_repository_without_toolchain_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let first = workspace_with_member(temp.path(), "crates/first");
    let second = workspace_with_member(temp.path(), "crates/second");
    let metadata = cargo_metadata(&[first.join("Cargo.toml"), second.join("Cargo.toml")]);
    let workspace = CargoWorkspace::new(temp.path().to_owned());
    let calls = AtomicUsize::new(0);
    let run_command = |command: Command| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(command.get_program(), "cargo");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(temp.path()));
        assert!(format!("{command:?}").contains(r#"RUSTUP_AUTO_INSTALL="0""#));
        futures::future::ready(Ok(metadata.clone()))
    };

    for member in [&first, &second] {
        assert!(!block_on(needs_dry_run(
            &modes(&["rust"]),
            member,
            temp.path(),
            &workspace,
            &run_command,
        )));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
