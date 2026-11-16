/// Helper function to wrap text to fit within a maximum width, breaking on word boundaries.
pub fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let needs_space = !current.is_empty();
        let prospective_len = current.len() + if needs_space { 1 } else { 0 } + word.len();

        if current.is_empty() {
            if word.len() <= max_width {
                current.push_str(word);
            } else {
                // Single word longer than max width: emit it as its own line
                lines.push(word.to_string());
            }
        } else if prospective_len <= max_width {
            if needs_space {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            // Push current line and start a new one with the word (or emit if too long)
            if !current.is_empty() {
                if current.len() < max_width {
                    let pad = max_width - current.len();
                    current.extend(std::iter::repeat_n(' ', pad));
                }
                lines.push(std::mem::take(&mut current));
            }
            if word.len() <= max_width {
                current.push_str(word);
            } else {
                lines.push(word.to_string());
            }
        }
    }

    if !current.is_empty() {
        if current.len() < max_width {
            let pad = max_width - current.len();
            current.extend(std::iter::repeat_n(' ', pad));
        }
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

/// Render a labeled, wrapped field as a single multi-line cell.
pub fn render_labeled_wrapped_field(label: &str, text: &str, width: usize) -> String {
    let mut out = String::new();
    let wrapped = word_wrap(text, width);
    let indent = " ".repeat(label.len() + 2); // align under "{label}: "
    for (i, line) in wrapped.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{label}: {line}"));
        } else {
            out.push('\n');
            out.push_str(&indent);
            out.push_str(line);
        }
    }
    out
}

/// Convert an identifier like a slug (e.g. "slack", "github-actions") into a
/// human-friendly, title-cased name (e.g. "Slack", "Github Actions").
pub fn title_case_identifier(s: &str) -> String {
    let mut parts = Vec::new();
    for part in s.split(|c: char| !c.is_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        let mut chars = part.chars();
        let first = chars
            .next()
            .expect("non-empty identifier component must have a first character");
        let mut word = String::new();
        word.extend(first.to_uppercase());
        word.push_str(&chars.as_str().to_lowercase());
        parts.push(word);
    }
    if parts.is_empty() {
        s.to_string()
    } else {
        parts.join(" ")
    }
}
