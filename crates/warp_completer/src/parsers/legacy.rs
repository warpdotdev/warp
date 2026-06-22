//! The "legacy" implementation of internal command parsing logic that depends on the legacy:
//! command signature struct (`warp_command_signatures::Signature`).
use itertools::Itertools;
use warp_command_signatures::{DynamicCompletionData, IsArgumentOptional, Opt, Signature};

use super::hir::{Command, Expression, ShellCommand};
use super::{parse_unclassified_command, LiteCommand, ParseError};
use crate::completer::TopLevelCommandCaseSensitivity;
use crate::meta::{HasSpan, Span, Spanned, SpannedItem};
use crate::parsers::hir::Flags;
use crate::parsers::{
    parse_arg, parse_dollar_expr, ArgumentError, FlagArgumentsCardinality, FlagSignature,
    ParsedExpression, ParsedToken,
};
use crate::signatures::CommandRegistry;

#[derive(Clone, Copy)]
/// A `Signature` (and its corresponding generator) contained at a given index.
pub struct SignatureAtTokenIndex<'a> {
    pub signature: &'a Signature,
    pub dynamic_completion_data: Option<&'a DynamicCompletionData>,
    pub token_index: usize,
}

impl<'a> SignatureAtTokenIndex<'a> {
    pub fn new(
        signature: &'a Signature,
        dynamic_completion_data: Option<&'a DynamicCompletionData>,
        index: usize,
    ) -> Self {
        SignatureAtTokenIndex {
            signature,
            dynamic_completion_data,
            token_index: index,
        }
    }
}

/// Checks whether a `LiteCommand` is in the registry.
pub(super) fn parse_command(
    lite_cmd: &LiteCommand,
    tokens: &[&str],
    parser_scope: &CommandRegistry,
    command_case_sensitivity: TopLevelCommandCaseSensitivity,
) -> (Option<Command>, Option<ParseError>) {
    let mut error: Option<ParseError> = None;

    if lite_cmd.parts.is_empty() {
        (None, None)
    } else if let Some(found_signature) = parser_scope.signature_from_tokens(
        tokens,
        lite_cmd.post_whitespace.is_some(),
        command_case_sensitivity,
    ) {
        let (internal_command, err) = parse_internal_command(
            lite_cmd,
            found_signature.signature,
            found_signature.token_index,
        );

        error = error.or(err);
        (Some(Command::Classified(internal_command)), error)
    } else {
        let (command, error) = parse_unclassified_command(lite_cmd);
        (Some(command), error)
    }
}

