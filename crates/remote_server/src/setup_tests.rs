#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::process::Stdio;

#[cfg(unix)]
use command::blocking::Command;

use super::*;

#[test]
fn parse_uname_linux_x86_64() {
    let platform = parse_uname_output("Linux x86_64").unwrap();
    assert_eq!(platform.os, RemoteOs::Linux);
    assert_eq!(platform.arch, RemoteArch::X86_64);
}

#[test]
fn parse_uname_linux_aarch64() {
    let platform = parse_uname_output("Linux aarch64").unwrap();
    assert_eq!(platform.os, RemoteOs::Linux);
    assert_eq!(platform.arch, RemoteArch::Aarch64);
}

#[test]
fn parse_uname_darwin_arm64() {
    let platform = parse_uname_output("Darwin arm64").unwrap();
    assert_eq!(platform.os, RemoteOs::MacOs);
    assert_eq!(platform.arch, RemoteArch::Aarch64);
}

#[test]
fn parse_uname_darwin_x86_64() {
    let platform = parse_uname_output("Darwin x86_64").unwrap();
    assert_eq!(platform.os, RemoteOs::MacOs);
    assert_eq!(platform.arch, RemoteArch::X86_64);
}

#[test]
fn parse_uname_unsupported_armv8l() {
    let result = parse_uname_output("Linux armv8l");
    match result {
        Err(crate::transport::Error::UnsupportedArch { arch }) => {
            assert_eq!(arch, "armv8l");
        }
        other => panic!("expected UnsupportedArch, got {other:?}"),
    }
}

#[test]
fn parse_uname_skips_shell_initialization_output() {
    let output = "Last login: Mon Apr  7 10:00:00 2025\nWelcome to Ubuntu\nLinux x86_64";
    let platform = parse_uname_output(output).unwrap();
    assert_eq!(platform.os, RemoteOs::Linux);
    assert_eq!(platform.arch, RemoteArch::X86_64);
}

#[test]
fn parse_uname_trims_whitespace() {
    let platform = parse_uname_output("  Linux x86_64  \n").unwrap();
    assert_eq!(platform.os, RemoteOs::Linux);
    assert_eq!(platform.arch, RemoteArch::X86_64);
}

#[test]
fn parse_uname_unsupported_os() {
    let result = parse_uname_output("Windows x86_64");
    match result {
        Err(crate::transport::Error::UnsupportedOs { os }) => {
            assert_eq!(os, "Windows");
        }
        other => panic!("expected UnsupportedOs, got {other:?}"),
    }
}

#[test]
fn parse_uname_unsupported_arch() {
    let result = parse_uname_output("Linux mips");
    match result {
        Err(crate::transport::Error::UnsupportedArch { arch }) => {
            assert_eq!(arch, "mips");
        }
        other => panic!("expected UnsupportedArch, got {other:?}"),
    }
}

#[test]
fn parse_uname_empty_output() {
    let result = parse_uname_output("");
    assert!(result.is_err());
}

#[test]
fn parse_uname_missing_arch() {
    let result = parse_uname_output("Linux");
    assert!(result.is_err());
}

