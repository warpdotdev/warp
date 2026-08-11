use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use enum_iterator::all;

use super::*;

const SESSION_ID: &str = "8f0b1c2d-3e4f-5061-7283-94a5b6c7d8e9";

fn declarations() -> &'static ResumeDeclarations {
    ResumeDeclarations::embedded()
}

fn flag(name: &str, value: Option<&str>) -> RecordedFlag {
    RecordedFlag {
        name: name.to_owned(),
        value: value.map(str::to_owned),
    }
}

fn claude_command(flags: &[RecordedFlag]) -> String {
    declarations()
        .build_resume_command(
            CLIAgent::Claude,
            SESSION_ID,
            flags,
            PermissionPosture::Carry,
        )
        .expect("Claude is declared and the identifier is well formed")
}

#[test]
fn embedded_declarations_parse_and_name_only_known_agents() {
    let declarations = declarations();
    let declared: Vec<CLIAgent> = all::<CLIAgent>()
        .filter(|agent| declarations.supports(*agent))
        .collect();

    assert!(
        declared.contains(&CLIAgent::Claude),
        "expected Claude to be declared, got {declared:?}"
    );
    assert!(
        declared.contains(&CLIAgent::Codex),
        "expected Codex to be declared, got {declared:?}"
    );
    assert!(
        !declared.contains(&CLIAgent::Unknown),
        "Unknown has no binary and must never be declared"
    );
    for agent in &declared {
        assert!(
            !agent.command_prefix().is_empty(),
            "{agent:?} is declared but has no command prefix to resume with"
        );
    }
}

#[test]
fn agents_without_a_verified_resume_are_undeclared() {
    for agent in [CLIAgent::Gemini, CLIAgent::WarpTui] {
        assert!(
            !declarations().supports(agent),
            "{agent:?} must stay undeclared"
        );
        assert_eq!(
            declarations().build_resume_command(agent, SESSION_ID, &[], PermissionPosture::Carry),
            None,
            "{agent:?} must not build any invocation"
        );
    }
}

#[test]
fn unknown_agent_name_is_rejected() {
    let contents = r#"
[agents.Claud]
resume = { form = "flag", flag = "--resume" }
identifier = { shape = "bare_token", max_length = 128 }
"#;

    assert!(
        ResumeDeclarations::parse(contents).is_err(),
        "a misspelled agent name must be rejected, not silently ignored"
    );
}

#[test]
fn agent_without_a_binary_is_rejected() {
    let contents = r#"
[agents.Unknown]
resume = { form = "flag", flag = "--resume" }
identifier = { shape = "bare_token", max_length = 128 }
"#;

    assert!(
        ResumeDeclarations::parse(contents).is_err(),
        "an agent with no command prefix has nothing to resume with"
    );
}

#[test]
fn malformed_resume_shape_is_rejected() {
    let identifier = r#"identifier = { shape = "bare_token", max_length = 128 }"#;
    let malformed = [
        // Unknown invocation form.
        r#"resume = { form = "environment_variable", name = "SESSION" }"#,
        // Flag form without the flag it must pass.
        r#"resume = { form = "flag" }"#,
        // Subcommand form without the subcommand it must run.
        r#"resume = { form = "subcommand" }"#,
        // Flag form carrying a subcommand it would never use.
        r#"resume = { form = "flag", flag = "--resume", subcommand = "resume" }"#,
        // A flag that is not a flag.
        r#"resume = { form = "flag", flag = "resume" }"#,
        // An invocation fragment that would reach the shell unquoted.
        r#"resume = { form = "subcommand", subcommand = "resume; rm -rf /" }"#,
    ];

    for resume in malformed {
        let contents = format!("[agents.Claude]\n{resume}\n{identifier}\n");
        assert!(
            ResumeDeclarations::parse(&contents).is_err(),
            "expected rejection of malformed invocation: {resume}"
        );
    }
}

