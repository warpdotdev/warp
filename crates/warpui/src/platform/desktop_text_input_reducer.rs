//! Pure, host-testable event classification for the desktop text-input bridge.
//!
//! This module intentionally avoids any `web_sys` calls and lives outside the wasm-only
//! `platform::wasm` module tree, so its unit tests run under ordinary native
//! `cargo nextest run -p warpui`, independent of a live browser environment. The DOM-facing
//! listeners in [`crate::platform::wasm::desktop_text_input`] extract plain data from browser
//! events and hand it to the functions here to decide what (if anything) to dispatch to Warp.

// Only `platform::wasm::desktop_text_input` (wasm-only) actually calls into this module; on every
// other target it is otherwise-unused code kept solely so its tests run under native
// `cargo nextest`.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::ops::Range;

use crate::event::{Event, KeyEventDetails, KeyState};
use crate::keymap::Keystroke;
use crate::platform::KEYS_TO_IGNORE;
use crate::platform::keyboard::KeyCode;

/// The sentinel value kept in the bridge's `<textarea>`, matching the mobile hidden input's
/// sentinel-character pattern.
pub(crate) const SENTINEL: &str = " ";

/// Whether a raw DOM keyboard event was a `keydown` or a `keyup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DomKeyEventKind {
    Down,
    Up,
}

/// A platform-neutral snapshot of a browser `KeyboardEvent`, captured by the desktop bridge's own
/// listeners. A focused `<textarea>` keeps winit's canvas keyboard listeners from ever seeing the
/// event, so the bridge must convert it independently instead of relying on winit's own
/// `KeyEvent`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DesktopKeyboardPayload {
    pub kind: DomKeyEventKind,
    /// The DOM `KeyboardEvent.key` value, e.g. `"a"`, `"Enter"`, `"ArrowLeft"`.
    pub key: String,
    /// The DOM `KeyboardEvent.code` value, e.g. `"KeyA"`, `"ShiftLeft"`.
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The browser's `metaKey`, which corresponds to Warp's `cmd`.
    pub meta: bool,
    pub is_composing: bool,
}

/// The result of converting a [`DesktopKeyboardPayload`] into UI-framework events.
///
/// Public (rather than `pub(crate)`, like the rest of this module) only because it is embedded in
/// the publicly-declared `DesktopTextInputEvent::Key` variant in
/// `crate::platform::wasm::desktop_text_input`.
#[derive(Debug, Clone)]
pub enum KeyConversion {
    /// A hardware key was pressed. Dispatch `event` first; only if it goes unhandled (and the
    /// keystroke doesn't include Cmd) should `chars` be dispatched as `TypedCharacters`.
    Down { event: Event, chars: Option<String> },
    /// A modifier key transitioned. Physical left/right identity is preserved via `key_code`.
    ModifierChanged { key_code: KeyCode, state: KeyState },
}

/// The direction of a deletion inferred from a browser `input` event's `inputType`.
///
/// Public for the same reason as [`KeyConversion`]: it is embedded in the publicly-declared
/// `DesktopTextInputEvent::Delete` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteDirection {
    Backward,
    Forward,
}

/// Classification of a non-composing `input` event's `inputType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputClassification {
    Insert,
    Delete(DeleteDirection),
    /// An `inputType` the bridge does not forward (e.g. formatting commands).
    Unsupported,
}

/// Converts a desktop bridge keyboard payload into the events Warp should dispatch, or `None` if
/// the browser should keep native ownership of the key (composition in progress, a key the bridge
/// does not forward as a dedicated event, or a keystroke on the ignore list, e.g. paste).
pub(crate) fn convert_key(payload: &DesktopKeyboardPayload) -> Option<KeyConversion> {
    // Composition owns the key stream until it commits; let the browser drive it.
    if payload.is_composing {
        return None;
    }

    if let Some(key_code) = modifier_key_code(&payload.code) {
        let state = match payload.kind {
            DomKeyEventKind::Down => KeyState::Pressed,
            DomKeyEventKind::Up => KeyState::Released,
        };
        return Some(KeyConversion::ModifierChanged { key_code, state });
    }

    // Non-modifier keyups don't drive any Warp behavior, matching the canvas keyboard path.
    if payload.kind != DomKeyEventKind::Down {
        return None;
    }

    let (key, key_down_chars, fallback_chars) =
        classify_key(&payload.key, payload.ctrl, payload.shift)?;

    let keystroke = Keystroke {
        ctrl: payload.ctrl,
        alt: payload.alt,
        shift: payload.shift,
        cmd: payload.meta,
        meta: false,
        key,
    };

    if KEYS_TO_IGNORE.contains(&keystroke) {
        return None;
    }

    Some(KeyConversion::Down {
        event: Event::KeyDown {
            keystroke,
            chars: key_down_chars,
            details: KeyEventDetails::default(),
            is_composing: false,
        },
        chars: fallback_chars,
    })
}

