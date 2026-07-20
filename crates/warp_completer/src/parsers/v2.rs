//! Contains the V2 implementation of internal command parsing logic that depends on the new,
//! JS-compatible command signature struct (`crate::signatures::CommandSignature`).
use super::hir::{self, Expression, Flags, ShellCommand};
use super::{
    ArgumentError, FlagArgumentsCardinality, FlagSignature, LiteCommand, ParseError,
    ParsedExpression, ParsedToken, parse_arg, parse_dollar_expr, parse_unclassified_command,
};
use crate::completer::TopLevelCommandCaseSensitivity;
use crate::meta::{HasSpan, Span, Spanned, SpannedItem};
use crate::signatures::{
    Command, CommandRegistry, Opt, get_matching_signature_for_tokenized_input,
};

pub(super) fn parse_command(
    lite_cmd: &LiteCommand,
    tokens: &[&str],
    command_registry: &CommandRegistry,
    // TODO(CORE-2810)
    _command_case_sensitivity: TopLevelCommandCaseSensitivity,
) -> (Option<hir::Command>, Option<ParseError>) {
    let mut error: Option<ParseError> = None;

    if lite_cmd.parts.is_empty() {
        return (None, None);
    }

    if let Some((found_signature, token_index)) = get_matching_signature_for_tokenized_input(
        tokens,
        lite_cmd.post_whitespace.is_some(),
        command_registry,
    ) {
        let (internal_command, err) =
            parse_internal_command(lite_cmd, found_signature, token_index);

        error = error.or(err);
        return (Some(hir::Command::Classified(internal_command)), error);
    }

    let (command, error) = parse_unclassified_command(lite_cmd);
    (Some(command), error)
}

