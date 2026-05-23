use warp_command_signatures::{Argument, ArgumentType, GeneratorName, Template, TemplateType};

use crate::meta::{HasSpan, Span, Spanned, SpannedItem};
use crate::parsers::{ParsedExpression, ParsedToken};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// The name goes up to the last subcommand.
    /// For example, it would be "git checkout" if there are any args after "checkout", not "git".
    pub name: String,
    pub name_span: Span,
    pub args: CommandCallInfo,
}

impl ShellCommand {
    pub fn new(name: ParsedToken, name_span: Span, full_span: Span) -> Self {
        let command_name = name.as_str().to_owned();
        let spanned_command = ParsedExpression::new(Expression::Command, name).spanned(name_span);
        Self {
            name: command_name,
            name_span,
            args: CommandCallInfo::new(spanned_command, full_span),
        }
    }

    /// Returns the last positional, if any, in the command.
    pub fn last_positional(&self) -> Option<Spanned<&ParsedToken>> {
        self.args
            .positionals
            .as_ref()
            .and_then(|positionals| positionals.last())
            .map(|expression| expression.item.value().spanned(expression.span))
    }

    /// Returns the name and span of the last named argument (if any) in the command.
    pub fn last_named_argument(&self) -> Option<Spanned<NamedArgument<'_>>> {
        self.args.flags.as_ref().and_then(|named_args| {
            named_args
                .iter()
                .filter_map(|flag| match &flag.flag_type {
                    FlagType::Argument {
                        value: parsed_expression,
                        ..
                    } => Some(
                        NamedArgument {
                            name: flag.name.as_str(),
                            parsed_token: parsed_expression.value(),
                        }
                        .spanned(parsed_expression.span),
                    ),
                    _ => None,
                })
                .fold(None, |acc, named_arg| match acc {
                    None => Some(named_arg),
                    Some(acc_named_arg) => {
                        if named_arg.span.end() > acc_named_arg.span.end() {
                            Some(named_arg)
                        } else {
                            Some(acc_named_arg)
                        }
                    }
                })
        })
    }
}

#[derive(Debug)]
pub struct NamedArgument<'a> {
    pub name: &'a str,
    pub parsed_token: &'a ParsedToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCommand {
    pub name: ParsedToken,
    pub name_span: Span,
    pub args: CommandCallInfo,
}

impl ExternalCommand {
    pub fn new(name: ParsedToken, name_span: Span, full_span: Span) -> Self {
        let command_name =
            ParsedExpression::new(Expression::Literal, name.clone()).spanned(full_span);
        Self {
            name,
            name_span,
            args: CommandCallInfo::new(command_name, full_span),
        }
    }
}

impl HasSpan for ExternalCommand {
    fn span(&self) -> Span {
        self.name_span.until(self.args.span)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A command that was successfully parsed via a signature in the `CommandRegistry`.
    Classified(ShellCommand),

    /// A command that is not in the `CommandRegistry`.
    Unclassified(ExternalCommand),
}

impl Command {
    pub fn command_name_span(&self) -> Spanned<&str> {
        match self {
            Command::Classified(shell_command) => {
                shell_command.name.as_str().spanned(shell_command.name_span)
            }
            Command::Unclassified(external_command) => external_command
                .name
                .as_str()
                .spanned(external_command.name_span),
        }
    }

    pub fn name_span(&self) -> Span {
        match self {
            Self::Classified(command) => command.name_span,
            Self::Unclassified(command) => command.name_span,
        }
    }

