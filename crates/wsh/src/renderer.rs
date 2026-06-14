use std::fmt::Write as _;

const CSI: &str = "\x1b[";
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const REVERSE: &str = "\x1b[7m";

const DIM_CYAN: &str = "\x1b[2;36m";
const DIM_YELLOW: &str = "\x1b[2;33m";
const DIM_RED: &str = "\x1b[2;31m";
const DIM_MAGENTA: &str = "\x1b[2;35m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    AgentText,
    CommandRun,
    CommandOutput,
    Thinking,
    Error,
}

#[derive(Debug, Clone)]
pub struct AgentBlock {
    pub kind: BlockKind,
    pub header: Option<String>,
    pub body: String,
}

impl BlockKind {
    fn color_code(self) -> &'static str {
        match self {
            BlockKind::AgentText => DIM_CYAN,
            BlockKind::CommandRun => DIM_YELLOW,
            BlockKind::CommandOutput => "",
            BlockKind::Thinking => DIM_MAGENTA,
            BlockKind::Error => DIM_RED,
        }
    }
}

pub fn render_block(block: &AgentBlock, width: u16) -> Vec<u8> {
    let w = width as usize;
    if w < 6 {
        return Vec::new();
    }
    let color = block.kind.color_code();
    let inner = w - 4; // │ + space + content + space + │
    let mut out = String::new();

    // Top border — \r ensures cursor starts at column 1 in raw mode.
    let _ = write!(out, "\r{color}╭─");
    if let Some(header) = &block.header {
        let max_header = inner.saturating_sub(2);
        let truncated: String = header.chars().take(max_header).collect();
        let _ = write!(out, " {truncated} ");
        let used = truncated.chars().count() + 2; // space + header + space
        let remaining = (w - 4).saturating_sub(used);
        for _ in 0..remaining {
            out.push('─');
        }
    } else {
        for _ in 0..(w - 4) {
            out.push('─');
        }
    }
    let _ = write!(out, "─╮{RESET}\r\n");

    // Body lines
    let lines = wrap_text(&block.body, inner);
    let body_lines = if lines.is_empty() { vec![String::new()] } else { lines };
    for line in &body_lines {
        let visible_len = line.chars().count();
        let padding = inner.saturating_sub(visible_len);
        let _ = write!(out, "\r{color}│{RESET} {line}");
        for _ in 0..padding {
            out.push(' ');
        }
        let _ = write!(out, " {color}│{RESET}\r\n");
    }

    // Bottom border
    let _ = write!(out, "\r{color}╰");
    for _ in 0..(w - 2) {
        out.push('─');
    }
    let _ = write!(out, "╯{RESET}\r\n");

    out.into_bytes()
}

pub fn render_thinking_indicator() -> Vec<u8> {
    format!("{DIM_MAGENTA}⠋ thinking...{RESET}\r\n").into_bytes()
}

pub fn render_agent_input_prompt(input: &str, cursor_pos: usize) -> Vec<u8> {
    // Show: 🤖 > {input_before_cursor}█{input_after_cursor}
    // Use bold for the prompt prefix.
    let before: String = input.chars().take(cursor_pos).collect();
    let after: String = input.chars().skip(cursor_pos).collect();

    let mut out = String::new();
    let _ = write!(out, "{BOLD}🤖 > {RESET}{before}");
    // Reverse-video block cursor on the char at cursor_pos, or a space if at end.
    let cursor_char = input.chars().nth(cursor_pos).unwrap_or(' ');
    let _ = write!(out, "{REVERSE}{cursor_char}{RESET}");
    if !after.is_empty() {
        // after starts with cursor_char which we already rendered
        let rest: String = after.chars().skip(1).collect();
        let _ = write!(out, "{rest}");
    }
    out.into_bytes()
}

pub fn render_status_bar(mode: &str, model: &str, hint: &str, width: u16) -> Vec<u8> {
    let w = width as usize;
    let left = format!(" {mode} │ {model}");
    let right = format!("{hint} ");

    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let gap = w.saturating_sub(left_len + right_len);

    let mut bar = String::with_capacity(w + 32);
    let _ = write!(bar, "{REVERSE}");
    bar.push_str(&left);
    for _ in 0..gap {
        bar.push(' ');
    }
    bar.push_str(&right);
    // If content was shorter than width, we already padded. If longer, truncation
    // happened implicitly (we just don't pad).
    let _ = write!(bar, "{RESET}");
    bar.into_bytes()
}

