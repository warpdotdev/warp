//! Credits file contents the model verifiably observed through completed shell
//! commands into [`ObservedFileContents`], so a later `create_file` over the
//! same path counts as an informed overwrite.
//!
//! Two command shapes are recognized: whole-file reads (`cat <path>`) and
//! single-target writes (`> path`, `cat > path << EOF`, `| tee path`). Parsing
//! alone never credits anything: the file's on-disk content must be verifiably
//! known to the model — byte-equal to the command output for reads, and for
//! writes byte-equal to the output or contained in the heredoc body the model
//! wrote — so partial reads, transformed output, and writes of computed content
//! are never credited.

use std::borrow::Cow;

use warpui::{AppContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::observed_file_contents::{ContentFingerprint, ObservedFileContents};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::ShellLaunchData;

/// Size cap checked against file metadata before the file is read. Skipping the read
/// avoids slurping a large file only to run a comparison that is nearly certain to fail.
const MAX_CREDITED_FILE_BYTES: u64 = 1024 * 1024;

/// A file-content observation implied by a completed shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShellFileObservation {
    /// The command dumped a single file's full content to the terminal.
    WholeFileRead { path: String },
    /// The command wrote a single file whose final content the model may know
    /// verbatim.
    Write { path: String },
}

impl ShellFileObservation {
    fn path(&self) -> &str {
        match self {
            Self::WholeFileRead { path } | Self::Write { path } => path,
        }
    }
}

/// Records a fingerprint of the observed file's on-disk content for
/// `conversation_id` when a completed local shell command verifiably exposed
/// that exact content to the model.
pub(crate) fn credit_command_file_observations(
    conversation_id: AIConversationId,
    command: &str,
    output: &str,
    shell: &Option<ShellLaunchData>,
    current_working_directory: &Option<String>,
    app: &mut AppContext,
) {
    if !app.has_singleton_model::<ObservedFileContents>() {
        return;
    }
    let Some(observation) = parse_shell_file_observation(command) else {
        return;
    };
    let absolute_path =
        host_native_absolute_path(observation.path(), shell, current_working_directory);
    match std::fs::metadata(&absolute_path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_CREDITED_FILE_BYTES => {}
        _ => return,
    }
    let Ok(disk_content) = std::fs::read_to_string(&absolute_path) else {
        return;
    };
    if !confirms_disk_content(&observation, command, output, &disk_content) {
        return;
    }
    let fingerprint = ContentFingerprint::of(&disk_content);
    ObservedFileContents::handle(app).update(app, |model, _| {
        model.record(conversation_id, absolute_path, fingerprint);
    });
}

/// Whether the completed command verifiably put the file's exact on-disk
/// content in front of the model.
fn confirms_disk_content(
    observation: &ShellFileObservation,
    command: &str,
    output: &str,
    disk_content: &str,
) -> bool {
    let disk = normalize_newlines(disk_content);
    let trimmed_disk = disk.trim_end_matches('\n');
    if trimmed_disk.is_empty() {
        return false;
    }
    let output = normalize_newlines(output);
    match observation {
        ShellFileObservation::WholeFileRead { .. } => output.trim_end_matches('\n') == trimmed_disk,
        // The model knows the written bytes when they came back out (`tee`) or
        // when it spelled them out in a heredoc. Content computed by the
        // command (e.g. `ls > f`) matches neither and is never credited.
        ShellFileObservation::Write { .. } => {
            output.trim_end_matches('\n') == trimmed_disk
                || heredoc_body(command)
                    .is_some_and(|body| normalize_newlines(body).contains(trimmed_disk))
        }
    }
}

/// The heredoc body of a write command: every line after the first, which is
/// the only part of a command the model spells out verbatim.
///
/// Searching the whole command instead would credit any file whose content
/// coincides with a substring of it — including the redirect target itself, as
/// in `ls > out.txt` listing only `out.txt`.
fn heredoc_body(command: &str) -> Option<&str> {
    let (first_line, body) = command.split_once('\n')?;
    let tokens = tokenize(first_line)?;
    tokens
        .iter()
        .any(|token| matches!(token, Token::Heredoc))
        .then_some(body)
}

