use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};

#[derive(Debug, Clone, clap::Args)]
pub struct EnvironmentCheckoutArgs {
    #[arg(long)]
    pub requests_file: PathBuf,
    #[arg(long)]
    pub failure_report: PathBuf,
    #[arg(long)]
    pub remove_origins_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutBatch {
    pub working_dir: PathBuf,
    pub repositories: Vec<CheckoutRequest>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutFailure {
    pub request_index: usize,
    pub kind: CheckoutFailureKind,
    pub output: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutFailureReport {
    pub failures: Vec<CheckoutFailure>,
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
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let batch: Self = serde_json::from_slice(bytes).map_err(|_| "invalid checkout JSON")?;
        batch.validate()?;
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
#[path = "environment_checkout_tests.rs"]
mod tests;
