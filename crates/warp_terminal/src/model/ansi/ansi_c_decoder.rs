//! This module is used to parse shell output that is ANSI-C quoted. This occurs when `printf` is
//! used with the argument `%q` and escapes non-printable characters with the POSIX
//! `$''` syntax.
//! For more information, see
//! https://www.gnu.org/software/bash/manual/html_node/ANSI_002dC-Quoting.html.

/// Determines if the given shell output is ANSI-C quoted.
pub(super) fn is_ansi_c_quoted(s: &str) -> bool {
    s.starts_with("$\'") && s.ends_with("\'")
}

/// Unescapes shell output that has is ANSI-C quoted (i.e. of the form `$''`).
/// This function guarantees that non-escaped output will be preserved, but ill-formatted escape
/// sequences may not be preserved.
pub(super) fn parse_ansi_c_quoted_string(quoted_string: String) -> String {
    // https://superuser.com/questions/301353/escape-non-printing-characters-in-a-function-for-a-bash-prompt

    // In bash, we trigger an empty block to get the shell to repaint the prompt.
    if quoted_string.trim().is_empty() {
        return quoted_string;
    }
    log::debug!("Attempting to parse the following ANSI C escaped shell output: {quoted_string}");

    let Some(quoted_string_without_prefix) = quoted_string.strip_prefix("$\'") else {
        log::warn!("Tried to parse ANSI-C quoted string but $\' prefix was not present");
        return quoted_string;
    };
    let Some(quoted_string_trimmed) = quoted_string_without_prefix.strip_suffix("\'") else {
        log::warn!("Tried to parse ANSI-C quoted string but but \' suffix was not present");
        return quoted_string;
    };
    let mut unescaped_string = String::new();
    let mut char_indices_iter = quoted_string_trimmed.char_indices().peekable();
    while let Some((current_char_idx, current_char)) = char_indices_iter.next() {
        // First, we check for a character that isn't escaped.
        if current_char != '\\' {
            unescaped_string.push(current_char);
            continue;
        }

        // From here, we assume that the sequence from the current character onwards is an escaped
        // sequence that needs to be un-escaped.
        // We also assume that each character is one byte for the sequence.

        // Strip out ranges of non-visible characters.
        if matches!(
            quoted_string_trimmed.get(current_char_idx..current_char_idx + 4),
            Some("\\001") | Some("\\002")
        ) {
            // Omit these from the result.
            char_indices_iter.nth(2); // Skips the next 3 chars.
            continue;
        }

        // Check for common two byte characters.
        let mut found_two_character_sequence = true;
        let Some((next_character_idx, next_character)) = char_indices_iter.next() else {
            log::warn!("Was parsing escaped sequence but unexpectedly ran out of characters");
            break;
        };
        // Adapted from https://www.gnu.org/software/bash/manual/html_node/ANSI_002dC-Quoting.html.
        match next_character {
            // alert (bell)
            'a' => unescaped_string.push('\x07'),
            // backspace
            'b' => unescaped_string.push('\x08'),
            // an escape character (not ANSI C)
            'E' | 'e' => unescaped_string.push('\x1b'),
            // form feed
            'f' => unescaped_string.push('\x0c'),
            'n' => unescaped_string.push('\n'),
            'r' => unescaped_string.push('\r'),
            't' => unescaped_string.push('\t'),
            // vertical tab
            'v' => unescaped_string.push('\x0b'),
            '\\' => unescaped_string.push('\\'),
            '\'' => unescaped_string.push('\''),
            '\"' => unescaped_string.push('\"'),
            '?' => unescaped_string.push('?'),
            _ => {
                found_two_character_sequence = false;
            }
        }

        if found_two_character_sequence {
            continue;
        }

        // Instead of accumulating characters using the chars iterator, we assume that each
        // character is one byte. If a multi-byte character is present, then the sequence is
        // formatted incorrectly and this will fail.
        match next_character {
            // \xHH
            // the eight-bit character whose value is the hexadecimal value HH (one or two hex digits)
            'x' => {
                let start_idx = next_character_idx + next_character.len_utf8();
                let Some(hex_value) = quoted_string_trimmed.get(start_idx..start_idx + 2) else {
                    log::warn!("Could not parse value of the form \\xHH");
                    continue;
                };
                match u8::from_str_radix(hex_value, 16).map(|n| n as char) {
                    Ok(c) => {
                        unescaped_string.push(c);
                    }
                    Err(err) => {
                        log::warn!("Could not convert \\x{hex_value} into char: {err:#}");
                    }
                }
                char_indices_iter.nth(1); // Skips the next 2 chars.
            }
            // \uHHHH
            // the Unicode (ISO/IEC 10646) character whose value is the hexadecimal value HHHH (one to four hex digits)
            'u' => {
                let start_idx = next_character_idx + next_character.len_utf8();
                let Some(hex_value) = quoted_string_trimmed.get(start_idx..start_idx + 4) else {
                    log::warn!("Could not parse value of the form \\uHHHH");
                    continue;
                };
                match u32::from_str_radix(hex_value, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(c) => {
                        unescaped_string.push(c);
                    }
                    None => {
                        log::warn!("Could not convert \\x{hex_value} into char");
                    }
                }
                char_indices_iter.nth(3); // Skips the next 4 chars.
            }
            // \UHHHHHHHH
            // the Unicode (ISO/IEC 10646) character whose value is the hexadecimal value HHHHHHHH (one to eight hex digits)
            'U' => {
                let start_idx = next_character_idx + next_character.len_utf8();
                let Some(hex_value) = quoted_string_trimmed.get(start_idx..start_idx + 8) else {
                    log::warn!("Could not parse value of the form \\uHHHH");
                    continue;
                };
                match u32::from_str_radix(hex_value, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(c) => {
                        unescaped_string.push(c);
                    }
                    None => {
                        log::warn!("Could not convert \\x{hex_value} into char");
                    }
                }
                char_indices_iter.nth(7); // Skips the next 8 chars.
            }
            // \cx
            // a control-x character
            'c' => {
                if let Some(c) = char_indices_iter.next().map(|(_, c)| c) {
                    // Control character must be within '@'..='_' or the DELETE character.
                    if matches!(c, '\x40'..='\x5f' | '\x7f') {
                        let control_character = ((c as u8) - 64) as char;
                        unescaped_string.push(control_character);
                    } else {
                        log::warn!("Found invalid control character");
                    }
                } else {
                    log::warn!("Could not get control character");
                }
            }
            _ => {
                // We assume the input is of the form \nnn, which is the eight-bit character whose
                // value is the octal value nnn (one to three octal digits).
                if let Some(octal_value) =
                    quoted_string_trimmed.get(next_character_idx..next_character_idx + 3)
                {
                    match u32::from_str_radix(octal_value, 8)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        Some(c) => unescaped_string.push(c),
                        None => {
                            log::warn!("Could not parse value of the form \\nnn (octal value)");
                        }
                    }
                } else {
                    log::warn!("Could not get octal value");
                }
                // Skip the next 2 chars, since we've already consumed the first octal digit.
                char_indices_iter.nth(1);
            }
        }
    }

    unescaped_string
}

#[cfg(test)]
#[path = "ansi_c_decoder_tests.rs"]
mod tests;
