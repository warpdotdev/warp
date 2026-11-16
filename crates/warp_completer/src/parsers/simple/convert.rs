use std::fmt;

use super::{Command, Part};
use crate::meta::{Span, Spanned};
use crate::parsers::{LiteCommand, LiteGroup, LitePipeline, LiteRootNode};

impl From<Spanned<Command>> for LiteCommand {
    fn from(command: Spanned<Command>) -> Self {
        let post_whitespace = command.item.parts.last().and_then(|part| {
            if part.span.end() < command.span.end() {
                Some(Span::new(part.span.end(), command.span.end()))
            } else {
                None
            }
        });

        LiteCommand {
            parts: command.item.parts.into_iter().map(Into::into).collect(),
            post_whitespace,
        }
    }
}

impl From<Spanned<Part>> for Spanned<String> {
    fn from(spanned: Spanned<Part>) -> Spanned<String> {
        spanned.map(|part| part.to_string())
    }
}

impl From<Option<LiteCommand>> for LiteRootNode {
    fn from(command: Option<LiteCommand>) -> Self {
        LiteRootNode {
            groups: vec![LiteGroup {
                pipelines: vec![LitePipeline {
                    commands: command.into_iter().collect(),
                }],
            }],
        }
    }
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Literal(value) => f.write_str(value.as_str()),
            Part::OpenSubshell(_) | Part::ClosedSubshell(_) => {
                // Since we aren't evaluating the subshell, include a placeholder value
                f.write_str("$(...)")
            }
            Part::Concatenated(parts) => {
                for part in parts {
                    part.item.fmt(f)?;
                }
                Ok(())
            }
        }
    }
}