/// Does a full parse of an internal command using the lite-ly parse command as a starting point
/// This main focus at this level is to understand what flags were passed in, what positional
/// arguments were passed in, what rest arguments were passed in and to ensure that the basic
/// requirements in terms of number of each were met.
fn parse_internal_command(
    lite_cmd: &LiteCommand,
    signature: &Signature,
    mut idx: usize,
) -> (ShellCommand, Option<ParseError>) {
    log::debug!("parsing internal command {lite_cmd:?}");

    // This is a known internal command, so we need to work with the arguments and parse them according to the expected types
    let (name, name_span) = (
        lite_cmd.parts[0..(idx + 1)]
            .iter()
            .map(|x| x.item.clone())
            .collect::<Vec<String>>()
            .join(" "),
        Span::new(
            lite_cmd.parts[0].span.start(),
            lite_cmd.parts[idx].span.end(),
        ),
    );

    let mut internal_command = ShellCommand::new(ParsedToken(name), name_span, lite_cmd.span());
    internal_command.args.flags = Some(Flags::new());
    internal_command.args.ending_whitespace = lite_cmd.post_whitespace;

    let mut current_positional = 0;
    let mut named = Flags::new();
    let mut positional = vec![];
    let mut error = None;
    idx += 1; // Start where the arguments begin

    while idx < lite_cmd.parts.len() {
        if lite_cmd.parts[idx].item.starts_with('-') && lite_cmd.parts[idx].item.len() > 1 {
            let (named_types, err) =
                get_flag_signature_spec(signature, &internal_command, &lite_cmd.parts[idx]);

            if err.is_none() {
                for FlagSignature {
                    name: full_name,
                    is_switch,
                    arguments_cardinality,
                    arguments,
                } in named_types
                {
                    if is_switch {
                        // Switch flag (without arguments)
                        named.insert_flag_with_no_argument(full_name, lite_cmd.parts[idx].span);
                    } else if let Some(eq_pos) = lite_cmd.parts[idx].item.find('=') {
                        // Self-contained option (--key=value)
                        let token = &lite_cmd.parts[idx];
                        let value_offset = eq_pos + 1;

                        let value_span =
                            Span::new(token.span.start() + value_offset, token.span.end());
                        let value = token.item[value_offset..].to_string().spanned(value_span);
                        // We expect there to be exactly one arg in the case of a --key=value flag.
                        let arg_signature = arguments.first();
                        let (arg, err) = parse_arg(&value, arg_signature);
                        // Flag name span covers only the flag name (before '=').
                        let flag_name_span =
                            Span::new(token.span.start(), token.span.start() + eq_pos);
                        named.insert_flag_with_argument(full_name, flag_name_span, arg);

                        error = error.or(err);
                    } else if idx == lite_cmd.parts.len() - 1 {
                        // Named argument with missing value
                        error = error.or_else(|| {
                            Some(ParseError::argument_error(
                                lite_cmd.parts[0].clone(),
                                ArgumentError::MissingValueForName {
                                    name: full_name.spanned(lite_cmd.parts[idx].span),
                                    missing_arg_index: 0,
                                },
                            ))
                        });
                    } else {
                        // Named argument with following value(s).
                        // Since an option can have multiple arguments (any of which can be variadic),
                        // we should exhaust as many following args as possible.
                        let flag_idx = idx;

                        let end = match arguments_cardinality {
                            FlagArgumentsCardinality::Variadic => lite_cmd.parts.len(),
                            FlagArgumentsCardinality::Fixed(num_args) => flag_idx + num_args + 1,
                        };
                        let mut argument_idx = idx + 1;

                        // Exhaust as many args as we expect but stop early if we see another option.
                        // Note that since we are incrementing index again in the outer loop. Let's check
                        // boundary on the NEXT token rather than the current token.
                        while argument_idx < end.min(lite_cmd.parts.len())
                            && !lite_cmd
                                .parts
                                .get(argument_idx)
                                .is_some_and(|part| part.item.starts_with('-'))
                        {
                            let arg_signature_idx = argument_idx - flag_idx - 1;
                            // Even though the completion spec technically allows for a variadic arg to not be the last arg,
                            // it does not make sense and doesn't happen in practice, so we assume it's the last.
                            let arg_signature = if arg_signature_idx >= arguments.len()
                                && matches!(
                                    arguments_cardinality,
                                    FlagArgumentsCardinality::Variadic
                                ) {
                                arguments.last()
                            } else {
                                arguments.get(arg_signature_idx)
                            };
                            let (arg, err) =
                                parse_arg(&lite_cmd.parts[argument_idx], arg_signature);
                            named.insert_flag_with_argument(
                                full_name.clone(),
                                lite_cmd.parts[flag_idx].span,
                                arg,
                            );
                            error = error.or(err);
                            argument_idx += 1;
                        }

                        // If there were a fixed number of arguments for the option, make sure
                        // they were all exhausted. Otherwise, we're missing an arg.
                        if matches!(arguments_cardinality, FlagArgumentsCardinality::Fixed(_))
                            && argument_idx != end
                        {
                            error = error.or_else(|| {
                                Some(ParseError::argument_error(
                                    lite_cmd.parts[0].clone(),
                                    ArgumentError::MissingValueForName {
                                        name: full_name.spanned(lite_cmd.parts[flag_idx].span),
                                        missing_arg_index: argument_idx - flag_idx - 1,
                                    },
                                ))
                            });
                        }

                        // argument_idx here is one index overshoot of the last argument
                        // token. Set the current index to be the index of last argument.
                        idx = argument_idx - 1;

                        // We consumed the argument(s) for the option, so we should stop iterating the
                        // possible matching flags. This case shouldn't happen as CLIs don't
                        // generally support adding multiple flags with values in a single position
                        break;
                    }
                }
            } else {
                positional.push(
                    ParsedExpression::new(
                        Expression::Unknown,
                        ParsedToken(lite_cmd.parts[idx].item.clone()),
                    )
                    .spanned(lite_cmd.parts[idx].span),
                );

                error = error.or(err);
            }
        } else if !signature.arguments().is_empty()
            && signature.arguments().len() > current_positional
        {
            let arg = {
                let arg_signature = &signature.arguments()[current_positional];
                let (expr, err) = parse_arg(&lite_cmd.parts[idx], Some(arg_signature));

                error = error.or(err);
                expr
            };

            positional.push(arg);
            current_positional += 1;
        } else if let Some(arg_signature) = signature.arguments().iter().rfind(|a| a.is_variadic())
        {
            let (arg, err) = parse_arg(&lite_cmd.parts[idx], Some(arg_signature));
            error = error.or(err);

            positional.push(arg);
            current_positional += 1;
        } else {
            let expression = if lite_cmd.parts[idx].item.starts_with('$') {
                parse_dollar_expr(&lite_cmd.parts[idx])
            } else {
                ParsedExpression::new(
                    Expression::Unknown,
                    ParsedToken(lite_cmd.parts[idx].item.clone()),
                )
                .spanned(lite_cmd.parts[idx].span)
            };

            positional.push(expression);

            error = error.or_else(|| {
                Some(ParseError::argument_error(
                    lite_cmd.parts[0].clone(),
                    ArgumentError::UnexpectedArgument(lite_cmd.parts[idx].clone()),
                ))
            });
        }

        idx += 1;
    }

    if let Some(arguments) = &signature.arguments {
        // Count the required positional arguments and ensure these have been met
        let mut required_arg_count = 0;
        for positional_arg in arguments {
            if positional_arg.optional == IsArgumentOptional::Required {
                required_arg_count += 1;
            }
        }

        if positional.len() < required_arg_count {
            let arg = &arguments[positional.len()];
            error = error.or_else(|| {
                Some(ParseError::argument_error(
                    lite_cmd.parts[0].clone(),
                    ArgumentError::MissingMandatoryPositional {
                        name: arg.display_name.clone(),
                        positional_index: positional.len(),
                    },
                ))
            });
        }
    }

    if !named.is_empty() {
        internal_command.args.flags = Some(named);
    }

    if !positional.is_empty() {
        internal_command.args.positionals = Some(positional);
    }

    (internal_command, error)
}