#[test]
fn identity_dir_name_is_short_hash() {
    let name = remote_server_identity_dir_name("a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    assert_eq!(name.len(), 8, "identity dir should be 8 hex chars: {name}");
    assert!(
        name.chars().all(|c| c.is_ascii_hexdigit()),
        "identity dir should be hex: {name}"
    );
}

#[test]
fn identity_dir_name_is_deterministic() {
    let key = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    assert_eq!(
        remote_server_identity_dir_name(key),
        remote_server_identity_dir_name(key)
    );
}

#[test]
fn identity_dir_name_differs_for_different_keys() {
    assert_ne!(
        remote_server_identity_dir_name("key-a"),
        remote_server_identity_dir_name("key-b")
    );
}

#[test]
fn data_dir_uses_percent_encoded_identity_key() {
    let data_dir = remote_server_daemon_data_dir("user@example.com/ssh host");
    assert_eq!(
        data_dir,
        format!(
            "{}/user%40example%2Ecom%2Fssh%20host/data",
            remote_server_dir()
        )
    );
}

#[test]
fn data_dir_handles_empty_identity_key() {
    let data_dir = remote_server_daemon_data_dir("");
    assert_eq!(data_dir, format!("{}/empty/data", remote_server_dir()));
}

#[test]
fn daemon_dir_and_data_dir_use_different_identity_paths() {
    let key = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let daemon_dir = remote_server_daemon_dir(key);
    let data_dir = remote_server_daemon_data_dir(key);
    // Daemon dir uses the 8-char hash.
    assert!(daemon_dir.contains(&remote_server_identity_dir_name(key)));
    // Data dir uses the full key (no collision risk for persistent state).
    assert!(data_dir.contains(key));
    // They must be different paths.
    assert!(!data_dir.starts_with(&daemon_dir));
}

#[test]
fn state_is_ready() {
    assert!(RemoteServerSetupState::Ready.is_ready());
    assert!(!RemoteServerSetupState::Checking.is_ready());
    assert!(!RemoteServerSetupState::Initializing.is_ready());
}

#[test]
fn state_is_failed() {
    assert!(RemoteServerSetupState::Failed {
        error: "test".into()
    }
    .is_failed());
    assert!(!RemoteServerSetupState::Ready.is_failed());
}

#[test]
fn state_is_terminal() {
    assert!(RemoteServerSetupState::Ready.is_terminal());
    assert!(RemoteServerSetupState::Failed {
        error: "test".into()
    }
    .is_terminal());
    assert!(RemoteServerSetupState::Unsupported {
        reason: UnsupportedReason::NonGlibc {
            name: "musl".into()
        }
    }
    .is_terminal());
    assert!(!RemoteServerSetupState::Checking.is_terminal());
    assert!(!RemoteServerSetupState::Installing {
        progress_percent: None,
    }
    .is_terminal());
    assert!(!RemoteServerSetupState::Updating.is_terminal());
    assert!(!RemoteServerSetupState::Initializing.is_terminal());
}

#[test]
fn parse_preinstall_supported_glibc() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=glibc\n\
                  libc_version=2.35\n\
                  status=supported\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(result.status, PreinstallStatus::Supported);
    assert_eq!(result.libc, RemoteLibc::Glibc(GlibcVersion::new(2, 35)));
    assert!(result.is_supported());
}

#[test]
fn parse_preinstall_unsupported_glibc_too_old() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=glibc\n\
                  libc_version=2.17\n\
                  status=unsupported\n\
                  reason=glibc_too_old\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(
        result.status,
        PreinstallStatus::Unsupported {
            reason: UnsupportedReason::GlibcTooOld {
                detected: GlibcVersion::new(2, 17),
                required: GlibcVersion::new(2, 31),
            }
        }
    );
    assert!(!result.is_supported());
}

#[test]
fn parse_preinstall_unsupported_non_glibc() {
    let stdout = "required_glibc=2.31\n\
                  libc_family=musl\n\
                  status=unsupported\n\
                  reason=non_glibc\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(
        result.status,
        PreinstallStatus::Unsupported {
            reason: UnsupportedReason::NonGlibc {
                name: "musl".to_string()
            }
        }
    );
    assert_eq!(
        result.libc,
        RemoteLibc::NonGlibc {
            name: "musl".to_string()
        }
    );
    assert!(!result.is_supported());
}

#[test]
fn bundled_resources_dir_is_global_and_version_independent() {
    let dir = remote_server_bundled_resources_dir();
    assert_eq!(
        dir,
        format!("{}/{}", remote_server_dir(), BUNDLED_RESOURCES_DIR_NAME)
    );
    // The whole point of the global location: no version in the path.
    assert!(!dir.contains(remote_server_artifact_version()));
}

#[test]
fn binary_check_runs_version() {
    assert_eq!(
        binary_check_command(),
        format!("{} --version", remote_server_binary())
    );
}

#[test]
fn removal_command_removes_binary_but_leaves_global_resources() {
    let command = remote_server_removal_command();
    assert_eq!(command, format!("rm -f {}", remote_server_binary()));
    assert!(!command.contains(BUNDLED_RESOURCES_DIR_NAME));
}

#[test]
fn install_script_substitutes_bundled_resources_dir_name() {
    let script = install_script(None);
    assert!(!script.contains("{bundled_resources_dir_name}"));
    assert!(script.contains(&format!("$install_dir/{BUNDLED_RESOURCES_DIR_NAME}")));
}