/// Normalizes CRLF to LF, mirroring [`ContentFingerprint::of`] and the
/// file-read tooling.
fn normalize_newlines(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Parses a command into the file observation it implies, if any.
///
/// Only the first line is parsed as command syntax; later lines are permitted
/// only as a heredoc body. Commands using constructs the parser does not model
/// (unterminated quotes, expansions in the target path, multiple or appending
/// write targets, fd or combined redirects) yield `None`.
pub(crate) fn parse_shell_file_observation(command: &str) -> Option<ShellFileObservation> {
    let (first_line, rest) = match command.split_once('\n') {
        Some((first_line, rest)) => (first_line, Some(rest)),
        None => (command, None),
    };
    let tokens = tokenize(first_line)?;
    let has_heredoc = tokens.iter().any(|token| matches!(token, Token::Heredoc));
    if rest.is_some() && !has_heredoc {
        return None;
    }

    if let Some(path) = parse_whole_file_read(&tokens) {
        return Some(ShellFileObservation::WholeFileRead { path });
    }
    parse_single_write_target(&tokens).map(|path| ShellFileObservation::Write { path })
}

/// Matches `cat [flags] <path>` with no operators and exactly one operand.
fn parse_whole_file_read(tokens: &[Token]) -> Option<String> {
    let mut words = Vec::new();
    for token in tokens {
        match token {
            Token::Word {
                text,
                has_unquoted_special,
            } => words.push((text, *has_unquoted_special)),
            _ => return None,
        }
    }
    let ((first, _), rest) = words.split_first()?;
    if *first != "cat" {
        return None;
    }
    let operands: Vec<_> = rest
        .iter()
        .filter(|(text, _)| !text.starts_with('-'))
        .collect();
    match operands[..] {
        [(path, false)] if !path.is_empty() => Some((*path).clone()),
        _ => None,
    }
}

/// Finds the single stdout write target of the command: a `>` redirect or a
/// piped `tee`. Appends, fd/combined redirects, expansions in the target, and
/// multiple targets disqualify the command.
fn parse_single_write_target(tokens: &[Token]) -> Option<String> {
    let mut targets = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            Token::RedirectOut { append } => {
                if *append {
                    return None;
                }
                let Some(Token::Word {
                    text,
                    has_unquoted_special: false,
                }) = tokens.get(index + 1)
                else {
                    return None;
                };
                targets.push(text.clone());
                index += 2;
            }
            Token::Pipe => {
                index += 1;
                let is_tee =
                    matches!(tokens.get(index), Some(Token::Word { text, .. }) if text == "tee");
                if !is_tee {
                    continue;
                }
                index += 1;
                while let Some(Token::Word {
                    text,
                    has_unquoted_special,
                }) = tokens.get(index)
                {
                    if text.starts_with('-') {
                        // Appending makes the final content unknowable from
                        // this command alone.
                        if text == "-a" || text == "--append" {
                            return None;
                        }
                    } else {
                        if *has_unquoted_special {
                            return None;
                        }
                        targets.push(text.clone());
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    match &targets[..] {
        [path] if !path.is_empty() => Some(path.clone()),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word {
        text: String,
        /// Whether the word contains an unescaped expansion or glob character,
        /// making its literal value unknowable without evaluating the shell.
        has_unquoted_special: bool,
    },
    Pipe,
    /// `;`, `&&`, `||`, a lone `&`, or a subshell paren: a command boundary.
    Separator,
    /// `>` or `>>` writing stdout (no fd prefix, or fd 1).
    RedirectOut {
        append: bool,
    },
    /// A redirect that does not write stdout alone: `2>`, `&>`, `<`, `<<<`.
    OtherRedirect,
    /// `<<`: the remainder of the command is a heredoc body.
    Heredoc,
}

/// Splits a single command line into quote-aware words and operators.
/// Returns `None` for lines the tokenizer cannot model (unterminated quotes).
fn tokenize(line: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    let mut word = String::new();
    let mut word_started = false;
    let mut has_unquoted_special = false;

    fn flush(
        tokens: &mut Vec<Token>,
        word: &mut String,
        word_started: &mut bool,
        has_unquoted_special: &mut bool,
    ) {
        if *word_started {
            tokens.push(Token::Word {
                text: std::mem::take(word),
                has_unquoted_special: *has_unquoted_special,
            });
            *word_started = false;
            *has_unquoted_special = false;
        }
    }

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                word_started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => word.push(inner),
                        None => return None,
                    }
                }
            }
            '"' => {
                word_started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped) => word.push(escaped),
                            None => return None,
                        },
                        Some(inner @ ('$' | '`')) => {
                            has_unquoted_special = true;
                            word.push(inner);
                        }
                        Some(inner) => word.push(inner),
                        None => return None,
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    word_started = true;
                    word.push(escaped);
                }
            }
            ' ' | '\t' => flush(
                &mut tokens,
                &mut word,
                &mut word_started,
                &mut has_unquoted_special,
            ),
            '|' => {
                flush(
                    &mut tokens,
                    &mut word,
                    &mut word_started,
                    &mut has_unquoted_special,
                );
                if chars.peek() == Some(&'|') {
                    chars.next();
                    tokens.push(Token::Separator);
                } else {
                    tokens.push(Token::Pipe);
                }
            }
            ';' | '(' | ')' => {
                flush(
                    &mut tokens,
                    &mut word,
                    &mut word_started,
                    &mut has_unquoted_special,
                );
                tokens.push(Token::Separator);
            }
            '&' => {
                flush(
                    &mut tokens,
                    &mut word,
                    &mut word_started,
                    &mut has_unquoted_special,
                );
                match chars.peek() {
                    Some('&') => {
                        chars.next();
                        tokens.push(Token::Separator);
                    }
                    Some('>') => {
                        chars.next();
                        if chars.peek() == Some(&'>') {
                            chars.next();
                        }
                        tokens.push(Token::OtherRedirect);
                    }
                    _ => tokens.push(Token::Separator),
                }
            }
            '>' => {
                // An all-digit word attached directly to `>` is an fd redirect
                // (`2> err.log`); of those, only fd 1 writes stdout.
                let attached_digits = word_started
                    && !word.is_empty()
                    && word.chars().all(|digit| digit.is_ascii_digit());
                let fd_redirect = attached_digits && word != "1";
                if attached_digits {
                    // The digits belong to the redirect, not to an operand.
                    word.clear();
                    word_started = false;
                    has_unquoted_special = false;
                }
                flush(
                    &mut tokens,
                    &mut word,
                    &mut word_started,
                    &mut has_unquoted_special,
                );
                let append = chars.peek() == Some(&'>');
                if append {
                    chars.next();
                }
                if fd_redirect {
                    tokens.push(Token::OtherRedirect);
                } else {
                    tokens.push(Token::RedirectOut { append });
                }
            }
            '<' => {
                flush(
                    &mut tokens,
                    &mut word,
                    &mut word_started,
                    &mut has_unquoted_special,
                );
                if chars.peek() == Some(&'<') {
                    chars.next();
                    if chars.peek() == Some(&'<') {
                        chars.next();
                        tokens.push(Token::OtherRedirect);
                    } else {
                        if chars.peek() == Some(&'-') {
                            chars.next();
                        }
                        tokens.push(Token::Heredoc);
                    }
                } else {
                    tokens.push(Token::OtherRedirect);
                }
            }
            '$' | '`' | '*' | '?' | '[' => {
                word_started = true;
                has_unquoted_special = true;
                word.push(c);
            }
            _ => {
                word_started = true;
                word.push(c);
            }
        }
    }
    flush(
        &mut tokens,
        &mut word,
        &mut word_started,
        &mut has_unquoted_special,
    );
    Some(tokens)
}

#[cfg(test)]
#[path = "shell_file_observations_tests.rs"]
mod tests;
