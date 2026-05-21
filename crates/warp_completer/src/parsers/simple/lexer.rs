use super::{token::Token, EscapeChar};
use crate::meta::{Span, Spanned, SpannedItem};

/// Iterator for converting a string into a series of Tokens
///
/// This step is very naive, it does not attempt to understand the various contexts in which tokens
/// can appear (e.g. within double or single quotes, within subshells nested within double quotes).
/// The parser will track all of the necessary state in order to properly interpret each token
/// given the context.
pub struct Lexer<'a> {
    /// The source string that we are tokenizing
    source: &'a str,
    escape_char: EscapeChar,
    /// The current byte index in the source
    pos: usize,
    /// Processed character that we have seen but not yet yielded
    queued: Option<Classified<'a>>,
    /// Whether to consider quotes as literals in the lexer.
    parse_quotes_as_literals: bool,
}

/// The classification of a character into a known Token, Raw character value (for Literal tokens)
/// or an Escaped token (via `\`). This intermediate classification helps to process Literal and
/// Escaped tokens properly without needing to handle the specific parser context.
enum Classified<'a> {
    Token(Spanned<Token<'a>>),
    Raw(Spanned<char>),
    Escaped {
        escape_char: Spanned<Token<'a>>,
        next_token: Option<Spanned<Token<'a>>>,
    },
}

impl From<Spanned<char>> for Classified<'_> {
    fn from(chr: Spanned<char>) -> Self {
        Classified::Raw(chr)
    }
}

impl<'a> From<Spanned<Token<'a>>> for Classified<'a> {
    fn from(tok: Spanned<Token<'a>>) -> Self {
        Classified::Token(tok)
    }
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, escape_char: EscapeChar, parse_quotes_as_literals: bool) -> Self {
        Lexer {
            source,
            escape_char,
            pos: 0,
            queued: None,
            parse_quotes_as_literals,
        }
    }

    /// Advance the underlying iterator one step, returning the next character and its span
    fn step(&mut self) -> Option<(Span, char)> {
        let (span, chr) = self.peek()?;

        self.pos = span.end();
        Some((span, chr))
    }

    /// Peek at the next character without consuming it, returning the span as well
    fn peek(&mut self) -> Option<(Span, char)> {
        let start = self.pos;
        let chr = self.source[start..].chars().next()?;
        let end = start + chr.len_utf8();

        let span = Span::new(start, end);
        Some((span, chr))
    }

    /// Classify the next character as either a token, escaped token, or raw value.
    ///
    /// Note that some tokens will consume multiple characters.
    fn classify_next(&mut self) -> Option<Classified<'a>> {
        if self.queued.is_some() {
            return self.queued.take();
        }

        let (span, chr) = self.step()?;

        Some(match chr {
            '|' => {
                if let Some((next, '|')) = self.peek() {
                    self.next();
                    Token::LogicalOr.spanned(span.until(next)).into()
                } else {
                    Token::Pipe.spanned(span).into()
                }
            }
            '&' => {
                if let Some((next, '&')) = self.peek() {
                    self.next();
                    Token::LogicalAnd.spanned(span.until(next)).into()
                } else {
                    Token::Ampersand.spanned(span).into()
                }
            }
            ';' => Token::Semicolon.spanned(span).into(),
            '\n' => Token::Newline.spanned(span).into(),
            '(' => Token::OpenParen.spanned(span).into(),
            ')' => Token::CloseParen.spanned(span).into(),
            '{' => Token::OpenCurly.spanned(span).into(),
            '}' => Token::CloseCurly.spanned(span).into(),
            '$' => Token::Dollar.spanned(span).into(),
            '\'' if !self.parse_quotes_as_literals => Token::SingleQuote.spanned(span).into(),
            '"' if !self.parse_quotes_as_literals => Token::DoubleQuote.spanned(span).into(),
            '<' => Token::RedirectInput.spanned(span).into(),
            '>' => Token::RedirectOutput.spanned(span).into(),
            '\\' | '`' if self.escape_char.is_char(chr) => {
                // Consume the following character as a token by itself
                // Note: Since the internal Lexer uses spans relative to its slice, so we need
                // to adjust them to match our positions
                let pos_adjust = self.pos;
                let next_token = self
                    .step()
                    .and_then(|(span, _)| {
                        Lexer::new(
                            span.slice(self.source),
                            self.escape_char,
                            self.parse_quotes_as_literals,
                        )
                        .next()
                    })
                    .map(|token| {
                        let new_start = token.span.start() + pos_adjust;
                        let new_end = token.span.end() + pos_adjust;

                        token.item.spanned(Span::new(new_start, new_end))
                    });
                Classified::Escaped {
                    escape_char: Token::EscapeChar(span.slice(self.source)).spanned(span),
                    next_token,
                }
            }
            '`' => Token::Backtick.spanned(span).into(),
            c if c.is_whitespace() => {
                // Note: We handle newlines earlier in the match, so this excludes newlines
                let mut span = span;

                while let Some((next_span, next_chr)) = self.peek() {
                    if next_chr.is_whitespace() && next_chr != '\n' {
                        span = span.until(next_span);
                        self.step();
                    } else {
                        break;
                    }
                }

                Token::Whitespace(span.slice(self.source))
                    .spanned(span)
                    .into()
            }
            c => c.spanned(span).into(),
        })
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Spanned<Token<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.classify_next() {
            None => None,
            Some(Classified::Token(t)) => Some(t),
            Some(Classified::Escaped {
                escape_char,
                next_token,
            }) => {
                // Note: For escaped values, we still need to include a backslash token, because
                // there are some contexts (e.g. within a single-quoted value) where the backslash
                // can be interpreted as a literal value. It's left up to the parser to determine
                // how to handle the backslash.
                self.queued = next_token.map(Into::into);
                Some(escape_char)
            }
            Some(Classified::Raw(chr)) => {
                let mut span = chr.span;

                loop {
                    match self.classify_next() {
                        Some(Classified::Raw(chr)) => {
                            span = span.until(chr.span);
                        }
                        other => {
                            self.queued = other;
                            break;
                        }
                    }
                }

                let value = span.slice(self.source);
                Some(Token::Literal(value).spanned(span))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let lower = match &self.queued {
            Some(_) => 1,
            None => 0,
        };
        let upper = self.source.len() - self.pos;

        if upper < lower {
            (lower, None)
        } else {
            (lower, Some(upper))
        }
    }
}

#[cfg(test)]
#[path = "lexer_tests.rs"]
mod tests;
