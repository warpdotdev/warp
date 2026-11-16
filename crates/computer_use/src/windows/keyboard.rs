//! Keyboard input handling for Windows using SendInput.

use std::collections::HashMap;
use std::mem::size_of;
use std::ptr;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC,
    MapVirtualKeyExW, SendInput, VIRTUAL_KEY, VK_LSHIFT, VK_RSHIFT, VK_SHIFT, VkKeyScanExW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::Key;

/// How a logical [`Key`] was resolved for dispatch.
enum ResolvedKey {
    /// Dispatch via virtual-key code / scan code. May auto-press `VK_SHIFT`.
    Vk { vk: u16, needs_shift: bool },
    /// Dispatch as a UTF-16 code unit via `KEYEVENTF_UNICODE`. Used as a fallback when the layout
    /// would require ctrl/alt to produce the character (e.g., AltGr-accessed keys on European
    /// layouts). Unicode input bypasses the keyboard layout entirely, so no modifier handling is
    /// needed.
    Unicode(u16),
}

/// Bookkeeping for a logical key we've sent a down event for. We store the *resolved* state at
/// `key_down` time so that `key_up` can release exactly what we pressed, even if the active
/// keyboard layout has since changed (e.g., the user hit the IME/layout-switch hotkey between down
/// and up).
enum PressedKey {
    /// Key was dispatched via virtual-key code / scan code.
    Vk {
        /// Virtual-key code that was dispatched on `key_down`.
        vk: u16,
        /// Whether we auto-pressed `VK_SHIFT` for this key; the matching `key_up` is responsible
        /// for releasing shift when the last auto-shifted entry goes away.
        auto_shifted: bool,
    },
    /// Key was dispatched as a UTF-16 code unit via `KEYEVENTF_UNICODE`. Release sends the same
    /// unit up. No shift bookkeeping because Unicode input bypasses the keyboard layout.
    Unicode(u16),
}

