use super::iter::ParserInput;
use super::token::Token;
use super::{Command, Part};
use crate::meta::{Span, Spanned, SpannedItem};

/// Parser that converts a command string into a list of commands, with arguments separated
///
/// Handles subshells, quotes, double quotes and escapes (e.g. `\` for paths with spaces)
pub(super) struct Parser<I>
where
    I: IntoIterator,
{
    tokens: ParserInput<I>,
    contains_redirection: bool,
}

pub struct ParsedResult {
    pub commands: Vec<Spanned<Command>>,

    /// Whether or not redirection operators (i.e. '>', '<') were
    /// found when parsing.
    pub contains_redirection: bool,
}

impl<'a, I> Parser<I>
where
    I: IntoIterator<Item = Spanned<Token<'a>>>,
{
    pub fn new(input: I) -> Self {
        Self::with_tokens(ParserInput::new(input))
    }

    fn with_tokens(tokens: ParserInput<I>) -> Self {
        Parser {
            tokens,
            contains_redirection: false,
        }
    }

    pub fn parse(mut self) -> ParsedResult {
        let commands = self.parse_command_list(None);
        ParsedResult {
            commands,
            contains_redirection: self.contains_redirection,
        }
    }

    /// Generate a span from the given start to the current token position
    fn span(&self, start: usize) -> Span {
        Span::new(start, self.tokens.pos())
    }

    /// Skip over any whitespace tokens
    fn skip_whitespace(&mut self) {
        while let Some(Token::Whitespace(_)) = self.tokens.peek() {
            self.tokens.next();
        }
    }

    /// Parse a list of commands, separated by logical operators, pipes, semicolons, etc.
    ///
    /// Can optionally include a delimiting token that will mark the end of the command list
    fn parse_command_list(&mut self, delimiter: Option<Token>) -> Vec<Spanned<Command>> {
        let mut commands = Vec::new();
        loop {
            self.skip_whitespace();
            if delimiter.is_some() && delimiter.as_ref() == self.tokens.peek() {
                break;
            }

            // TODO: Add errors if there is a command in the list and the token isn't a logical
            // separator, as it's invalid to connect commands with a grouping operator
            match self.tokens.peek() {
                Some(Token::OpenParen) => {
                    self.tokens.next();
                    let nested = self.parse_command_list(Some(Token::CloseParen));
                    commands.extend(nested);
                }
                Some(Token::OpenCurly) => {
                    self.tokens.next();
                    let nested = self.parse_command_list(Some(Token::CloseCurly));
                    commands.extend(nested);
                }
                Some(Token::CloseParen | Token::CloseCurly) => {
                    // If we got here, that means we are hitting a grouping close when we aren't
                    // expecting it. This is generally a parse error, however for now we will treat
                    // it as a separator between commands.
                    // TODO: Add an error for an unexpected character
                    self.tokens.next();
                }
                Some(Token::RedirectInput | Token::RedirectOutput) => {
                    self.contains_redirection = true;
                    self.tokens.next();
                }
                Some(t) if is_valid_command_separator(t) => {
                    // Valid separator between commands, for now we naively consume it and don't
                    // store semantic information about the kind of separation.
                    self.tokens.next();
                }
                Some(_) => {
                    // All other tokens are part of commands
                    commands.push(self.parse_command());
                }
                None => break,
            }
        }

        // TODO: Add an error if the delimiter wasn't found

        commands
    }

    /// Parse an individual command from the input
    fn parse_command(&mut self) -> Spanned<Command> {
        let start = self.tokens.pos();
        let mut parts = Vec::new();
        loop {
            self.skip_whitespace();

            match self.tokens.peek() {
                None => break,
                Some(Token::RedirectInput | Token::RedirectOutput) => {
                    self.contains_redirection = true;
                    self.tokens.next();
                }
                Some(t) if is_command_terminator(t) => {
                    break;
                }
                _ => {
                    parts.push(self.parse_part(parts.is_empty()));
                }
            }
        }

        Command::new(parts).spanned(self.span(start))
    }

    /// Parse a single part of a command (e.g. argument) from the input
    fn parse_part(&mut self, first_part: bool) -> Spanned<Part> {
        let mut builder = PartBuilder::new(self.tokens.pos());

        loop {
            match self.tokens.peek() {
                None => break,
                Some(Token::Whitespace(_)) => break,
                Some(Token::RedirectInput | Token::RedirectOutput) => {
                    self.contains_redirection = true;
                    self.tokens.next();
                }
                Some(t) if is_command_terminator(t) => {
                    break;
                }
                Some(Token::DoubleQuote) => {
                    builder.add_part(self.parse_double_quoted_part());
                }
                Some(Token::SingleQuote) => {
                    builder.add_part(self.parse_single_quoted_part());
                }
                Some(Token::Backtick) => {
                    builder.add_part(self.parse_backticked_subshell());
                }
                Some(Token::Dollar) => {
                    if let Some(Token::OpenParen) = self.tokens.peekpeek() {
                        builder.add_part(self.parse_dollar_subshell());
                    } else {
                        // Consume the dollar token
                        self.tokens.next();
                        builder.add_raw(Token::Dollar.as_str());
                    }
                }
                Some(Token::EscapeChar(c)) => {
                    let c = c.to_owned();
                    // Consume the backslash
                    self.tokens.next();
                    // In non-quoted contexts, backslash escapes all characters except newlines
                    // Newline immediately following a backslash is treated as line continuation
                    // Regardless, the backslash is not included in the raw value (besides some
                    // special cases)
                    match self.tokens.next() {
                        None => {
                            // Include the backslash if it is the last character in the output, as
                            // there is nothing for it to escape
                            builder.add_raw(c);
                        }
                        Some(Token::Newline) => {}
                        Some(Token::Literal("~")) => {
                            // Include the backslash if the input is `\~` to enable us to
                            // distinguish tildes that need to be expanded into the home directory
                            // and the raw string.
                            builder.add_raw(c);
                            builder.add_raw("~");
                        }
                        Some(token) => {
                            // Include the backslash if this is the first part of the command, so that we
                            //  can support escaping aliases.  eg: with alias ls=exa, the command \ls
                            //  should be treated as the command, and we should not remove the backslash.
                            if first_part && builder.is_empty() {
                                builder.add_raw(c);
                            }
                            builder.add_raw(token.as_str());
                        }
                    }
                }
                // All tokens not handled specially are treated as literals
                Some(literal) => {
                    builder.add_raw(literal.as_str());
                    // Consume the token
                    self.tokens.next();
                }
            }
        }

        builder.complete(self.tokens.pos())
    }

    /// Parse a backticked subshell section
    ///
    /// Note: This must only be called when the next character is a backtick
    fn parse_backticked_subshell(&mut self) -> Spanned<Part> {
        let start = self.tokens.pos();

        // Consume the backtick token
        let check = self.tokens.next();
        debug_assert!(matches!(check, Some(Token::Backtick)));

        // Create a parser using the buffered tokens between here and the next backtick
        // We can't use parse_command_list directly because Backtick isn't a command terminator,
        // and we can't make it a command terminator because it's symmetric: It also represents the
        // _start_ of a subshell.
        let mut sub_parser = Parser::with_tokens(self.tokens.until_backtick());
        let command_list = sub_parser.parse_command_list(None);

        // Consume the closing backtick if available and classify if the subshell was closed
        match self.tokens.next() {
            None => Part::OpenSubshell(command_list).spanned(self.span(start)),
            Some(t) => {
                debug_assert!(t == Token::Backtick);
                Part::ClosedSubshell(command_list).spanned(self.span(start))
            }
        }
    }

    /// Parse a dollar subshell section, i.e. one surrounded by $()
    ///
    /// Note: This must only be called when the next two characters are $(
    fn parse_dollar_subshell(&mut self) -> Spanned<Part> {
        let start = self.tokens.pos();

        // Consume the dollar and open paren tokens
        let check = self.tokens.next();
        debug_assert!(matches!(check, Some(Token::Dollar)));
        let check = self.tokens.next();
        debug_assert!(matches!(check, Some(Token::OpenParen)));

        let command_list = self.parse_command_list(Some(Token::CloseParen));

        // Consume the close paren if available and classify the subshell was closed
        match self.tokens.next() {
            None => Part::OpenSubshell(command_list).spanned(self.span(start)),
            Some(t) => {
                debug_assert!(t == Token::CloseParen);
                Part::ClosedSubshell(command_list).spanned(self.span(start))
            }
        }
    }

    /// Parse a double-quoted section
    ///
    /// Within double quotes, most tokens are treated as literal. The exceptions are:
    ///
    /// - Backticks (`) still start subshell sections
    /// - $() can still be used for subshell sections
    /// - Backslash (\) escapes only some characters: $ ` " \ \n
    ///
    /// Note: This must only be called when the next token is a double quote
    fn parse_double_quoted_part(&mut self) -> Spanned<Part> {
        let mut builder = PartBuilder::new(self.tokens.pos());

        // Consume the opening double quote
        let check = self.tokens.next();
        debug_assert!(matches!(check, Some(Token::DoubleQuote)));

        loop {
            match self.tokens.peek() {
                None => break,
                Some(Token::DoubleQuote) => {
                    // Consume the closing double quote
                    self.tokens.next();
                    break;
                }
                Some(Token::EscapeChar(c)) => {
                    let c = c.to_owned();
                    // Consume the backslash
                    self.tokens.next();
                    // Check the following character for escape behavior
                    match self.tokens.next() {
                        Some(Token::Newline) => {
                            // Within double quotes, newline following backslash is still treated
                            // as line continuation, so neither is included in the raw output
                        }
                        Some(
                            token @ Token::Dollar
                            | token @ Token::Backtick
                            | token @ Token::DoubleQuote
                            | token @ Token::EscapeChar(_),
                        ) => {
                            builder.add_raw(token.as_str());
                        }
                        Some(token) => {
                            // For all other characters, we include the backslash as well
                            builder.add_raw(c);
                            builder.add_raw(token.as_str());
                        }
                        None => {
                            builder.add_raw(c);
                        }
                    }
                }
                Some(Token::Backtick) => {
                    builder.add_part(self.parse_backticked_subshell());
                }
                Some(Token::Dollar) => {
                    if let Some(Token::OpenParen) = self.tokens.peekpeek() {
                        builder.add_part(self.parse_dollar_subshell());
                    } else {
                        // Consume the dollar token
                        self.tokens.next();
                        builder.add_raw(Token::Dollar.as_str());
                    }
                }
                Some(token) => {
                    builder.add_raw(token.as_str());
                    // Consume the token
                    self.tokens.next();
                }
            }
        }

        builder.complete(self.tokens.pos())
    }

    /// Parse a single-quoted section
    ///
    /// Within single quotes, all tokens are literal until the closing single quote
    /// Note: This must only be called when the next character is a single quote
    fn parse_single_quoted_part(&mut self) -> Spanned<Part> {
        let start = self.tokens.pos();
        let mut buffer = String::new();

        // Consume the starting single quote
        let check = self.tokens.next();
        debug_assert!(matches!(check, Some(Token::SingleQuote)));

        while let Some(token) = self.tokens.next() {
            match token {
                Token::SingleQuote => break,
                t => buffer.push_str(t.as_str()),
            }
        }

        // TODO: Add an error if we hit the end of the output without finding a closing quote

        Part::Literal(buffer).spanned(self.span(start))
    }
}

