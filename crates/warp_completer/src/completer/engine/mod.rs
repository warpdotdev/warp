mod argument;
mod command;
mod flag;
pub(crate) mod path;
mod variable;

pub use argument::complete as argument_suggestions;
pub use command::complete as command_suggestions;
pub use flag::complete as flag_suggestions;
pub use path::{EngineDirEntry, EngineFileType};
pub use variable::suggestions as variable_suggestions;

cfg_if::cfg_if! {
    if #[cfg(feature = "v2")] {
        mod v2;
        use v2::argument_name_at_index_for_command;
    } else {
        mod legacy;
        use legacy::argument_name_at_index_for_command;
    }
}

use crate::completer::{CompletionContext, TopLevelCommandCaseSensitivity};
use crate::meta::{HasSpan, Span, Spanned, SpannedItem};
use crate::parsers::hir::{Command, Expression, ExternalCommand, FlagType, ShellCommand};
use crate::parsers::{
    ArgumentError, ClassifiedCommand, ParseError, ParseErrorReason, ParsedExpression, ParsedToken,
};
use crate::signatures::CommandRegistry;

pub type CompletionLocation = Spanned<LocationType>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LocationType {
    Command {
        /// Whether the command is in the registry.
        is_recognized: bool,
        parsed_token: ParsedToken,
    },
    Flag {
        /// The name of the command.
        command_name: Spanned<String>,
        /// The name of the flag. This will be `None` if no part of the flag was provided. For
        /// example, `git ` would provide a `Flag` location type with an empty `flag_name`.
        flag_name: Option<Spanned<String>>,
    },
    Argument {
        /// The name of the command.
        command_name: Spanned<String>,
        /// The name of the argument that has been typed so far. This will be `None` if no part of
        /// the argument was provided, such as `cd `.
        argument_name: Option<String>,
        parsed_token: ParsedToken,
    },
    Variable {
        parsed_token: ParsedToken,
    },
}

impl LocationType {
    pub fn is_command(&self) -> bool {
        matches!(self, LocationType::Command { .. })
    }
}

pub struct Flatten<'s> {
    line: &'s str,
    context: &'s CommandRegistry,
    error: Option<&'s ParseError>,
    command: Spanned<String>,
    flag: Option<String>,
}

impl<'s> Flatten<'s> {
    /// Converts a spanned `Expression` into a completion location for use in `WarpCompleter`.
    fn expression(&self, e: &Spanned<ParsedExpression>) -> Vec<CompletionLocation> {
        match e.item.expression() {
            Expression::Command => {
                vec![
                    LocationType::Command {
                        is_recognized: true,
                        parsed_token: e.item.value().to_owned(),
                    }
                    .spanned(e.span),
                ]
            }
            Expression::Literal => {
                vec![
                    LocationType::Argument {
                        command_name: self.command.clone(),
                        argument_name: self.flag.clone(),
                        parsed_token: e.item.value().clone(),
                    }
                    .spanned(e.span),
                ]
            }
            Expression::ValidatableArgument(_) => {
                vec![
                    LocationType::Argument {
                        command_name: self.command.clone(),
                        argument_name: self.flag.clone(),
                        parsed_token: e.item.value().to_owned(),
                    }
                    .spanned(e.span),
                ]
            }
            Expression::Unknown => Vec::new(),
            Expression::Variable => vec![
                LocationType::Variable {
                    parsed_token: e.item.value().to_owned(),
                }
                .spanned(e.span),
            ],
        }
    }

