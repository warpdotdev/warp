use std::collections::HashMap;

use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGKeyCode};

use super::post::PostTarget;
use super::{activation, keycode_cache, window};
use crate::{Key, Target};

/// Manages keyboard state and posts keyboard events to the system.
pub struct Keyboard {
    /// Cache of character-to-keycode mappings for the current keyboard layout.
    cache: HashMap<char, CGKeyCode>,
    /// Where synthesized events are delivered.
    post_target: PostTarget,
    /// The window id and pid of the current window target, used to activate the window before
    /// keyboard events are posted. `None` when targeting the HID tap (screen/frontmost behavior).
    ///
    /// Mouse events activate the target window lazily from the event location; keyboard events
    /// carry no coordinates, so activation must be triggered explicitly here instead.
    window_context: Option<(u32, libc::pid_t)>,
    /// The currently-held modifier flags, accumulated from modifier key-down/up events.
    ///
    /// Synthetic modifier key events posted via `CGEventPostToPid` do not update the session's
    /// modifier state, so we track it ourselves and stamp it onto every key event. Without this,
    /// a shortcut sent as discrete events (e.g. Command-down, n-down, n-up, Command-up) arrives as
    /// a plain "n" and is treated as text rather than as Cmd+N.
    current_flags: CGEventFlags,
    /// The owner (client conversation id) of the background session, tagged onto window
    /// activations so teardown can be scoped to this session. `None` when unowned.
    session_owner: Option<String>,
}

impl Keyboard {
    pub fn new(target: PostTarget) -> Self {
        Self {
            cache: keycode_cache::build_cache(),
            post_target: target,
            window_context: None,
            current_flags: CGEventFlags::empty(),
            session_owner: None,
        }
    }

    /// Sets the owner tagged onto background-window activations triggered by this keyboard.
    pub fn set_session_owner(&mut self, owner: Option<String>) {
        self.session_owner = owner;
    }

    /// Sets where subsequent synthesized key events are delivered. Called per-action so typing
    /// can be routed to a specific background process.
    pub fn set_target(&mut self, target: Target) {
        self.post_target = match target {
            Target::Screen => PostTarget::HidTap,
            Target::Window { pid, .. } => PostTarget::Pid(pid as libc::pid_t),
        };
        self.window_context = match target {
            Target::Window { window_id, pid } => Some((window_id, pid as libc::pid_t)),
            Target::Screen => None,
        };
    }

    /// Sends a key down event for the given key.
    ///
    /// If the key is a modifier, its mask is folded into the held-modifier flags first so the
    /// key-down itself carries the now-active modifier; the flags are then stamped on the event.
    pub fn key_down(&mut self, key: &Key) -> Result<(), String> {
        self.ensure_window_activated();
        let keycode = self.resolve_keycode(key)?;
        if let Some(mask) = modifier_mask(keycode) {
            self.current_flags |= mask;
        }
        post_key_event(keycode, true, self.current_flags, self.post_target)
    }

    /// Sends a key up event for the given key.
    ///
    /// If the key is a modifier, its mask is removed from the held-modifier flags so the key-up
    /// reflects the modifier being released; the updated flags are then stamped on the event.
    ///
    /// Activation is not triggered here: `KeyUp` always follows a `KeyDown` that already
    /// activated the window, and a lone `KeyUp` with no prior `KeyDown` is a no-op regardless.
    pub fn key_up(&mut self, key: &Key) -> Result<(), String> {
        let keycode = self.resolve_keycode(key)?;
        if let Some(mask) = modifier_mask(keycode) {
            self.current_flags &= !mask;
        }
        post_key_event(keycode, false, self.current_flags, self.post_target)
    }

    /// Simulates typing text by sending Quartz events.
    pub fn type_text(&self, text: &str) -> Result<(), String> {
        self.ensure_window_activated();
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);

        // Send one character at a time for better compatibility with various applications.
        for ch in text.chars() {
            // For now, send each character using the unicode method.  This is easier than using
            // virtual key codes, but may not be supported in all applications.
            //
            // TODO(vorporeal): when sending an ASCII character, send it using virtual key codes
            // for better compatibility.
            type_unicode_char(ch, source.as_deref(), self.post_target)?;
        }