/// Maps a DOM `KeyboardEvent.code` value to the corresponding modifier [`KeyCode`], or `None` if
/// `code` does not identify a modifier key.
fn modifier_key_code(code: &str) -> Option<KeyCode> {
    Some(match code {
        "AltLeft" => KeyCode::AltLeft,
        "AltRight" => KeyCode::AltRight,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ShiftRight" => KeyCode::ShiftRight,
        "ControlLeft" => KeyCode::ControlLeft,
        "ControlRight" => KeyCode::ControlRight,
        "MetaLeft" => KeyCode::SuperLeft,
        "MetaRight" => KeyCode::SuperRight,
        _ => return None,
    })
}

/// Classifies a DOM `KeyboardEvent.key` value, returning `(keystroke_key, key_down_chars,
/// fallback_chars)`:
/// - `keystroke_key` is the key string used by [`Keystroke`].
/// - `key_down_chars` is the text carried on the dispatched `KeyDown` event itself, which raw
///   terminal input reads directly (e.g. Ctrl-C must carry `"\x03"`, not `"c"`).
/// - `fallback_chars` is the text to dispatch as `TypedCharacters` if the `KeyDown` goes
///   unhandled.
///
/// A single character is treated as printable text; everything else is looked up in the
/// named-key table. Named keys carry no chars of their own.
fn classify_key(key: &str, ctrl: bool, shift: bool) -> Option<(String, String, Option<String>)> {
    let mut chars_iter = key.chars();
    let (Some(single_char), None) = (chars_iter.next(), chars_iter.next()) else {
        let name = named_key(key)?;
        return Some((name.to_string(), String::new(), None));
    };

    let keystroke_key = if shift {
        single_char.to_uppercase().to_string()
    } else {
        single_char.to_lowercase().to_string()
    };

    let key_down_chars = if ctrl {
        control_character_for(single_char.to_ascii_lowercase())
            .map(str::to_string)
            .unwrap_or_else(|| key.to_string())
    } else {
        key.to_string()
    };

    Some((keystroke_key, key_down_chars, Some(key.to_string())))
}

/// Maps the DOM `KeyboardEvent.key` values for non-printable keys to the key strings expected by
/// [`Keystroke`]. Describes the same UI Events `key` values as `convert_key` in the canvas
/// keyboard path (`windowing::winit::event_loop::key_events`), which sources them from winit
/// instead of directly from the DOM.
fn named_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "Enter" => "enter",
        "Tab" => "tab",
        "ArrowDown" => "down",
        "ArrowLeft" => "left",
        "ArrowRight" => "right",
        "ArrowUp" => "up",
        "End" => "end",
        "Home" => "home",
        "PageDown" => "pagedown",
        "PageUp" => "pageup",
        "Backspace" => "backspace",
        "Delete" => "delete",
        "Insert" => "insert",
        "Escape" => "escape",
        "F1" => "f1",
        "F2" => "f2",
        "F3" => "f3",
        "F4" => "f4",
        "F5" => "f5",
        "F6" => "f6",
        "F7" => "f7",
        "F8" => "f8",
        "F9" => "f9",
        "F10" => "f10",
        "F11" => "f11",
        "F12" => "f12",
        "F13" => "f13",
        "F14" => "f14",
        "F15" => "f15",
        "F16" => "f16",
        "F17" => "f17",
        "F18" => "f18",
        "F19" => "f19",
        "F20" => "f20",
        "F21" => "f21",
        "F22" => "f22",
        "F23" => "f23",
        "F24" => "f24",
        "F25" => "f25",
        "F26" => "f26",
        "F27" => "f27",
        "F28" => "f28",
        "F29" => "f29",
        "F30" => "f30",
        "F31" => "f31",
        "F32" => "f32",
        "F33" => "f33",
        "F34" => "f34",
        "F35" => "f35",
        _ => return None,
    })
}

