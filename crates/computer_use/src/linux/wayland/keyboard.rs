//! Keyboard input handling for Wayland via the RemoteDesktop portal.

use std::collections::HashSet;

use ashpd::desktop::Session;
use ashpd::desktop::remote_desktop::{KeyState, RemoteDesktop};

use super::super::keysym::{XK_SHIFT_L, char_to_keysym, keysym_needs_shift};
use crate::Key;

/// Keyboard state for tracking auto-shifted keys.
pub struct Keyboard {
    /// Keysyms currently held that required auto-shift.
    /// When non-empty, shift is being held by us.
    auto_shift_keys: HashSet<i32>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            auto_shift_keys: HashSet::new(),
        }
    }

    /// Types a string of text by sending keysym events.
    pub async fn type_text<'a>(
        &mut self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        text: &str,
    ) -> Result<(), String> {
        for ch in text.chars() {
            self.type_char(remote_desktop, session, ch).await?;
        }
        Ok(())
    }

    /// Sends a key down event for the given key.
    ///
    /// For `Key::Char`, this will automatically press shift if needed.
    pub async fn key_down<'a>(
        &mut self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        key: &Key,
    ) -> Result<(), String> {
        let (keysym, needs_shift) = self.resolve_key(key);

        if needs_shift {
            // Press shift only if this is the first auto-shifted key.
            if self.auto_shift_keys.is_empty() {
                self.press_shift(remote_desktop, session).await?;
            }
            self.auto_shift_keys.insert(keysym);
        }

        remote_desktop
            .notify_keyboard_keysym(session, keysym, KeyState::Pressed)
            .await
            .map_err(|e| format!("Failed to send key down: {e}"))
    }

    /// Sends a key up event for the given key.
    ///
    /// For `Key::Char`, this will automatically release shift if it was auto-pressed
    /// and this is the last auto-shifted key being released.
    pub async fn key_up<'a>(
        &mut self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        key: &Key,
    ) -> Result<(), String> {
        let (keysym, _) = self.resolve_key(key);

        remote_desktop
            .notify_keyboard_keysym(session, keysym, KeyState::Released)
            .await
            .map_err(|e| format!("Failed to send key up: {e}"))?;

        // Release shift only if this key was auto-shifted and it's the last one.
        if self.auto_shift_keys.remove(&keysym) && self.auto_shift_keys.is_empty() {
            self.release_shift(remote_desktop, session).await?;
        }

        Ok(())
    }

    async fn type_char<'a>(
        &mut self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
        ch: char,
    ) -> Result<(), String> {
        let keysym = char_to_keysym(ch) as i32;
        let needs_shift = keysym_needs_shift(keysym as u32);

        // Press shift if needed, then press and release the key, then release shift.
        if needs_shift {
            self.press_shift(remote_desktop, session).await?;
        }

        remote_desktop
            .notify_keyboard_keysym(session, keysym, KeyState::Pressed)
            .await
            .map_err(|e| format!("Failed to send key press: {e}"))?;

        remote_desktop
            .notify_keyboard_keysym(session, keysym, KeyState::Released)
            .await
            .map_err(|e| format!("Failed to send key release: {e}"))?;

        if needs_shift {
            self.release_shift(remote_desktop, session).await?;
        }

        Ok(())
    }

    /// Resolves a `Key` to a keysym and shift requirement.
    fn resolve_key(&self, key: &Key) -> (i32, bool) {
        match key {
            Key::Keycode(keysym) => {
                // Key::Keycode uses X11 keysyms, which we can use directly.
                (*keysym, false)
            }
            Key::Char(ch) => {
                let keysym = char_to_keysym(*ch) as i32;
                let needs_shift = keysym_needs_shift(keysym as u32);
                (keysym, needs_shift)
            }
        }
    }

    async fn press_shift<'a>(
        &self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
    ) -> Result<(), String> {
        remote_desktop
            .notify_keyboard_keysym(session, XK_SHIFT_L as i32, KeyState::Pressed)
            .await
            .map_err(|e| format!("Failed to press shift: {e}"))
    }

    async fn release_shift<'a>(
        &self,
        remote_desktop: &RemoteDesktop<'a>,
        session: &Session<'a, RemoteDesktop<'a>>,
    ) -> Result<(), String> {
        remote_desktop
            .notify_keyboard_keysym(session, XK_SHIFT_L as i32, KeyState::Released)
            .await
            .map_err(|e| format!("Failed to release shift: {e}"))
    }
}
