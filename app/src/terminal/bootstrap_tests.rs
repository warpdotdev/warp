use std::os::unix::fs::PermissionsExt;
use std::{fs, process};

use command::blocking::Command;

const BASH_BODY: &str = include_str!("../../assets/bundled/bootstrap/bash_body.sh");

#[test]
fn warp_precmd_does_not_expand_ps1_when_honoring_it() {
    if !bash_supports_prompt_expansion() {
        return;
    }
    let output = run_warp_precmd(
        r#"
counter=0
PS1='$((++counter))'
WARP_HONOR_PS1=1
WARP_PS1_EXPANSION_SUPPORTED=1
warp_precmd
printf '%s\n' "$counter"
"#,
    );

    assert_eq!(output, "0\n");
}

#[test]
fn warp_precmd_does_not_use_child_shell_fallback_when_honoring_ps1() {
    let marker_file =
        std::env::temp_dir().join(format!("warp-honor-ps1-fallback-marker-{}", process::id()));
    let fallback_shell =
        std::env::temp_dir().join(format!("warp-honor-ps1-fallback-shell-{}", process::id()));
    _ = fs::remove_file(&marker_file);
    fs::write(
        &fallback_shell,
        "#!/bin/sh\nprintf invoked > \"$WARP_PS1_TEST_MARKER_FILE\"\nprintf '\\nexpanded\\n'\n",
    )
    .expect("fallback shell should be writable");
    fs::set_permissions(&fallback_shell, fs::Permissions::from_mode(0o755))
        .expect("fallback shell should be executable");
    let output = run_warp_precmd(&format!(
        r#"
export WARP_PS1_TEST_MARKER_FILE="{}"
BASH="{}"
PS1='prompt'
WARP_HONOR_PS1=1
WARP_PS1_EXPANSION_SUPPORTED=0
warp_precmd
printf '%s\n' "$last_message"
"#,
        marker_file.display(),
        fallback_shell.display()
    ));
    let fallback_was_invoked = marker_file.exists();
    _ = fs::remove_file(&marker_file);
    _ = fs::remove_file(&fallback_shell);

    assert!(output.contains(r#""ps1": """#));
    assert!(!fallback_was_invoked);
}

#[test]
fn warp_precmd_preserves_prompt_preview_metadata_when_not_honoring_ps1() {
    if !bash_supports_prompt_expansion() {
        return;
    }
    let output = run_warp_precmd(
        r#"
counter=0
PS1='preview:$((++counter))'
WARP_HONOR_PS1=0
WARP_PS1_EXPANSION_SUPPORTED=1
warp_precmd
printf '%s\n' "$counter"
printf '%s\n' "$last_message"
"#,
    );

    assert!(output.starts_with("1\n"));
    assert!(output.contains(r#""ps1": "preview:1""#));
    assert!(output.contains(r#""honor_ps1": false"#));
}

fn bash_supports_prompt_expansion() -> bool {
    Command::new("bash")
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "(( BASH_VERSINFO[0] > 4 || BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4 ))",
        ])
        .status()
        .expect("bash should be available")
        .success()
}

fn run_warp_precmd(test_body: &str) -> String {
    let function_start = BASH_BODY
        .find("    warp_precmd () {\n")
        .expect("warp_precmd should exist");
    let function_end = BASH_BODY[function_start..]
        .find("\n    }\n\n    warp_clear_on_next_block")
        .expect("warp_precmd should have a closing brace")
        + function_start
        + "\n    }".len();
    let warp_precmd = &BASH_BODY[function_start..function_end];
    let harness = format!(
        r#"
{warp_precmd}
warp_send_json_message() {{ last_message="$1"; }}
warp_maybe_send_reset_grid_osc() {{ :; }}
warp_input_reporting_supported() {{ printf 1; }}
warp_ps1_expanding_supported() {{ printf 1; }}
warp_escape_ps1() {{ printf '%s' "$1"; }}
warp_escape_json() {{ printf '%s' "$1"; }}
history() {{ :; }}
bind() {{ :; }}
WARP_SESSION_ID=1
WARP_IN_MSYS2=false
WARP_INPUT_REPORTING_SUPPORTED=1
WARP_BOOTSTRAPPED=
block_id=0
{test_body}
"#
    );
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &harness])
        .output()
        .expect("bash should be available");

    assert!(
        output.status.success(),
        "bash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("bash output should be UTF-8")
}