/// Manages keyboard state and posts keyboard events to the system.
///
/// Callers must pair each `key_down` with a matching `key_up` for the same logical key before the
/// next `key_down` on that key. `pressed_keys` is keyed by the original logical `Key`, so repeated
/// `key_down` of the same key without an intervening `key_up` overwrites the earlier bookkeeping.
///
/// **Auto-shift contract**: only the *first* `Key::Char` that requires shift while no shift is
/// already held is recorded as the shift "owner" (`auto_shifted: true`). Subsequent shifted chars
/// pressed while shift remains held ride on that first press (`auto_shifted: false`). Releasing
/// the first char releases `VK_SHIFT`, leaving the later chars physically held without shift — the
/// OS will produce unshifted output for them on their eventual `key_up`. Callers that need
/// multiple shifted chars held simultaneously should use `Key::Keycode(VK_SHIFT.0)` directly.
pub struct Keyboard {
    /// Logical keys currently pressed, keyed by the caller-supplied `Key`. Storing the resolved VK
    /// and auto-shift flag here — rather than re-resolving in `key_up` — ensures we release the
    /// exact key we pressed even if the active keyboard layout changes between the two calls.
    pressed_keys: HashMap<Key, PressedKey>,
    /// Set to `true` when a synthetic `VK_SHIFT` release dispatch failed, meaning shift may still
    /// be held in the OS with no `pressed_keys` entry to release it. We retry the release at the
    /// top of every subsequent `key_down` / `key_up` until it succeeds.
    pending_shift_release: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            pressed_keys: HashMap::new(),
            pending_shift_release: false,
        }
    }

    /// Retries a previously-failed synthetic shift release if one is outstanding. Called at the
    /// top of every public mutating entrypoint so a transient `SendInput` failure can't leave
    /// shift stuck across the rest of the session.
    fn flush_pending_shift_release(&mut self, hkl: HKL) {
        if self.pending_shift_release && send_vk(VK_SHIFT.0, true, hkl).is_ok() {
            self.pending_shift_release = false;
        }
    }

    /// Whether any currently-pressed logical key has `auto_shifted: true` — i.e., this
    /// `Keyboard` is responsible for the `VK_SHIFT` currently held down. Short-circuits on the
    /// first match, unlike a count-based check.
    fn has_auto_shifted_press(&self) -> bool {
        self.pressed_keys.values().any(|p| {
            matches!(
                p,
                PressedKey::Vk {
                    auto_shifted: true,
                    ..
                }
            )
        })
    }

    /// Whether any currently-pressed logical key is an explicit shift keycode
    /// (`Key::Keycode(VK_SHIFT | VK_LSHIFT | VK_RSHIFT)`). Used to avoid
    /// synthesizing shift presses/releases on top of the caller's own shift state.
    fn explicit_shift_held(&self) -> bool {
        self.pressed_keys.values().any(|p| {
            matches!(
                p,
                PressedKey::Vk { vk, auto_shifted: false } if is_shift_vk(*vk)
            )
        })
    }

    /// Sends a key down event for the given key.
    ///
    /// For `Key::Char`, this will automatically press shift if needed. We skip the synthetic
    /// shift press if the caller is already holding shift explicitly (via
    /// `Key::Keycode(VK_SHIFT)`), and conversely `key_up` won't synthesize a shift release while
    /// that explicit shift entry is still tracked. The shift press and the main VK press are
    /// batched into a single `SendInput` call so other input can't be interleaved in the common
    /// case. `SendInput` can still partially succeed (e.g., UIPI blocks the main-VK entry after
    /// shift was already delivered); when that happens we best-effort release shift before
    /// returning, and if that release itself fails we mark it pending so a subsequent call can
    /// retry.
    pub fn key_down(&mut self, key: &Key) -> Result<(), String> {
        // Resolve the foreground window's keyboard layout once per public call so every
        // `SendInput` entry we build against it (shift + main VK) sees a consistent snapshot and
        // we avoid three redundant Win32 queries per INPUT.
        let hkl = foreground_keyboard_layout();
        self.flush_pending_shift_release(hkl);

        let resolved = resolve_key(key, hkl)?;
        match resolved {
            ResolvedKey::Vk { vk, needs_shift } => self.key_down_vk(key, vk, needs_shift, hkl),
            ResolvedKey::Unicode(unit) => {
                // Unicode dispatch bypasses the keyboard layout, so no shift bookkeeping is
                // required. A single down event is sufficient; `key_up` will send the matching
                // up event using the unit we record here.
                send_inputs(&[make_unicode_input(unit, false)])?;
                self.pressed_keys
                    .insert(key.clone(), PressedKey::Unicode(unit));
                Ok(())
            }
        }
    }

    /// Shared `Key::Keycode` / shift-auto `Key::Char` path for [`key_down`].
    fn key_down_vk(
        &mut self,
        key: &Key,
        vk: u16,
        needs_shift: bool,
        hkl: HKL,
    ) -> Result<(), String> {
        // Only send a fresh shift press if no other pressed key is already holding shift, whether
        // auto-shifted by us or explicitly pressed by the caller.
        let shift_already_held = self.has_auto_shifted_press() || self.explicit_shift_held();
        let pressed_shift_now = needs_shift && !shift_already_held;

        let mut inputs: Vec<INPUT> = Vec::with_capacity(2);
        if pressed_shift_now {
            inputs.push(build_vk_input(VK_SHIFT.0, false, hkl));
        }
        inputs.push(build_vk_input(vk, false, hkl));

        // Dispatch first; only record the press after `SendInput` succeeds so `pressed_keys`
        // never reflects a press we didn't actually send.
        let (sent, result) = send_inputs_tracked(&inputs);
        if let Err(e) = result {
            // Partial-send: only compensate for shift if it was actually dispatched. When
            // `sent == 0` `SendInput` failed before queueing anything (e.g., UIPI block on the
            // first event), so the shift `INPUT` never reached the OS and synthesizing a
            // `VK_SHIFT` up here would spuriously release the real user's shift if they happen
            // to be holding it. The shift entry is always `inputs[0]` when `pressed_shift_now`,
            // so `sent >= 1` tells us it went through.
            if pressed_shift_now && sent >= 1 && send_vk(VK_SHIFT.0, true, hkl).is_err() {
                self.pending_shift_release = true;
            }
            return Err(e);
        }
        // `auto_shifted` records whether *we* actually pressed shift for this key, not whether
        // the key needed shift. Otherwise if another source of shift (explicit `VK_SHIFT`
        // keycode, earlier auto-shifted key) was already down and released before this key,
        // `key_up` would synthesize a spurious `VK_SHIFT` release that the OS never asked for.
        //
        // If a caller violates the pair-each-down-with-an-up contract and issues two `key_down`s
        // for the same logical key, preserve the `auto_shifted: true` bit so the matching
        // `key_up` still releases the shift we pressed originally.
        let already_auto_shifted = matches!(
            self.pressed_keys.get(key),
            Some(PressedKey::Vk {
                auto_shifted: true,
                ..
            })
        );
        self.pressed_keys.insert(
            key.clone(),
            PressedKey::Vk {
                vk,
                auto_shifted: pressed_shift_now || already_auto_shifted,
            },
        );
        Ok(())
    }

    /// Sends a key up event for the given key.
    ///
    /// Uses the VK recorded at `key_down` time (not a fresh resolution against the current
    /// keyboard layout), so a mid-action layout switch still releases the key we originally
    /// pressed. If no prior `key_down` is tracked, we fall back to resolving now for best-effort
    /// delivery.
    ///
    /// The shift release is attempted even when the primary key-up fails so that a single
    /// `SendInput` failure does not leave shift stuck. If the shift release itself fails, we
    /// mark it pending so the next `key_down` / `key_up` retries it.
    pub fn key_up(&mut self, key: &Key) -> Result<(), String> {
        let hkl = foreground_keyboard_layout();
        self.flush_pending_shift_release(hkl);

        let Some(pressed) = self.pressed_keys.remove(key) else {
            // No recorded press; resolve against the current layout as a best-effort fallback so a
            // stray `key_up` still reaches the OS.
            return match resolve_key(key, hkl)? {
                ResolvedKey::Vk { vk, .. } => send_vk(vk, true, hkl),
                ResolvedKey::Unicode(unit) => send_inputs(&[make_unicode_input(unit, true)]),
            };
        };

        match pressed {
            PressedKey::Unicode(unit) => {
                // Unicode dispatch has no shift bookkeeping; just send the matching up.
                send_inputs(&[make_unicode_input(unit, true)])
            }
            PressedKey::Vk { vk, auto_shifted } => {
                let primary = send_vk(vk, true, hkl);

                // Attempt shift release regardless of whether the primary key-up succeeded, so a
                // single `SendInput` failure can't leave shift stuck. Only release if this was
                // an auto-shifted key, no other auto-shifted keys remain, and the caller isn't
                // holding shift explicitly.
                let should_release_shift =
                    auto_shifted && !self.has_auto_shifted_press() && !self.explicit_shift_held();
                let shift_result = if should_release_shift {
                    match send_vk(VK_SHIFT.0, true, hkl) {
                        Ok(()) => Ok(()),
                        Err(e) => {
                            // Mark the release as pending so the next call retries. Without
                            // this, shift stays held in the OS with nothing left in
                            // `pressed_keys` to release it.
                            self.pending_shift_release = true;
                            Err(e)
                        }
                    }
                } else {
                    Ok(())
                };

                // Report the primary failure first, falling back to the shift-release failure
                // if the primary succeeded.
                primary.and(shift_result)
            }
        }
    }

    /// Simulates typing text by sending Unicode keyboard events.
    ///
    /// Using `KEYEVENTF_UNICODE` bypasses the keyboard layout and works with any character the
    /// target application can accept as Unicode input. The entire string is batched into a single
    /// `SendInput` call so the OS cannot interleave other input between characters.
    ///
    /// Takes `&mut self` so it can `flush_pending_shift_release` like `key_down`/`key_up`,
    /// otherwise a stuck auto-shift from a prior failed release would persist through the whole
    /// typing call and on into any non-keyboard actions that follow.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.flush_pending_shift_release(foreground_keyboard_layout());
        // Each UTF-16 code unit produces one down + one up INPUT. The UTF-8 byte length is a
        // valid upper bound on the UTF-16 unit count (single-byte ASCII → 1 unit, 2-byte → 1
        // unit, 3-byte BMP → 1 unit, 4-byte supplementary → 2 units), so `bytes * 2` never
        // under-counts. This over-allocates ~3x for 3-byte UTF-8 strings (CJK, Cyrillic) but
        // avoids an extra O(n) `chars().count()` pass just to size the buffer.
        let mut inputs: Vec<INPUT> = Vec::with_capacity(text.len().saturating_mul(2));
        for ch in text.chars() {
            let mut buf = [0u16; 2];
            let encoded = ch.encode_utf16(&mut buf);
            // Emit a down/up pair per UTF-16 unit. `KEYEVENTF_UNICODE` delivered via
            // `TranslateMessage` / `WM_CHAR` expects each surrogate as its own down/up pair;
            // emitting all downs then all ups has been observed to drop half of the sequence in
            // some targets.
            for &unit in encoded.iter() {
                inputs.push(make_unicode_input(unit, false));
                inputs.push(make_unicode_input(unit, true));
            }
        }
        send_inputs(&inputs)
    }
}

