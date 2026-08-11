//! Per-agent resume support, declared as embedded configuration.
//!
//! Support for an agent is a declaration in `resources/cli_agent_resume/agents.toml`
//! rather than a restore path in code: the file supplies the invocation shape that
//! reattaches to a prior session, the allowlist of flags carried over from the user's
//! own invocation, and the shape each of those values must match. It never names an
//! executable — the binary comes from [`CLIAgent::command_prefix`].
//!
//! Recorded values are untrusted. They come from a local database file that any
//! process running as the user can write, and the string built here is parsed by an
//! interactive shell, so a stored value is a code-execution primitive until it has
//! been checked. Validation therefore happens when the invocation is built, not when
//! the flags were captured, and a value that fails is dropped rather than repaired.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use warp_errors::report_error;

use crate::terminal::CLIAgent;

/// Trailing comment appended to every built resume invocation so the shell keeps it
/// out of history: a resume is Warp's line, not something the user typed.
///
/// Matched literally by the bootstrap scripts in `app/assets/bundled/bootstrap/`
/// (`zsh_body.sh`, `bash_body.sh`, `pwsh.ps1`); changing this string means changing
/// all three. `#` starts a comment in all three shells, so the marker stays inert.
pub const RESUME_HISTORY_MARKER: &str = "warp_resume_agent_session";

const EMBEDDED_DECLARATIONS: &str = include_str!("../../resources/cli_agent_resume/agents.toml");

/// How recently the recorded state must have been observed for its permission-posture flags to
/// ride along into the resume. Past this, the pane still resumes — just at the posture the agent
/// defaults to.
///
/// Provisional: 12 hours is a placeholder pending field data on how long a pane realistically sits
/// between the last observation and the restart. The shipped value is a rollout decision, not an
/// implementation one.
pub const PERMISSION_POSTURE_FRESHNESS: Duration = Duration::from_secs(12 * 60 * 60);

/// Whether the permission posture recorded alongside a session is still the user's live choice.
///
/// The user's own invocation is the authority on the posture it ran at, but that authority
/// expires: a pane whose recording is a week old is not a window the user is still standing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPosture {
    /// Observed inside [`PERMISSION_POSTURE_FRESHNESS`]: carry the flags the user chose.
    Carry,
    /// Observed outside it, or at an age no clock can vouch for. Resume without them.
    Drop,
}

impl PermissionPosture {
    /// The posture for state last observed at `observed_at`, judged against `now`.
    pub fn for_observation(observed_at: NaiveDateTime, now: NaiveDateTime) -> Self {
        match (now - observed_at).to_std() {
            Ok(age) if age <= PERMISSION_POSTURE_FRESHNESS => PermissionPosture::Carry,
            // A negative age means the clock moved backwards between the recording and this
            // restart, so nothing here can vouch for how old the recording is. An age that
            // cannot be verified is not a fresh one.
            _ => PermissionPosture::Drop,
        }
    }
}

/// Characters a [`ValueShape::BareToken`] may contain on top of ASCII alphanumerics.
const BARE_TOKEN_PUNCTUATION: &[char] = &['.', '_', '-', '+', ':', '@'];

/// The shape a value must match to survive into a built invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueShape {
    /// The flag stands alone; anything recorded alongside it is not this flag.
    Boolean,
    /// ASCII alphanumerics plus [`BARE_TOKEN_PUNCTUATION`], not starting with `-`.
    BareToken,
    /// [`ValueShape::BareToken`] plus `/`.
    PathLike,
}

impl ValueShape {
    fn accepts_char(self, c: char) -> bool {
        match self {
            ValueShape::Boolean => false,
            ValueShape::BareToken => {
                c.is_ascii_alphanumeric() || BARE_TOKEN_PUNCTUATION.contains(&c)
            }
            ValueShape::PathLike => c == '/' || ValueShape::BareToken.accepts_char(c),
        }
    }

    /// Whether `value` may be passed to a shell once quoted. Everything outside the
    /// declared character set is rejected, which covers whitespace, shell
    /// metacharacters, globs, newlines, quotes and non-ASCII lookalikes in one rule.
    fn accepts(self, value: &str, max_length: usize) -> bool {
        !value.is_empty()
            && value.len() <= max_length
            // A value that opens with `-` would be read as a flag rather than the
            // value of the flag it follows.
            && !value.starts_with('-')
            && value.chars().all(|c| self.accepts_char(c))
    }
}