    fn unclassified_command(
        &self,
        command: &ExternalCommand,
        command_case_sensitivity: TopLevelCommandCaseSensitivity,
    ) -> Vec<CompletionLocation> {
        let capacity = 1
            + command
                .args
                .flags
                .as_ref()
                .map_or(0, |flags| flags.flags.len())
            + command
                .args
                .positionals
                .as_ref()
                .map_or(0, |positionals| positionals.len());
        let mut result = Vec::with_capacity(capacity);

        match command.args.command_name.item.expression() {
            Expression::Command | Expression::Literal => result.push(
                LocationType::Command {
                    is_recognized: false,
                    parsed_token: command.name.clone(),
                }
                .spanned(command.name_span),
            ),
            _ => (),
        }

        if let Some(flags) = &command.args.flags {
            for flag in flags.iter() {
                match &flag.flag_type {
                    FlagType::NoArgument => {
                        result.push(
                            LocationType::Flag {
                                command_name: command
                                    .name
                                    .as_str()
                                    .to_owned()
                                    .spanned(command.name_span),
                                flag_name: None,
                            }
                            .spanned(flag.name_span),
                        );
                    }

                    FlagType::Argument { value } => {
                        result.push(
                            LocationType::Flag {
                                command_name: command
                                    .name
                                    .as_str()
                                    .to_owned()
                                    .spanned(command.name_span),
                                flag_name: None,
                            }
                            .spanned(flag.name_span),
                        );
                        result.append(&mut self.with_flag(flag.name.clone()).expression(value));
                    }
                }
            }
        }

        if let Some(positionals) = &command.args.positionals {
            let positionals = positionals.iter();

            result.extend(positionals.enumerate().flat_map(|(idx, positional)| {
                match positional.item.expression() {
                    Expression::Unknown => {
                        let unknown = positional.span.slice(self.line);
                        let location = if unknown.starts_with('-') {
                            LocationType::Flag {
                                command_name: command
                                    .name
                                    .as_str()
                                    .to_owned()
                                    .spanned(command.name_span),
                                flag_name: Some(unknown.to_string().spanned(positional.span)),
                            }
                        } else {
                            LocationType::Argument {
                                command_name: command
                                    .name
                                    .as_str()
                                    .to_owned()
                                    .spanned(command.name_span),
                                argument_name: argument_name_at_index_for_command(
                                    command.name_span.slice(self.line),
                                    idx,
                                    self.context,
                                    command_case_sensitivity,
                                ),
                                parsed_token: positional.item.value().clone(),
                            }
                        };

                        vec![location.spanned(positional.span)]
                    }
                    _ => self.expression(positional),
                }
            }));
        }

        result
    }

    fn command(
        &self,
        command: &ShellCommand,
        command_case_sensitivity: TopLevelCommandCaseSensitivity,
    ) -> Vec<CompletionLocation> {
        let capacity = 1
            + command
                .args
                .flags
                .as_ref()
                .map_or(0, |flags| flags.flags.len())
            + command
                .args
                .positionals
                .as_ref()
                .map_or(0, |positionals| positionals.len());
        let mut result = Vec::with_capacity(capacity);

        let parsed_expression = &command.args.command_name.item;
        match parsed_expression.expression() {
            Expression::Command => {
                result.push(
                    LocationType::Command {
                        is_recognized: true,
                        parsed_token: parsed_expression.value().to_owned(),
                    }
                    .spanned(command.name_span),
                );
            }
            Expression::Literal => {
                result.push(
                    LocationType::Command {
                        is_recognized: false,
                        parsed_token: parsed_expression.value().to_owned(),
                    }
                    .spanned(command.name_span),
                );
            }
            _ => (),
        }

        if let Some(positionals) = &command.args.positionals {
            let positionals = positionals.iter();

            result.extend(positionals.enumerate().flat_map(|(idx, positional)| {
                match positional.item.expression() {
                    Expression::Unknown => {
                        let unknown = positional.span.slice(self.line);
                        let location = if unknown.starts_with('-') {
                            LocationType::Flag {
                                command_name: command.name.clone().spanned(command.name_span),
                                flag_name: Some(unknown.to_string().spanned(positional.span)),
                            }
                        } else {
                            LocationType::Argument {
                                command_name: command.name.clone().spanned(command.name_span),
                                argument_name: argument_name_at_index_for_command(
                                    command.name_span.slice(self.line),
                                    idx,
                                    self.context,
                                    command_case_sensitivity,
                                ),
                                parsed_token: positional.item.value().clone(),
                            }
                        };

                        vec![location.spanned(positional.span)]
                    }
                    _ => self.expression(positional),
                }
            }));
        }

        if let Some(flags) = &command.args.flags {
            for flag in flags.iter() {
                result.push(
                    LocationType::Flag {
                        command_name: command.name.clone().spanned(command.name_span),
                        flag_name: Some(
                            flag.name_span
                                .slice(self.line)
                                .to_string()
                                .spanned(flag.name_span),
                        ),
                    }
                    .spanned(flag.name_span),
                );
                if let FlagType::Argument { value } = &flag.flag_type {
                    result.append(&mut self.with_flag(flag.name.clone()).expression(value));
                }
            }
        }

        // Flags will not appear in [`crate::parsers::hir::CommandCallInfo::flags`] if they require
        // an argument but it's missing. In that case, it is a parse error. We still want known
        // flags which are missing an argument to be treated as a possible completion location. For
        // example, the cursor may be at the end of `npx --shell`, where `--shell` requires an
        // argument, and may actually want to complete the option `--shell-auto-fallback` instead.
        // So, check for this case in the parse error.
        if let Some(ParseError {
            reason:
                ParseErrorReason::ArgumentError {
                    error: ArgumentError::MissingValueForName { name, .. },
                    ..
                },
        }) = self.error
        {
            result.push(
                LocationType::Flag {
                    command_name: command.name.clone().spanned(command.name_span),
                    flag_name: Some(name.clone()),
                }
                .spanned(name.span()),
            )
        }

        result
    }

