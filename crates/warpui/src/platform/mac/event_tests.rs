use super::keystroke_key;

fn not_consulted() -> Option<String> {
    panic!("the ASCII-capable layout should not be consulted for this keystroke")
}

#[test]
fn a_command_keystroke_on_a_non_latin_letter_takes_the_ascii_capable_key() {
    // Korean 2-Set reports ㅑ for the I key; Cmd+I must still be `cmd-i`.
    assert_eq!(
        keystroke_key("ㅑ", true, || Some("i".to_owned())).as_deref(),
        Some("i")
    );
    // Shift is part of the translation, the way it is on the ASCII-capable layout.
    assert_eq!(
        keystroke_key("ㅑ", true, || Some("I".to_owned())).as_deref(),
        Some("I")
    );
}

#[test]
fn a_keystroke_without_command_keeps_the_layouts_character() {
    assert_eq!(
        keystroke_key("ㅑ", false, not_consulted).as_deref(),
        Some("ㅑ")
    );
}

#[test]
fn an_ascii_character_is_left_alone() {
    assert_eq!(
        keystroke_key("i", true, not_consulted).as_deref(),
        Some("i")
    );
    assert_eq!(
        keystroke_key("!", true, not_consulted).as_deref(),
        Some("!")
    );
}

#[test]
fn a_named_key_keeps_its_name() {
    // AppKit reports the up arrow as the private-use character U+F700.
    assert_eq!(
        keystroke_key("\u{F700}", true, not_consulted).as_deref(),
        Some("up")
    );
}

#[test]
fn a_layout_that_is_already_ascii_capable_keeps_its_own_letters() {
    // German reports ö for its own key, and the ASCII-capable layout is German itself,
    // so the lookup returns nothing and the keystroke stays `cmd-ö`.
    assert_eq!(keystroke_key("ö", true, || None).as_deref(), Some("ö"));
}

#[test]
fn no_characters_means_no_key() {
    assert_eq!(keystroke_key("", true, not_consulted), None);
}
