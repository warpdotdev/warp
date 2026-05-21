use std::{
    iter::{Enumerate, Peekable},
    ops::Range,
};

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedArgument {
    chars_range: Range<usize>,
    result: ParsedArgumentResult,
}

impl ParsedArgument {
    pub fn chars_range(&self) -> Range<usize> {
        self.chars_range.clone()
    }

    pub fn result(&self) -> &ParsedArgumentResult {
        &self.result
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedArgumentResult {
    Valid { current_word_index: usize },
    Invalid,
}

enum Symbol {
    DoubleOpeningBraces,
    DoubleClosingBraces,
    Whitespace,
    Character,
}

fn parse_characters(character: char, next_character: Option<char>) -> Symbol {
    if double_opening_braces(character, next_character) {
        Symbol::DoubleOpeningBraces
    } else if double_closing_braces(character, next_character) {
        Symbol::DoubleClosingBraces
    } else if character.is_whitespace() {
        Symbol::Whitespace
    } else {
        Symbol::Character
    }
}

enum ParserState {
    Word,
    /// Identifies when iterator is in a period of consecutive whitespace characters and
    /// argument is not open. (In an argument, this will get handled as an invalid character.)
    Whitespace,
    /// State when argument has started, as identified by double opening braces (`{{`).
    Argument {
        /// Start index, in characters. Points to first character of argument, not including braces.
        char_start_index: usize,
        /// `true` while only valid argument characters are seen.
        is_valid: bool,
        /// `true` if an argument begins with an escape sequence of three opening braces (`{{{`).
        is_escaped: bool,
    },
}

fn valid_arg_character(character: char, is_first_char: bool) -> bool {
    if is_first_char {
        character.is_alphabetic() || character == '-' || character == '_'
    } else {
        character.is_alphanumeric() || character == '-' || character == '_'
    }
}

fn double_opening_braces(character: char, next_character: Option<char>) -> bool {
    character == '{' && next_character == Some('{')
}

fn double_closing_braces(character: char, next_character: Option<char>) -> bool {
    character == '}' && next_character == Some('}')
}

fn current_is_first_char_in_argument(start_index: usize, character_index: usize) -> bool {
    start_index == character_index
}

/// Iterator that parses through input chars, and returns the next `ParsedArgument` or None.
/// The goal of this parser is to identify the locations of valid and invalid arguments
/// by their start and end indexes, for a given command string. Valid arguments are also
/// returned with their word index in the string, if needed for history diffs as in
/// `ArgumentsState::from_string()`. Words are separated by whitespace, 2+ opening
/// braces (`{`), or 2+ closing braces (`}`).
///
/// For an argument to be valid, it must start with `a-zA-Z_-` and be composed only of `a-zA-Z0-9_-`.
/// (These rules are derived from POSIX naming conventions.)
///
/// The iterator implementation steps through each character and identifies if it is the start or end
/// of an argument/word. Once an argument is closed (whether valid or invalid), it is returned with its
/// start index, end index, and validity result (`ParsedArgumentResult` enum) in a `ParsedArgument` object.
pub struct ParsedArgumentsIterator<I>
where
    I: IntoIterator,
{
    /// Iterator holding chars of passed command string (expects `Chars`).
    char_iter: Peekable<Enumerate<I::IntoIter>>,
    parser_state: ParserState,
    /// Number of words seen. Count includes valid/invalid arguments; increments when separator
    /// character is encountered (whitespace, 2+ opening braces (`{`), or 2+ closing braces (`}`)).
    word_count: usize,
}

impl<I> ParsedArgumentsIterator<I>
where
    I: IntoIterator<Item = char>,
{
    pub fn new(string_chars: I) -> Self {
        Self {
            char_iter: string_chars.into_iter().enumerate().peekable(),
            parser_state: ParserState::Word,
            word_count: 0,
        }
    }

    fn peek_is_none_or_whitespace(&mut self) -> bool {
        match self.char_iter.peek() {
            Some((_, character)) => character.is_whitespace(),
            None => true,
        }
    }

    fn start_argument(&mut self, argument_name_start_index: usize, is_escaped: bool) {
        self.parser_state = ParserState::Argument {
            char_start_index: argument_name_start_index,
            is_valid: true,
            is_escaped,
        };
    }

    fn parsed_argument(
        &self,
        start_index: usize,
        current_index: usize,
        is_valid: bool,
    ) -> Option<ParsedArgument> {
        let argument_name_empty = current_is_first_char_in_argument(start_index, current_index);

        if !argument_name_empty {
            let chars_range = start_index..current_index;

            let result = if is_valid {
                ParsedArgumentResult::Valid {
                    current_word_index: self.word_count - 1,
                }
            } else {
                ParsedArgumentResult::Invalid
            };

            Some(ParsedArgument {
                chars_range,
                result,
            })
        } else {
            None
        }
    }

    /// Calls `next()` on `self.char_iter` until `peek()` returns None or not `repeating_character`
    fn iterate_until_next_is_not(&mut self, repeating_character: char) -> i32 {
        let mut chars_skipped = 0;
        while let Some(_) = self.char_iter.next() {
            chars_skipped += 1;
            if let Some((_, character)) = self.char_iter.peek()
                && *character != repeating_character
            {
                break;
            }
        }
        chars_skipped
    }

    pub fn word_count(&self) -> usize {
        self.word_count
    }
}

impl<I> Iterator for ParsedArgumentsIterator<I>
where
    I: IntoIterator<Item = char>,
{
    type Item = ParsedArgument;

    /// Returns next `ParsedArgument` item or None if no more can be found. An argument starts with
    /// two opening braces `{{` and finishes with two closing braces `}}`. If every character inside
    /// these braces is valid, the argument is deemed valid and its result is `ParsedArgument::Valid`.
    /// If not, the argument is invalid and its result is `ParsedArgument::Invalid`.
    ///
    /// The implementation calls next() on the `char_iter` iterator to obtain the next character and its
    /// index in the string. This continues until an argument is found (or `char_iter` runs out of chars).
    /// The match statement handles:
    ///     (1) DoubleOpeningBraces: start arguments
    ///     (2) DoubleClosingBraces: ends arguments, will return an argument if found
    ///     (3) Character: if argument in process, check for validity. Else, check if it was
    ///         preceded by whitespace (and if so, increment word count).
    ///     (4) Whitespace: invalid argument character
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (character_index, character) = self.char_iter.next()?;
            let next_character = self.char_iter.peek().map(|(_, next_char)| *next_char);

            let parsed_symbol = parse_characters(character, next_character);
            let whitespace_ending = matches!(self.parser_state, ParserState::Whitespace)
                && !matches!(parsed_symbol, Symbol::Whitespace);

            match parsed_symbol {
                Symbol::DoubleOpeningBraces => {
                    self.word_count += 1;

                    // If we consumed more than one extra open bracket, we've entered a potential escape scenario (three brackets indicate arg escape)
                    let is_escaped_open = self.iterate_until_next_is_not('{') > 1;

                    let argument_name_start_index = self.char_iter.peek().map(|(index, _)| *index);
                    if let Some(start_index) = argument_name_start_index {
                        self.start_argument(start_index, is_escaped_open);
                    }
                }
                Symbol::DoubleClosingBraces => {
                    let parsed_argument = match self.parser_state {
                        ParserState::Argument {
                            char_start_index,
                            is_valid,
                            ..
                        } => self.parsed_argument(char_start_index, character_index, is_valid),
                        _ => None,
                    };

                    // If we end by consuming more than one extra close bracket, and we started the argument with an escape open sequence, we should escape this argument
                    let is_escape_closed = self.iterate_until_next_is_not('}') > 1;
                    let is_escaped = match self.parser_state {
                        ParserState::Argument {
                            is_escaped: is_escaped_open,
                            ..
                        } => is_escaped_open && is_escape_closed,
                        _ => false,
                    };

                    if self.peek_is_none_or_whitespace() {
                        self.parser_state = ParserState::Whitespace;
                    } else {
                        // If encountered }}e, `e` starts a new word
                        self.word_count += 1;
                        self.parser_state = ParserState::Word;
                    }

                    if parsed_argument.is_some() && !is_escaped {
                        return parsed_argument;
                    }
                }
                Symbol::Character => match self.parser_state {
                    ParserState::Argument {
                        char_start_index,
                        is_escaped,
                        ..
                    } => {
                        if !valid_arg_character(
                            character,
                            current_is_first_char_in_argument(char_start_index, character_index),
                        ) {
                            self.parser_state = ParserState::Argument {
                                char_start_index,
                                is_valid: false,
                                is_escaped,
                            }
                        }
                    }
                    _ => {
                        // At least one non-whitespace char guarantees that at least 1 word exists
                        // If whitespace ending, means new word seen
                        if self.word_count == 0 || whitespace_ending {
                            self.word_count += 1;
                        }
                        self.parser_state = ParserState::Word;
                    }
                },
                Symbol::Whitespace => match self.parser_state {
                    ParserState::Argument {
                        char_start_index,
                        is_escaped,
                        ..
                    } => {
                        self.parser_state = ParserState::Argument {
                            char_start_index,
                            is_valid: false,
                            is_escaped,
                        };
                    }
                    _ => {
                        self.parser_state = ParserState::Whitespace;
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