/// Resolves a `Key` to either a virtual-key dispatch (with optional auto-shift) or a Unicode
/// code-unit dispatch.
fn resolve_key(key: &Key, hkl: HKL) -> Result<ResolvedKey, String> {
    match key {
        Key::Keycode(code) => {
            let vk = u16::try_from(*code).map_err(|_| {
                format!(
                    "Invalid virtual-key code {code}: must be in range 0..={}",
                    u16::MAX
                )
            })?;
            // For explicit VKs, the caller manages modifiers.
            Ok(ResolvedKey::Vk {
                vk,
                needs_shift: false,
            })
        }
        Key::Char(ch) => resolve_char(*ch, hkl),
    }
}

/// Resolves a character to either a VK (with optional shift) or a Unicode code-unit dispatch,
/// using the given keyboard layout handle (typically the foreground window's). This matches what
/// a real keystroke would look like to the target application when the user is running a
/// different input language / IME than Warp's thread.
///
/// Falls back to `ResolvedKey::Unicode` when the layout would require ctrl/alt to produce the
/// character (e.g., AltGr-accessed keys on several European layouts) so `Key::Char` remains
/// portable across layouts instead of erroring out.
fn resolve_char(ch: char, hkl: HKL) -> Result<ResolvedKey, String> {
    // VkKeyScanExW only supports characters in the BMP (single UTF-16 unit). Supplementary-plane
    // characters still work via the Unicode path (they'd need a surrogate pair there, which is
    // what `type_text` handles); `Key::Char` is a single `char` so callers can't currently
    // express a supplementary-plane key event here.
    let mut buf = [0u16; 2];
    let encoded = ch.encode_utf16(&mut buf);
    if encoded.len() != 1 {
        return Err(format!(
            "Character '{ch}' is outside the Basic Multilingual Plane (BMP); use TypeText for emoji and other supplementary-plane characters"
        ));
    }
    let unit = encoded[0];

    // SAFETY: `VkKeyScanExW` is a pure query and is safe to call from any thread; `hkl` is
    // either a valid HKL or null (null falls back to the calling thread's layout).
    let result = unsafe { VkKeyScanExW(unit, hkl) };
    if result == -1 {
        // No VK mapping at all in this layout; fall back to Unicode dispatch.
        return Ok(ResolvedKey::Unicode(unit));
    }

    // Low byte is the VK code; high byte is the shift state.
    //   bit 0: shift, bit 1: ctrl, bit 2: alt.
    let bytes = result.to_le_bytes();
    let vk = bytes[0] as u16;
    let shift_state = bytes[1];
    let needs_shift = (shift_state & 0x01) != 0;
    let needs_ctrl = (shift_state & 0x02) != 0;
    let needs_alt = (shift_state & 0x04) != 0;

    if needs_ctrl || needs_alt {
        // Character requires ctrl and/or alt (e.g., AltGr on European layouts). Synthesizing
        // those modifiers can also trigger unwanted shortcuts in the target app, so fall back
        // to layout-bypassing Unicode dispatch instead.
        return Ok(ResolvedKey::Unicode(unit));
    }

    Ok(ResolvedKey::Vk { vk, needs_shift })
}

