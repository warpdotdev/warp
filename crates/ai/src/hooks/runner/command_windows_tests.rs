use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use futures::executor::block_on;
use tempfile::TempDir;

use super::*;
use crate::hooks::test_util::{test_binding_id, test_source_id};

fn runner() -> CommandHookRunner {
    CommandHookRunner::new(Arc::new(|| None))
}

fn invocation(cwd: &Path, command: &str, stdin: &[u8]) -> HookInvocation {
    HookInvocation {
        binding: test_binding_id(test_source_id("hooks.json"), "tool.pre", 0),
        command: command.to_owned(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        stdin: stdin.to_vec(),
        timeout: Duration::from_secs(10),
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

fn run(invocation: HookInvocation) -> HookProcessOutcome {
    block_on(runner().run(invocation))
}

#[test]
fn a_powershell_hook_receives_the_payload_through_input() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("hook.ps1"),
        "[Console]::Out.Write(@($input)[0])\nexit 0\n",
    )
    .expect("write hook");

    let outcome = run(invocation(
        dir.path(),
        r".\hook.ps1",
        br#"{"command":"Get-ChildItem"}"#,
    ));

    assert_eq!(outcome.exit, ProcessExit::Code(0));
    assert_eq!(outcome.stdout, br#"{"command":"Get-ChildItem"}"#);
}

#[test]
fn a_native_program_exit_code_is_preserved() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("hook.cmd"), "@exit /b 2\r\n").expect("write hook");

    let outcome = run(invocation(dir.path(), r"& .\hook.cmd", b"{}"));

    assert_eq!(outcome.exit, ProcessExit::Code(2));
}
