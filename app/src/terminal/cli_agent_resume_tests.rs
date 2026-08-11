use enum_iterator::all;

use super::*;

const VALID_ID: &str = "998add3f-6339-423a-9ea9-2fc6e2d3fe9b";

#[test]
fn resume_commands_are_exact_per_agent() {
    assert_eq!(
        resume_command(CLIAgent::Claude, VALID_ID).as_deref(),
        Some("claude --resume 998add3f-6339-423a-9ea9-2fc6e2d3fe9b")
    );
    assert_eq!(
        resume_command(CLIAgent::Codex, VALID_ID).as_deref(),
        Some("codex resume 998add3f-6339-423a-9ea9-2fc6e2d3fe9b")
    );
    assert_eq!(
        resume_command(CLIAgent::CursorCli, VALID_ID).as_deref(),
        Some("agent --resume 998add3f-6339-423a-9ea9-2fc6e2d3fe9b")
    );
    assert_eq!(
        resume_command(CLIAgent::OpenCode, VALID_ID).as_deref(),
        Some("opencode --session 998add3f-6339-423a-9ea9-2fc6e2d3fe9b")
    );
}

#[test]
fn unsupported_agents_return_none_not_a_guess() {
    for agent in [
        CLIAgent::Gemini,
        CLIAgent::Amp,
        CLIAgent::Droid,
        CLIAgent::Copilot,
        CLIAgent::Pi,
        CLIAgent::OhMyPi,
        CLIAgent::Auggie,
        CLIAgent::Goose,
        CLIAgent::Hermes,
        CLIAgent::Vibe,
        CLIAgent::Antigravity,
        CLIAgent::Unknown,
    ] {
        assert_eq!(resume_command(agent, VALID_ID), None, "{agent:?}");
    }
}

#[test]
fn cursor_never_emits_a_bare_resume() {
    // A bare `agent --resume` opens Cursor's own picker; an invalid id must
    // yield no command at all rather than a command without the id.
    assert_eq!(resume_command(CLIAgent::CursorCli, ""), None);
    assert_eq!(resume_command(CLIAgent::CursorCli, "not a uuid"), None);
}

#[test]
fn no_command_ever_carries_the_headless_driver_flags() {
    for agent in all::<CLIAgent>() {
        for cmd in [resume_command(agent, VALID_ID), continue_command(agent)]
            .into_iter()
            .flatten()
        {
            assert!(
                !cmd.contains("--dangerously-skip-permissions"),
                "{agent:?}: {cmd}"
            );
            assert!(!cmd.contains('<'), "{agent:?}: {cmd}");
        }
    }
}

#[test]
fn continue_commands_are_exact() {
    assert_eq!(
        continue_command(CLIAgent::Claude).as_deref(),
        Some("claude --continue")
    );
    assert_eq!(
        continue_command(CLIAgent::CursorCli).as_deref(),
        Some("agent --continue")
    );
    assert_eq!(
        continue_command(CLIAgent::OpenCode).as_deref(),
        Some("opencode --continue")
    );
    assert_eq!(continue_command(CLIAgent::Codex), None);
    assert_eq!(continue_command(CLIAgent::Unknown), None);
}

#[test]
fn injection_attempts_are_rejected_at_validation() {
    let attacks = [
        "abc; rm -rf ~",
        "abc && curl evil",
        "abc`id`",
        "abc$(id)",
        "abc | tee /etc/passwd",
        "abc'def",
        "abc\"def",
        "abc def",
        "abc\ndef",
        "abc\rdef",
        "abc\tdef",
        "abc\0def",
        "abc\u{2028}def",
        "ábc-non-ascii",
        "",
        // Over the 64-char bound.
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ];
    for attack in attacks {
        assert!(!is_valid_session_id(attack), "accepted: {attack:?}");
        assert_eq!(
            resume_command(CLIAgent::Claude, attack),
            None,
            "built a command for: {attack:?}"
        );
    }
}

#[test]
fn newline_specifically_can_never_reach_a_prefill() {
    // Prefill is not self-securing: a newline inside a prefilled line would
    // submit it on arrival. This is the single most important rejection.
    for id in ["a\nb", "\n", "a\n", "\nrm -rf ~"] {
        assert_eq!(resume_command(CLIAgent::Claude, id), None);
    }
}

#[test]
fn valid_ids_accept_the_shapes_agents_actually_produce() {
    // Claude/Codex/Cursor UUIDs, and underscore/short tokens.
    for id in [
        VALID_ID,
        "61f785ca-1c31-4671-a420-f89c47875750",
        "abc_123",
        "A",
    ] {
        assert!(is_valid_session_id(id), "rejected: {id:?}");
    }
}