    pub fn last_token(&self) -> ParsedToken {
        let command_call_info = match self {
            Self::Classified(command) => &command.args,
            Self::Unclassified(command) => &command.args,
        };
        if command_call_info.ending_whitespace.is_some() {
            return ParsedToken::empty();
        }
        if let Some(positionals) = &command_call_info.positionals {
            if let Some(last) = positionals.last() {
                return last.parsed_token.clone();
            }
        }
        command_call_info.command_name.parsed_token.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCallInfo {
    pub command_name: Spanned<ParsedExpression>,
    pub positionals: Option<Vec<Spanned<ParsedExpression>>>,
    pub flags: Option<Flags>,
    /// Any additional whitespace at the end of the command.
    pub ending_whitespace: Option<Span>,
    pub span: Span,
}

impl CommandCallInfo {
    pub fn new(head: Spanned<ParsedExpression>, span: Span) -> CommandCallInfo {
        CommandCallInfo {
            command_name: head,
            positionals: None,
            flags: None,
            ending_whitespace: None,
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgType {
    File,
    Folder,
    Generator(GeneratorName),
}

impl ArgType {
    /// Returns a list of argument types we should validate.
    /// If there are any argument types that we haven't implemented validation for, returns empty vec
    /// because we should assume the argument is valid.
    pub fn get_arg_types_to_validate_from_arg_signature(arg_signature: &Argument) -> Vec<Self> {
        let mut arg_types = vec![];
        if arg_signature.skip_generator_validation {
            return arg_types;
        }
        for argument_type in arg_signature.argument_types.iter() {
            match argument_type {
                ArgumentType::Template(Template {
                    type_name: TemplateType::FilesAndFolders,
                    ..
                }) => arg_types.extend(vec![ArgType::File, ArgType::Folder]),
                ArgumentType::Template(Template {
                    type_name: TemplateType::Files { must_exist: true },
                    ..
                }) => arg_types.push(ArgType::File),
                ArgumentType::Template(Template {
                    type_name: TemplateType::Folders { must_exist: true },
                    ..
                }) => arg_types.push(ArgType::Folder),
                ArgumentType::Generator(generator_name) => {
                    arg_types.push(ArgType::Generator(generator_name.clone()))
                }
                // We don't want to validate against hard coded suggestions.
                ArgumentType::Suggestion(_) => (),
                // By the time we do arg validation, the command already has aliases expanded.
                // We don't need to do anything here.
                ArgumentType::Alias(_) => (),
                _ => {
                    // If we encounter an argument type that we haven't implemented validation for,
                    // we should assume the argument is valid.
                    // Return empty vec because there's no need to validate known argument types.
                    return vec![];
                }
            }
        }
        arg_types
    }
}

/// Identifies the expression type of a parsed token.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Expression {
    /// The expression is a literal value that will be interpreted by the command. This could be
    /// a subcommand, an argument, or a flag that will be assigned meaning once we tie it to a
    /// specific completion spec.
    Literal,
    /// The expression is an environment variable.
    Variable,
    /// The expression is a top-level command (such as `git`).
    Command,
    /// The expression is an argument that has been identified by the command signature that we have an
    /// exhaustive list of arg types to validate against.
    /// Contains the exhaustive list of arg types we should validate against, where at least one must pass validation.
    /// Note that if the argument can be a type that we don't have a validation for, it will be parsed as Expression::Literal
    /// instead.
    /// For example, in `git checkout {arg}` the arg can be a file/folder/git branch. If we don't have a git branch validation,
    /// the arg would be parsed as a Literal instead of ValidatableArgument([file, folder]) because the list is not
    /// exhaustive.
    ValidatableArgument(Vec<ArgType>),
    /// We were not able to identify what part of the command is.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagType {
    /// The flag requires no argument, such as `git --help`
    NoArgument,
    /// The flag has an argument, such as  `git checkout -b {BRANCH_NAME}`.
    Argument { value: Spanned<ParsedExpression> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    /// The name of the flag.
    pub name: String,
    pub name_span: Span,
    pub flag_type: FlagType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    pub(crate) flags: Vec<Flag>,
}

impl Flags {
    pub fn new() -> Flags {
        Default::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Flag> {
        self.flags.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

impl Flags {
    pub fn insert_flag_with_no_argument(&mut self, name: impl Into<String>, span: Span) {
        let name = name.into();
        self.flags.push(Flag {
            name,
            name_span: span,
            flag_type: FlagType::NoArgument,
        });
    }

    pub fn insert_flag_with_argument(
        &mut self,
        name: impl Into<String>,
        flag_span: Span,
        expr: Spanned<ParsedExpression>,
    ) {
        self.flags.push(Flag {
            name: name.into(),
            name_span: flag_span,
            flag_type: FlagType::Argument { value: expr },
        })
    }
}
