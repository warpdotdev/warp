cfg_if::cfg_if! {
    if #[cfg(feature = "v2")] {
        mod v2;
        pub use v2::*;
    } else {
        mod legacy;
        pub use legacy::*;
    }
}

use crate::meta::{Span, SpannedItem};
use crate::parsers::hir::{Expression, ShellCommand};
use crate::parsers::{ParsedExpression, ParsedToken};

/// Creates a new empty positional arg in a shell_command. This is useful before evaluating args
/// so that we don't include the extra whitespace at the end of the command (e.g. "cd ") within the
/// top level command.
fn add_extra_positional(shell_command: &mut ShellCommand, cursor: &Span) {
    let mut positional =
        vec![ParsedExpression::new(Expression::Literal, ParsedToken::empty()).spanned(*cursor)];

    match shell_command.args.positionals.take() {
        Some(mut positionals) => {
            positionals.append(&mut positional);
            shell_command.args.positionals = Some(positionals);
        }
        None => shell_command.args.positionals = Some(positional),
    };
}
