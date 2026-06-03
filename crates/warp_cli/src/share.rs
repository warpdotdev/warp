use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use clap::builder::{EnumValueParser, PossibleValue};
use clap::error::ErrorKind;
use clap::{Arg, Args, Command, ValueEnum};

/// Arguments for sharing a session or other object.
#[derive(Debug, Clone, Args)]
pub struct ShareArgs {
    /// Share the agent's session
    ///
    /// Learn more at https://docs.warp.dev/knowledge-and-collaboration/session-sharing
    #[arg(long = "share", value_name = "RECIPIENTS", num_args=0..=1)]
    pub share: Option<Vec<ShareRequest>>,
}

impl ShareArgs {
    /// Returns `true` if the session should be shared.
    pub fn is_shared(&self) -> bool {
        self.share.is_some()
    }
}

/// An individual sharing request, identifying:
/// * Who to share with
/// * Their permission level
#[derive(Debug, Clone)]
pub struct ShareRequest {
    pub subject: ShareSubject,
    pub access_level: ShareAccessLevel,
}

impl fmt::Display for ShareRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.subject {
            ShareSubject::Team => write!(f, "team:{}", self.access_level)?,
            ShareSubject::Public => write!(f, "public:{}", self.access_level)?,
            ShareSubject::User { email } => write!(f, "{email}:{}", self.access_level)?,
        }
        Ok(())
    }
}

impl clap::builder::ValueParserFactory for ShareRequest {
    type Parser = ShareRequestParser;

    fn value_parser() -> Self::Parser {
        ShareRequestParser
    }
}

#[derive(Copy, Clone)]
pub struct ShareRequestParser;

impl clap::builder::TypedValueParser for ShareRequestParser {
    type Value = ShareRequest;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let value_str = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                i18n::t("warp_cli.share.error.invalid_recipient"),
            )
        })?;

        // If there's a `:`, treat the first part as the subject and the second as the access level. Otherwise, default to `view` access.
        let (subject_str, level_str) = match value_str.split_once(':') {
            Some((subject, level)) => (subject, Some(level)),
            None => (value_str, None),
        };

        let subject = ShareSubject::from_str(subject_str)?;
        let access_level = match level_str {
            Some(level) => EnumValueParser::new().parse_ref(cmd, arg, OsStr::new(level))?,
            None => ShareAccessLevel::View,
        };

        Ok(ShareRequest {
            subject,
            access_level,
        })
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            [
                PossibleValue::new("team:view")
                    .help(i18n::t("warp_cli.share.possible.team_view"))
                    .alias("team"),
                PossibleValue::new("team:edit").help(i18n::t("warp_cli.share.possible.team_edit")),
                PossibleValue::new("public:view")
                    .help(i18n::t("warp_cli.share.possible.public_view"))
                    .alias("public"),
                PossibleValue::new("public:edit")
                    .help(i18n::t("warp_cli.share.possible.public_edit")),
                PossibleValue::new("<user@email.com>:view")
                    .help(i18n::t("warp_cli.share.possible.user_view"))
                    .alias("<user@email.com>"),
                PossibleValue::new("<user@email.com>:edit")
                    .help(i18n::t("warp_cli.share.possible.user_edit")),
            ]
            .into_iter(),
        ))
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ShareAccessLevel {
    View,
    Edit,
}

impl fmt::Display for ShareAccessLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareAccessLevel::View => write!(f, "view"),
            ShareAccessLevel::Edit => write!(f, "edit"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ShareSubject {
    /// Share with everyone on the caller's current team.
    Team,
    /// Share with anyone who has the link (anyone-with-link ACL).
    /// Subject to the workspace-level anyone-with-link sharing setting.
    Public,
    /// Share with an individual user by email.
    User { email: String },
}

impl FromStr for ShareSubject {
    type Err = clap::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "team" => Ok(ShareSubject::Team),
            "public" => Ok(ShareSubject::Public),
            email if email.contains('@') => Ok(ShareSubject::User {
                email: email.to_string(),
            }),
            other => Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                i18n::t("warp_cli.share.error.invalid_subject").replace("{subject}", other),
            )),
        }
    }
}

#[cfg(test)]
#[path = "share_tests.rs"]
mod tests;
