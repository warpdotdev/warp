//! Extracts the files a shell command appears to be writing to.
//!
//! A command that redirects its output to a file produces no terminal output, so
//! the file itself is the only place its progress is visible. This module finds
//! those files from the command text alone; it never touches the filesystem, so
//! it stays a pure function that is cheap to call and easy to test.

use std::collections::HashSet;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;

/// Maximum number of redirect targets tracked for a single command. Commands
/// that write to more files than this are rare, and each tracked file costs a
/// `stat` per sample.
pub const MAX_TRACKED_FILES: usize = 4;

/// Flags whose argument names an output file.
const OUTPUT_FLAGS: &[&str] = &[
    "-o",
    "-O",
    "--output",
    "--log-file",
    "--logfile",
    "--output-file",
];

/// Paths that are never worth tracking because they cannot grow.
const IGNORED_PATHS: &[&str] = &[
    "/dev/null",
    "/dev/stdout",
    "/dev/stderr",
    "/dev/tty",
    "nul",
    "NUL",
];

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    /// An output redirection operator (`>`, `>>`, `2>`, `&>`, …).
    RedirectOut,
    /// A separator that ends the current simple command (`|`, `;`, `&&`, …).
    Separator,
}

/// Returns the files `command` appears to write to, resolved against `cwd`.
///
/// Only syntactic evidence is used: output redirections, `tee` arguments, and
/// conventional output flags. The result is deduplicated, capped at
/// [`MAX_TRACKED_FILES`], and never includes sinks like `/dev/null`.
pub fn parse_redirect_targets(command: &str, cwd: Option<&Path>) -> Vec<PathBuf> {
    let tokens = tokenize(command);

    let mut targets: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |raw: &str, targets: &mut Vec<String>| {
        if is_ignored_target(raw) {
            return;
        }
        if seen.insert(raw.to_owned()) {
            targets.push(raw.to_owned());
        }
    };

    // Set while walking the arguments of a `tee`, which writes to every path
    // argument it is given rather than to a single flag-named file.
    let mut in_tee_args = false;

    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Separator => in_tee_args = false,
            Token::RedirectOut => {
                if let Some(Token::Word(target)) = tokens.get(index + 1) {
                    push(target, &mut targets);
                    index += 1;
                }
            }
            Token::Word(word) => {
                if is_command_name(word, "tee") {
                    in_tee_args = true;
                } else if let Some(flag_value) = output_flag_value(word) {
                    push(&flag_value, &mut targets);
                } else if OUTPUT_FLAGS.contains(&word.as_str()) {
                    if let Some(Token::Word(target)) = tokens.get(index + 1) {
                        push(target, &mut targets);
                        index += 1;
                    }
                } else if in_tee_args && !word.starts_with('-') {
                    push(word, &mut targets);
                }
            }
        }
        index += 1;
    }

    targets
        .into_iter()
        .map(|target| resolve(&target, cwd))
        .take(MAX_TRACKED_FILES)
        .collect()
}

/// Whether `word` invokes `name`, allowing for an absolute path (`/usr/bin/tee`).
fn is_command_name(word: &str, name: &str) -> bool {
    word == name || word.rsplit('/').next() == Some(name) && word.contains('/')
}

/// The value of a `--flag=value` style output flag, if `word` is one.
fn output_flag_value(word: &str) -> Option<String> {
    let (flag, value) = word.split_once('=')?;
    (OUTPUT_FLAGS.contains(&flag) && !value.is_empty()).then(|| value.to_owned())
}

fn is_ignored_target(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('-')
        // Unexpanded globs and substitutions can't be resolved to a real path.
        || target.contains(['*', '?', '$', '`'])
        || IGNORED_PATHS.contains(&target)
}

fn resolve(target: &str, cwd: Option<&Path>) -> PathBuf {
    let path = Path::new(target);
    match cwd {
        Some(cwd) if path.is_relative() => cwd.join(path),
        _ => path.to_path_buf(),
    }
}

/// Splits a command into words, output-redirection operators, and separators.
///
/// This is deliberately not a full shell parser: it understands quoting well
/// enough to keep filenames intact, and treats everything it does not recognize
/// as an ordinary word.
fn tokenize(command: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut chars = command.chars().peekable();

    // Emits any word accumulated so far. Declared as a closure over locals would
    // borrow them for the whole loop, so this stays a macro-free helper call.
    fn flush(word: &mut String, tokens: &mut Vec<Token>) {
        if !word.is_empty() {
            tokens.push(Token::Word(std::mem::take(word)));
        }
    }

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    word.push(escaped);
                }
            }
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    word.push(quoted);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                word.push(escaped);
                            }
                        }
                        _ => word.push(quoted),
                    }
                }
            }
            ch if ch.is_whitespace() => flush(&mut word, &mut tokens),
            '>' => {
                // A leading fd number (`2>`) or `&` (`&>`) belongs to the operator,
                // not to the preceding word.
                if word == "&" || word.chars().all(|c| c.is_ascii_digit()) {
                    word.clear();
                } else {
                    flush(&mut word, &mut tokens);
                }
                // `>>` appends and `>|` clobbers; both still name a file.
                while matches!(chars.peek(), Some('>') | Some('|')) {
                    chars.next();
                }
                // `>&` duplicates a descriptor when a descriptor follows (`2>&1`),
                // but names a file in the csh-style `>&log` form.
                if chars.peek() == Some(&'&') {
                    chars.next();
                    if consume_descriptor(&mut chars) {
                        continue;
                    }
                }
                tokens.push(Token::RedirectOut);
            }
            '<' => {
                flush(&mut word, &mut tokens);
                // Input redirection; skip the operator and let its target be read
                // as an ordinary word, which no rule above will match.
                while matches!(chars.peek(), Some('<') | Some('&')) {
                    chars.next();
                }
            }
            '|' | ';' | '&' => {
                // `&` only separates when it isn't the start of an `&>` operator,
                // which the `'>'` arm handles via the accumulated word.
                if ch == '&' && chars.peek() == Some(&'>') {
                    word.push(ch);
                    continue;
                }
                flush(&mut word, &mut tokens);
                while chars.peek() == Some(&ch) {
                    chars.next();
                }
                tokens.push(Token::Separator);
            }
            _ => word.push(ch),
        }
    }
    flush(&mut word, &mut tokens);

    tokens
}

/// Consumes the descriptor reference following a `>&`, reporting whether one
/// was there. `-` closes the descriptor; digits duplicate it.
fn consume_descriptor(chars: &mut Peekable<Chars>) -> bool {
    match chars.peek() {
        Some('-') => {
            chars.next();
            true
        }
        Some(ch) if ch.is_ascii_digit() => {
            while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                chars.next();
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "lrc_redirect_tests.rs"]
mod tests;