        Ok(())
    }

    /// Ensures the target window is activated before keyboard events are posted.
    ///
    /// Mouse events activate the target window lazily from the event location (see
    /// [`super::mouse::Mouse`]); keyboard events carry no coordinates, so activation must be
    /// triggered here instead. The call is idempotent per `(pid, window)` pair, so calling it
    /// before each keyboard event is cheap and never re-sends the activation primer.
    fn ensure_window_activated(&self) {
        let Some((window_id, pid)) = self.window_context else {
            return;
        };
        if let Some(info) = window::window_by_id(window_id) {
            activation::ensure_activated(pid, &info, self.session_owner.as_deref());
        }
    }

    /// Resolves a Key to a CGKeyCode.
    ///
    /// The key can be:
    /// - A keycode (platform-specific virtual keycode)
    /// - A character (looked up via the current keyboard layout)
    fn resolve_keycode(&self, key: &Key) -> Result<CGKeyCode, String> {
        match key {
            Key::Keycode(code) => CGKeyCode::try_from(*code).map_err(|_| {
                format!(
                    "Invalid keycode {code}: must be in range 0..={}",
                    CGKeyCode::MAX
                )
            }),
            Key::Char(ch) => self
                .cache
                .get(ch)
                .copied()
                .ok_or_else(|| format!("No keycode found for character '{}'", ch)),
        }
    }
}

/// Maps a modifier virtual keycode to its `CGEventFlags` mask, handling both the left and right
/// variants. Returns `None` for non-modifier keys.
fn modifier_mask(keycode: CGKeyCode) -> Option<CGEventFlags> {
    Some(match keycode {
        // Command: left 0x37 (55), right 0x36 (54).
        54 | 55 => CGEventFlags::MaskCommand,
        // Shift: left 0x38 (56), right 0x3C (60).
        56 | 60 => CGEventFlags::MaskShift,
        // Control: left 0x3B (59), right 0x3E (62).
        59 | 62 => CGEventFlags::MaskControl,
        // Option/Alt: left 0x3A (58), right 0x3D (61).
        58 | 61 => CGEventFlags::MaskAlternate,
        // Fn: 0x3F (63).
        63 => CGEventFlags::MaskSecondaryFn,
        // Caps Lock: 0x39 (57).
        57 => CGEventFlags::MaskAlphaShift,
        _ => return None,
    })
}

/// Posts a key event (down or up) for the given virtual keycode, stamping the currently-held
/// modifier flags so shortcuts route through the app's key-equivalent handling.
fn post_key_event(
    keycode: CGKeyCode,
    is_down: bool,
    flags: CGEventFlags,
    target: PostTarget,
) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let event =
        CGEvent::new_keyboard_event(source.as_deref(), keycode, is_down).ok_or_else(|| {
            let direction = if is_down { "down" } else { "up" };
            format!("Failed to create key {direction} event for keycode {keycode}")
        })?;
    CGEvent::set_flags(Some(&event), flags);
    target.post(&event);
    Ok(())
}

/// Generates a Quartz event signifying the typing of a single Unicode character.
fn type_unicode_char(
    ch: char,
    source: Option<&CGEventSource>,
    target: PostTarget,
) -> Result<(), String> {
    let mut buf = [0u16; 2];
    let encoded = ch.encode_utf16(&mut buf);

    // Create a key down event (virtual key code 0 is used as a placeholder).
    let key_down = CGEvent::new_keyboard_event(source, 0, true)
        .ok_or("Failed to create key down event for TypeText.")?;

    // Set the unicode string on the event.
    // Safety: encoded is a valid UTF-16 buffer with the correct length.
    unsafe {
        CGEvent::keyboard_set_unicode_string(
            Some(&key_down),
            encoded.len() as u64,
            encoded.as_ptr(),
        );
    }

    // Clear any modifier flags that might interfere.
    CGEvent::set_flags(Some(&key_down), CGEventFlags::empty());

    // Post the key down event.
    target.post(&key_down);

    // Create and post a corresponding key up event.
    let key_up = CGEvent::new_keyboard_event(source, 0, false)
        .ok_or("Failed to create key up event for TypeText.")?;
    CGEvent::set_flags(Some(&key_up), CGEventFlags::empty());
    target.post(&key_up);

    Ok(())
}
