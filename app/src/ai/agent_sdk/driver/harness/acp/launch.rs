use warp_cli::agent::Harness;

/// The process to run for a harness driven over the Agent Client Protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcpLaunchSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl AcpLaunchSpec {
    /// The ACP adapter shipped alongside each harness CLI, or `None` for harnesses that have no
    /// ACP transport.
    pub(crate) fn for_harness(harness: Harness) -> Option<Self> {
        let (program, args): (&str, &[&str]) = match harness {
            Harness::Claude => ("claude-code-acp", &[]),
            Harness::Codex => ("codex-acp", &[]),
            Harness::Gemini => ("gemini", &["--experimental-acp"]),
            Harness::Oz | Harness::OpenCode | Harness::Unknown => return None,
        };
        Some(Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        })
    }
}