/// Builds the `INPUT` record for a single key down or key up event on the given virtual-key
/// code, without dispatching it. See [`send_vk`] for the full description of the scan-code
/// translation. The caller supplies the target keyboard layout so shift + main-VK entries built
/// for the same public call can share a consistent snapshot.
fn build_vk_input(vk: u16, is_up: bool, hkl: HKL) -> INPUT {
    // SAFETY: `MapVirtualKeyExW` has no preconditions; reads the given HKL (null = calling
    // thread's layout) and returns 0 if no mapping exists.
    let scan = unsafe { MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_VSC, Some(hkl)) } as u16;

    let mut flag_bits: u32 = 0;
    let (w_vk, w_scan) = if scan != 0 {
        flag_bits |= KEYEVENTF_SCANCODE.0;
        if is_extended_vk(vk) {
            flag_bits |= KEYEVENTF_EXTENDEDKEY.0;
        }
        (0u16, scan)
    } else {
        // No scan-code mapping for this VK; dispatch by virtual-key code.
        (vk, 0u16)
    };
    if is_up {
        flag_bits |= KEYEVENTF_KEYUP.0;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(w_vk),
                wScan: w_scan,
                dwFlags: KEYBD_EVENT_FLAGS(flag_bits),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Sends a single key down or key up event for the given virtual-key code, resolved against the
/// given keyboard layout.
///
/// We translate the virtual-key code to a hardware scan code via `MapVirtualKeyExW` and dispatch
/// with `KEYEVENTF_SCANCODE` (plus `KEYEVENTF_EXTENDEDKEY` for keys that require the 0xE0
/// prefix). This reaches targets that filter synthesized VK-only events (games, some
/// remote-desktop clients). The OS still translates the scan code back into the corresponding
/// virtual-key code for standard window messages, so VK-reading consumers are unaffected. If no
/// scan-code mapping exists we fall back to VK-only dispatch.
fn send_vk(vk: u16, is_up: bool, hkl: HKL) -> Result<(), String> {
    send_inputs(&[build_vk_input(vk, is_up, hkl)])
}

/// Returns true if `vk` is one of the shift virtual-key codes (generic / left / right).
fn is_shift_vk(vk: u16) -> bool {
    vk == VK_SHIFT.0 || vk == VK_LSHIFT.0 || vk == VK_RSHIFT.0
}

/// Returns the keyboard layout (`HKL`) currently active on the foreground window's thread,
/// falling back to the calling thread's layout (HKL `0`) if there is no foreground window. Using
/// the foreground window's HKL makes `Key::Char` resolution match what a real keystroke would
/// produce for the target application, which matters in multilingual setups where Warp's thread
/// layout can differ from the app's.
fn foreground_keyboard_layout() -> HKL {
    // SAFETY: `GetForegroundWindow` has no preconditions; returns null if no foreground window.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        // SAFETY: `GetKeyboardLayout(0)` returns the calling thread's layout.
        return unsafe { GetKeyboardLayout(0) };
    }
    // SAFETY: `hwnd` is a valid window handle; we pass a null `lpdwProcessId`.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(ptr::null_mut())) };
    // SAFETY: `GetKeyboardLayout` has no preconditions; 0 means "calling thread's layout".
    unsafe { GetKeyboardLayout(thread_id) }
}

