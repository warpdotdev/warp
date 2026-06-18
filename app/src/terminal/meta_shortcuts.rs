use warpui::keymap::Keystroke;

/// Whether this keystroke should dispatch an action in Warp despite the
/// [`warpui::event::Event::KeyDown::is_composing`] being true.
///
/// Generally, we ignore all `KeyDown` events if the `is_composing` field is true. However it's
/// possible to have keybinding conflicts between terminal apps which use the meta key and MacOS
/// "dead keys". Dead keys are used to add diacritical marks to other characters. They are
/// triggered by ⌥ and another letter. On the US layout, the dead keys are ⌥e, ⌥u, ⌥i, ⌥n, and ⌥`.
/// They are different for other layouts, e.g. the Croatian layout has ⌥k. Ideally, we could check
/// if a specific keystroke is a dead key. AFAICT the OS doesn't expose this, and it would be too
/// difficult to maintain a map of layouts to lists of dead keys. Therefore, this function just
/// checks if the keystroke is meta + one letter.
///
/// https://support.apple.com/guide/mac-help/enter-characters-with-accent-marks-on-mac-mh27474/mac#mchl45cdda7f
///
/// This function is intended to be used when handling [`warpui::event::Event::KeyDown`] when its
/// `is_composing` is true. Returning true _does not_ mean the keystroke is a dead key necessarily.
/// Interpreting it that way would result in false-positives. Rather, it means the app may dispatch
/// an action for the keystroke _despite_ the fact that `is_composing` is true.
/// Note that this is only relevant with [`crate::settings::ExtraMetaKeys`] enabled.
pub(super) fn handle_keystroke_despite_composing(keystroke: &Keystroke) -> bool {
    // This conflict only occurs on MacOS.
    if !cfg!(target_os = "macos") {
        return false;
    }
    if keystroke.cmd || keystroke.ctrl || keystroke.shift || keystroke.alt || !keystroke.meta {
        return false;
    }
    if keystroke.key.len() != 1 {
        return false;
    }
    true
}

#[cfg(all(test, target_os = "macos"))]
#[path = "meta_shortcuts_tests.rs"]
mod tests;
