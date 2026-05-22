pub mod overrides;

use std::collections::HashMap;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use lazy_static::lazy_static;
use memo_map::MemoMap;
use overrides::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ChannelVersions {
    pub dev: ChannelVersion,
    pub preview: ChannelVersion,
    pub stable: ChannelVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelogs: Option<ChannelChangelogs>,
}

impl std::fmt::Display for ChannelVersions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dev: {:?}; preview: {:?}; stable: {:?}",
            self.dev, self.preview, self.stable
        )
    }
}

lazy_static! {
    static ref VERSION_RE: Regex = Regex::new(r"v(\d+)\.(.+)\.(.+)_(\d+)").unwrap();

    // Cached mapping of version strings to semantic versions.
    static ref PARSED_VERSIONS_CACHE: MemoMap<String, ParsedVersion> = Default::default();
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct ParsedVersion {
    major: usize,
    date: NaiveDateTime,
    patch: usize,
}

impl TryFrom<&str> for ParsedVersion {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self> {
        PARSED_VERSIONS_CACHE
            .get_or_try_insert(value, || {
                VERSION_RE
                    .captures(value)
                    .and_then(|captures| {
                        let date_str = captures.get(2)?.as_str();
                        let date =
                            NaiveDateTime::parse_from_str(date_str, "%Y.%m.%d.%H.%M").ok()?;
                        Some(ParsedVersion {
                            major: captures.get(1)?.as_str().parse().ok()?,
                            date,
                            patch: captures.get(4)?.as_str().parse().ok()?,
                        })
                    })
                    .context("Can't parse string into Version")
            })
            .cloned()
    }
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.date, self.patch).cmp(&(other.major, other.date, other.patch))
    }
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChannelVersion {
    #[serde(flatten)]
    version_info: VersionInfo,
    /// Any overrides which should be applied for this channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    overrides: Vec<VersionOverride>,
}

impl ChannelVersion {
    pub fn new(version_info: VersionInfo) -> Self {
        Self {
            version_info,
            overrides: vec![],
        }
    }

    /// Returns the version information, with any applicable overrides applied
    /// based on the current execution environment.
    pub fn version_info(&self) -> VersionInfo {
        let context = overrides::Context::from_env();
        self.version_info
            .with_overrides_applied(&self.overrides, &context)
    }

    /// Returns the version information, with any applicable overrides applied
    /// based on the provided context.
    pub fn version_info_for_execution_context(&self, context: &overrides::Context) -> VersionInfo {
        self.version_info
            .with_overrides_applied(&self.overrides, context)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub version: String,
    /// The version to download for new users from the download page. This is not used on the client
    /// other than in the `apply_overrides` binary used from the `channel-versions` repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_for_new_users: Option<String>,
    /// The time by which the client needs to be updated, after which
    /// the user sees a warning banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_by: Option<DateTime<FixedOffset>>,
    /// If specified, this field indicates the oldest version of the client that is still
    /// supported. Any version before this version is not supported and the user should update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_cutoff: Option<String>,
    /// If specified, this field indicates the latest client version that has a prominent update.
    /// Versions before `prominent_update` should display the prominent update UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prominent_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_rollback: Option<bool>,
    /// The version to use for CLI downloads, falling back to `version` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
}

impl VersionInfo {
    pub fn new(version: String) -> Self {
        Self {
            version,
            update_by: None,
            soft_cutoff: None,
            last_prominent_update: None,
            version_for_new_users: None,
            is_rollback: None,
            cli_version: None,
        }
    }

    /// Returns the CLI version, falling back to the app version if not set.
    pub fn cli_version(&self) -> &str {
        self.cli_version.as_deref().unwrap_or(&self.version)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChannelChangelogs {
    // Maps of changelogs by version
    pub dev: HashMap<String, Changelog>,
    pub preview: HashMap<String, Changelog>,
    pub stable: HashMap<String, Changelog>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Changelog {
    pub date: DateTime<FixedOffset>,
    pub sections: Vec<Section>,
    #[serde(default = "default_markdown_sections")]
    pub markdown_sections: Vec<MarkdownSection>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oz_updates: Vec<String>,
}

// Default value for when the changelog JSON doesn't have the markdown_sections field
fn default_markdown_sections() -> Vec<MarkdownSection> {
    vec![
        MarkdownSection {
            title: "New features".to_string(),
            markdown: "".to_string(),
        },
        MarkdownSection {
            title: "Improvements".to_string(),
            markdown: "".to_string(),
        },
        MarkdownSection {
            title: "Coming soon".to_string(),
            markdown: "".to_string(),
        },
    ]
}

impl std::fmt::Display for Changelog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.sections
                .iter()
                .fold(String::new(), |mut output, item| {
                    let _ = write!(output, "{item}\n\n");
                    output
                })
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub items: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MarkdownSection {
    pub title: String,
    pub markdown: String,
}

impl std::fmt::Display for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:\n{}",
            self.title,
            self.items.iter().fold(String::new(), |mut output, item| {
                let _ = writeln!(output, "- {item}");
                output
            })
        )
    }
}

#[cfg(test)]
#[path = "channel_versions_tests.rs"]
mod tests;