// This is a forked version of `super::parse_internal_command` that works with the V2 `CommandSignature` struct.
fn parse_internal_command(
    lite_command: &LiteCommand,
    command_signature: &Command,
    mut command_token_index: usize,
) -> (ShellCommand, Option<ParseError>) {
    log::debug!("parsing internal command {lite_command:?}");

    // This is a known internal command, so we need to work with the arguments and parse them according to the expected types
    let (name, name_span) = (
        lite_command.parts[0..(command_token_index + 1)]
            .iter()
            .map(|x| x.item.clone())
            .collect::<Vec<String>>()
            .join(" "),
        Span::new(
            lite_command.parts[0].span.start(),
            lite_command.parts[command_token_index].span.end(),
        ),
    );

    let mut internal_command = ShellCommand::new(ParsedToken(name), name_span, lite_command.span());
    internal_command.args.flags = Some(Flags::new());
    internal_command.args.ending_whitespace = lite_command.post_whitespace;

    let mut current_positional = 0;
    let mut named = Flags::new();
    let mut positional = vec![];
    let mut error = None;
    command_token_index += 1; // Start where the arguments begin

    while command_token_index < lite_command.parts.len() {
        if lite_command.parts[command_token_index]
            .item
            .starts_with('-')
            && lite_command.parts[command_token_index].item.len() > 1
        {
            let (named_types, err) = get_flag_signature_spec(
                command_signature,
                &internal_command,
                &lite_command.parts[command_token_index],
            );

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
                        named.insert_flag_with_no_argument(
                            full_name,
                            lite_command.parts[command_token_index].span,
                        );
                    } else if lite_command.parts[command_token_index].item.contains('=') {
                        // Self-contained option (--key=value)
                        let mut offset = 0;

                        let value = lite_command.parts[command_token_index]
                            .item
                            .chars()
                            .skip_while(|prop| {
                                offset += 1;
                                *prop != '='
                            })
                            .nth(1);

                        offset = if value.is_none() { offset - 1 } else { offset };

                        let flag_value = Span::new(
                            lite_command.parts[command_token_index].span.start() + offset,
                            lite_command.parts[command_token_index].span.end(),
                        );
                        let value = lite_command.parts[command_token_index].item[offset..]
                            .to_string()
                            .spanned(flag_value);
                        // We expect there to be exactly one arg in the case of a --key=value flag.
                        let arg_signature = arguments.first();
                        let (arg, err) = parse_arg(&value, arg_signature);
                        named.insert_flag_with_argument(
                            full_name,
                            lite_command.parts[command_token_index].span,
                            arg,
                        );

                        error = error.or(err);
                    } else if command_token_index == lite_command.parts.len() - 1 {
                        // Named argument with missing value
                        error = error.or_else(|| {
                            Some(ParseError::argument_error(
                                lite_command.parts[0].clone(),
                                ArgumentError::MissingValueForName {
                                    name: full_name
                                        .spanned(lite_command.parts[command_token_index].span),
                                    missing_arg_index: 0,
                                },
                            ))
                        });
                    } else {
                        // Named argument with following value(s).
                        // Since an option can have multiple arguments (any of which can be variadic),
                        // we should exhaust as many following args as possible.
                        let flag_idx = command_token_index;

                        let end = match arguments_cardinality {
                            FlagArgumentsCardinality::Variadic => lite_command.parts.len(),
                            FlagArgumentsCardinality::Fixed(num_args) => flag_idx + num_args + 1,
                        };
                        let mut argument_idx = command_token_index + 1;

                        // Exhaust as many args as we expect but stop early if we see another option.
                        // Note that since we are incrementing index again in the outer loop. Let's check
                        // boundary on the NEXT token rather than the current token.
                        while argument_idx < end.min(lite_command.parts.len())
                            && !lite_command
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
                                parse_arg(&lite_command.parts[argument_idx], arg_signature);
                            named.insert_flag_with_argument(
                                full_name.clone(),
                                lite_command.parts[flag_idx].span,
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
                                    lite_command.parts[0].clone(),
                                    ArgumentError::MissingValueForName {
                                        name: full_name
                                            .spanned(lite_command.parts[command_token_index].span),
                                        missing_arg_index: argument_idx - flag_idx - 1,
                                    },
                                ))
                            });
                        }

                        // argument_idx here is one index overshoot of the last argument
                        // token. Set the current index to be the index of last argument.
                        command_token_index = argument_idx - 1;

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
                        ParsedToken(lite_command.parts[command_token_index].item.clone()),
                    )
                    .spanned(lite_command.parts[command_token_index].span),
                );

                error = error.or(err);
            }
        } else if !command_signature.arguments.is_empty()
            && command_signature.arguments.len() > current_positional
        {
            let arg = {
                // TODO: pass v2 Argument signature to parse_arg which only accepts v1 Argument
                // let arg_signature = &command_signature.arguments[current_positional];
                let (expr, err) = parse_arg(&lite_command.parts[command_token_index], None);

                error = error.or(err);
                expr
            };

            positional.push(arg);
            current_positional += 1;
        } else if let Some(_arg_signature) = command_signature
            .arguments
            .iter()
            .rfind(|a| a.is_variadic())
        {
            // TODO: pass v2 Argument signature to parse_arg which only accepts v1 Argument
            let (arg, err) = parse_arg(&lite_command.parts[command_token_index], None);
            error = error.or(err);

            positional.push(arg);
            current_positional += 1;
        } else {
            let expression = if lite_command.parts[command_token_index]
                .item
                .starts_with('$')
            {
                parse_dollar_expr(&lite_command.parts[command_token_index])
            } else {
                ParsedExpression::new(
                    Expression::Unknown,
                    ParsedToken(lite_command.parts[command_token_index].item.clone()),
                )
                .spanned(lite_command.parts[command_token_index].span)
            };

            positional.push(expression);

            error = error.or_else(|| {
                Some(ParseError::argument_error(
                    lite_command.parts[0].clone(),
                    ArgumentError::UnexpectedArgument(
                        lite_command.parts[command_token_index].clone(),
                    ),
                ))
            });
        }

        command_token_index += 1;
    }

    let command_arguments = &command_signature.arguments;

    // Count the required positional arguments and ensure these have been met
    let mut required_arg_count = 0;
    for positional_arg in command_arguments.iter() {
        if !positional_arg.optional {
            required_arg_count += 1;
        }
    }

    if positional.len() < required_arg_count {
        let arg = &command_arguments[positional.len()];
        error = error.or_else(|| {
            Some(ParseError::argument_error(
                lite_command.parts[0].clone(),
                ArgumentError::MissingMandatoryPositional {
                    name: Some(arg.name.clone()),
                    positional_index: positional.len(),
                },
            ))
        });
    }

    if !named.is_empty() {
        internal_command.args.flags = Some(named);
    }

    if !positional.is_empty() {
        internal_command.args.positionals = Some(positional);
    }

    (internal_command, error)
}

