pub mod parser;

use parser::{ParsedArgumentResult, ParsedArgumentsIterator};
use std::collections::{HashMap, HashSet};

pub fn get_arguments(template: &str) -> Vec<String> {
    let mut char_to_byte: Vec<usize> = template
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .collect();
    char_to_byte.push(template.len());

    ParsedArgumentsIterator::new(template.chars())
        .filter_map(|parsed| {
            if let ParsedArgumentResult::Valid { .. } = parsed.result() {
                let name_range = parsed.chars_range();

                if name_range.start >= 2 {
                    let name_start_byte = char_to_byte[name_range.start];
                    let name_end_byte = char_to_byte[name_range.end];
                    let name = template[name_start_byte..name_end_byte].to_string();

                    Some(name)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<HashSet<String>>()
        .into_iter()
        .collect()
}

pub fn render_template(template: &str, context: &HashMap<String, String>) -> String {
    // Map char indices to byte indices for slicing.
    let mut char_to_byte: Vec<usize> = template
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .collect();
    char_to_byte.push(template.len());

    let mut out = String::with_capacity(template.len());
    let mut cursor_byte = 0usize;

    for parsed in ParsedArgumentsIterator::new(template.chars()) {
        if let ParsedArgumentResult::Valid { .. } = parsed.result() {
            let name_range = parsed.chars_range();
            // The iterator yields only valid unescaped args without whitespace; the
            // braces are not included in the returned range. For valid args, there
            // must be exactly two braces on each side.
            if name_range.start >= 2 {
                let placeholder_start_char = name_range.start - 2;
                let placeholder_end_char = name_range.end + 2; // exclusive

                let start_byte = char_to_byte[placeholder_start_char];
                let name_start_byte = char_to_byte[name_range.start];
                let name_end_byte = char_to_byte[name_range.end];
                let end_byte = char_to_byte[placeholder_end_char];

                // Append unchanged prefix
                if cursor_byte < start_byte {
                    out.push_str(&template[cursor_byte..start_byte]);
                }

                let var_name = &template[name_start_byte..name_end_byte];
                if let Some(value) = context.get(var_name) {
                    out.push_str(value);
                } else {
                    // If no value provided, keep original placeholder
                    out.push_str(&template[start_byte..end_byte]);
                }

                cursor_byte = end_byte;
            }
        }
    }

    // Append suffix
    if cursor_byte < template.len() {
        out.push_str(&template[cursor_byte..]);
    }

    out
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