pub fn clear_line() -> Vec<u8> {
    b"\x1b[2K\r".to_vec()
}

pub fn move_to_status_bar_row(rows: u16) -> Vec<u8> {
    format!("{CSI}{rows};1H").into_bytes()
}

pub fn save_cursor() -> Vec<u8> {
    b"\x1b[s".to_vec()
}

pub fn restore_cursor() -> Vec<u8> {
    b"\x1b[u".to_vec()
}

pub fn set_scroll_region(top: u16, bottom: u16) -> Vec<u8> {
    format!("{CSI}{top};{bottom}r").into_bytes()
}

pub fn reset_scroll_region() -> Vec<u8> {
    b"\x1b[r".to_vec()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut col = 0usize;
        for word in raw_line.split_inclusive(' ') {
            let wlen = word.chars().count();
            if col > 0 && col + wlen > width {
                result.push(current);
                current = String::new();
                col = 0;
            }
            // Hard-break words longer than width
            if wlen > width && col == 0 {
                for ch in word.chars() {
                    if col >= width {
                        result.push(current);
                        current = String::new();
                        col = 0;
                    }
                    current.push(ch);
                    col += 1;
                }
            } else {
                current.push_str(word);
                col += wlen;
            }
        }
        if !current.is_empty() || col == 0 {
            result.push(current);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_block_contains_box_drawing() {
        let block = AgentBlock {
            kind: BlockKind::AgentText,
            header: Some("test header".into()),
            body: "hello world".into(),
        };
        let out = render_block(&block, 40);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains('╭'));
        assert!(s.contains('╮'));
        assert!(s.contains('╰'));
        assert!(s.contains('╯'));
        assert!(s.contains('│'));
        assert!(s.contains("test header"));
        assert!(s.contains("hello world"));
    }

    #[test]
    fn render_block_empty_body() {
        let block = AgentBlock {
            kind: BlockKind::Error,
            header: None,
            body: String::new(),
        };
        let out = render_block(&block, 20);
        let s = String::from_utf8(out).unwrap();
        // Should still produce a box with an empty content line
        assert!(s.contains('╭'));
        assert!(s.contains('╯'));
        let line_count = s.lines().count();
        assert!(line_count >= 3, "expected at least 3 lines (top, body, bottom), got {line_count}");
    }

    #[test]
    fn render_block_multiline_body() {
        let block = AgentBlock {
            kind: BlockKind::CommandOutput,
            header: None,
            body: "line one\nline two\nline three".into(),
        };
        let out = render_block(&block, 30);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("line one"));
        assert!(s.contains("line two"));
        assert!(s.contains("line three"));
    }

    #[test]
    fn render_block_wraps_long_lines() {
        let block = AgentBlock {
            kind: BlockKind::AgentText,
            header: None,
            body: "a ".repeat(50), // 100 chars of "a a a ..."
        };
        let out = render_block(&block, 20);
        let s = String::from_utf8(out).unwrap();
        // With width=20, inner=16, so the long body must wrap across multiple lines
        let body_lines: Vec<&str> = s.lines().filter(|l| l.contains('│') && !l.contains('╭') && !l.contains('╰')).collect();
        assert!(body_lines.len() > 1, "expected wrapping, got {} body lines", body_lines.len());
    }

    #[test]
    fn render_block_too_narrow_returns_empty() {
        let block = AgentBlock {
            kind: BlockKind::AgentText,
            header: None,
            body: "hi".into(),
        };
        let out = render_block(&block, 5);
        assert!(out.is_empty());
    }

    #[test]
    fn status_bar_visible_width() {
        let out = render_status_bar("AGENT", "gpt-4", "Ctrl+C to cancel", 60);
        let s = String::from_utf8(out).unwrap();
        // Strip ANSI escapes and measure visible chars
        let visible: String = strip_ansi(&s);
        assert_eq!(
            visible.chars().count(),
            60,
            "expected 60 visible chars, got {}: {:?}",
            visible.chars().count(),
            visible
        );
    }

    #[test]
    fn status_bar_wide() {
        let out = render_status_bar("SHELL", "claude", "q: quit", 120);
        let s = String::from_utf8(out).unwrap();
        let visible = strip_ansi(&s);
        assert_eq!(visible.chars().count(), 120);
    }

    #[test]
    fn wrap_text_basic() {
        let lines = wrap_text("hello world foo bar", 10);
        for line in &lines {
            assert!(line.chars().count() <= 10, "line too long: {line:?}");
        }
        let joined = lines.join(" ");
        assert!(joined.contains("hello"));
        assert!(joined.contains("bar"));
    }

    #[test]
    fn wrap_text_hard_break() {
        let long = "abcdefghijklmnop"; // 16 chars, no spaces
        let lines = wrap_text(long, 5);
        for line in &lines {
            assert!(line.chars().count() <= 5, "line too long: {line:?}");
        }
        let reassembled: String = lines.concat();
        assert_eq!(reassembled, long);
    }

    #[test]
    fn wrap_text_preserves_newlines() {
        let lines = wrap_text("a\nb\nc", 80);
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn wrap_text_empty() {
        let lines = wrap_text("", 10);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn clear_line_bytes() {
        assert_eq!(clear_line(), b"\x1b[2K\r");
    }

    #[test]
    fn save_restore_cursor() {
        assert_eq!(save_cursor(), b"\x1b[s");
        assert_eq!(restore_cursor(), b"\x1b[u");
    }

    #[test]
    fn scroll_region_sequences() {
        assert_eq!(set_scroll_region(1, 24), b"\x1b[1;24r");
        assert_eq!(reset_scroll_region(), b"\x1b[r");
    }

    #[test]
    fn move_to_row() {
        assert_eq!(move_to_status_bar_row(25), b"\x1b[25;1H");
    }

    #[test]
    fn thinking_indicator_contains_text() {
        let out = render_thinking_indicator();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("thinking..."));
        assert!(s.contains("⠋"));
    }

    #[test]
    fn agent_input_prompt_basic() {
        let out = render_agent_input_prompt("hello", 3);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("🤖 > "));
        assert!(s.contains("hel"));
    }

    #[test]
    fn agent_input_prompt_cursor_at_end() {
        let out = render_agent_input_prompt("abc", 3);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("🤖 > "));
        assert!(s.contains("abc"));
    }

    #[test]
    fn render_block_command_run_has_yellow() {
        let block = AgentBlock {
            kind: BlockKind::CommandRun,
            header: Some("Running: ls".into()),
            body: "output".into(),
        };
        let out = render_block(&block, 40);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[2;33m"), "expected dim yellow ANSI code");
    }

    #[test]
    fn render_block_uses_crlf() {
        let block = AgentBlock {
            kind: BlockKind::AgentText,
            header: Some("test".into()),
            body: "body".into(),
        };
        let out = render_block(&block, 30);
        // Every line ending must be \r\n for raw-mode terminals.
        assert!(!out.windows(1).any(|w| w == b"\n") || out.windows(2).filter(|w| w == b"\r\n").count() > 0,
            "render_block must use \\r\\n line endings");
        // More specific: count bare \n (not preceded by \r)
        let bare_lf = out.windows(2).enumerate().filter(|(_i, w)| {
            w[1] == b'\n' && w[0] != b'\r'
        }).count();
        // Also check the first byte isn't a bare \n
        let first_is_bare_lf = out.first() == Some(&b'\n');
        assert!(!first_is_bare_lf && bare_lf == 0,
            "found {bare_lf} bare LF(s) without preceding CR");
    }

    #[test]
    fn thinking_indicator_uses_crlf() {
        let out = render_thinking_indicator();
        assert!(out.ends_with(b"\r\n"), "thinking indicator must end with \\r\\n");
    }

    /// Strip ANSI escape sequences for visible-width assertions.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // Skip until a letter (the terminator of the escape sequence)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}
