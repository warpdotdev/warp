//! Indic script Unicode helpers shared between the PTY input handler and the
//! terminal grid renderer.
//!
//! Layer B shaping fix: treat virama-conjunct second consonants, vowel signs,
//! anusvara, and visarga as zero-width (width-0) in the PTY input path so that
//! each Indic syllable cluster occupies exactly one terminal cell instead of
//! two or three.  The renderer then calls Core Text on the full cluster string
//! stored in that single cell, which produces correctly-shaped conjunct glyphs
//! with no inter-cluster gaps.

// Sinhala block: U+0D80–U+0DFF.  Include it so that U+0DCA (Sinhala virama)
// can actually trigger the conjunct path when paired with a Sinhala consonant.
const INDIC_BLOCK_END: u32 = 0x0DFF;

/// Returns `true` if `c` is an Indic virama (halant / killer mark) — the
/// combining character that, when placed after a consonant, suppresses its
/// inherent vowel and (if followed by another consonant) signals a conjunct.
///
/// Viramas are already Unicode zero-width (Mn) so the PTY assigns them
/// width = 0; they are pushed into the preceding cell by the existing
/// `push_zerowidth` path.  This function is used to detect whether the
/// PREVIOUS cell already has a virama attached (indicating the next
/// consonant should also be pulled into that cell).
#[inline]
pub fn is_indic_virama(c: char) -> bool {
    matches!(
        c,
        '\u{094D}' | // Devanagari
        '\u{09CD}' | // Bengali
        '\u{0A4D}' | // Gurmukhi
        '\u{0ACD}' | // Gujarati
        '\u{0B4D}' | // Odia
        '\u{0BCD}' | // Tamil
        '\u{0C4D}' | // Telugu  ← primary target
        '\u{0CCD}' | // Kannada
        '\u{0D4D}' | // Malayalam
        '\u{0DCA}'   // Sinhala
    )
}

/// Returns `true` if `c` is an Indic spacing combining mark (Unicode category
/// Mc) or candrabindu/anusvara/visarga that visually attaches to the preceding
/// consonant within the same syllable.
///
/// These characters have `unicode_width == 1` (spacing marks occupy a cell in
/// the POSIX/xterm model), so without the Layer B fix the PTY places each in
/// its own cell, creating inter-syllable gaps.  By detecting them here we can
/// override their width to 0 and push them into the preceding cell.
#[inline]
pub fn is_indic_combining(c: char) -> bool {
    matches!(c,
        // ── Telugu (U+0C00–U+0C7F) ──────────────────────────────────────────
        // candrabindu, anusvara, visarga
        '\u{0C01}'..='\u{0C03}' |
        // vowel signs AA … vocalic RR
        '\u{0C3E}'..='\u{0C44}' |
        // vowel signs E, EE, AI
        '\u{0C46}'..='\u{0C48}' |
        // vowel signs O, OO, AU
        '\u{0C4A}'..='\u{0C4C}' |
        // vowel signs vocalic L, vocalic LL
        '\u{0C62}' | '\u{0C63}' |
        // ── Devanagari (U+0900–U+097F) ──────────────────────────────────────
        '\u{0900}'..='\u{0903}' |
        '\u{093A}'..='\u{093C}' |
        '\u{093E}'..='\u{094C}' |
        '\u{0955}'..='\u{0957}' |
        '\u{0962}' | '\u{0963}' |
        // ── Bengali (U+0980–U+09FF) ─────────────────────────────────────────
        '\u{09BE}'..='\u{09C4}' |
        '\u{09C7}' | '\u{09C8}' |
        '\u{09CB}' | '\u{09CC}' |
        '\u{09D7}' |
        // ── Gujarati (U+0A80–U+0AFF) ────────────────────────────────────────
        '\u{0ABE}'..='\u{0AC5}' |
        '\u{0AC7}'..='\u{0AC9}' |
        '\u{0ACB}' | '\u{0ACC}' |
        // ── Tamil (U+0B80–U+0BFF) ───────────────────────────────────────────
        '\u{0BBE}'..='\u{0BC2}' |
        '\u{0BC6}'..='\u{0BC8}' |
        '\u{0BCA}'..='\u{0BCC}' |
        '\u{0BD7}' |
        // ── Kannada (U+0C80–U+0CFF) ─────────────────────────────────────────
        '\u{0CBE}'..='\u{0CC4}' |
        '\u{0CC6}'..='\u{0CC8}' |
        '\u{0CCA}'..='\u{0CCC}' |
        '\u{0CE2}' | '\u{0CE3}' |
        // ── Malayalam (U+0D00–U+0D7F) ───────────────────────────────────────
        '\u{0D3E}'..='\u{0D44}' |
        '\u{0D46}'..='\u{0D48}' |
        '\u{0D4A}'..='\u{0D4C}' |
        '\u{0D57}'
    )
}

