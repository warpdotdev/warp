use std::error;

use super::*;

#[test]
fn test_handle_keystroke_despite_composing() -> Result<(), Box<dyn error::Error>> {
    assert!(handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-i"
    )?));
    assert!(handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-u"
    )?));
    assert!(handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-`"
    )?));
    assert!(handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-n"
    )?));
    assert!(handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-e"
    )?));

    assert!(!handle_keystroke_despite_composing(&Keystroke::parse(
        "alt-i"
    )?));
    assert!(!handle_keystroke_despite_composing(&Keystroke::parse(
        "ctrl-i"
    )?));
    assert!(!handle_keystroke_despite_composing(&Keystroke::parse(
        "meta-shift-I"
    )?));

    Ok(())
}