/// Maps a printable ASCII character to the control code it produces when Ctrl is held (e.g.
/// Ctrl-C produces ETX). Mirrors `CONTROL_CHARACTER_MAP` in
/// `windowing::winit::event_loop::key_events`; duplicated (rather than shared) to keep this
/// module's DOM-independent conversion logic decoupled from the winit-specific canvas keyboard
/// path. Keep the two tables in sync if either changes.
fn control_character_for(c: char) -> Option<&'static str> {
    Some(match c {
        '@' => "\x00",
        'a' => "\x01",
        'b' => "\x02",
        'c' => "\x03",
        'd' => "\x04",
        'e' => "\x05",
        'f' => "\x06",
        'g' => "\x07",
        'h' => "\x08",
        'i' => "\x09",
        'j' => "\x0A",
        'k' => "\x0B",
        'l' => "\x0C",
        'm' => "\x0D",
        'n' => "\x0E",
        'o' => "\x0F",
        'p' => "\x10",
        'q' => "\x11",
        'r' => "\x12",
        's' => "\x13",
        't' => "\x14",
        'u' => "\x15",
        'v' => "\x16",
        'w' => "\x17",
        'x' => "\x18",
        'y' => "\x19",
        'z' => "\x1A",
        '[' => "\x1B",
        '\\' => "\x1C",
        ']' => "\x1D",
        '^' => "\x1E",
        '_' => "\x1F",
        _ => return None,
    })
}

/// Classifies a non-composing `input` event's `inputType` into an insertion or deletion.
///
/// Deletions are matched explicitly, as are non-textual operations (formatting, undo/redo) that
/// the bridge intentionally never forwards. Everything else - including the standard `insert*`
/// types, an empty/absent `inputType` (e.g. a tool that mutates `.value` directly and dispatches a
/// generic `Event` instead of a real `InputEvent`), and any `inputType` this bridge doesn't
/// specifically recognize - is treated as a potential insertion. The caller falls back to diffing
/// the textarea's value against the sentinel when `InputEvent.data` is absent, which is the path
/// generic direct-mutation tools rely on.
pub(crate) fn classify_input_type(input_type: &str) -> InputClassification {
    match input_type {
        "deleteContentBackward"
        | "deleteWordBackward"
        | "deleteSoftLineBackward"
        | "deleteHardLineBackward"
        | "deleteEntireSoftLine" => InputClassification::Delete(DeleteDirection::Backward),
        "deleteContentForward"
        | "deleteWordForward"
        | "deleteSoftLineForward"
        | "deleteHardLineForward" => InputClassification::Delete(DeleteDirection::Forward),
        "formatBold"
        | "formatItalic"
        | "formatUnderline"
        | "formatStrikeThrough"
        | "formatSuperscript"
        | "formatSubscript"
        | "formatJustifyFull"
        | "formatJustifyCenter"
        | "formatJustifyRight"
        | "formatJustifyLeft"
        | "formatIndent"
        | "formatOutdent"
        | "formatRemove"
        | "formatSetBlockTextDirection"
        | "formatSetInlineTextDirection"
        | "formatBackColor"
        | "formatFontColor"
        | "formatFontName"
        | "historyUndo"
        | "historyRedo" => InputClassification::Unsupported,
        _ => InputClassification::Insert,
    }
}

/// Given the bridge's sentinel prefix and the textarea's current value, extracts the text a
/// direct DOM mutation (e.g. a tool that sets `.value` and fires a generic `input` event without
/// `InputEvent.data`) inserted. Handles two shapes of direct mutation:
/// - The sentinel is still present as a prefix (the tool appended after it): the inserted text is
///   whatever follows the sentinel.
/// - The sentinel is gone entirely (the tool replaced `.value` wholesale, e.g. a dictation tool
///   like MacWhisper assigning the full transcription): the entire value is the inserted text.
///
/// Returns `None` if nothing was actually inserted (the value is unchanged, or empty).
pub(crate) fn extract_inserted_text(sentinel: &str, value: &str) -> Option<String> {
    let inserted = value.strip_prefix(sentinel).unwrap_or(value);
    (!inserted.is_empty()).then(|| inserted.to_string())
}

