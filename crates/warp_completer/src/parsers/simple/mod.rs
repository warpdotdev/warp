use crate::meta::{Spanned, SpannedItem};

mod convert;
mod iter;
mod lexer;
mod parser;
mod token;

use lexer::Lexer;
use parser::Parser;
use string_offset::ByteOffset;
use warp_util::path::EscapeChar;

use crate::parsers::LiteCommand;

/// Parse the input and return the last unclosed command to complete on using the completions
/// infrastructure.
///
/// To make sure we are completing the correct thing, this will pull off the last unclosed command
/// from the parsed input
pub fn parse_for_completions<S: AsRef<str>>(
    source: S,
    escape_char: EscapeChar,
    parse_quotes_as_literals: bool,
) -> Option<LiteCommand> {
    let parser = Parser::new(Lexer::new(
        source.as_ref(),
        escape_char,
        parse_quotes_as_literals,
    ));
    let commands = parser.parse().commands;
    commands
        .into_iter()
        .next_back()
        .map(last_unclosed_command)
        .map(Into::into)
}

/// Returns the name of the top-level command in `source`.
///
/// For example, if the source is "PAGER=0 git log", the top-level command is "git".
/// If there are multiple top-level commands (e.g. `ls && git diff`), the first
/// one is returned.
pub fn top_level_command<S: AsRef<str>>(source: S, escape_char: EscapeChar) -> Option<String> {
    let parser = Parser::new(Lexer::new(source.as_ref(), escape_char, false));
    let mut command = parser.parse().commands.into_iter().next()?;
    command.item.remove_leading_env_vars();

    match command
        .parts
        .first()
        .map(|p: &Spanned<Part>| p.item.clone())
    {
        Some(Part::Literal(p)) => Some(p),
        _ => None,
    }
}

/// Parses the command to retrieve the command at the given cursor pos.
/// This is needed for command x-ray since we need to run the completion engine on tokens
/// that might be in the middle of a command or subcommand.
///
/// For example, if our command was `cd ~/Desktop $(cd ~/foo)` and our cursor
/// was at `/foo` then we want the subcommand `cd ~/foo` instead of the parent command.
pub fn command_at_cursor_position<S: AsRef<str>>(
    source: S,
    escape_char: EscapeChar,
    pos: ByteOffset,
) -> Option<LiteCommand> {
    let parser = Parser::new(Lexer::new(source.as_ref(), escape_char, false));
    let commands = parser.parse().commands;
    command_at_cursor(commands, pos).map(Into::into)
}

/// Parses the string to retrieve all commands (separated into iterator items).
/// This is needed for error underlining since we need to determine whether each
/// command is valid or not.
///
/// For example, if our command was `git commit && git log`, we would return
/// an iterator of 2 items - the parsed commands for "git commit" and "git log".
pub fn all_parsed_commands<S: AsRef<str>>(
    source: S,
    escape_char: EscapeChar,
) -> impl Iterator<Item = LiteCommand> {
    let parser = Parser::new(Lexer::new(source.as_ref(), escape_char, false));
    parser.parse().commands.into_iter().map(|mut cmd| {
        cmd.item.remove_leading_env_vars();
        cmd.into()
    })
}

/// Given a `command` string, returns:
/// 1. the subcommands that make it up, including the recomposed commands at each level of nesting.
///    For example, given "ls $(foo | echo)", this API returns ["foo", "echo", "foo | echo", "ls $(foo | echo)"]
/// 2. whether or not the `command` included any redirection operators (i.e. '>' or '<')
pub fn decompose_command(command: &str, escape_char: EscapeChar) -> (Vec<String>, bool) {
    let parser = Parser::new(Lexer::new(command, escape_char, false));
    let res = parser.parse();
    let (commands, contains_redirection) = (res.commands, res.contains_redirection);

    (
        commands
            .into_iter()
            .flat_map(|cmd| cmd.item.decompose(command))
            .collect(),
        contains_redirection,
    )
}