/// Returns `true` if `c` falls in one of the South Asian script Unicode blocks
/// (U+0900–U+0DFF: Devanagari … Sinhala), covering all scripts for which
/// `is_indic_virama` recognises a virama codepoint.
#[inline]
pub fn is_in_indic_block(c: char) -> bool {
    matches!(c as u32, 0x0900..=INDIC_BLOCK_END)
}

/// Returns `true` if the string `s` (a cell's combined content) contains any
/// character in an Indic script block.  Used by the renderer to decide whether
/// a cell's Str content should be shaped via the full `layout_line` path
/// (which renders all Core Text glyphs) rather than the single-glyph
/// `glyph_for_string` cache.
#[inline]
pub fn is_indic_str(s: &str) -> bool {
    s.chars().any(is_in_indic_block)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_indic_virama ──────────────────────────────────────────────────────

    #[test]
    fn virama_telugu() {
        assert!(is_indic_virama('\u{0C4D}'), "Telugu virama U+0C4D");
    }

    #[test]
    fn virama_devanagari() {
        assert!(is_indic_virama('\u{094D}'), "Devanagari virama U+094D");
    }

    #[test]
    fn virama_sinhala() {
        assert!(is_indic_virama('\u{0DCA}'), "Sinhala virama U+0DCA");
    }

    #[test]
    fn virama_ascii_not_virama() {
        assert!(!is_indic_virama('a'));
        assert!(!is_indic_virama('\0'));
    }

    // ── is_indic_combining ───────────────────────────────────────────────────

    #[test]
    fn combining_telugu_vowel_sign_u() {
        // 'ు' = U+0C41 — range 0C3E..=0C44
        assert!(is_indic_combining('\u{0C41}'), "'ు' U+0C41");
    }

    #[test]
    fn combining_telugu_anusvara() {
        // 'ం' = U+0C02 — range 0C01..=0C03
        assert!(is_indic_combining('\u{0C02}'), "'ం' U+0C02");
    }

    #[test]
    fn combining_telugu_aa() {
        // 'ా' = U+0C3E
        assert!(is_indic_combining('\u{0C3E}'), "'ా' U+0C3E");
    }

    #[test]
    fn combining_ascii_not_combining() {
        assert!(!is_indic_combining('a'));
        assert!(!is_indic_combining('\0'));
    }

    #[test]
    fn combining_cjk_not_combining() {
        assert!(!is_indic_combining('中'));
    }

    // ── is_in_indic_block ────────────────────────────────────────────────────

    #[test]
    fn block_telugu_consonant() {
        // 'ప' = U+0C2A
        assert!(is_in_indic_block('\u{0C2A}'), "'ప' U+0C2A");
    }

    #[test]
    fn block_telugu_ra() {
        // 'ర' = U+0C30
        assert!(is_in_indic_block('\u{0C30}'), "'ర' U+0C30");
    }

    #[test]
    fn block_devanagari_ka() {
        assert!(is_in_indic_block('\u{0915}'), "Devanagari KA U+0915");
    }

    #[test]
    fn block_sinhala_consonant() {
        // Sinhala LA = U+0DBD — now in range since block extended to 0x0DFF
        assert!(is_in_indic_block('\u{0DBD}'), "Sinhala LA U+0DBD");
    }

    #[test]
    fn block_ascii_excluded() {
        assert!(!is_in_indic_block('a'));
        assert!(!is_in_indic_block('Z'));
    }

    #[test]
    fn block_cjk_excluded() {
        assert!(!is_in_indic_block('中'));
    }

    #[test]
    fn block_nul_excluded() {
        assert!(!is_in_indic_block('\0'));
    }

    // ── is_indic_str ─────────────────────────────────────────────────────────

    #[test]
    fn indic_str_telugu_cluster() {
        assert!(is_indic_str("ప్ర"), "Telugu cluster string");
        assert!(is_indic_str("భు"), "Telugu vowel-attached syllable");
        assert!(is_indic_str("క్ష"), "Telugu conjunct");
    }

    #[test]
    fn indic_str_ascii_false() {
        assert!(!is_indic_str("hello"));
        assert!(!is_indic_str(""));
    }

    #[test]
    fn indic_str_mixed_ascii_indic() {
        // A string with even one Indic char should return true.
        assert!(is_indic_str("xప"));
    }
}
