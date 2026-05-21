use std::{iter::Peekable, mem, str::Chars};

use itertools::Itertools;

/// Parses a query into tokens, taking into account delimiters.
pub fn parse_query_into_tokens(query: &str) -> Vec<String> {
    let parser = SentenceParser {
        chars: query.chars().peekable(),
        active_delimiter: None,
        active_token: String::new(),
    };

    parser.into_iter().collect_vec()
}

#[derive(PartialEq, Eq)]
enum WordDelimiter {
    Separator,
    DoubleQuote,
    SingleQuote,
    Backtick,
    Whitespace,
}

fn convert_char_to_delimiter(c: char) -> Option<WordDelimiter> {
    match c {
        '\'' => Some(WordDelimiter::SingleQuote),
        '"' => Some(WordDelimiter::DoubleQuote),
        '`' => Some(WordDelimiter::Backtick),
        ',' | '.' | '!' | '?' => Some(WordDelimiter::Separator),
        c if c.is_whitespace() => Some(WordDelimiter::Whitespace),
        _ => None,
    }
}

/// Parse a sentence into tokens for natural language classification.
/// The main difference from the default "split_whitespace" parser is
/// for matching double quotes and single quotes, we would keep their
/// closed content as one token.
struct SentenceParser<'a> {
    chars: Peekable<Chars<'a>>,
    active_delimiter: Option<WordDelimiter>,
    active_token: String,
}

impl Iterator for SentenceParser<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = self.chars.next() {
            let delimiter = convert_char_to_delimiter(c);
            let next_delimiter = self.chars.peek().map(|c| convert_char_to_delimiter(*c));

            match delimiter {
                Some(WordDelimiter::Whitespace) if self.active_delimiter.is_none() => {
                    if self.active_token.is_empty() {
                        continue;
                    }

                    return Some(mem::take(&mut self.active_token));
                }
                Some(WordDelimiter::Separator) if self.active_delimiter.is_none() => {
                    if self.active_token.is_empty() {
                        continue;
                    }

                    // If next_delimiter is not whitespace or None, this means the delimiter
                    // is in the middle of a word. In this case, push the plain text character
                    // to the active token.
                    if next_delimiter
                        .map(|c| c == Some(WordDelimiter::Whitespace))
                        .unwrap_or(true)
                    {
                        return Some(mem::take(&mut self.active_token));
                    } else {
                        self.active_token.push(c);
                    }
                }
                Some(WordDelimiter::DoubleQuote) => {
                    let complete_quote =
                        if self.active_delimiter == Some(WordDelimiter::DoubleQuote) {
                            self.active_delimiter = None;
                            true
                        } else if !self.active_token.is_empty() || self.active_delimiter.is_some() {
                            false
                        } else {
                            self.active_delimiter = Some(WordDelimiter::DoubleQuote);
                            false
                        };

                    self.active_token.push(c);
                    if complete_quote {
                        let token = mem::take(&mut self.active_token);
                        // Skip empty quotes since this could be an in progress edit.
                        if token == "\"\"" {
                            continue;
                        }
                        return Some(token);
                    }
                }
                Some(WordDelimiter::Backtick) => {
                    let complete_quote = if self.active_delimiter == Some(WordDelimiter::Backtick) {
                        self.active_delimiter = None;
                        true
                    } else if !self.active_token.is_empty() || self.active_delimiter.is_some() {
                        false
                    } else {
                        self.active_delimiter = Some(WordDelimiter::Backtick);
                        false
                    };

                    self.active_token.push(c);
                    if complete_quote {
                        let token = mem::take(&mut self.active_token);
                        return Some(token);
                    }
                }
                Some(WordDelimiter::SingleQuote) => {
                    let complete_quote =
                        if self.active_delimiter == Some(WordDelimiter::SingleQuote) {
                            self.active_delimiter = None;
                            true
                        } else if !self.active_token.is_empty() || self.active_delimiter.is_some() {
                            false
                        } else {
                            self.active_delimiter = Some(WordDelimiter::SingleQuote);
                            false
                        };

                    self.active_token.push(c);
                    if complete_quote {
                        let token = mem::take(&mut self.active_token);
                        // Skip empty quotes since this could be an in progress edit.
                        if token == "''" {
                            continue;
                        }
                        return Some(token);
                    }
                }
                _ => self.active_token.push(c),
            }
        }

        if self.active_token.is_empty() {
            return None;
        }

        Some(mem::take(&mut self.active_token))
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
pub mod tests;
