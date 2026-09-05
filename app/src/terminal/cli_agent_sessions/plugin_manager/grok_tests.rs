use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use command::blocking::Command;
use serde_json::Value;

use super::{
    GrokPluginManager, HOOK_JSON_FILE, MINIMUM_PLUGIN_VERSION, PLUGIN_SCRIPT_REL, VERSION_FILE,
    install_plugin_files,
};
use crate::terminal::cli_agent_sessions::plugin_manager::CliAgentPluginManager;

struct GrokHomeGuard {
    original: Option<OsString>,
}

impl GrokHomeGuard {
    fn set(value: impl AsRef<OsStr>) -> Self {
        let original = std::env::var_os("GROK_HOME");
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("GROK_HOME", value) };
        Self { original }
    }
}

impl Drop for GrokHomeGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(original) => {
                // TODO: Audit that the environment access only happens in single-threaded code.
                unsafe { std::env::set_var("GROK_HOME", original) };
            }
            None => {
                // TODO: Audit that the environment access only happens in single-threaded code.
                unsafe { std::env::remove_var("GROK_HOME") };
            }
        }
    }
}

#[cfg(unix)]
fn run_plugin_script(command: &mut Command, input: &[u8]) -> Value {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let payload = output
        .stderr
        .strip_prefix(b"\x1b]777;notify;warp://cli-agent;")
        .and_then(|bytes| bytes.strip_suffix(b"\x07"))
        .expect("script should emit one OSC 777 notification");
    serde_json::from_slice(payload).unwrap()
}

#[cfg(unix)]
fn run_installed_hook(
    hooks_dir: &Path,
    hook_config: &Value,
    hook_name: &str,
    input: &[u8],
) -> Value {
    let command = hook_config["hooks"][hook_name][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    run_plugin_script(
        Command::new("/bin/bash")
            .arg("-c")
            .arg(command)
            .current_dir(hooks_dir),
        input,
    )
}
#[test]
#[serial_test::serial]
fn install_writes_current_plugin_files_with_executable_script() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());

    install_plugin_files().expect("install should succeed");

    let hooks = dir.path().join("hooks");
    assert!(GrokPluginManager.is_installed());
    assert!(!GrokPluginManager.needs_update());
    assert_eq!(
        std::fs::read_to_string(hooks.join(VERSION_FILE))
            .unwrap()
            .trim(),
        MINIMUM_PLUGIN_VERSION
    );
    let hook_config: Value =
        serde_json::from_slice(&std::fs::read(hooks.join(HOOK_JSON_FILE)).unwrap()).unwrap();
    assert_eq!(
        hook_config["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "bin/warp-plugin.sh SessionStart"
    );
    assert_eq!(
        hook_config["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        "bin/warp-plugin.sh UserPromptSubmit"
    );
    assert_eq!(
        hook_config["hooks"]["Stop"][0]["hooks"][0]["command"],
        "bin/warp-plugin.sh Stop"
    );
    assert_eq!(
        hook_config["hooks"]["StopFailure"][0]["hooks"][0]["command"],
        "bin/warp-plugin.sh StopFailure"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(hooks.join(PLUGIN_SCRIPT_REL))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "plugin script should be executable");
    }
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn installed_lifecycle_commands_emit_events_and_preserve_escaped_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());
    install_plugin_files().unwrap();
    let hooks = dir.path().join("hooks");
    let hook_config: Value =
        serde_json::from_slice(&std::fs::read(hooks.join(HOOK_JSON_FILE)).unwrap()).unwrap();

    let input = serde_json::json!({
        "hookEventName": "UserPromptSubmit",
        "sessionId": "session-\"quoted\"\\path\nnext",
        "cwd": "/tmp/\"quoted\"\\path\nnext",
        "prompt": "say \"hello\" from C:\\tools\nthen continue",
        "error": "line one\n\"line two\"\\tail",
    });
    let input = input.to_string();

    let session_start = run_installed_hook(&hooks, &hook_config, "SessionStart", input.as_bytes());
    assert_eq!(session_start["event"], "session_start");

    let prompt_submit =
        run_installed_hook(&hooks, &hook_config, "UserPromptSubmit", input.as_bytes());
    assert_eq!(prompt_submit["event"], "prompt_submit");
    assert_eq!(
        prompt_submit["session_id"],
        "session-\"quoted\"\\path\nnext"
    );
    assert_eq!(prompt_submit["cwd"], "/tmp/\"quoted\"\\path\nnext");
    assert_eq!(
        prompt_submit["query"],
        "say \"hello\" from C:\\tools\nthen continue"
    );
    assert_eq!(prompt_submit["error_type"], "line one\n\"line two\"\\tail");

    let stop = run_installed_hook(&hooks, &hook_config, "Stop", input.as_bytes());
    assert_eq!(stop["event"], "stop");

    let stop_failure = run_installed_hook(&hooks, &hook_config, "StopFailure", input.as_bytes());
    assert_eq!(stop_failure["event"], "stop_failure");
}
#[cfg(unix)]
#[test]
#[serial_test::serial]
fn installed_script_without_json_parser_emits_minimal_event() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());
    install_plugin_files().unwrap();
    let empty_path = dir.path().join("empty-path");
    std::fs::create_dir(&empty_path).unwrap();

    let event = run_plugin_script(
        Command::new("/bin/bash")
            .arg(dir.path().join("hooks").join(PLUGIN_SCRIPT_REL))
            .arg("Stop")
            .env("PATH", empty_path),
        br#"{"prompt":"ignored without a parser"}"#,
    );
    assert_eq!(
        event,
        serde_json::json!({
            "v": 1,
            "agent": "grok",
            "event": "stop",
            "plugin_version": MINIMUM_PLUGIN_VERSION,
        })
    );
}

#[test]
#[serial_test::serial]
fn installed_plugin_without_version_needs_update() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());
    install_plugin_files().unwrap();
    std::fs::remove_file(dir.path().join("hooks").join(VERSION_FILE)).unwrap();

    assert!(GrokPluginManager.is_installed());
    assert!(GrokPluginManager.needs_update());
}

#[test]
#[serial_test::serial]
fn installed_old_plugin_needs_update() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());
    install_plugin_files().unwrap();
    std::fs::write(dir.path().join("hooks").join(VERSION_FILE), "0.0.1\n").unwrap();

    assert!(GrokPluginManager.is_installed());
    assert!(GrokPluginManager.needs_update());
}

#[test]
#[serial_test::serial]
fn missing_plugin_is_not_installed_or_outdated() {
    let dir = tempfile::tempdir().unwrap();
    let _grok_home = GrokHomeGuard::set(dir.path());

    assert!(!GrokPluginManager.is_installed());
    assert!(!GrokPluginManager.needs_update());
}
