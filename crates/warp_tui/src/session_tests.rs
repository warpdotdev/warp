use clap::Parser;

use super::{TuiArgs, parse_resume_token};

#[test]
fn parses_resume_server_token() {
    let token = uuid::Uuid::new_v4().to_string();
    let args = TuiArgs::try_parse_from([
        "warp",
        "--resume",
        token.as_str(),
        "--api-key",
        "test-api-key",
    ])
    .expect("TUI launch arguments should parse together");

    assert_eq!(args.resume.as_deref(), Some(token.as_str()));
    assert_eq!(args.api_key.as_deref(), Some("test-api-key"));
    assert_eq!(
        parse_resume_token(token.clone())
            .expect("UUID token should validate")
            .as_str(),
        token
    );
}

#[test]
fn rejects_malformed_resume_server_token() {
    let error = parse_resume_token("not-a-token".to_owned())
        .expect_err("non-UUID token should be rejected");

    assert!(
        error
            .to_string()
            .contains("invalid server conversation token")
    );
}

#[test]
fn accepts_startup_without_resume() {
    let args = TuiArgs::try_parse_from(["warp"]).expect("empty arguments should parse");

    assert_eq!(args.resume, None);
    assert_eq!(args.api_key, None);
}

#[test]
fn version_flag_prints_cli_version() {
    let error = TuiArgs::try_parse_from(["warp", "--version"])
        .expect_err("--version should short-circuit clap parsing");

    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
    // `run()` prints only CLI_VERSION (no binary-name precursor). Clap's
    // DisplayVersion payload still contains the configured version string.
    assert!(
        error.to_string().contains(super::CLI_VERSION),
        "--version should be backed by the configured CLI version"
    );
}

#[test]
fn resume_shell_command_uses_channel_cli_names() {
    use warp_core::channel::Channel;

    let token = "00000000-0000-0000-0000-000000000000";
    let cases = [
        (
            Channel::Stable,
            "warp --resume 00000000-0000-0000-0000-000000000000",
        ),
        (
            Channel::Dev,
            "warp-dev --resume 00000000-0000-0000-0000-000000000000",
        ),
        (
            Channel::Local,
            "warp-dev --resume 00000000-0000-0000-0000-000000000000",
        ),
        (
            Channel::Preview,
            "warp-preview --resume 00000000-0000-0000-0000-000000000000",
        ),
        (
            Channel::Oss,
            "warp-oss --resume 00000000-0000-0000-0000-000000000000",
        ),
        (
            Channel::Integration,
            "warp-integration --resume 00000000-0000-0000-0000-000000000000",
        ),
    ];

    for (channel, expected) in cases {
        assert_eq!(super::tui_resume_shell_command(channel, token), expected);
    }
}