/// Retrieves the smallest complete command at a given pos.
///
/// Ex: For the dummy command "git status $(git stash) && git checkout main" -
/// pos 11 would return the subcommand corresponding to "git stash" and
/// pos 27 would return the command corresponding to "git checkout main"
/// Returns None if the pos is not in the range of the command.
fn command_at_cursor(commands: Vec<Spanned<Command>>, pos: ByteOffset) -> Option<Spanned<Command>> {
    // First search for the in range top level command
    if let Some(in_range_command) = commands.into_iter().find(|command| {
        command.span.start() <= pos.as_usize() && command.span.end() >= pos.as_usize()
    }) {
        if let Some(in_range_part) = in_range_command
            .item
            .parts
            .iter()
            .find(|part| part.span.start() <= pos.as_usize() && part.span.end() >= pos.as_usize())
            .cloned()
        {
            match in_range_part.item {
                Part::ClosedSubshell(inner) | Part::OpenSubshell(inner) if !inner.is_empty() => {
                    // If there's a subshell, recurse to find that in range command
                    if let Some(sub_command) = command_at_cursor(inner, pos) {
                        return Some(sub_command);
                    }
                }
                _ => {}
            }
        }
        return Some(in_range_command);
    }
    None
}

/// Retrieve the last unclosed command contained within a given command
///
/// Note: This may be the command itself, if it doesn't contain an open subshell
fn last_unclosed_command(mut command: Spanned<Command>) -> Spanned<Command> {
    // Check if the last part of the command is an OpenSubshell with at least one subcommand
    // If it is, we repeat the process with the final command of the subshell
    if let Some(final_part) = command.item.parts.pop() {
        match final_part.item {
            Part::OpenSubshell(mut inner) => {
                if !inner.is_empty() {
                    return last_unclosed_command(inner.pop().unwrap());
                } else {
                    command
                        .item
                        .parts
                        .push(Part::OpenSubshell(inner).spanned(final_part.span));
                }
            }
            _ => {
                command.item.parts.push(final_part);
            }
        }
    }

    command
}

/// A parsed command, made up of a number of parts.
///
/// Each part represents an argument, with the first part being the command itself
#[derive(Debug, PartialEq, Eq, Clone)]
struct Command {
    parts: Vec<Spanned<Part>>,
}

impl Command {
    pub fn new(parts: Vec<Spanned<Part>>) -> Self {
        Command { parts }
    }

    pub fn decompose(self, src: &str) -> Vec<String> {
        let this_command = self
            .parts
            .first()
            .zip(self.parts.last())
            .map(|(first, last)| src[first.span.start()..last.span.end()].trim().to_string());

        let mut all_commands = vec![];
        let mut this_command_has_literal = false;

        for part in self.parts {
            match part.item {
                Part::Literal(_) => {
                    this_command_has_literal = true;
                }
                Part::ClosedSubshell(s) | Part::OpenSubshell(s) => {
                    // Add the total subcommand as a command.
                    if let Some((first, last)) = s.first().zip(s.last()) {
                        all_commands
                            .push(src[first.span.start()..last.span.end()].trim().to_string());
                    }

                    // Recursively decompose each command in the subcommand.
                    all_commands.extend(s.into_iter().flat_map(|c| c.item.decompose(src)));
                }
                Part::Concatenated(s) => {
                    // Add the total concatenation as a command.
                    if let Some((first, last)) = s.first().zip(s.last()) {
                        all_commands
                            .push(src[first.span.start()..last.span.end()].trim().to_string());
                    }

                    // Recursively decompose each part in the concatenation.
                    let new_command = Command::new(s);
                    all_commands.extend(new_command.decompose(src));
                }
            }
        }

        // If this command has a literal, add it to the list.
        match this_command {
            Some(c) if this_command_has_literal => all_commands.push(c.to_string()),
            _ => {}
        }

        all_commands
    }

    /// Removes the leading env-var assignments (i.e. 'KEY=VALUE' literals) from the command.
    pub fn remove_leading_env_vars(&mut self) {
        while !self.parts.is_empty() {
            let Some(first_command_part) = self.parts.first() else {
                break;
            };
            if first_command_part.to_string().split('=').count() != 2 {
                break;
            }
            self.parts.remove(0);
        }
    }
}

/// An individual part of a command (argument or command name)
#[derive(Debug, PartialEq, Eq, Clone)]
enum Part {
    /// Raw string value
    Literal(String),

    /// Subshell call containing one or more commands that was properly closed
    ///
    /// For example, $(cat file.txt) or `ls -la`
    ClosedSubshell(Vec<Spanned<Command>>),

    /// Subshell call that was _not_ properly closed in the input
    ///
    /// For example, $(cat file.txt or `ls -la
    OpenSubshell(Vec<Spanned<Command>>),

    /// Concatenation of several literal and/or subshell parts
    ///
    /// For example, Hello"World" would be a concatenation of /Hello/ and /"World"/ as individual
    /// parts
    Concatenated(Vec<Spanned<Part>>),
}
