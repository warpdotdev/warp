//! Utilities to check whether a command at the cursor position is likely a package installer command.

use crate::completer::SessionContext;
use crate::terminal::alias::is_expandable_alias;
use string_offset::ByteOffset;
use warp_completer::parsers::simple::command_at_cursor_position;
use warp_util::path::ShellFamily;

/// Returns true if the command at the cursor position is likely a package installer command that would allow `@` at the start of the package name.
/// This excludes package managers like Rust or Go that do not allow `@` at the start of the package name.
pub fn command_at_cursor_has_common_package_installer_prefix(
    buffer: &str,
    at_index: usize,
    shell_family: ShellFamily,
    is_alias_expansion_enabled: bool,
    session_context: Option<&SessionContext>,
) -> bool {
    let Some(cmd) = command_at_cursor_position(
        buffer,
        shell_family.escape_char(),
        ByteOffset::from(at_index),
    ) else {
        return false;
    };

    let mut segment_text = cmd.joined_by_space();

    // If alias auto-expansion is disabled, expand the first token internally using the session alias map.
    if !is_alias_expansion_enabled {
        if let (Some(first_token), Some(session_context)) = (cmd.parts.first(), session_context) {
            if let Some(alias_value) = session_context.session.alias_value(&first_token.item) {
                if is_expandable_alias(&first_token.item, alias_value) {
                    // Replace the leading first token in the segment with the alias value.
                    let suffix = &segment_text[first_token.item.len()..];
                    let mut new_text = String::with_capacity(alias_value.len() + suffix.len());
                    new_text.push_str(alias_value);
                    new_text.push_str(suffix);
                    segment_text = new_text;
                }
            }
        }
    }

    is_at_context_package_installer_prefix(&segment_text)
}

/// Returns true if the buffer text likely indicates the user is typing a package
/// name in a package installer command where '@' is commonly part of the name
/// (e.g., npm/pnpm/yarn/bun scoped packages, Homebrew versioned formulae,
/// Go module versions, Python pip installs, Cargo add versions).
fn is_at_context_package_installer_prefix(buffer_text: &str) -> bool {
    let s = buffer_text.trim_start().to_ascii_lowercase();

    // Keep this list conservative and specific (include necessary subcommands) to avoid
    // false positives like `npm run` or `yarn dev`. Include a trailing space when appropriate.
    const PREFIXES: &[&str] = &[
        // Node ecosystem
        "npm ",
        "npx ",
        "pnpm ",
        "yarn ",
        "bun ",
        "bunx ",
        "deno ",
        "astro add ",
        "turbo ",
        // Python installers
        "pip ",
        "pip3 ",
        "python -m pip ",
        "python3 -m pip ",
        "py -m pip ",
        "uv pip ",
        "poetry ",
        // Ruby
        "gem ",
        "bundle ",
    ];

    PREFIXES.iter().any(|p| s.starts_with(p))
}

#[cfg(test)]
#[path = "package_installers_tests.rs"]
mod tests;