impl From<&Opt> for FlagArgumentsCardinality {
    fn from(option: &Opt) -> Self {
        if option.arguments().iter().any(|arg| arg.is_variadic) {
            FlagArgumentsCardinality::Variadic
        } else {
            FlagArgumentsCardinality::Fixed(
                option
                    .arguments()
                    .iter()
                    .filter(|arg| arg.is_required())
                    .count(),
            )
        }
    }
}

fn get_flag_signature_spec<'a>(
    signature: &'a Signature,
    cmd: &'a ShellCommand,
    arg: &'a Spanned<String>,
) -> (Vec<FlagSignature<'a>>, Option<ParseError>) {
    // If it's not a flag, don't bother with it.
    if !arg.item.starts_with('-') {
        return (vec![], None);
    }

    let case_insensitive_flags = signature.parser_directives.always_case_insensitive;

    let mut flag = arg.item.as_str();
    let mut output = vec![];
    let mut error = None;

    if signature.parser_directives.flags_are_posix_noncompliant || flag.starts_with("--") {
        if let Some((flag_name, _value)) = flag.split_once('=') {
            flag = flag_name;
        }

        if signature.parser_directives.flags_match_unique_prefix {
            let all_prefix_matches = signature
                .options()
                .iter()
                .filter_map(|opt| {
                    opt.names()
                        .find(|name| {
                            if case_insensitive_flags {
                                name.to_lowercase()
                                    .starts_with(flag.to_lowercase().as_str())
                            } else {
                                name.starts_with(flag)
                            }
                        })
                        .map(|name| FlagSignature {
                            name: name.to_owned(),
                            is_switch: opt.is_switch(),
                            arguments_cardinality: FlagArgumentsCardinality::from(opt),
                            arguments: opt.arguments(),
                        })
                })
                .exactly_one();
            match all_prefix_matches {
                Ok(matched_flag) => output.push(matched_flag),
                Err(_) => {
                    error = Some(ParseError::argument_error(
                        cmd.name.to_string().spanned(cmd.name_span),
                        ArgumentError::UnexpectedFlag(arg.clone()),
                    ));
                }
            }
        } else if let Some(option) = signature.options().iter().find(|option| {
            if case_insensitive_flags {
                option
                    .names()
                    .map(str::to_lowercase)
                    .contains(&flag.to_lowercase())
            } else {
                option.names().contains(&flag)
            }
        }) {
            output.push(FlagSignature {
                name: flag.to_owned(),
                is_switch: option.is_switch(),
                arguments_cardinality: FlagArgumentsCardinality::from(option),
                arguments: option.arguments(),
            });
        } else {
            error = Some(ParseError::argument_error(
                cmd.name.to_string().spanned(cmd.name_span),
                ArgumentError::UnexpectedFlag(arg.clone()),
            ));
        }

    // Short flag(s) expected. They might be grouped, e.g. -Alh
    } else {
        let mut starting_pos = arg.span.start() + 1;
        // Loop over each letter as its own option, i.e. instead of "-Alh", process -A, -l, -h, etc.
        for c in flag.trim_start_matches('-').chars() {
            let ungrouped_flag = format!("-{c}");

            if let Some(option) = signature.options().iter().find(|option| {
                if case_insensitive_flags {
                    option
                        .names()
                        .map(str::to_lowercase)
                        .contains(&ungrouped_flag.to_lowercase())
                } else {
                    option.names().contains(&ungrouped_flag.as_str())
                }
            }) {
                // TODO(alokedesai): Check if we should be using short or long here
                output.push(FlagSignature {
                    name: ungrouped_flag,
                    is_switch: option.is_switch(),
                    arguments_cardinality: FlagArgumentsCardinality::from(option),
                    arguments: option.arguments(),
                });
            } else {
                error = Some(ParseError::argument_error(
                    cmd.name.to_string().spanned(cmd.name_span),
                    ArgumentError::UnexpectedFlag(
                        arg.item
                            .clone()
                            .spanned(Span::new(starting_pos, starting_pos + c.len_utf8())),
                    ),
                ));
            }

            starting_pos += c.len_utf8();
        }
    }

    (output, error)
}