    /// Flattens the block into a Vec of completion locations
    pub fn completion_locations(
        &self,
        command: &Command,
        command_case_sensitivity: TopLevelCommandCaseSensitivity,
    ) -> Vec<CompletionLocation> {
        match command {
            Command::Classified(cmd) => self.command(cmd, command_case_sensitivity),
            Command::Unclassified(cmd) => self.unclassified_command(cmd, command_case_sensitivity),
        }
    }

    pub fn new(
        line: &'s str,
        context: &'s CommandRegistry,
        command: Spanned<String>,
        error: Option<&'s ParseError>,
    ) -> Flatten<'s> {
        Flatten {
            line,
            context,
            error,
            command,
            flag: None,
        }
    }

    pub fn with_flag(&self, flag: String) -> Flatten<'s> {
        Flatten {
            line: self.line,
            context: self.context,
            error: self.error,
            command: self.command.clone(),
            flag: Some(flag),
        }
    }
}

/// Characters that precede a command name
const BEFORE_COMMAND_CHARS: &[char] = &['|', '(', ';'];

/// Determines the completion location for a given block at the given cursor position
pub fn completion_location(
    ctx: &dyn CompletionContext,
    line: &str,
    classified_command: Option<&ClassifiedCommand>,
) -> Vec<CompletionLocation> {
    let (command, error) = match classified_command {
        Some(command_with_error) => (&command_with_error.command, &command_with_error.error),
        // If there's no command--treat the completion location as a single foreign command to
        // surface top level commands.
        None => {
            return vec![
                LocationType::Command {
                    is_recognized: false,
                    parsed_token: ParsedToken::empty(),
                }
                .spanned(Span::default()),
            ];
        }
    };

    let completion_engine = Flatten::new(
        line,
        ctx.command_registry(),
        command.command_name_span().map(ToOwned::to_owned),
        error.as_ref(),
    );
    let locations = completion_engine.completion_locations(command, ctx.command_case_sensitivity());

    if locations.is_empty() {
        vec![
            LocationType::Command {
                is_recognized: false,
                parsed_token: ParsedToken::empty(),
            }
            .spanned(Span::default()),
        ]
    } else {
        let mut command = None;
        let mut prev = None;
        for loc in locations {
            // We don't use span.contains because we want to include the end. This handles the case
            // where the cursor is just after the text (i.e., no space between cursor and text)
            if loc.span.start() <= line.len() && line.len() <= loc.span.end() {
                // The parser sees the "-" in `cmd -` as an argument, but the user is likely
                // expecting a flag.
                return match loc.item {
                    LocationType::Argument {
                        command_name: ref cmd,
                        ..
                    } => {
                        let cmd = cmd.clone();
                        if loc.span.slice(line) == "-" {
                            let span = loc.span;
                            return vec![
                                loc,
                                LocationType::Flag {
                                    command_name: cmd,
                                    flag_name: Some("-".to_owned().spanned(span)),
                                }
                                .spanned(span),
                            ];
                        }
                        // This ensures that flags are not included if the user has already typed a
                        // non "-" such as "git c<tab>"
                        vec![loc]
                    }
                    _ => vec![loc],
                };
            } else if line.len() < loc.span.start() {
                break;
            }

            if loc.item.is_command() {
                command = Some(String::from(loc.span.slice(line)).spanned(loc.span));
            }

            prev = Some(loc);
        }

        if let Some(prev) = prev {
            // Cursor is between locations (or at the end). Look at the line to see if the cursor
            // is after some character that would imply we're in the command position.
            let start = prev.span.end();
            if line[start..].contains(BEFORE_COMMAND_CHARS) {
                vec![
                    LocationType::Command {
                        is_recognized: true,
                        parsed_token: ParsedToken::empty(),
                    }
                    .spanned(Span::new(line.len(), line.len())),
                ]
            } else if let Some(command) = command {
                let arg_location = LocationType::Argument {
                    command_name: command.clone(),
                    argument_name: None,
                    parsed_token: ParsedToken::empty(),
                }
                .spanned(Span::new(line.len(), line.len()));

                let flag_location = LocationType::Flag {
                    command_name: command,
                    flag_name: None,
                }
                .spanned(Span::new(line.len(), line.len()));

                vec![arg_location, flag_location]
            } else {
                vec![]
            }
        } else {
            // Cursor is before any possible completion location, so must be a command
            vec![
                LocationType::Command {
                    is_recognized: true,
                    parsed_token: ParsedToken::empty(),
                }
                .spanned(Span::new(line.len(), line.len())),
            ]
        }
    }
}

#[cfg(not(feature = "v2"))]
#[cfg(test)]
#[path = "test.rs"]
mod tests;
