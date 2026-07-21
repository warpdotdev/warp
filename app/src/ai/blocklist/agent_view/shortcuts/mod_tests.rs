use super::autoexecute_shortcut_text;

#[test]
fn autoexecute_shortcut_text_describes_the_next_toggle() {
    assert_eq!(autoexecute_shortcut_text(false), "toggle on");
    assert_eq!(autoexecute_shortcut_text(true), "toggle off");
}
