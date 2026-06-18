use unicode_general_category::{get_general_category, GeneralCategory};

/// The default word-boundary characters.
pub const DEFAULT_WORD_BOUNDARY_CHARS: [char; 33] = [
    '`', '~', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '-', '=', '+', '[', '{', ']', '}',
    '\\', '|', ';', ':', '\'', '"', ',', '.', '<', '>', '/', '?', '«', '»',
];

/// Default subword-boundary characters: basically just underscores for now (snake_case)
pub const SUBWORD_BOUNDARY_CHARS: [char; 1] = ['_'];

/// Split a string slice at the next word boundary, returning before and after the word boundary
///
/// The next word boundary is the transition from not in a word (i.e. in a separator) to
/// in a word. The first slice returned goes from start of the input slice to the word boundary.
/// The second slice returns goes from the start of the new word to the end of the input slice.
pub fn split_at_next_word_start(text: &str) -> (&str, &str) {
    let mut in_word = true;
    let mut byte_index = 0;
    for c in text.chars() {
        if in_word {
            if is_default_word_boundary(c) {
                in_word = false;
            }
        } else if !is_default_word_boundary(c) {
            break;
        }
        byte_index += c.len_utf8();
    }

    text.split_at(byte_index)
}

/// Default logic for determining if a character is a word separator. Word separators are
/// whitespace or a specific set of punctuation characters.
pub fn is_default_word_boundary(c: char) -> bool {
    if c.is_whitespace() || DEFAULT_WORD_BOUNDARY_CHARS.contains(&c) {
        return true;
    }

    // The list above only covers ASCII punctuation, so full-width / CJK punctuation
    // such as `，` (U+FF0C) or `。` (U+3002) would otherwise be treated as part of a
    // word and over-extend double-click selection. Treat non-ASCII Unicode
    // punctuation as a boundary as well, mirroring the file-link separator logic.
    //
    // Only the bracket/quote and "other" punctuation categories are included.
    // Connector punctuation (underscores) is deliberately excluded so it remains a
    // *subword* boundary for snake_case rather than a word boundary, and letters —
    // including CJK ideographs like `你好世界` — stay part of the word so they are not
    // split into single characters.
    if c.is_ascii() {
        return false;
    }
    matches!(
        get_general_category(c),
        GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

/// Logic for determining if a character is a subword separator.
/// Subword separators include all the default word separators
/// (whitespace or a specific set of punctuation characters)
/// and subword-specific separators (underscores, for snake_case).
pub fn is_subword_boundary_char(c: char) -> bool {
    is_default_word_boundary(c) || SUBWORD_BOUNDARY_CHARS.contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_punctuation_and_whitespace_are_boundaries() {
        assert!(is_default_word_boundary(','));
        assert!(is_default_word_boundary('.'));
        assert!(is_default_word_boundary(' '));
        assert!(is_default_word_boundary('\t'));
    }

    #[test]
    fn full_width_and_cjk_punctuation_are_boundaries() {
        // Full-width / CJK punctuation should act as a boundary the same way ASCII
        // punctuation does (see issue: double-click over-extends past `，`).
        for c in [
            '，', // U+FF0C FULLWIDTH COMMA
            '。', // U+3002 IDEOGRAPHIC FULL STOP
            '！', // U+FF01 FULLWIDTH EXCLAMATION MARK
            '？', // U+FF1F FULLWIDTH QUESTION MARK
            '；', // U+FF1B FULLWIDTH SEMICOLON
            '：', // U+FF1A FULLWIDTH COLON
            '、', // U+3001 IDEOGRAPHIC COMMA
            '「', // U+300C LEFT CORNER BRACKET
            '」', // U+300D RIGHT CORNER BRACKET
            '（', // U+FF08 FULLWIDTH LEFT PARENTHESIS
            '）', // U+FF09 FULLWIDTH RIGHT PARENTHESIS
            '《', // U+300A LEFT DOUBLE ANGLE BRACKET
            '》', // U+300B RIGHT DOUBLE ANGLE BRACKET
        ] {
            assert!(
                is_default_word_boundary(c),
                "expected {c:?} (U+{:04X}) to be a word boundary",
                c as u32
            );
        }
    }

    #[test]
    fn cjk_ideographs_are_not_boundaries() {
        // Ideographs themselves must stay part of the word, otherwise selecting CJK
        // text like `你好世界` would break into single characters.
        for c in ['你', '好', '世', '界', 'a', 'Z', '0'] {
            assert!(
                !is_default_word_boundary(c),
                "expected {c:?} not to be a word boundary"
            );
        }
    }

    #[test]
    fn underscore_is_only_a_subword_boundary() {
        // `_` must remain a subword (snake_case) boundary, not a word boundary, so
        // double-clicking selects the whole `snake_case` identifier.
        assert!(!is_default_word_boundary('_'));
        assert!(is_subword_boundary_char('_'));
    }

    #[test]
    fn split_stops_at_full_width_punctuation() {
        // Double-clicking `test` in `test，next` should not pull in the following word.
        assert_eq!(split_at_next_word_start("test，next"), ("test，", "next"));
        // ASCII comma already behaved this way; confirm it is unchanged.
        assert_eq!(split_at_next_word_start("test,next"), ("test,", "next"));
        // Pure CJK runs are a single word (no internal boundaries).
        assert_eq!(split_at_next_word_start("你好世界"), ("你好世界", ""));
    }
}