#[test]
fn malformed_value_declaration_is_rejected() {
    let resume = r#"resume = { form = "flag", flag = "--resume" }"#;
    let identifier = r#"identifier = { shape = "bare_token", max_length = 128 }"#;
    let malformed = [
        // A value shape with no length bound.
        format!("[agents.Claude]\n{resume}\nidentifier = {{ shape = \"bare_token\" }}\n"),
        // An identifier that carries no value at all.
        format!("[agents.Claude]\n{resume}\nidentifier = {{ shape = \"boolean\" }}\n"),
        // A boolean flag with a length bound it can never use.
        format!(
            "[agents.Claude]\n{resume}\n{identifier}\n[agents.Claude.flags]\n\
             \"--strict-mcp-config\" = {{ shape = \"boolean\", max_length = 8 }}\n"
        ),
        // A flag whose value shape has no length bound.
        format!(
            "[agents.Claude]\n{resume}\n{identifier}\n[agents.Claude.flags]\n\
             \"--model\" = {{ shape = \"bare_token\" }}\n"
        ),
        // An allowlist entry that is not a flag.
        format!(
            "[agents.Claude]\n{resume}\n{identifier}\n[agents.Claude.flags]\n\
             \"model\" = {{ shape = \"bare_token\", max_length = 8 }}\n"
        ),
        // An unknown value shape.
        format!(
            "[agents.Claude]\n{resume}\n{identifier}\n[agents.Claude.flags]\n\
             \"--model\" = {{ shape = \"anything\", max_length = 8 }}\n"
        ),
        // An alias for the session identifier, which is never spelled as a flag.
        format!(
            "[agents.Claude]\n{resume}\n\
             identifier = {{ shape = \"bare_token\", max_length = 128, aliases = [\"--id\"] }}\n"
        ),
        // An alias that is not a flag spelling.
        format!(
            "[agents.Claude]\n{resume}\n{identifier}\n[agents.Claude.flags]\n\
             \"--model\" = {{ shape = \"bare_token\", max_length = 8, aliases = [\"model\"] }}\n"
        ),
        // A key the declaration format does not define.
        format!("[agents.Claude]\n{resume}\n{identifier}\nexecutable = \"claude\"\n"),
    ];

    for contents in malformed {
        assert!(
            ResumeDeclarations::parse(&contents).is_err(),
            "expected rejection of malformed declaration:\n{contents}"
        );
    }
}

#[test]
fn flag_form_agent_builds_a_resume_flag_invocation() {
    assert_eq!(
        claude_command(&[]),
        format!("claude --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}")
    );
}

