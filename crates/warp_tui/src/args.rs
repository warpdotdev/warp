use anyhow::Result;
use warp::tui_export::ServerConversationToken;

/// Arguments accepted by the TUI frontend after worker dispatch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TuiArgs {
    pub(super) prompt: Option<String>,
    pub(super) server_conversation_token: Option<ServerConversationToken>,
}

impl TuiArgs {
    /// Parses TUI frontend arguments from the current process environment.
    pub(crate) fn from_env() -> Result<Self> {
        Self::parse(std::env::args().skip(1))
    }

    /// Parses TUI frontend arguments.
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--prompt" => {
                    parsed.prompt = Some(
                        args.next()
                            .ok_or_else(|| anyhow::anyhow!("--prompt requires a value"))?,
                    );
                }
                "--conversation-id" => {
                    let server_conversation_token = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--conversation-id requires a value"))?;
                    parsed.server_conversation_token =
                        Some(ServerConversationToken::new(server_conversation_token));
                }
                other => return Err(anyhow::anyhow!("Unknown argument: {other}")),
            }
        }
        Ok(parsed)
    }
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
