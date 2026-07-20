use anyhow::{Result, anyhow};
use url::Url;

/// Actions that can be performed via the `warp://linear/...` deeplink.
#[derive(Debug, PartialEq, Eq)]
pub enum LinearAction {
    /// Open a new agent view tab to work on a Linear issue.
    WorkOnIssue,
}

impl LinearAction {
    pub fn parse(url: &Url) -> Result<Self> {
        match url.path() {
            "/work" => Ok(Self::WorkOnIssue),
            other => Err(anyhow!(
                "Received \"linear\" intent with unexpected path: {other}"
            )),
        }
    }
}

/// Arguments for the `WorkOnIssue` Linear deeplink action.
/// We may extend this with a branch, path, or other metadata.
#[derive(Debug, Clone)]
pub struct LinearIssueWork {
    /// Prompt provided by Linear for the issue to work on.
    pub prompt: Option<String>,
}

impl LinearIssueWork {
    pub fn from_url(url: &Url) -> Self {
        let prompt = url
            .query_pairs()
            .find(|(key, _)| key == "prompt")
            .map(|(_, value)| value.into_owned())
            .filter(|s| !s.is_empty());
        Self { prompt }
    }
}