#[test]
fn subcommand_form_agent_builds_a_resume_subcommand_invocation() {
    assert_eq!(
        declarations().build_resume_command(
            CLIAgent::Codex,
            SESSION_ID,
            &[],
            PermissionPosture::Carry
        ),
        Some(format!(
            "codex resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        ))
    );
}

/// AE5: nothing beyond the resume pointer when nothing was recorded.
#[test]
fn empty_recorded_set_carries_no_flags() {
    let command = claude_command(&[]);
    let flag_count = command
        .split_whitespace()
        .filter(|token| token.starts_with("--"))
        .count();

    assert_eq!(flag_count, 1, "expected only the resume flag in {command}");
}

/// AE6: the permission posture rides along when, and only when, it was recorded.
#[test]
fn recorded_permission_bypass_flag_is_carried_verbatim() {
    let command = claude_command(&[flag("--dangerously-skip-permissions", None)]);

    assert_eq!(
        command,
        format!(
            "claude --dangerously-skip-permissions --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        )
    );
}

#[test]
fn builder_adds_no_flag_of_its_own() {
    let command = claude_command(&[]);
    let codex_command = declarations()
        .build_resume_command(CLIAgent::Codex, SESSION_ID, &[], PermissionPosture::Carry)
        .expect("Codex is declared");

    for unwanted in [
        "--dangerously-skip-permissions",
        "--dangerously-bypass-approvals-and-sandbox",
        "--dangerously-bypass-hook-trust",
        "--fork-session",
        "--session-id",
        "--permission-mode",
        "--model",
    ] {
        assert!(
            !command.contains(unwanted),
            "{command} must not contain {unwanted}"
        );
        assert!(
            !codex_command.contains(unwanted),
            "{codex_command} must not contain {unwanted}"
        );
    }
}

/// AE18: a hostile stored value takes the whole flag down with it and leaves no
/// trace in the built string.
#[test]
fn unsafe_values_drop_the_flag_without_leaking_a_fragment() {
    let bare = claude_command(&[]);
    let hostile = [
        "sonnet;pwn",
        "sonnet$(pwn)",
        "sonnet`pwn`",
        "sonnet&&pwn",
        "sonnet|pwn",
        "sonnet*pwn",
        "sonnet\npwn",
        "sonnet'pwn",
        "sonnet pwn",
        "sonnet\"pwn\"",
        "sonnet>pwn",
        "sonnet<pwn",
        "sonnet{pwn}",
        "$(pwn)",
        "~/pwn",
        "sonnet\\pwn",
        "-pwn",
        "--pwn",
        "sonnet\u{0}pwn",
        "sonnet\u{7}pwn",
    ];

    for value in hostile {
        let command = claude_command(&[flag("--model", Some(value))]);

        assert_eq!(command, bare, "value {value:?} must leave no trace");
        assert!(
            !command.contains("pwn"),
            "value {value:?} leaked into {command}"
        );
    }
}

/// AE18: losing every flag still leaves a usable resume.
#[test]
fn dropping_every_flag_still_builds_a_bare_resume() {
    let command = claude_command(&[
        flag("--model", Some("sonnet;pwn")),
        flag("--permission-mode", Some("plan pwn")),
        flag("--agent", Some("$(pwn)")),
    ]);

    assert_eq!(
        command,
        format!("claude --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}")
    );
}

/// AE19: a resume pointer that fails its own shape is not recoverable.
#[test]
fn unusable_identifier_yields_no_command() {
    let unusable = [
        "",
        "abc; rm -rf /",
        "abc'pwn",
        "$(pwn)",
        "-abc",
        "abc pwn",
        "abc\npwn",
        &"a".repeat(129),
    ];

    for identifier in unusable {
        assert_eq!(
            declarations().build_resume_command(
                CLIAgent::Claude,
                identifier,
                &[],
                PermissionPosture::Carry
            ),
            None,
            "identifier {identifier:?} must yield no command at all"
        );
        assert_eq!(
            declarations().build_resume_command(
                CLIAgent::Claude,
                identifier,
                &[flag("--dangerously-skip-permissions", None)],
                PermissionPosture::Carry
            ),
            None,
            "identifier {identifier:?} must yield no command even with valid flags"
        );
    }
}

#[test]
fn overlong_value_is_dropped() {
    let within_bound = "a".repeat(32);
    let over_bound = "a".repeat(33);

    assert!(
        claude_command(&[flag("--permission-mode", Some(&within_bound))])
            .contains(&format!("--permission-mode '{within_bound}'")),
        "a value at the declared bound must survive"
    );
    assert_eq!(
        claude_command(&[flag("--permission-mode", Some(&over_bound))]),
        claude_command(&[]),
        "a value past the declared bound must be dropped"
    );
}

#[test]
fn surviving_values_are_shell_quoted() {
    let command = claude_command(&[
        flag("--model", Some("claude-opus-4-5")),
        flag("--permission-mode", Some("plan")),
    ]);

    assert_eq!(
        command,
        format!(
            "claude --model 'claude-opus-4-5' --permission-mode 'plan' --resume '{SESSION_ID}' \
             # {RESUME_HISTORY_MARKER}"
        )
    );
}

#[test]
fn flag_outside_the_allowlist_is_dropped_at_build_time() {
    let command = claude_command(&[
        flag("--fork-session", None),
        flag("--session-id", Some("11111111-2222-3333-4444-555555555555")),
        flag("--retired-flag", Some("value")),
        flag("--model", Some("sonnet")),
    ]);

    assert_eq!(
        command,
        format!("claude --model 'sonnet' --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}")
    );
}

#[test]
fn boolean_flag_recorded_with_a_value_is_dropped() {
    assert_eq!(
        claude_command(&[flag("--dangerously-skip-permissions", Some("true"))]),
        claude_command(&[]),
        "a boolean flag carrying a value did not come from the declared shape"
    );
}

#[test]
fn value_flag_recorded_without_a_value_is_dropped() {
    assert_eq!(
        claude_command(&[flag("--model", None)]),
        claude_command(&[]),
        "a value flag with nothing to pass is not a flag we can rebuild"
    );
}

#[test]
fn built_invocation_carries_the_history_marker() {
    let command = claude_command(&[flag("--model", Some("sonnet"))]);

    assert_eq!(RESUME_HISTORY_MARKER, "warp_resume_agent_session");
    assert!(
        command.ends_with(&format!(" # {RESUME_HISTORY_MARKER}")),
        "{command} must end with the shell-history marker comment"
    );
}

/// R18: the marker suppresses nothing on its own — it suppresses where a bootstrap script matches
/// it. Pinning the constant without reading the scripts is what let a shell ship with the resume
/// landing in the user's history file, so this asks the scripts themselves.
///
/// The question is per shell, not per script: a script that can be told to omit Warp's own in-band
/// generator from history can be told to omit the resume too, and one that cannot needs the other
/// mechanism instead.
#[test]
fn every_shell_that_suppresses_warps_own_commands_suppresses_the_resume() {
    const BASH: &str = include_str!("../../assets/bundled/bootstrap/bash_body.sh");
    const ZSH: &str = include_str!("../../assets/bundled/bootstrap/zsh_body.sh");
    const PWSH: &str = include_str!("../../assets/bundled/bootstrap/pwsh.ps1");
    const FISH: &str = include_str!("../../assets/bundled/bootstrap/fish.sh");

    // Each script paired with how it spells the in-band generator command it already keeps out of
    // history, which is the evidence that this shell takes history patterns at all.
    for (script, contents, in_band_command) in [
        ("bash_body.sh", BASH, "warp_run_generator_command"),
        ("zsh_body.sh", ZSH, "warp_run_generator_command"),
        ("pwsh.ps1", PWSH, "Warp-Run-GeneratorCommand"),
    ] {
        assert!(
            contents.contains(in_band_command),
            "precondition: {script} filters {in_band_command} out of history"
        );
        assert!(
            contents.contains(RESUME_HISTORY_MARKER),
            "{script} filters {in_band_command} but not the resume marker, so a resume run under \
             this shell lands in the user's history file"
        );
    }

    // Fish takes no history patterns at all, which is why it matches no marker. Its mechanism is
    // stated here rather than left as an exemption, so removing it fails this test.
    assert!(
        FISH.contains("warp_run_generator_command"),
        "precondition: fish runs the same in-band generator command"
    );
    assert!(
        !FISH.contains(RESUME_HISTORY_MARKER),
        "fish gained a marker filter; the leading space this asserts below may now be redundant"
    );
    let command = claude_command(&[]);
    assert!(
        history_suppressed_resume_command(Some(ShellType::Fish), command.clone()).starts_with(' '),
        "fish omits leading-space commands from history, and that is all it offers"
    );
    for shell in [ShellType::Bash, ShellType::Zsh, ShellType::PowerShell] {
        assert_eq!(
            history_suppressed_resume_command(Some(shell), command.clone()),
            command,
            "{shell:?} matches the marker and needs nothing added to the line"
        );
    }
    assert_eq!(
        history_suppressed_resume_command(None, command.clone()),
        command,
        "a shell Warp cannot identify keeps the line it built"
    );
}

// The undeclared flag that carries a value sits last on purpose: its value is indistinguishable
// from a positional, and scanning stops at the first of those.
#[test]
fn extractor_keeps_only_allowlisted_flags() {
    let recorded = declarations().extract_resume_flags(
        CLIAgent::Claude,
        &[
            "--model",
            "sonnet",
            "--fork-session",
            "--dangerously-skip-permissions",
            "--session-id",
            "11111111-2222-3333-4444-555555555555",
            "write me a test",
        ],
    );

    assert_eq!(
        recorded,
        vec![
            flag("--model", Some("sonnet")),
            flag("--dangerously-skip-permissions", None),
        ]
    );
}

/// A prompt positional ends the flags, so the words after it are the user's text rather than
/// choices they made. Both barriers are needed: the caller tokenizes so the prompt arrives as one
/// word, and this stops at it so an agent that took a bare positional cannot leak flags either.
#[test]
fn extractor_stops_at_the_first_positional() {
    assert_eq!(
        declarations().extract_resume_flags(
            CLIAgent::Claude,
            &[
                "--model",
                "opus",
                "fix the build",
                "--dangerously-skip-permissions",
            ],
        ),
        vec![flag("--model", Some("opus"))]
    );
}

/// `--` is the same statement made explicitly, and an agent that honors it would read what
/// follows as text however flag-shaped it is.
#[test]
fn extractor_stops_at_an_end_of_flags_marker() {
    assert_eq!(
        declarations().extract_resume_flags(
            CLIAgent::Claude,
            &["--model", "opus", "--", "--dangerously-skip-permissions"],
        ),
        vec![flag("--model", Some("opus"))]
    );
}

#[test]
fn extractor_normalizes_the_equals_form() {
    let recorded = declarations()
        .extract_resume_flags(CLIAgent::Claude, &["--model=sonnet", "--agent=tester"]);

    assert_eq!(
        recorded,
        vec![
            flag("--model", Some("sonnet")),
            flag("--agent", Some("tester")),
        ]
    );
}

#[test]
fn extractor_records_hostile_values_for_the_builder_to_reject() {
    let recorded =
        declarations().extract_resume_flags(CLIAgent::Claude, &["--model", "sonnet;pwn"]);

    assert_eq!(
        recorded,
        vec![flag("--model", Some("sonnet;pwn"))],
        "capture records what it saw; the builder is what validates it"
    );
    assert_eq!(claude_command(&recorded), claude_command(&[]));
}

#[test]
fn extractor_resolves_declared_aliases_to_the_canonical_flag() {
    let contents = r#"
[agents.Claude]
resume = { form = "flag", flag = "--resume" }
identifier = { shape = "bare_token", max_length = 128 }

[agents.Claude.flags]
"--permission-mode" = { shape = "bare_token", max_length = 32, aliases = ["--permissionMode"] }
"#;
    let declarations = ResumeDeclarations::parse(contents).expect("declaration is well formed");

    assert_eq!(
        declarations.extract_resume_flags(CLIAgent::Claude, &["--permissionMode", "plan"]),
        vec![flag("--permission-mode", Some("plan"))]
    );
    assert_eq!(
        declarations.build_resume_command(
            CLIAgent::Claude,
            SESSION_ID,
            &[flag("--permissionMode", Some("plan"))],
            PermissionPosture::Carry
        ),
        Some(format!(
            "claude --permission-mode 'plan' --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        )),
        "an alias recorded earlier still resolves when the invocation is built"
    );
}

#[test]
fn extractor_yields_nothing_for_an_undeclared_agent() {
    assert!(
        declarations()
            .extract_resume_flags(CLIAgent::Gemini, &["--model", "gemini-3-pro"])
            .is_empty()
    );
}

/// The recording timestamps the window is measured against, expressed as an age.
fn observed_hours_ago(hours: i64) -> (NaiveDateTime, NaiveDateTime) {
    let now = NaiveDate::from_ymd_opt(2026, 8, 11)
        .expect("date should be valid")
        .and_hms_opt(21, 0, 0)
        .expect("time should be valid");
    (now - TimeDelta::hours(hours), now)
}

/// AE17: state carrying a permission-bypass flag, last observed outside the freshness window,
/// resumes the same conversation without the bypass.
#[test]
fn stale_recording_resumes_the_same_session_without_its_permission_posture() {
    let (observed_at, now) = observed_hours_ago(13);
    let posture = PermissionPosture::for_observation(observed_at, now);

    let command = declarations()
        .build_resume_command(
            CLIAgent::Claude,
            SESSION_ID,
            &[
                flag("--dangerously-skip-permissions", None),
                flag("--model", Some("sonnet")),
            ],
            posture,
        )
        .expect("a stale posture must still produce a resume");

    assert_eq!(
        command,
        format!("claude --model 'sonnet' --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"),
        "the pane still resumes its conversation, just without the elevation"
    );
}

/// AE6: the same recording, observed inside the window, keeps the posture the user chose.
#[test]
fn fresh_recording_keeps_its_permission_posture() {
    let (observed_at, now) = observed_hours_ago(11);
    let posture = PermissionPosture::for_observation(observed_at, now);

    assert_eq!(posture, PermissionPosture::Carry);
    assert_eq!(
        declarations().build_resume_command(
            CLIAgent::Claude,
            SESSION_ID,
            &[flag("--dangerously-skip-permissions", None)],
            posture,
        ),
        Some(format!(
            "claude --dangerously-skip-permissions --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        ))
    );
}

/// Both sides of the boundary itself, so the window is a real bound rather than a rounding.
#[test]
fn freshness_window_is_bounded_at_twelve_hours() {
    assert_eq!(
        PERMISSION_POSTURE_FRESHNESS,
        Duration::from_secs(12 * 60 * 60)
    );

    let (at_the_bound, now) = observed_hours_ago(12);
    assert_eq!(
        PermissionPosture::for_observation(at_the_bound, now),
        PermissionPosture::Carry,
        "state observed exactly at the bound is still inside the window"
    );
    assert_eq!(
        PermissionPosture::for_observation(at_the_bound - TimeDelta::seconds(1), now),
        PermissionPosture::Drop,
        "one second past the bound is outside it"
    );
}

/// A clock that moved backwards cannot vouch for an age, and an unverifiable age is not a fresh
/// one. Otherwise a stale recording could be revived by moving the machine clock back.
#[test]
fn recording_from_the_future_is_not_treated_as_fresh() {
    let (_, now) = observed_hours_ago(0);

    assert_eq!(
        PermissionPosture::for_observation(now + TimeDelta::hours(1), now),
        PermissionPosture::Drop
    );
}

/// Posture is a property of the flag, not of whether it carries a value: `--permission-mode`
/// chooses a posture just as much as the bypass switch does.
#[test]
fn stale_recording_drops_a_valued_permission_posture_flag() {
    let (observed_at, now) = observed_hours_ago(13);
    let posture = PermissionPosture::for_observation(observed_at, now);

    assert_eq!(
        declarations().build_resume_command(
            CLIAgent::Claude,
            SESSION_ID,
            &[flag("--permission-mode", Some("bypassPermissions"))],
            posture,
        ),
        Some(format!(
            "claude --resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        ))
    );
}

#[test]
fn stale_recording_drops_the_codex_bypass_flag() {
    let (observed_at, now) = observed_hours_ago(24);
    let posture = PermissionPosture::for_observation(observed_at, now);

    assert_eq!(
        declarations().build_resume_command(
            CLIAgent::Codex,
            SESSION_ID,
            &[flag("--dangerously-bypass-approvals-and-sandbox", None)],
            posture,
        ),
        Some(format!(
            "codex resume '{SESSION_ID}' # {RESUME_HISTORY_MARKER}"
        ))
    );
}

/// The permission-posture set is spelled out here so that adding a flag to the declaration file
/// has to be a deliberate R22 decision rather than a silent one: a new posture flag fails this
/// test until it is acknowledged, and un-marking an existing one fails it too.
#[test]
fn declared_permission_posture_flags_are_exactly_the_acknowledged_ones() {
    let mut claude = declarations().permission_posture_flags(CLIAgent::Claude);
    claude.sort_unstable();
    assert_eq!(
        claude,
        vec!["--dangerously-skip-permissions", "--permission-mode"]
    );

    assert_eq!(
        declarations().permission_posture_flags(CLIAgent::Codex),
        vec!["--dangerously-bypass-approvals-and-sandbox"]
    );
}