fn get_flag_signature_spec<'a>(
    command_signature: &'a Command,
    cmd: &'a ShellCommand,
    arg: &'a Spanned<String>,
) -> (Vec<FlagSignature<'a>>, Option<ParseError>) {
    if arg.item.starts_with('-') {
        // It's a flag (or set of flags)
        let mut output = vec![];
        let mut error = None;

        let remainder: String = arg.item.chars().skip(1).collect();

        if remainder.starts_with('-') {
            // Long flag expected
            let mut remainder: String = remainder.chars().skip(1).collect();

            if remainder.contains('=') {
                let assignment: Vec<_> = remainder.split('=').collect();

                if assignment.len() != 2 {
                    error = Some(ParseError::argument_error(
                        cmd.name.to_string().spanned(cmd.name_span),
                        ArgumentError::InvalidValueForFlag(arg.clone()),
                    ));
                } else {
                    remainder = assignment[0].to_string();
                }
            }

            let mut found_remainder = false;
            for option in command_signature.options.iter() {
                if longhand_representations_for_opt(option).contains(&remainder) {
                    output.push(FlagSignature {
                        name: remainder.clone(),
                        is_switch: option.arguments.is_empty(),
                        // TODO: pass v2 Argument signature to parse_arg which only accepts v1 Argument
                        arguments: &[],
                        arguments_cardinality: FlagArgumentsCardinality::from(option),
                    });
                    found_remainder = true;
                }
            }

            if !found_remainder {
                error = Some(ParseError::argument_error(
                    cmd.name.to_string().spanned(cmd.name_span),
                    ArgumentError::UnexpectedFlag(arg.clone()),
                ));
            }
        } else {
            // Short flag(s) expected
            let mut starting_pos = arg.span.start() + 1;
            for c in remainder.chars() {
                let mut found = false;

                for option in command_signature.options.iter() {
                    if shorthand_representations_for_opt(option).contains(&c.to_string()) {
                        // TODO(alokedesai): Check if we should be using short or long here
                        output.push(FlagSignature {
                            name: c.to_string(),
                            is_switch: option.arguments.is_empty(),
                            // TODO: pass v2 Argument signature to parse_arg which only accepts v1 Argument
                            arguments: &[],
                            arguments_cardinality: FlagArgumentsCardinality::from(option),
                        });
                        found = true;
                    }
                }

                if !found {
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
    } else {
        // It's not a flag, so don't bother with it
        (vec![], None)
    }
}

impl From<&Opt> for FlagArgumentsCardinality {
    fn from(option: &Opt) -> Self {
        if option.arguments.iter().any(|arg| arg.is_variadic()) {
            FlagArgumentsCardinality::Variadic
        } else {
            FlagArgumentsCardinality::Fixed(
                option.arguments.iter().filter(|arg| !arg.optional).count(),
            )
        }
    }
}

fn shorthand_representations_for_opt(opt: &Opt) -> Vec<String> {
    opt.name
        .iter()
        .filter(|s| s.starts_with('-') && !s.starts_with("--"))
        .map(|s| s[1..].to_string())
        .collect()
}

fn longhand_representations_for_opt(opt: &Opt) -> Vec<String> {
    opt.name
        .iter()
        .filter(|s| s.starts_with("--"))
        .map(|s| s[2..].to_string())
        .collect()
}