/// Returns true if the given virtual-key code is an "extended" key (scan code prefixed with
/// 0xE0). `MapVirtualKeyW(MAPVK_VK_TO_VSC)` strips the 0xE0 prefix, so we set
/// `KEYEVENTF_EXTENDEDKEY` ourselves for these VKs.
fn is_extended_vk(vk: u16) -> bool {
    // Values from <winuser.h>. See "About Keyboard Input" on MSDN for the canonical list of
    // extended keys.
    const VK_PRIOR: u16 = 0x21;
    const VK_NEXT: u16 = 0x22;
    const VK_END: u16 = 0x23;
    const VK_HOME: u16 = 0x24;
    const VK_LEFT: u16 = 0x25;
    const VK_UP: u16 = 0x26;
    const VK_RIGHT: u16 = 0x27;
    const VK_DOWN: u16 = 0x28;
    const VK_SNAPSHOT: u16 = 0x2C;
    const VK_INSERT: u16 = 0x2D;
    const VK_DELETE: u16 = 0x2E;
    const VK_LWIN: u16 = 0x5B;
    const VK_RWIN: u16 = 0x5C;
    const VK_APPS: u16 = 0x5D;
    const VK_DIVIDE: u16 = 0x6F;
    const VK_NUMLOCK: u16 = 0x90;
    const VK_RCONTROL: u16 = 0xA3;
    const VK_RMENU: u16 = 0xA5;

    matches!(
        vk,
        VK_PRIOR
            | VK_NEXT
            | VK_END
            | VK_HOME
            | VK_LEFT
            | VK_UP
            | VK_RIGHT
            | VK_DOWN
            | VK_SNAPSHOT
            | VK_INSERT
            | VK_DELETE
            | VK_LWIN
            | VK_RWIN
            | VK_APPS
            | VK_DIVIDE
            | VK_NUMLOCK
            | VK_RCONTROL
            | VK_RMENU
    )
}

