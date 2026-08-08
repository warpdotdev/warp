use std::borrow::Cow;

use itertools::Itertools;
use warp_util::path::ShellFamily;
use warpui::clipboard::ClipboardContent;

/// Returns a string representation of the ClipboardContent with any paths properly escaped if there is a known shell. If not, do not escape the paths.
pub fn clipboard_content_with_escaped_paths(
    mut content: ClipboardContent,
    shell_family: Option<ShellFamily>,
    replace_newlines_with_spaces: bool,
) -> String {
    if replace_newlines_with_spaces {
        content = ClipboardContent {
            // Collapse CRLF first: replacing only `\n` would leave the `\r` of a
            // clipboard payload copied on Windows behind, and a lone `\r` paints as
            // nothing, so it silently corrupts the pasted value.
            plain_text: content
                .plain_text
                .replace("\r\n", "\n")
                .replace(['\n', '\r'], " "),
            ..content
        }
    }
    match content.paths {
        Some(paths) => paths
            .iter()
            .map(|path| match shell_family {
                Some(shell_family) => shell_family.escape(path),
                None => Cow::Borrowed(path.as_ref()),
            })
            .join(" "),
        None => content.plain_text,
    }
}
