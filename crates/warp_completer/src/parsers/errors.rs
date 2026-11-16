#![allow(dead_code)]
use getset::Getters;

use crate::meta::{Span, Spanned, SpannedItem};

#[derive(Getters, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct ParseError {
    #[get = "pub"]
    pub reason: ParseErrorReason,
}

impl ParseError {
    pub fn unexpected_eof(expected: impl Into<String>, span: Span) -> ParseError {
        ParseError {
            reason: ParseErrorReason::Eof {
                expected: expected.into(),
                span,
            },
        }
    }

    pub fn extra_tokens(actual: Spanned<impl Into<String>>) -> ParseError {
        let Spanned { span, item } = actual;

        ParseError {
            reason: ParseErrorReason::ExtraTokens {
                actual: item.into().spanned(span),
            },
        }
    }

    pub fn mismatch(expected: impl Into<String>, actual: Spanned<impl Into<String>>) -> ParseError {
        let Spanned { span, item } = actual;

        ParseError {
            reason: ParseErrorReason::Mismatch {
                expected: expected.into(),
                actual: item.into().spanned(span),
            },
        }
    }

    pub fn internal_error(message: Spanned<impl Into<String>>) -> ParseError {
        ParseError {
            reason: ParseErrorReason::InternalError {
                message: message.item.into().spanned(message.span),
            },
        }
    }

    pub fn argument_error(command: Spanned<impl Into<String>>, kind: ArgumentError) -> ParseError {
        ParseError {
            reason: ParseErrorReason::ArgumentError {
                command: command.item.into().spanned(command.span),
                error: kind,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub enum ParseErrorReason {
    Eof {
        expected: String,
        span: Span,
    },
    ExtraTokens {
        actual: Spanned<String>,
    },
    Mismatch {
        expected: String,
        actual: Spanned<String>,
    },
    InternalError {
        message: Spanned<String>,
    },
    ArgumentError {
        command: Spanned<String>,
        error: ArgumentError,
    },
}

/// ArgumentError describes various ways that the parser could fail because of unexpected arguments.
/// These errors correspond to problems that could be identified during expansion based on the
/// signature of a command.
#[derive(Debug, Eq, PartialEq, Clone, Ord, Hash, PartialOrd)]
pub enum ArgumentError {
    /// The command specified a mandatory flag, but it was missing.
    MissingMandatoryFlag(String),
    /// The command specified a mandatory positional argument, but it was missing.
    MissingMandatoryPositional {
        name: Option<String>,
        positional_index: usize,
    },
    /// A flag was found, and it should have been followed by a value, but no value was found.
    MissingValueForName {
        /// The name of the flag/option
        name: Spanned<String>,
        /// The index of the missing argument for the option,
        /// e.g. if the flag was -D arg1 arg2, then missing_arg_index=0 corresponds to arg1
        missing_arg_index: usize,
    },
    /// An argument was found, but the command does not recognize it.
    UnexpectedArgument(Spanned<String>),
    /// An flag was found, but the command does not recognize it.
    UnexpectedFlag(Spanned<String>),
    /// A recognized flag had an invalid value.
    InvalidValueForFlag(Spanned<String>),
}
