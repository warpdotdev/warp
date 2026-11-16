//! Keyboard input handling for X11 using XTEST.

use std::collections::HashSet;

use x11rb::connection::Connection;
use x11rb::protocol::xproto;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::super::keysym::{UPPERCASE_KEYSYMS, XK_SHIFT_L, char_to_keysym};
use crate::Key;

/// A resolved key with its keycode and shift requirement.
struct ResolvedKey {
    keycode: u8,
    needs_shift: bool,
}

/// Keyboard state tracking for an X11 connection.
pub struct Keyboard<'a> {
    conn: &'a RustConnection,
    keyboard_mapping: &'a xproto::GetKeyboardMappingReply,
    /// Keycodes currently held that required auto-shift.
    /// When non-empty, shift is being held by us.
    auto_shift_keys: HashSet<u8>,
}

impl<'a> Keyboard<'a> {
    pub fn new(
        conn: &'a RustConnection,
        keyboard_mapping: &'a xproto::GetKeyboardMappingReply,
    ) -> Self {
        Self {
            conn,
            keyboard_mapping,
            auto_shift_keys: HashSet::new(),
        }
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        // For each character, we need to find a keycode that produces it.
        // This is complex because X11 keycodes depend on the keyboard layout.
        for ch in text.chars() {
            self.type_char(ch)?;
        }
        Ok(())
    }

    /// Sends a key down event for the given key.
    ///
    /// For `Key::Char`, this will automatically press shift if needed.
    pub fn key_down(&mut self, key: &Key) -> Result<(), String> {
        let resolved = self.resolve_key(key)?;
        if resolved.needs_shift {
            // Press shift only if this is the first auto-shifted key.
            if self.auto_shift_keys.is_empty() {
                self.press_shift()?;
            }
            self.auto_shift_keys.insert(resolved.keycode);
        }
        self.key_press(resolved.keycode)
    }

    /// Sends a key up event for the given key.
    ///
    /// For `Key::Char`, this will automatically release shift if it was auto-pressed
    /// and this is the last auto-shifted key being released.
    pub fn key_up(&mut self, key: &Key) -> Result<(), String> {
        let resolved = self.resolve_key(key)?;
        self.key_release(resolved.keycode)?;
        // Release shift only if this key was auto-shifted and it's the last one.
        if self.auto_shift_keys.remove(&resolved.keycode) && self.auto_shift_keys.is_empty() {
            self.release_shift()?;
        }
        Ok(())
    }

    fn type_char(&mut self, ch: char) -> Result<(), String> {
        let resolved = self.resolve_key_for_char(ch)?;

        // Press shift if needed, then press and release the key, then release shift.
        if resolved.needs_shift {
            self.press_shift()?;
        }
        self.key_press(resolved.keycode)?;
        self.key_release(resolved.keycode)?;
        if resolved.needs_shift {
            self.release_shift()?;
        }
        Ok(())
    }

    /// Resolves a `Key` to a keycode and shift requirement.
    fn resolve_key(&self, key: &Key) -> Result<ResolvedKey, String> {
        match key {
            Key::Keycode(keysym) => {
                if *keysym < 0 {
                    return Err(format!("Invalid keysym: {keysym} (must be non-negative)"));
                }
                let keysym = *keysym as u32;
                let keycode = self.find_keycode_for_keysym(keysym)?;
                // For explicit keysyms, caller manages modifiers.
                Ok(ResolvedKey {
                    keycode,
                    needs_shift: false,
                })
            }
            Key::Char(ch) => self.resolve_key_for_char(*ch),
        }
    }

    /// Resolves a character to a keycode and shift requirement.
    fn resolve_key_for_char(&self, ch: char) -> Result<ResolvedKey, String> {
        let keysym = char_to_keysym(ch);
        let keycode = self.find_keycode_for_keysym(keysym)?;
        let needs_shift = self.keysym_needs_shift(keysym, keycode);
        Ok(ResolvedKey {
            keycode,
            needs_shift,
        })
    }

    /// Presses the shift key.
    fn press_shift(&mut self) -> Result<(), String> {
        let shift_keycode = self.find_keycode_for_keysym(XK_SHIFT_L)?;
        self.key_press(shift_keycode)
    }

    /// Releases the shift key.
    fn release_shift(&mut self) -> Result<(), String> {
        let shift_keycode = self.find_keycode_for_keysym(XK_SHIFT_L)?;
        self.key_release(shift_keycode)
    }

    fn key_press(&mut self, keycode: u8) -> Result<(), String> {
        self.conn
            .xtest_fake_input(
                xproto::KEY_PRESS_EVENT,
                keycode,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| format!("Failed to send key press: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    fn key_release(&mut self, keycode: u8) -> Result<(), String> {
        self.conn
            .xtest_fake_input(
                xproto::KEY_RELEASE_EVENT,
                keycode,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| format!("Failed to send key release: {e}"))?;

        self.conn
            .flush()
            .map_err(|e| format!("Failed to flush X11 connection: {e}"))?;

        Ok(())
    }

    fn find_keycode_for_keysym(&self, keysym: u32) -> Result<u8, String> {
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let keysyms_per_keycode = self.keyboard_mapping.keysyms_per_keycode as usize;

        // Search for a keycode that produces the desired keysym.
        for keycode in min_keycode..=max_keycode {
            let offset = (keycode - min_keycode) as usize * keysyms_per_keycode;
            for i in 0..keysyms_per_keycode {
                if offset + i < self.keyboard_mapping.keysyms.len()
                    && self.keyboard_mapping.keysyms[offset + i] == keysym
                {
                    return Ok(keycode);
                }
            }
        }

        // Try to find the unshifted version for uppercase letters.
        if UPPERCASE_KEYSYMS.contains(&keysym) {
            let lower = keysym + 0x20;
            return self.find_keycode_for_keysym(lower);
        }

        Err(format!(
            "No keycode found for keysym 0x{:x} (char: {:?})",
            keysym,
            char::from_u32(keysym)
        ))
    }

    fn keysym_needs_shift(&self, keysym: u32, keycode: u8) -> bool {
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;

        let keysyms_per_keycode = self.keyboard_mapping.keysyms_per_keycode as usize;
        let offset = (keycode - min_keycode) as usize * keysyms_per_keycode;

        // If the keysym is in position 0, no shift needed.
        // If it's in position 1, shift is needed.
        if offset < self.keyboard_mapping.keysyms.len()
            && self.keyboard_mapping.keysyms[offset] == keysym
        {
            return false;
        }
        if offset + 1 < self.keyboard_mapping.keysyms.len()
            && self.keyboard_mapping.keysyms[offset + 1] == keysym
        {
            return true;
        }

        // For uppercase letters, assume shift is needed.
        UPPERCASE_KEYSYMS.contains(&keysym)
    }
}