/// A resume-relevant flag recorded from the user's own invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedFlag {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DeclarationError {
    #[error("resume declarations are not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("`{0}` is not a CLI agent Warp knows")]
    UnknownAgent(String),
    #[error("`{0}` has no command prefix to resume with")]
    NoCommand(String),
    #[error("`{agent}` declares an unusable resume invocation: {reason}")]
    Invocation { agent: String, reason: &'static str },
    #[error("`{agent}` declares an unusable value for `{key}`: {reason}")]
    Value {
        agent: String,
        key: String,
        reason: &'static str,
    },
}

/// The two invocation shapes agents use to reattach to a session, as written in the
/// declaration file. Kept as one struct with optional members so that
/// `deny_unknown_fields` applies and a mismatched pair is rejected at load with a
/// reason rather than deserialized into a half-shape.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawResume {
    form: ResumeForm,
    flag: Option<String>,
    subcommand: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResumeForm {
    Flag,
    Subcommand,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawValue {
    shape: ValueShape,
    max_length: Option<usize>,
    #[serde(default)]
    aliases: Vec<String>,
    /// Whether this flag chooses the agent's permission posture rather than describing the
    /// session. Declared per flag because only the allowlist knows which spelling elevates.
    #[serde(default)]
    permission_posture: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawAgent {
    resume: RawResume,
    identifier: RawValue,
    #[serde(default)]
    flags: HashMap<String, RawValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawFile {
    agents: HashMap<String, RawAgent>,
}

/// A validated resume invocation shape.
#[derive(Debug)]
enum ResumeInvocation {
    /// `<binary> --resume <id>`.
    Flag(String),
    /// `<binary> resume <id>`.
    Subcommand(String),
}

#[derive(Debug)]
struct ValueDeclaration {
    shape: ValueShape,
    max_length: usize,
    permission_posture: bool,
}

impl ValueDeclaration {
    fn accepts(&self, value: &str) -> bool {
        self.shape.accepts(value, self.max_length)
    }
}

#[derive(Debug)]
struct AgentDeclaration {
    binary: &'static str,
    resume: ResumeInvocation,
    identifier: ValueDeclaration,
    flags: HashMap<String, ValueDeclaration>,
    /// Alternate spellings mapped to the allowlisted name they stand for.
    aliases: HashMap<String, String>,
}

impl AgentDeclaration {
    /// The allowlisted name `recorded` stands for, if any.
    fn canonical_name<'a>(&'a self, recorded: &'a str) -> Option<&'a str> {
        if self.flags.contains_key(recorded) {
            return Some(recorded);
        }
        self.aliases.get(recorded).map(String::as_str)
    }
}

#[derive(Debug, Default)]
pub struct ResumeDeclarations {
    agents: HashMap<CLIAgent, AgentDeclaration>,
}

impl ResumeDeclarations {
    /// The declarations embedded at build time. A file that fails to load leaves every
    /// agent unsupported, which costs a resume rather than risking a wrong one.
    pub fn embedded() -> &'static Self {
        static DECLARATIONS: LazyLock<ResumeDeclarations> = LazyLock::new(|| {
            ResumeDeclarations::parse(EMBEDDED_DECLARATIONS).unwrap_or_else(|_| {
                report_error!("embedded CLI agent resume declarations failed to load");
                ResumeDeclarations::default()
            })
        });
        &DECLARATIONS
    }

    fn parse(contents: &str) -> Result<Self, DeclarationError> {
        let raw: RawFile = toml::from_str(contents)?;
        let mut agents = HashMap::with_capacity(raw.agents.len());

        for (name, declaration) in raw.agents {
            let agent = CLIAgent::from_serialized_name(&name);
            if agent.to_serialized_name() != name {
                return Err(DeclarationError::UnknownAgent(name));
            }
            let binary = agent.command_prefix();
            if binary.is_empty() {
                return Err(DeclarationError::NoCommand(name));
            }
            agents.insert(agent, AgentDeclaration::build(&name, binary, declaration)?);
        }

        Ok(Self { agents })
    }

    pub fn supports(&self, agent: CLIAgent) -> bool {
        self.agents.contains_key(&agent)
    }

    /// The allowlisted flags present in an agent's own command line, with alternate
    /// spellings resolved to the name the allowlist uses.
    ///
    /// Values are recorded as they were seen. They are untrusted either way — the
    /// store they land in is writable by other processes — so checking them here would
    /// buy nothing that [`Self::build_resume_command`] does not have to redo.
    pub fn extract_resume_flags(
        &self,
        agent: CLIAgent,
        args: &[impl AsRef<str>],
    ) -> Vec<RecordedFlag> {
        let Some(declaration) = self.agents.get(&agent) else {
            return Vec::new();
        };

        let mut recorded = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].as_ref();
            index += 1;
            if !arg.starts_with('-') {
                continue;
            }

            let (spelling, inline_value) = match arg.split_once('=') {
                Some((spelling, value)) => (spelling, Some(value)),
                None => (arg, None),
            };
            let Some(name) = declaration.canonical_name(spelling) else {
                continue;
            };
            let takes_value = declaration
                .flags
                .get(name)
                .is_some_and(|flag| flag.shape != ValueShape::Boolean);

            let value = match (takes_value, inline_value) {
                (false, None) => None,
                // A boolean flag given a value is not the flag the allowlist declared.
                (false, Some(_)) => continue,
                (true, Some(value)) => Some(value.to_owned()),
                (true, None) => {
                    // A separated value never opens with `-`; taking one that does
                    // would swallow the next flag.
                    let next = args
                        .get(index)
                        .map(AsRef::as_ref)
                        .filter(|next| !next.starts_with('-'));
                    let Some(next) = next else {
                        continue;
                    };
                    index += 1;
                    Some(next.to_owned())
                }
            };
            recorded.push(RecordedFlag {
                name: name.to_owned(),
                value,
            });
        }

        recorded
    }

    /// The shell command that reattaches `agent` to `identifier`, carrying whichever of
    /// `flags` still validate and `posture` still admits.
    ///
    /// Returns `None` when the agent is undeclared or the resume pointer itself fails
    /// its declared shape: without a usable pointer there is no invocation to salvage. A
    /// [`PermissionPosture::Drop`] never costs the resume, only the elevation.
    /// The allowlisted flags `agent` declares as choosing a permission posture.
    pub fn permission_posture_flags(&self, agent: CLIAgent) -> Vec<&str> {
        let Some(declaration) = self.agents.get(&agent) else {
            return Vec::new();
        };
        declaration
            .flags
            .iter()
            .filter(|(_, declared)| declared.permission_posture)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    pub fn build_resume_command(
        &self,
        agent: CLIAgent,
        identifier: &str,
        flags: &[RecordedFlag],
        posture: PermissionPosture,
    ) -> Option<String> {
        let declaration = self.agents.get(&agent)?;
        if !declaration.identifier.accepts(identifier) {
            return None;
        }

        let mut command = declaration.binary.to_owned();
        if let ResumeInvocation::Subcommand(subcommand) = &declaration.resume {
            command.push(' ');
            command.push_str(subcommand);
        }

        // Flags go ahead of the identifier because some agents take a trailing prompt
        // positional, which would otherwise swallow everything after it.
        for flag in flags {
            let Some(name) = declaration.canonical_name(&flag.name) else {
                continue;
            };
            let Some(declared) = declaration.flags.get(name) else {
                continue;
            };
            // R22: an elevation the user chose is theirs to keep only while the observation
            // behind it is recent. Past the window the flag goes and the resume stays.
            if declared.permission_posture && posture == PermissionPosture::Drop {
                continue;
            }
            match (declared.shape, flag.value.as_deref()) {
                (ValueShape::Boolean, None) => {
                    command.push(' ');
                    command.push_str(name);
                }
                (ValueShape::Boolean, Some(_)) => continue,
                (_, Some(value)) if declared.accepts(value) => {
                    let Some(quoted) = shell_quote(value) else {
                        continue;
                    };
                    command.push(' ');
                    command.push_str(name);
                    command.push(' ');
                    command.push_str(&quoted);
                }
                (_, _) => continue,
            }
        }

        if let ResumeInvocation::Flag(flag) = &declaration.resume {
            command.push(' ');
            command.push_str(flag);
        }
        command.push(' ');
        command.push_str(&shell_quote(identifier)?);
        command.push_str(" # ");
        command.push_str(RESUME_HISTORY_MARKER);
        Some(command)
    }
}

impl AgentDeclaration {
    fn build(
        name: &str,
        binary: &'static str,
        raw: RawAgent,
    ) -> Result<AgentDeclaration, DeclarationError> {
        let resume = ResumeInvocation::build(name, raw.resume)?;
        let invalid_identifier = |reason| DeclarationError::Value {
            agent: name.to_owned(),
            key: "identifier".to_owned(),
            reason,
        };
        if !raw.identifier.aliases.is_empty() {
            return Err(invalid_identifier("an identifier is not spelled as a flag"));
        }
        if raw.identifier.permission_posture {
            return Err(invalid_identifier(
                "a session pointer chooses no permission posture",
            ));
        }
        let identifier = ValueDeclaration::build(name, "identifier", raw.identifier)?;
        if identifier.shape == ValueShape::Boolean {
            return Err(invalid_identifier(
                "a session identifier is a value, not a bare flag",
            ));
        }

        let mut flags = HashMap::with_capacity(raw.flags.len());
        let mut aliases = HashMap::new();
        for (flag, value) in raw.flags {
            if !is_flag_spelling(&flag) {
                return Err(DeclarationError::Value {
                    agent: name.to_owned(),
                    key: flag,
                    reason: "an allowlist entry has to be a `--flag`",
                });
            }
            for alias in &value.aliases {
                if !is_flag_spelling(alias) {
                    return Err(DeclarationError::Value {
                        agent: name.to_owned(),
                        key: alias.clone(),
                        reason: "an alias has to be a flag spelling",
                    });
                }
                aliases.insert(alias.clone(), flag.clone());
            }
            flags.insert(flag.clone(), ValueDeclaration::build(name, &flag, value)?);
        }

        Ok(AgentDeclaration {
            binary,
            resume,
            identifier,
            flags,
            aliases,
        })
    }
}

impl ResumeInvocation {
    fn build(agent: &str, raw: RawResume) -> Result<ResumeInvocation, DeclarationError> {
        let invalid = |reason| DeclarationError::Invocation {
            agent: agent.to_owned(),
            reason,
        };
        match (raw.form, raw.flag, raw.subcommand) {
            (ResumeForm::Flag, Some(flag), None) if is_flag_spelling(&flag) => {
                Ok(ResumeInvocation::Flag(flag))
            }
            (ResumeForm::Flag, Some(_), None) => Err(invalid("the flag form needs a `--flag`")),
            (ResumeForm::Flag, _, _) => {
                Err(invalid("the flag form takes a `flag` and nothing else"))
            }
            (ResumeForm::Subcommand, None, Some(subcommand))
                if ValueShape::BareToken.accepts(&subcommand, MAX_INVOCATION_LENGTH) =>
            {
                Ok(ResumeInvocation::Subcommand(subcommand))
            }
            (ResumeForm::Subcommand, None, Some(_)) => {
                Err(invalid("the subcommand is not a bare word"))
            }
            (ResumeForm::Subcommand, _, _) => Err(invalid(
                "the subcommand form takes a `subcommand` and nothing else",
            )),
        }
    }
}

impl ValueDeclaration {
    fn build(agent: &str, key: &str, raw: RawValue) -> Result<ValueDeclaration, DeclarationError> {
        let invalid = |reason| DeclarationError::Value {
            agent: agent.to_owned(),
            key: key.to_owned(),
            reason,
        };
        let max_length = match (raw.shape, raw.max_length) {
            (ValueShape::Boolean, None) => 0,
            (ValueShape::Boolean, Some(_)) => {
                return Err(invalid("a boolean flag has no value to bound"));
            }
            (_, Some(max_length)) if max_length > 0 => max_length,
            (_, Some(_)) => return Err(invalid("a value bound of zero admits nothing")),
            (_, None) => return Err(invalid("a value needs a length bound")),
        };
        Ok(ValueDeclaration {
            shape: raw.shape,
            max_length,
            permission_posture: raw.permission_posture,
        })
    }
}

/// Longest invocation fragment the declaration file may supply.
const MAX_INVOCATION_LENGTH: usize = 64;

fn is_flag_spelling(candidate: &str) -> bool {
    candidate.starts_with("--")
        && ValueShape::BareToken.accepts(&candidate[2..], MAX_INVOCATION_LENGTH)
}

/// Wraps `value` in single quotes, which every shell Warp bootstraps treats as fully
/// literal. Refuses a value containing a single quote, which is the only character
/// that could end the wrapping: the shapes already reject it, so this is the second
/// of two independent barriers rather than the first.
fn shell_quote(value: &str) -> Option<String> {
    (!value.contains('\'')).then(|| format!("'{value}'"))
}

#[cfg(test)]
#[path = "cli_agent_resume_tests.rs"]
mod tests;
