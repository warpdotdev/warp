#[cfg_attr(feature = "v2", path = "v2.rs")]
#[cfg_attr(not(feature = "v2"), path = "legacy.rs")]
mod imp;
#[cfg(not(feature = "v2"))]
pub use imp::SignatureAtTokenIndex;
use imp::*;

mod errors;
pub use errors::{ArgumentError, ParseError, ParseErrorReason};
pub mod hir;
pub mod simple;

use derive_new::new;
use hir::{ArgType, Command, Expression, ExternalCommand};
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use warp_command_signatures::Argument;

use crate::completer::TopLevelCommandCaseSensitivity;
use crate::meta::{HasSpan, Span, Spanned, SpannedItem};
use crate::signatures::CommandRegistry;

lazy_static! {
    // Regex to test for a valid environment variable name, Environment variable names used by the
    // utilities in the Shell and Utilities volume of IEEE Std 1003.1-2001 consist solely of
    // upper or lowercase letters, digits, and the '_' (underscore) from the characters defined in
    // Portable Character Set and do not begin with a digit.
    static ref ENV_VAR_NAME_REGEX: Regex = Regex::new("^[$][a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
}

type SpannedKeyValue = (Spanned<String>, Spanned<String>);

/// A `LitePipeline` is a series of `LiteCommand`s separated by `|`.
#[derive(Debug, Clone, new)]
pub struct LitePipeline {
    pub commands: Vec<LiteCommand>,
}

impl HasSpan for LitePipeline {
    fn span(&self) -> Span {
        Span::from_list(&self.commands)
    }
}

/// A `LiteCommand` is a list of words that will get meaning when processed by
/// the parser.
#[derive(Debug, Default, Clone)]
pub struct LiteCommand {
    pub parts: Vec<Spanned<String>>,
    /// The pieces of the command that ended in a whitespace.
    pub post_whitespace: Option<Span>,
}

impl LiteCommand {
    /// Returns `parts` joined together by single spaces.
    pub fn joined_by_space(&self) -> String {
        self.parts.iter().map(|s| s.as_str()).join(" ")
    }
}

impl HasSpan for LiteCommand {
    fn span(&self) -> Span {
        let span = Span::from_list(&self.parts);
        self.post_whitespace
            .as_ref()
            .map(|whitespace| span.until(whitespace))
            .unwrap_or(span)
    }
}

/// A `LiteGroup` is a series of `LitePipeline`s, separated by `;`.
#[derive(Debug, Clone, new)]
pub struct LiteGroup {
    pub pipelines: Vec<LitePipeline>,
}

impl HasSpan for LiteGroup {
    fn span(&self) -> Span {
        Span::from_list(&self.pipelines)
    }
}

/// A `LiteRootNode` is the root node of the parsed AST. Essentially a series of `LiteGroup`s,
/// separated by newlines.
#[derive(Debug, Clone, new)]
pub struct LiteRootNode {
    pub groups: Vec<LiteGroup>,
}

impl HasSpan for LiteRootNode {
    fn span(&self) -> Span {
        Span::from_list(&self.groups)
    }
}

/// A command classified by its arguments, flags, etc. Optionally includes an error if there was
/// a parse error while trying to classify the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedCommand {
    pub command: Command,
    /// Environment variables specified on the command line.
    /// Each entry looks like "KEY=VALUE"
    pub env_vars: Vec<String>,
    pub error: Option<ParseError>,
}

/// Converts a `LiteCommand` into a `Command` that is annotated with its positional arguments,
/// flags, etc based on corresponding completion specs.
/// Modifies `tokens` to remove any environment variables.
/// Returns none if the command is unable to
/// be classified.
pub fn classify_command(
    lite_command: LiteCommand,
    tokens: &mut Vec<&str>,
    command_registry: &CommandRegistry,
    command_case_sensitivity: TopLevelCommandCaseSensitivity,
) -> Option<ClassifiedCommand> {
    let mut error = None;

    // TODO: potentially change to mutable reference (i.e. expand_shorthand_forms
    // directly mutates lite_command)?
    let (lite_command, vars, err) = expand_shorthand_forms(lite_command);
    if error.is_none() {
        error = err;
    }
    // Note that the caller of expand_shorthand_forms is responsible for updating
    // their version of tokens, given the output of variables in environment
    // variable assignments.
    let env_vars = tokens
        .drain(0..vars.len())
        .map(|s| s.to_string())
        .collect_vec();

    let (command, err) = parse_command(
        &lite_command,
        tokens,
        command_registry,
        command_case_sensitivity,
    );

    if error.is_none() {
        error = err;
    }

    command.map(|command| ClassifiedCommand {
        command,
        env_vars,
        error,
    })
}