/// Determine if a token is a valid separator between commands
fn is_valid_command_separator(token: &Token) -> bool {
    use Token::*;

    matches!(
        token,
        Pipe | LogicalOr | Ampersand | LogicalAnd | Semicolon | Newline
    )
}

/// Determine if a token is a terminator marking the end of a command
///
/// This includes all of the separator tokens as well as the grouping tokens
fn is_command_terminator(token: &Token) -> bool {
    use Token::*;

    is_valid_command_separator(token)
        || matches!(token, OpenParen | CloseParen | OpenCurly | CloseCurly)
}

/// Builder for handling combined command parts.
///
/// Tracks literal values and any nested parts, flattening `Part::Concatenated` into the current
/// list as necessary.
struct PartBuilder {
    start: usize,
    buffer: String,
    buffer_start: usize,
    sub_parts: Vec<Spanned<Part>>,
}

impl PartBuilder {
    fn new(start: usize) -> Self {
        Self {
            start,
            buffer: String::new(),
            buffer_start: start,
            sub_parts: Vec::new(),
        }
    }

    /// Add a whole known sub-Part to the current Part.
    ///
    /// Will complete any current Literal values into a `Part::Literal` entry.
    fn add_part(&mut self, part: Spanned<Part>) {
        if !self.buffer.is_empty() {
            let span = Span::new(self.buffer_start, part.span.start());
            let literal = Part::Literal(std::mem::take(&mut self.buffer)).spanned(span);
            self.sub_parts.push(literal);
        }

        self.buffer_start = part.span.end();

        match part.item {
            // Flatten any concatenated inner parts to create a single list
            Part::Concatenated(inner) => {
                self.sub_parts.extend(inner);
            }
            _ => self.sub_parts.push(part),
        }
    }

    /// Add a raw string value to the current part
    fn add_raw(&mut self, value: &str) {
        self.buffer.push_str(value);
    }

    /// Complete the Part, finishing any Literal entries and determining the appropriate `Part`
    /// variant to return based on the number of entries.
    fn complete(mut self, end: usize) -> Spanned<Part> {
        if !self.buffer.is_empty() {
            let span = Span::new(self.buffer_start, end);
            let literal = Part::Literal(self.buffer).spanned(span);
            self.sub_parts.push(literal);
        }

        let span = Span::new(self.start, end);
        match self.sub_parts.len() {
            0 => Part::Literal(String::new()).spanned(span),
            // Safety: We are checking the length, so if there is one element then pop will exist
            1 => self.sub_parts.pop().unwrap(),
            _ => Part::Concatenated(self.sub_parts).spanned(span),
        }
    }

    /// Check if the builder is empty.
    fn is_empty(&self) -> bool {
        self.buffer.is_empty() && self.sub_parts.is_empty()
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
