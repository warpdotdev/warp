use std::iter::Peekable;

use super::token::Token;
use crate::meta::Spanned;

/// Adapter for tracking the parser's current position in the input and allowing for double peek
pub struct ParserInput<I>
where
    I: IntoIterator,
{
    iter: Peekable<I::IntoIter>,
    peeked: Option<I::Item>,
    pos: usize,
    offset: usize,
}

impl<'a, I> ParserInput<I>
where
    I: IntoIterator<Item = Spanned<Token<'a>>>,
{
    pub fn new(input: I) -> Self {
        let iter = input.into_iter().peekable();
        ParserInput {
            iter,
            peeked: None,
            pos: 0,
            offset: 0,
        }
    }

    /// Get the next token from the input
    pub fn next(&mut self) -> Option<Token<'a>> {
        let next = self.peeked.take().or_else(|| self.iter.next());

        match next {
            Some(token) => {
                self.pos = self.offset + token.span.end();
                Some(token.item)
            }
            None => None,
        }
    }

    /// Get the next token from the input, including the span
    fn next_with_span(&mut self) -> Option<Spanned<Token<'a>>> {
        let next = self.peeked.take().or_else(|| self.iter.next());

        if let Some(token) = &next {
            self.pos = self.offset + token.span.end();
        }

        next
    }

    /// Get a reference to the next token without consuming it
    pub fn peek(&mut self) -> Option<&Token<'a>> {
        match &self.peeked {
            Some(value) => Some(&value.item),
            None => self.iter.peek().map(|t| &t.item),
        }
    }

    /// Get a reference to the token after the next, without consuming either
    pub fn peekpeek(&mut self) -> Option<&Token<'a>> {
        if self.peeked.is_none() {
            self.peeked = self.iter.next();
        }

        self.iter.peek().map(|t| &t.item)
    }

    /// Get the position where we are currently, immediately after the last element that was read
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Create a ParserInput that will yield tokens until a backtick is found and then stop,
    /// preserving the position.
    ///
    /// Note: This will buffer all of the tokens until the next backtick
    pub fn until_backtick(&mut self) -> ParserInput<Vec<Spanned<Token<'a>>>> {
        let mut buffer = Vec::new();
        let start = self.pos;

        while let Some(token) = self.peek() {
            match token {
                Token::Backtick => break,
                _ => {
                    // Safety: We are peeking first so the next token will always exist
                    buffer.push(self.next_with_span().unwrap());
                }
            }
        }

        ParserInput {
            iter: buffer.into_iter().peekable(),
            peeked: None,
            pos: start,
            offset: self.offset,
        }
    }
}