fn make_unicode_input(unit: u16, is_up: bool) -> INPUT {
    let flags = if is_up {
        KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE.0 | KEYEVENTF_KEYUP.0)
    } else {
        KEYEVENTF_UNICODE
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Dispatches a batch of `INPUT` events via `SendInput`.
fn send_inputs(inputs: &[INPUT]) -> Result<(), String> {
    send_inputs_tracked(inputs).1
}

/// Dispatches a batch of `INPUT` events via `SendInput`, returning the number of events the OS
/// actually queued alongside the pass/fail `Result`. Callers that need to take compensating
/// action keyed off partial delivery (e.g., "did the shift entry get through?") can branch on
/// the `sent` count; callers that only care about pass/fail can use [`send_inputs`] directly.
fn send_inputs_tracked(inputs: &[INPUT]) -> (u32, Result<(), String>) {
    if inputs.is_empty() {
        return (0, Ok(()));
    }

    // SAFETY: `inputs` is a valid slice of `INPUT` with the correct element size, and `SendInput`
    // does not retain the pointer beyond the call.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        // SAFETY: `GetLastError` has no preconditions; reads the calling thread's last-error.
        let last_error = unsafe { GetLastError() }.0;
        return (
            sent,
            Err(format!(
                "SendInput dispatched only {sent}/{} keyboard events \
                 (GetLastError={last_error}, blocked by UIPI or other input?)",
                inputs.len(),
            )),
        );
    }
    (sent, Ok(()))
}
