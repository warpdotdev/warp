pub trait TrimStringExt {
    fn trim_trailing_newline(&mut self);
}

impl TrimStringExt for String {
    fn trim_trailing_newline(&mut self) {
        if self.ends_with('\n') {
            self.pop();
        }
        if self.ends_with('\r') {
            self.pop();
        }
    }
}

/// Removes the spaces and tabs that end each line of `text`, keeping the line terminators.
///
/// A terminal grid pads rows out to the window width and fills blank rows with spaces, so text
/// copied from a selection would otherwise carry that invisible padding into the clipboard.
pub fn trim_trailing_spaces_per_line(text: &str) -> String {
    let mut trimmed = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, terminator) = match line.strip_suffix("\r\n") {
            Some(body) => (body, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(body) => (body, "\n"),
                None => (line, ""),
            },
        };
        trimmed.push_str(body.trim_end_matches([' ', '\t']));
        trimmed.push_str(terminator);
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::trim_trailing_spaces_per_line;

    #[test]
    fn trims_padding_from_every_line() {
        assert_eq!(
            trim_trailing_spaces_per_line("commands.   \n  indented   \n"),
            "commands.\n  indented\n"
        );
    }

    #[test]
    fn blank_lines_made_of_spaces_become_empty() {
        assert_eq!(
            trim_trailing_spaces_per_line("first\n      \nsecond"),
            "first\n\nsecond"
        );
    }

    #[test]
    fn keeps_leading_indent_and_inner_spaces() {
        assert_eq!(trim_trailing_spaces_per_line("  a  b \tc\t"), "  a  b \tc");
    }

    #[test]
    fn keeps_crlf_terminators() {
        assert_eq!(trim_trailing_spaces_per_line("a  \r\nb \r\n"), "a\r\nb\r\n");
    }

    #[test]
    fn non_ascii_text_is_untouched() {
        assert_eq!(
            trim_trailing_spaces_per_line("Привет, мир  \n你好 \n"),
            "Привет, мир\n你好\n"
        );
    }

    #[test]
    fn empty_and_all_space_input() {
        assert_eq!(trim_trailing_spaces_per_line(""), "");
        assert_eq!(trim_trailing_spaces_per_line("   "), "");
    }
}