fn parse_arg(
    lite_arg: &Spanned<String>,
    arg_signature: Option<&Argument>,
) -> (Spanned<ParsedExpression>, Option<ParseError>) {
    if lite_arg.item == "$" || ENV_VAR_NAME_REGEX.is_match(lite_arg.item.as_str()) {
        return (parse_dollar_expr(lite_arg), None);
    }
    let arg_types_to_validate =
        arg_signature.map(ArgType::get_arg_types_to_validate_from_arg_signature);
    if let Some(arg_types_to_validate) = arg_types_to_validate
        && !arg_types_to_validate.is_empty()
    {
        return (
            ParsedExpression::new(
                Expression::ValidatableArgument(arg_types_to_validate),
                ParsedToken(lite_arg.item.clone()),
            )
            .spanned(lite_arg.span),
            None,
        );
    }
    (
        ParsedExpression::new(Expression::Literal, ParsedToken(lite_arg.item.clone()))
            .spanned(lite_arg.span),
        None,
    )
}

fn trim_quotes(input: &str) -> String {
    let mut chars = input.chars();

    match (chars.next(), chars.next_back()) {
        (Some('\''), Some('\'')) => chars.collect(),
        (Some('"'), Some('"')) => chars.collect(),
        (Some('`'), Some('`')) => chars.collect(),
        _ => input.to_string(),
    }
}

/// Given a lite command and its tokens, remove the environment variable assignment token
/// from it for completion generation. Note that the caller should mutate their tokens
/// according to the vector of variables returned (remove them from the start)!
///
/// TODO: consolidate with [`Command::remove_leading_env_vars`].
fn expand_shorthand_forms(
    mut lite_command: LiteCommand,
) -> (LiteCommand, Vec<SpannedKeyValue>, Option<ParseError>) {
    let mut vars = Vec::new();
    while !lite_command.parts.is_empty() {
        let first_command_part = match lite_command.parts.first() {
            Some(part) if part.item != "=" && part.contains('=') => part.clone(),
            _ => return (lite_command, vars, None),
        };

        let assignment: Vec<_> = first_command_part.split('=').collect();
        if assignment.len() != 2 {
            return (
                lite_command,
                vars,
                Some(ParseError::mismatch(
                    "environment variable assignment",
                    first_command_part.clone(),
                )),
            );
        } else {
            let original_span = first_command_part.span;
            let (variable_name, value) = (assignment[0], trim_quotes(assignment[1]));

            lite_command.parts.remove(0);
            vars.push((
                variable_name.to_string().spanned(original_span),
                value.spanned(original_span),
            ));
        }
    }
    (lite_command, vars, None)
}

/// Parses a command that is not in the command registry.
fn parse_unclassified_command(lite_cmd: &LiteCommand) -> (Command, Option<ParseError>) {
    let mut error = None;

    let external_name = lite_cmd.parts[0].clone().map(|v| trim_quotes(&v));

    let mut external_command = ExternalCommand::new(
        ParsedToken(external_name.item),
        external_name.span,
        lite_cmd.span(),
    );
    external_command.args.ending_whitespace = lite_cmd.post_whitespace;

    let num_parts = lite_cmd.parts.len() - 1;
    let mut args = Vec::with_capacity(num_parts);

    if lite_cmd.parts.len() > 1 {
        for lite_arg in &lite_cmd.parts[1..] {
            let (expr, err) = parse_arg(lite_arg, None);
            if error.is_none() {
                error = err;
            }
            args.push(expr);
        }

        external_command.args.positionals = Some(args);
    }

    (Command::Unclassified(external_command), error)
}

/// The number of arguments for an option.
#[derive(Clone, Debug)]
enum FlagArgumentsCardinality {
    /// The option has exactly k required args.
    Fixed(usize),
    /// The option has some arg that is variadic
    /// (it might also have some number of non-variadic args).
    Variadic,
}

#[derive(Clone, Debug)]
struct FlagSignature<'a> {
    name: String,
    is_switch: bool,
    arguments_cardinality: FlagArgumentsCardinality,
    arguments: &'a [Argument],
}

fn parse_dollar_expr(lite_arg: &Spanned<String>) -> Spanned<ParsedExpression> {
    ParsedExpression::new(Expression::Variable, ParsedToken(lite_arg.item.clone()))
        .spanned(lite_arg.span)
}

/// Newtype denoting a token that has been parsed by the parser. For example, `foo\ bar` would have
/// would result in a parsed token value of `foo bar`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParsedToken(String);

impl ParsedToken {
    #[cfg(feature = "test-util")]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn empty() -> Self {
        Self("".into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// An expression with the parsed value that the expression corresponds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExpression {
    expression: Expression,
    parsed_token: ParsedToken,
}

impl ParsedExpression {
    pub fn new(expression: Expression, parsed_token: ParsedToken) -> Self {
        Self {
            expression,
            parsed_token,
        }
    }

    pub fn expression(&self) -> &Expression {
        &self.expression
    }

    pub fn value(&self) -> &ParsedToken {
        &self.parsed_token
    }
}

#[cfg(test)]
mod test;
