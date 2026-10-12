use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::PathBuf;

#[cfg(any(test, feature = "local_fs"))]
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutBatch {
    pub working_dir: PathBuf,
    pub repositories: Vec<CheckoutRequest>,
}

pub(super) fn sanitize_git_author_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_alphanumeric()
                || matches!(character, ' ' | '.' | '_' | '@' | '+' | '-' | '\'')
        }))
    .then(|| value.to_owned())
}

pub(super) fn sanitize_git_credential_username(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, '.' | '_' | '@' | '+' | '-')
        }))
    .then(|| value.to_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutRequest {
    pub source: RepositoryIdentity,
    pub checkout_name: String,
    #[serde(deserialize_with = "deserialize_head")]
    pub head: Option<RepositoryHeadRef>,
    pub fetch_branch_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckoutFailureKind {
    Clone,
    Checkout,
    RemoveOrigin,
}

/// What became of a single checkout request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutOutcome {
    /// Position of the request in the batch this outcome belongs to.
    pub request_index: usize,
    /// Why the checkout did not complete, absent when it succeeded.
    pub failure: Option<CheckoutFailureKind>,
    /// Redacted, size-bounded git output collected while the checkout ran.
    pub diagnostics: String,
    /// Wall-clock time the checkout took.
    pub duration_ms: u64,
    /// Commit the checkout ended on, absent when it was not resolved.
    pub resolved_head: Option<String>,
}

/// Everything the checkout helper reports back about the batch it ran.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutReport {
    pub outcomes: Vec<CheckoutOutcome>,
    pub identity_diagnostics: Option<CloneFailureIdentityDiagnostics>,
}

impl CheckoutReport {
    /// The outcomes of the checkouts that did not succeed.
    pub fn failures(&self) -> impl Iterator<Item = &CheckoutOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.failure.is_some())
    }
}

/// Resolved HEAD commit of each checkout, keyed by checkout name. Checkouts whose HEAD could not
/// be resolved are absent.
pub type ResolvedHeads = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneFailureCredentialIdentity {
    pub host: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneFailureIdentityDiagnostics {
    pub author: Option<String>,
    pub credentials: Vec<CloneFailureCredentialIdentity>,
}

impl fmt::Display for CloneFailureIdentityDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let author = self.author.as_deref().unwrap_or("unset");
        write!(formatter, "\nGit identity diagnostics:\n  Author: {author}")?;
        for credential in &self.credentials {
            let username = credential.username.as_deref().unwrap_or("unavailable");
            write!(
                formatter,
                "\n  Credential username for {}: {username}",
                credential.host
            )?;
        }
        Ok(())
    }
}

pub(super) fn parse_resolved_head_sha(line: &str) -> Option<String> {
    let sha = line.trim();
    is_valid_git_object_id(sha).then(|| sha.to_owned())
}

pub(super) fn is_valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn deserialize_head<'de, D>(deserializer: D) -> Result<Option<RepositoryHeadRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(
        tag = "type",
        content = "value",
        rename_all = "SCREAMING_SNAKE_CASE",
        deny_unknown_fields
    )]
    enum Head {
        CommitSha(String),
        Branch(String),
    }
    Option::<Head>::deserialize(deserializer).map(|head| {
        head.map(|head| match head {
            Head::CommitSha(value) => RepositoryHeadRef::CommitSha(value),
            Head::Branch(value) => RepositoryHeadRef::Branch(value),
        })
    })
}

impl CheckoutBatch {
    #[cfg(any(test, feature = "local_fs"))]
    pub fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        let batch: Self = serde_json::from_slice(bytes).context("invalid checkout JSON")?;
        batch.validate().map_err(anyhow::Error::msg)?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.working_dir.is_absolute() {
            return Err("checkout working directory must be absolute");
        }
        let mut targets = HashSet::new();
        for request in &self.repositories {
            request.validate()?;
            if !targets.insert(request.checkout_name.to_lowercase()) {
                return Err("duplicate checkout target");
            }
        }
        Ok(())
    }
}

impl CheckoutRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let owner = &self.source.repo_owner;
        let repo = &self.source.repo_name;
        let azure = self.source.code_forge == RepositoryForge::AzureDevOps;
        let valid_segment = |value: &str| {
            !value.is_empty()
                && value != "."
                && value != ".."
                && value.trim() == value
                && value.chars().all(|character| {
                    character.is_alphanumeric()
                        || matches!(character, '-' | '_' | '.')
                        || (azure && character == ' ')
                })
        };
        if !owner.split('/').all(valid_segment)
            || !valid_segment(repo)
            || (self.source.code_forge == RepositoryForge::GitHub && owner.contains('/'))
            || (azure && owner.split('/').count() != 2)
        {
            return Err("invalid repository identity");
        }
        let name = &self.checkout_name;
        let device_name = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
            || name.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                    )
            })
            || matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (device_name.len() == 4
                && (device_name.starts_with("COM") || device_name.starts_with("LPT"))
                && matches!(device_name.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("invalid checkout name");
        }
        match &self.head {
            Some(RepositoryHeadRef::CommitSha(sha)) => {
                if sha.len() != 40
                    || !sha
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                {
                    return Err("invalid checkout commit");
                }
            }
            Some(RepositoryHeadRef::Branch(value)) => {
                if value.is_empty()
                    || value.starts_with('-')
                    || value.starts_with('/')
                    || value == "@"
                    || value.ends_with(['/', '.'])
                    || value.contains("..")
                    || value.contains("@{")
                    || value.split('/').any(|part| {
                        part.is_empty() || part.starts_with('.') || part.ends_with(".lock")
                    })
                    || value.chars().any(|character| {
                        character.is_whitespace()
                            || character.is_control()
                            || matches!(character, ':' | '\\' | '*' | '?' | '[' | '^' | '~')
                    })
                {
                    return Err("invalid checkout ref");
                }
            }
            None => {}
        }
        if self.fetch_branch_only && !matches!(self.head, Some(RepositoryHeadRef::Branch(_))) {
            return Err("branch-only checkout requires a branch");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "environment_checkout_protocol_tests.rs"]
mod tests;