#[cfg(unix)]
fn make_test_tarball(
    test_root: &std::path::Path,
    tarball_name: &str,
    skill_content: &str,
    include_decoy: bool,
) -> std::path::PathBuf {
    let tar_source = test_root.join(format!("tar-source-{tarball_name}"));
    let resources = tar_source.join("resources/bundled/skills/test-skill");
    let tarball = test_root.join(format!("{tarball_name}.tar.gz"));
    fs::create_dir_all(&resources).unwrap();
    fs::write(
        tar_source.join("oz-test"),
        "#!/usr/bin/env bash\n[ \"$1\" = \"--version\" ]\n",
    )
    .unwrap();
    fs::write(resources.join("SKILL.md"), skill_content).unwrap();
    if include_decoy {
        // Decoy: skills may ship companion files whose names also start
        // with `oz`. The installer must not mistake them for the executable.
        fs::write(
            resources.join("oz-decoy.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();
    }

    let tar_output = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tar_source)
        .arg("oz-test")
        .arg("resources")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to create test tarball");
    assert!(
        tar_output.status.success(),
        "tar failed: {}",
        String::from_utf8_lossy(&tar_output.stderr)
    );
    tarball
}

#[cfg(unix)]
fn run_install_script(tarball: &std::path::Path, fake_home: &std::path::Path) {
    let script = install_script(Some(tarball.to_str().unwrap()));
    let install_output = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env("HOME", fake_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run install script");
    assert!(
        install_output.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&install_output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn install_script_installs_binary_and_global_resources() {
    let test_root = std::env::temp_dir().join(format!(
        "remote-server-global-install-{}",
        uuid::Uuid::new_v4()
    ));
    let fake_home = test_root.join("home");
    fs::create_dir_all(&fake_home).unwrap();

    let tarball = make_test_tarball(&test_root, "first", "test skill", true);
    run_install_script(&tarball, &fake_home);

    let resolve_remote_path = |path: String| path.replacen('~', fake_home.to_str().unwrap(), 1);
    let binary = resolve_remote_path(remote_server_binary());
    let resources = resolve_remote_path(remote_server_bundled_resources_dir());
    let skill_md = std::path::Path::new(&resources).join("bundled/skills/test-skill/SKILL.md");

    assert!(fs::metadata(&binary).unwrap().is_file());
    assert!(fs::metadata(&resources).unwrap().is_dir());
    assert_eq!(fs::read_to_string(&skill_md).unwrap(), "test skill");
    assert!(std::path::Path::new(&resources)
        .join("bundled/skills/test-skill/oz-decoy.sh")
        .is_file());

    let check_output = Command::new("bash")
        .arg("-c")
        .arg(binary_check_command())
        .env("HOME", &fake_home)
        .output()
        .expect("failed to run binary check command");
    assert!(check_output.status.success());

    // A later install fully replaces the global resources (last install
    // wins): updated content lands and files absent from the new artifact
    // disappear, proving a swap rather than a merge.
    let second_tarball = make_test_tarball(&test_root, "second", "updated skill", false);
    run_install_script(&second_tarball, &fake_home);
    assert_eq!(fs::read_to_string(&skill_md).unwrap(), "updated skill");
    assert!(!std::path::Path::new(&resources)
        .join("bundled/skills/test-skill/oz-decoy.sh")
        .exists());

    // Removal deletes the binary but leaves the global resources for the
    // next install to overwrite.
    let removal_output = Command::new("bash")
        .arg("-c")
        .arg(remote_server_removal_command())
        .env("HOME", &fake_home)
        .output()
        .expect("failed to run removal command");
    assert!(removal_output.status.success());
    assert!(!std::path::Path::new(&binary).exists());
    assert!(fs::metadata(&resources).unwrap().is_dir());

    fs::remove_dir_all(test_root).unwrap();
}

#[cfg(unix)]
#[test]
fn install_script_tolerates_tarball_without_resources() {
    let test_root = std::env::temp_dir().join(format!(
        "remote-server-no-resources-install-{}",
        uuid::Uuid::new_v4()
    ));
    let fake_home = test_root.join("home");
    let tar_source = test_root.join("tar-source");
    let tarball = test_root.join("oz.tar.gz");
    fs::create_dir_all(&fake_home).unwrap();
    fs::create_dir_all(&tar_source).unwrap();
    fs::write(
        tar_source.join("oz-test"),
        "#!/usr/bin/env bash\n[ \"$1\" = \"--version\" ]\n",
    )
    .unwrap();

    let tar_output = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tar_source)
        .arg("oz-test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to create test tarball");
    assert!(tar_output.status.success());

    run_install_script(&tarball, &fake_home);

    let resolve_remote_path = |path: String| path.replacen('~', fake_home.to_str().unwrap(), 1);
    let binary = resolve_remote_path(remote_server_binary());
    let resources = resolve_remote_path(remote_server_bundled_resources_dir());
    assert!(fs::metadata(&binary).unwrap().is_file());
    assert!(!std::path::Path::new(&resources).exists());

    fs::remove_dir_all(test_root).unwrap();
}

/// Regression: the install script's tilde-expansion logic must work
/// across the bash versions we actually invoke at install time
/// (`run_ssh_script` pipes the script into `bash -s` on the remote).
/// Two interpreter quirks have to be avoided simultaneously:
///
///   1. bash 3.2 (macOS `/bin/bash`) keeps inner double-quotes around
///      the replacement of `${var/pattern/replacement}` literal, so
///      `"$HOME"` ends up as 6 literal characters and the install
///      lands under a directory tree literally named `"`.
///   2. bash 5.2+ with `patsub_replacement` (default-on) treats `&`
///      in the replacement as the matched pattern, so a `$HOME`
///      containing `&` resolves to a `~`-substituted path.
///
/// Both bugs surface as the install binary landing somewhere Warp's
/// launch step doesn't look, producing a misleading "Response channel
/// closed before receiving a reply".
///
/// This test drives the *actual* production script (via
/// [`install_script`]) rather than a hand-copied snippet, and runs it
/// against several `HOME` values to exercise the patsub-`&` trap as
/// well as the quote-literal trap. We truncate just before `mkdir -p`
/// so no filesystem side effects leak out of the test, and append a
/// marker `printf` to capture the resolved `install_dir`.
///
/// Gated to Unix because the test invokes `/bin/bash` (or `bash` from
/// PATH) directly. The bug only matters on Unix remotes anyway —
/// Warp's remote-server SSH wrapper doesn't target Windows hosts.
#[cfg(unix)]
#[test]
fn install_script_tilde_expansion_resolves_correctly() {
    let bash = if std::path::Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "bash"
    };

    let script = install_script(None);
    let cutoff = script.find("mkdir -p \"$install_dir\"").expect(
        "install script no longer contains the `mkdir -p \"$install_dir\"` \
         checkpoint this test relies on; update the test alongside the \
         script change",
    );
    let probe = format!(
        "{prefix}\nprintf '%s' \"$install_dir\"\nexit 0\n",
        prefix = &script[..cutoff],
    );

    // Run the probe against a matrix of HOME values. The first is an
    // ordinary path; the second contains `&`, which exercises bash
    // 5.2's patsub_replacement (where it would otherwise expand to
    // the matched `~`).
    let cases = [
        ("/Users/test", "ordinary HOME"),
        (
            "/Users/A&B",
            "HOME with `&` (bash 5.2 patsub_replacement trap)",
        ),
    ];

    for (fake_home, label) in cases {
        let output = Command::new(bash)
            .arg("-c")
            .arg(&probe)
            .env("HOME", fake_home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("failed to spawn bash");

        assert!(
            output.status.success(),
            "[{label}] bash exited with {:?}: stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );

        let install_dir = String::from_utf8_lossy(&output.stdout);
        assert!(
            !install_dir.contains('"'),
            "[{label}] install_dir contains literal quote characters \
             (bash 3.2 quote-literal regression): {install_dir:?}",
        );

        // Cross-check against the production layout: tilde must
        // resolve to HOME, so the result equals `remote_server_dir()`
        // with the leading `~` replaced.
        let expected = remote_server_dir().replacen('~', fake_home, 1);
        assert_eq!(
            install_dir, expected,
            "[{label}] install_dir resolved incorrectly",
        );
    }
}

/// Regression: guards against re-introducing the
/// `${var/pattern/replacement}` tilde-substitution form, which has two
/// known interpreter bugs (see
/// [`install_script_tilde_expansion_resolves_correctly`] for details).
/// Complements the live bash test — the live test catches behavioural
/// regressions, this static check fails fast and explains *why* in
/// the assertion message so a future contributor doesn't have to
/// re-discover the constraints from a CI failure.
#[test]
fn install_script_avoids_pattern_substitution_for_tilde_expansion() {
    let template = INSTALL_SCRIPT_TEMPLATE;
    assert!(
        !template.contains(r"/#\~/"),
        "install_remote_server.sh uses `${{var/#\\~/...}}` for tilde \
         expansion. This form has two known interpreter bugs that \
         silently mis-resolve the install path:\n\
         \n\
           1. bash 3.2 (macOS /bin/bash) keeps inner double-quotes \
              around the replacement literal, so `\"$HOME\"` ends up \
              as 6 literal characters including the quotes.\n\
           2. bash 5.2+ enables `patsub_replacement` by default, which \
              makes `&` in the replacement expand to the matched \
              pattern, so a `$HOME` containing `&` resolves wrong.\n\
         \n\
         Use `case`/`${{var#\\~}}` instead — see install_remote_server.sh \
         for the pattern.",
    );
}

#[test]
fn version_hash_is_deterministic() {
    // version_hash uses the compile-time GIT_RELEASE_TAG which is typically
    // unset in test builds, so it returns None. We test the hashing logic
    // directly instead.
    use std::hash::{Hash, Hasher};

    let version = "v0.2026.05.13.09.15.stable_01";
    let hash = |v: &str| -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        v.hash(&mut hasher);
        format!("{:016x}", hasher.finish())[..8].to_string()
    };

    // Same input produces the same hash.
    assert_eq!(hash(version), hash(version));
    // Different inputs produce different hashes.
    assert_ne!(hash(version), hash("v0.2026.05.14.09.15.stable_01"));
    // Hash is exactly 8 hex chars.
    assert_eq!(hash(version).len(), 8);
    assert!(hash(version).chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn daemon_socket_name_is_short() {
    // Without GIT_RELEASE_TAG (typical in tests), falls back to unversioned.
    let name = daemon_socket_name();
    // In test builds without GIT_RELEASE_TAG, we get "server.sock".
    // In release builds, we get "server-{8hex}.sock" = 24 chars.
    // Either way, the name must be ≤ 24 chars.
    assert!(
        name.len() <= 24,
        "daemon_socket_name is too long ({} chars): {name}",
        name.len()
    );
    assert!(name.starts_with("server"));
    assert!(name.ends_with(".sock"));
}

#[test]
fn daemon_pid_name_is_short() {
    let name = daemon_pid_name();
    assert!(
        name.len() <= 22,
        "daemon_pid_name is too long ({} chars): {name}",
        name.len()
    );
    assert!(name.starts_with("server"));
    assert!(name.ends_with(".pid"));
}

#[test]
fn socket_path_fits_within_sun_path_worst_case() {
    // Worst case: preview channel (longest base dir) + 32-char username
    // (Linux max) + hashed identity (8 chars) + hashed socket (20 chars).
    //
    // Path: /home/{user}/.warp-preview/remote-server/{hash8}/server-{hash8}.sock
    //       6 + 32 + 1 + 29 + 8 + 1 + 20 = 97 bytes → well under 103 (macOS)
    let long_home = "/home/a]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]";
    let identity_dir = remote_server_identity_dir_name("a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    assert_eq!(identity_dir.len(), 8);

    let hashed_socket = "server-a1b2c3d4.sock";
    let old_socket = "server-v0.2026.05.13.09.15.stable_01.sock";

    // Use .warp-preview (longest channel base dir) for worst case.
    let daemon_dir = format!("{long_home}/.warp-preview/remote-server/{identity_dir}");

    let hashed_path = format!("{daemon_dir}/{hashed_socket}");

    // Must fit within macOS sun_path limit (103 bytes), the stricter of
    // the two platforms.
    assert!(
        hashed_path.len() <= 103,
        "hashed socket path exceeds macOS sun_path limit: {} bytes ({})",
        hashed_path.len(),
        hashed_path,
    );

    // The OLD naming scheme (full version + unhashed identity) should
    // exceed the limit, confirming the regression.
    let old_identity = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"; // 36 chars unhashed
    let old_daemon_dir = format!("{long_home}/.warp-preview/remote-server/{old_identity}");
    let old_full_path = format!("{old_daemon_dir}/{old_socket}");
    assert!(
        old_full_path.len() > 107,
        "old socket path should exceed Linux sun_path limit to confirm the \
         regression: {} bytes ({})",
        old_full_path.len(),
        old_full_path,
    );
}

#[test]
fn parse_uname_linux_amd64() {
    let platform = parse_uname_output("Linux amd64").unwrap();
    assert_eq!(platform.os, RemoteOs::Linux);
    assert_eq!(platform.arch, RemoteArch::X86_64);
}

#[test]
fn parse_uname_unsupported_armv7l() {
    let result = parse_uname_output("Linux armv7l");
    match result {
        Err(crate::transport::Error::UnsupportedArch { arch }) => {
            assert_eq!(arch, "armv7l");
        }
        other => panic!("expected UnsupportedArch, got {other:?}"),
    }
}

#[test]
fn parse_preinstall_missing_status_falls_open() {
    // Garbled / partial script output — missing status field. Confirms
    // the fail-open invariant: anything we can't positively classify as
    // unsupported degrades to Unknown and is treated as supported, so a
    // flaky probe doesn't block the install.
    let stdout = "libc_family=glibc\nlibc_version=2.35\n";
    let result = PreinstallCheckResult::parse(stdout);
    assert_eq!(result.status, PreinstallStatus::Unknown);
    assert!(result.is_supported());
}
