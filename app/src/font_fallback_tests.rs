use super::fallback_font_fn;

#[test]
fn shift_glyph_resolves_to_the_same_family_as_command_glyph() {
    let shift = fallback_font_fn('\u{21E7}').expect("⇧ should have a fallback font");
    let command = fallback_font_fn('\u{2318}').expect("⌘ should have a fallback font");

    assert_eq!(
        shift.name, command.name,
        "⇧ and ⌘ are rendered side-by-side in keybinding chips and must share a font \
         family so their weight and baseline match"
    );
}

#[test]
fn shift_glyph_does_not_resolve_to_hack_nerd_font() {
    let shift = fallback_font_fn('\u{21E7}').expect("⇧ should have a fallback font");

    assert_ne!(
        shift.name, "Hack Nerd Font",
        "Hack Nerd Font has different metrics/weight than the Noto Sans Symbols \
         fonts used for the other mac modifier glyphs"
    );
}

#[test]
fn neighboring_arrow_glyphs_still_resolve_to_hack_nerd_font() {
    // Sanity check that excluding U+21E7 from Hack Nerd Font's claim didn't
    // widen into its neighboring arrow glyphs, which aren't part of this bug.
    let left_arrow = fallback_font_fn('\u{21E0}').expect("left dashed arrow should have a font");
    let down_arrow = fallback_font_fn('\u{21E9}').expect("down white arrow should have a font");

    assert_eq!(left_arrow.name, "Hack Nerd Font");
    assert_eq!(down_arrow.name, "Hack Nerd Font");
}
