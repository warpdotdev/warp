use super::*;

/// Locks in [`Harness::config_name`] / [`Harness::from_config_name`] as a true inverse pair
/// for every variant that maps to a real, server-recognized harness. If a new variant is
/// added without a matching `from_config_name` arm, this round-trip test will fail.
#[test]
fn harness_config_name_round_trips_for_known_variants() {
    for harness in [
        Harness::Oz,
        Harness::Claude,
        Harness::OpenCode,
        Harness::Gemini,
        Harness::Codex,
        Harness::Pi,
    ] {
        assert_eq!(
            Harness::from_config_name(harness.config_name()),
            Some(harness),
            "round-trip failed for {harness:?}",
        );
    }
}

#[test]
fn harness_from_config_name_returns_none_for_unrecognized() {
    assert_eq!(Harness::from_config_name(""), None);
    assert_eq!(Harness::from_config_name("not-a-real-harness"), None);
}

#[derive(Debug, clap::Parser)]
struct RunArgsParser {
    #[command(flatten)]
    args: RunAgentArgs,
}

fn parse_run_args(args: &[&str]) -> RunAgentArgs {
    use clap::Parser as _;
    RunArgsParser::try_parse_from(std::iter::once("run").chain(args.iter().copied()))
        .expect("run args should parse")
        .args
}

#[test]
fn hook_flags_default_to_off() {
    let args = parse_run_args(&["--prompt", "hi"]);
    assert!(!args.hooks_strict);
    assert_eq!(args.hooks_on_ask, None);
    assert!(args.hook_dirs.is_empty());
}

#[test]
fn hook_flags_parse() {
    let args = parse_run_args(&[
        "--prompt",
        "hi",
        "--hooks-strict",
        "--hooks-on-ask",
        "fail-run",
        "--hook-dir",
        "/a",
        "--hook-dir",
        "rel",
    ]);
    assert!(args.hooks_strict);
    assert_eq!(args.hooks_on_ask, Some(HooksOnAsk::FailRun));
    assert_eq!(
        args.hook_dirs,
        vec![PathBuf::from("/a"), PathBuf::from("rel")]
    );
    let deny = parse_run_args(&["--prompt", "hi", "--hooks-on-ask", "deny"]);
    assert_eq!(deny.hooks_on_ask, Some(HooksOnAsk::Deny));
}

#[test]
fn harness_from_config_name_round_trips_unknown() {
    assert_eq!(
        Harness::from_config_name(Harness::Unknown.config_name()),
        Some(Harness::Unknown),
    );
}
