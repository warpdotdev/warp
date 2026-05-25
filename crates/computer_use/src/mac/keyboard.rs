use std::collections::HashMap;

use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGKeyCode};

use super::keycode_cache;
use super::post::PostTarget;
use crate::Key;

/// Manages keyboard state and posts keyboard events to the system.
pub struct Keyboard {
    /// Cache of character-to-keycode mappings for the current keyboard layout.
    cache: HashMap<char, CGKeyCode>,
    /// Where synthesized events are delivered.
    target: PostTarget,
}

impl Keyboard {
    pub fn new(target: PostTarget) -> Self {
        Self {
            cache: keycode_cache::build_cache(),
            target,
        }
    }

    /// Sends a key down event for the given key.
    pub fn key_down(&self, key: &Key) -> Result<(), String> {
        post_key_down(self.resolve_keycode(key)?, self.target)
    }

    /// Sends a key up event for the given key.
    pub fn key_up(&self, key: &Key) -> Result<(), String> {
        post_key_up(self.resolve_keycode(key)?, self.target)
    }

    /// Simulates typing text by sending Quartz events.
    pub fn type_text(&self, text: &str) -> Result<(), String> {
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);

        // Send one character at a time for better compatibility with various applications.
        for ch in text.chars() {
            // For now, send each character using the unicode method.  This is easier than using
            // virtual key codes, but may not be supported in all applications.
            //
            // TODO(vorporeal): when sending an ASCII character, send it using virtual key codes
            // for better compatibility.
            type_unicode_char(ch, source.as_deref(), self.target)?;
        }

        Ok(())
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

/// Posts a key down event for the given virtual keycode.
fn post_key_down(keycode: CGKeyCode, target: PostTarget) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let event = CGEvent::new_keyboard_event(source.as_deref(), keycode, true)
        .ok_or_else(|| format!("Failed to create key down event for keycode {}", keycode))?;
    target.post(&event);
    Ok(())
}

/// Posts a key up event for the given virtual keycode.
fn post_key_up(keycode: CGKeyCode, target: PostTarget) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let event = CGEvent::new_keyboard_event(source.as_deref(), keycode, false)
        .ok_or_else(|| format!("Failed to create key up event for keycode {}", keycode))?;
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
