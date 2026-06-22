use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use parking_lot::Mutex;
use winit::event_loop::EventLoopProxy;

use crate::keymap;
use crate::windowing::winit::app::CustomEvent;

/// Responsible for registering system-wide (global) hotkeys with the platform.
pub struct GlobalHotKeyHandler {
    platform_manager: std::cell::OnceCell<GlobalHotKeyManager>,
    /// Maps the [`global_hotkey::hotkey::HotKey::id`], an opaque, hash-based integer, to our
    /// [`keymap::Keystroke`].
    hotkey_map: Arc<Mutex<HashMap<u32, keymap::Keystroke>>>,
    event_loop_proxy: EventLoopProxy<CustomEvent>,
}

impl GlobalHotKeyHandler {
    pub fn new(
        event_loop_proxy: EventLoopProxy<CustomEvent>,
    ) -> Result<Self, global_hotkey::Error> {
        Ok(Self {
            platform_manager: Default::default(),
            hotkey_map: Default::default(),
            event_loop_proxy,
        })
    }

    pub fn register(&self, shortcut: keymap::Keystroke) {
        let hotkey = match hotkey_for_keystroke(&shortcut) {
            Ok(hotkey) => hotkey,
            Err(e) => {
                log::error!("invalid global hotkey: {e:?}");
                return;
            }
        };
        self.platform_manager().register(hotkey);
        self.hotkey_map.lock().insert(hotkey.id(), shortcut);
    }

    pub fn unregister(&self, shortcut: &keymap::Keystroke) {
        let hotkey = match hotkey_for_keystroke(shortcut) {
            Ok(hotkey) => hotkey,
            Err(e) => {
                log::error!("invalid global hotkey: {e:?}");
                return;
            }
        };
        self.platform_manager().unregister(hotkey);
        self.hotkey_map.lock().remove(&hotkey.id());
    }

    /// Returns a reference to a lazily-instantiated [`GlobalHotKeyManager`].
    ///
    /// We do this lazily because the [`GlobalHotKeyManager`] can interfere
    /// with other libraries that use Xlib, leading to crashes.  We don't want
    /// to run the risk of this happening for users who haven't set any global
    /// hotkeys.
    fn platform_manager(&self) -> &GlobalHotKeyManager {
        self.platform_manager.get_or_init(|| {
            let platform_manager =
                GlobalHotKeyManager::new().expect("x11 implementation never actually fails");
            let thread_hotkey_map = self.hotkey_map.clone();
            // When global hotkeys are triggered, events get published to a crossbeam channel.
            // Since crossbeam channels are not async, we don't want to receive this on our
            // background executor's thread pool, as that would block a thread. Therefore, we spawn
            // a dedicated thread for receiving these events.
            let event_loop_proxy = self.event_loop_proxy.clone();
            thread::spawn(move || {
                while let Ok(event) = GlobalHotKeyEvent::receiver().recv() {
                    // Trigger when the hotkey is released, _not_ pressed. This is due to an X11
                    // quirk where focus is transferred out of Warp windows after a global hotkey
                    // is pressed. This breaks our quake mode logic. However, focus is restored
                    // when the hotkey is released.
                    if event.state == HotKeyState::Released {
                        // Lookup the hash-based hotkey ID to the actual keystroke from our
                        // map.
                        if let Some(keystroke) = thread_hotkey_map.lock().get(&event.id) {
                            event_loop_proxy.send_event(CustomEvent::GlobalShortcutTriggered(
                                keystroke.clone(),
                            ));
                        }
                    }
                }
            });
            platform_manager
        })
    }
}

fn hotkey_for_keystroke(
    keystroke: &keymap::Keystroke,
) -> std::result::Result<HotKey, anyhow::Error> {
    let mut mods = Modifiers::empty();
    if keystroke.alt {
        mods |= Modifiers::ALT;
    }
    if keystroke.cmd {
        mods |= Modifiers::SUPER;
    }
    if keystroke.shift {
        mods |= Modifiers::SHIFT;
    }
    if keystroke.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if keystroke.meta {
        mods |= Modifiers::META;
    }
    let key = if keystroke.key.len() == 1 {
        let c = keystroke
            .key
            .chars()
            .next()
            .expect("validated length already");
        match c {
            '`' | '~' => Code::Backquote,
            '-' | '_' => Code::Minus,
            '=' | '+' => Code::Equal,
            '0'..='9' => Code::from_str(&format!("Digit{c}"))?,
            '\t' => Code::Tab,
            '!' => Code::Digit1,
            '@' => Code::Digit2,
            '#' => Code::Digit3,
            '$' => Code::Digit4,
            '%' => Code::Digit5,
            '^' => Code::Digit6,
            '&' => Code::Digit7,
            '*' => Code::Digit8,
            '(' => Code::Digit9,
            ')' => Code::Digit0,
            'a'..='z' | 'A'..='Z' => Code::from_str(&format!("Key{}", c.to_ascii_uppercase()))?,
            '[' | '{' => Code::BracketLeft,
            ']' | '}' => Code::BracketRight,
            '\\' | '|' => Code::Backslash,
            ';' => Code::Semicolon,
            '\'' | '"' => Code::Quote,
            ',' | '<' => Code::Comma,
            '.' | '>' => Code::Period,
            '/' | '?' => Code::Slash,
            'ろ' => Code::IntlRo,
            '¥' => Code::IntlYen,
            ' ' => Code::Space,
            _ => anyhow::bail!("Invalid global hotkey: {c}"),
        }
    } else {
        // Must map each of [`keymap::VALID_SPECIAL_KEYS`] to [`global_hotkey::hotkey::Code`].
        match keystroke.key.as_str() {
            "backspace" => Code::Backspace,
            "tab" => Code::Tab,
            "enter" => Code::Enter,
            "up" => Code::ArrowUp,
            "down" => Code::ArrowDown,
            "left" => Code::ArrowLeft,
            "right" => Code::ArrowRight,
            "home" => Code::Home,
            "end" => Code::End,
            "pageup" => Code::PageUp,
            "pagedown" => Code::PageDown,
            "insert" => Code::Insert,
            "delete" => Code::Delete,
            "escape" => Code::Escape,
            "numpadenter" => Code::NumpadEnter,
            "f1" => Code::F1,
            "f2" => Code::F2,
            "f3" => Code::F3,
            "f4" => Code::F4,
            "f5" => Code::F5,
            "f6" => Code::F6,
            "f7" => Code::F7,
            "f8" => Code::F8,
            "f9" => Code::F9,
            "f10" => Code::F10,
            "f11" => Code::F11,
            "f12" => Code::F12,
            "f13" => Code::F13,
            "f14" => Code::F14,
            "f15" => Code::F15,
            "f16" => Code::F16,
            "f17" => Code::F17,
            "f18" => Code::F18,
            "f19" => Code::F19,
            "f20" => Code::F20,
            s => anyhow::bail!("Invalid global hotkey: {s}"),
        }
    };

    Ok(HotKey::new(Some(mods), key))
}