/// Builds the existing Warp key event for a deletion inferred from an `input` event, matching the
/// key strings the canvas path uses for Backspace and Delete.
pub(crate) fn key_event_for_delete(direction: DeleteDirection) -> Event {
    let key = match direction {
        DeleteDirection::Backward => "backspace",
        DeleteDirection::Forward => "delete",
    };
    Event::KeyDown {
        keystroke: Keystroke {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            meta: false,
            key: key.to_string(),
        },
        chars: String::new(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

/// Derives the composition selection range (relative to the composed text) from the textarea's
/// raw DOM selection offsets, which include the sentinel prefix. Offsets are in UTF-16 code
/// units, matching both the DOM's selection API and `marked_text_utf16_len`.
pub(crate) fn composition_selection_range(
    sentinel_len: usize,
    marked_text_utf16_len: usize,
    selection_start: usize,
    selection_end: usize,
) -> Range<usize> {
    let clamp = |value: usize| {
        value
            .saturating_sub(sentinel_len)
            .min(marked_text_utf16_len)
    };
    clamp(selection_start)..clamp(selection_end)
}

/// What the desktop bridge should do in response to a composition or `input` event, as decided by
/// [`CompositionTracker`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CompositionAction {
    /// Dispatch `SetMarkedText` with this preedit text and selection.
    Update {
        text: String,
        selection: Range<usize>,
    },
    /// Dispatch `ClearMarkedText`, then `TypedCharacters` with this committed text.
    Commit(String),
    /// Dispatch `ClearMarkedText` only; nothing was committed.
    Cancel,
}

/// Tracks IME composition state for the desktop bridge across its `composition*` and `input`
/// listeners.
///
/// Browsers fire a trailing `input` event immediately after a non-empty `compositionend`, carrying
/// the same text `compositionend` already committed. Deciding what to do with each event through
/// this single, DOM-independent state machine (rather than duplicating the bookkeeping inline in
/// each listener) is what lets that double-commit scenario be covered by an ordinary native unit
/// test instead of only a live browser session.
#[derive(Debug, Default)]
pub(crate) struct CompositionTracker {
    composing: bool,
    /// The text committed by the most recent `compositionend`, while its matching trailing
    /// `input` event is still expected. Consumed (one-shot) by `should_ignore_input_event`,
    /// which only suppresses an `input` event whose data actually matches this text - so a
    /// distinct `input` event that arrives instead (e.g. a fast follow-on keystroke, or a
    /// browser that never fires the trailing duplicate) is processed normally instead of being
    /// silently dropped.
    pending_commit: Option<String>,
}

impl CompositionTracker {
    pub(crate) fn is_composing(&self) -> bool {
        self.composing
    }

    pub(crate) fn on_composition_start(&mut self) {
        self.composing = true;
    }

    pub(crate) fn on_composition_update(
        &self,
        text: String,
        selection: Range<usize>,
    ) -> CompositionAction {
        CompositionAction::Update { text, selection }
    }

    /// Call when `compositionend` fires. A non-empty commit arms suppression for the trailing
    /// `input` event that immediately follows it.
    ///
    /// Always clears any suppression left over from an earlier commit first: if that commit's
    /// own trailing `input` event never arrived (a browser quirk, or a fast follow-on
    /// composition), the earlier `pending_commit` would otherwise still be armed here and could
    /// go on to swallow an unrelated, later `input` event that happens to carry the same text.
    pub(crate) fn on_composition_end(&mut self, data: String) -> CompositionAction {
        self.composing = false;
        self.pending_commit = None;
        if data.is_empty() {
            CompositionAction::Cancel
        } else {
            self.pending_commit = Some(data.clone());
            CompositionAction::Commit(data)
        }
    }

    /// Call for every `input` event, composing or not, before applying any other input handling.
    /// `data` is the event's `InputEvent.data`, used to confirm a pending trailing duplicate
    /// before dropping it. Returns `true` if the caller must drop this event entirely: either it
    /// arrived mid-composition (composition events own the text), or its data matches the
    /// trailing `input` event a committing `compositionend` already accounted for.
    pub(crate) fn should_ignore_input_event(
        &mut self,
        is_composing_event: bool,
        data: Option<&str>,
    ) -> bool {
        if let Some(expected) = self.pending_commit.take()
            && data == Some(expected.as_str())
        {
            return true;
        }
        self.composing || is_composing_event
    }

    /// Call on blur or when the caret-owning surface changes, so a composition in progress on the
    /// old surface is abandoned (never committed into the new one) and stale suppression can't
    /// leak into unrelated input on the new surface. Returns the action to dispatch, if any.
    pub(crate) fn reset(&mut self) -> Option<CompositionAction> {
        self.pending_commit = None;
        std::mem::take(&mut self.composing).then_some(CompositionAction::Cancel)
    }
}

#[cfg(test)]
#[path = "desktop_text_input_reducer_tests.rs"]
mod tests;
