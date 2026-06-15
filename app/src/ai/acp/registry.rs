use std::path::PathBuf;

use warp_cli::agent::Harness;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalAcpAgentSpec {
    pub(crate) harness: Harness,
    pub(crate) command: &'static str,
    pub(crate) args: &'static [&'static str],
    pub(crate) install_url: &'static str,
    pub(crate) default_models: &'static [&'static str],
    pub(crate) supports_resume: bool,
}

pub(crate) fn agent_specs() -> &'static [LocalAcpAgentSpec] {
    &[
        LocalAcpAgentSpec {
            harness: Harness::Claude,
            command: "claude-agent-acp",
            args: &[],
            install_url: "https://docs.anthropic.com/en/docs/claude-code",
            default_models: &[],
            supports_resume: true,
        },
        LocalAcpAgentSpec {
            harness: Harness::Codex,
            command: "codex-acp",
            args: &[],
            install_url: "https://github.com/zed-industries/codex-acp",
            default_models: &[],
            supports_resume: true,
        },
        LocalAcpAgentSpec {
            harness: Harness::Gemini,
            command: "gemini",
            args: &["--acp"],
            install_url: "https://geminicli.com/docs/cli/acp-mode/",
            // Gemini exposes models via `unstable_setSessionModel`, not config_options.
            default_models: &[
                "gemini-2.5-pro",
                "gemini-2.5-flash",
                "gemini-2.5-flash-lite",
            ],
            supports_resume: false,
        },
        LocalAcpAgentSpec {
            harness: Harness::Cursor,
            command: "cursor-acp",
            args: &[],
            install_url: "https://github.com/raphaelluethy/cursor-acp",
            default_models: &[],
            supports_resume: true,
        },
        LocalAcpAgentSpec {
            harness: Harness::Devin,
            command: "devin",
            args: &["acp"],
            install_url: "https://docs.devin.ai/cli/reference/commands#devin-acp",
            default_models: &[],
            supports_resume: true,
        },
    ]
}

pub(crate) fn spec_for_harness(harness: Harness) -> Option<&'static LocalAcpAgentSpec> {
    agent_specs().iter().find(|spec| spec.harness == harness)
}

pub(crate) fn is_local_acp_harness(harness: Harness) -> bool {
    spec_for_harness(harness).is_some()
}

pub(crate) fn command_for_harness(harness: Harness) -> Option<(PathBuf, Vec<String>)> {
    let spec = spec_for_harness(harness)?;
    Some((
        PathBuf::from(spec.command),
        spec.args.iter().map(|arg| (*arg).to_string()).collect(),
    ))
}

pub(crate) fn should_auto_authenticate(harness: Harness) -> bool {
    match harness {
        // Devin's ACP server advertises browser auth methods even when `devin acp`
        // is already usable from an authenticated CLI. Calling authenticate() here
        // opens a browser window from Warp and breaks the normal local CLI path.
        Harness::Devin => false,
        _ => true,
    }
}

pub(crate) fn default_session_mode(harness: Harness) -> Option<&'static str> {
    match harness {
        // cursor-acp uses a dedicated "yolo" session mode that auto-approves tools.
        Harness::Cursor => Some("yolo"),
        _ => None,
    }
}

pub(crate) fn process_env_for_harness(harness: Harness) -> &'static [(&'static str, &'static str)] {
    match harness {
        Harness::Cursor => &[("CURSOR_ACP_DEFAULT_MODE", "yolo")],
        _ => &[],
    }
}
