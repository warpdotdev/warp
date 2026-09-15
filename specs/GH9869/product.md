# Product spec: Voice input hotkey picker — Caps Lock, F-keys, custom keys (GH-9869)

## Problem

The voice input hotkey picker in **Settings → Agents → Warp Agent →
Voice** offers a fixed list of ten modifier/function keys:

```
None | Fn | Option (Left/Right) | Control (Left/Right) | Command (Left/Right) | Shift (Left/Right)
```

The list is missing the two most common keys users repurpose for
push-to-talk in real-world setups:

- **Caps Lock** — the canonical "I never use this for typing" key,
  remapped to push-to-talk in Discord, Krisp, Whisper, and most
  VoIP/voice apps. macOS even surfaces "remap Caps Lock" as a
  first-class system setting.
- **Individual F-keys (F1–F12, and F13+ on extended keyboards)** —
  isolated from the typing flow, naturally hold-to-record. A common
  ergonomic choice on streamdecks, ortholinear keyboards, and
  external programmable keypads where users dedicate F13–F19 to app
  shortcuts.

There is also no escape hatch: if a user has an ergonomic key Warp's
fixed list does not name (e.g. an extra mouse button mapped to a key,
a Hyper key, an unused punctuation key), they cannot bind voice input
to it. The only workaround today is to give up on Warp's voice input
hotkey and use the OS-wide voice input shortcut instead, which loses
the in-app integration.

## Goal

Users can bind voice input to any single physical key Warp's
underlying `KeyCode` enum can identify, with first-class UI for the
two most-requested additions (Caps Lock, F-keys) and a press-to-capture
escape hatch for everything else.

## Non-goals (V1)

- **Modifier-combination hotkeys** (Cmd+Shift+V etc.). The current
  semantics are "hold one physical key"; introducing chord support
  changes the press/release tracking and the dropdown's
  expressiveness substantially. Track separately.
- **Per-tab or per-profile voice hotkeys.** The setting stays global
  per device.
- **Cloud sync.** The setting is intentionally `SyncToCloud::Never`
  ([app/src/settings/ai.rs:146](app/src/settings/ai.rs)) because the
  same person uses different keyboards on different devices. V1
  preserves this.
- **Re-binding the system-wide voice input hotkey.** This spec is
  scoped to Warp's in-app voice input only.
- **Custom display names for captured keys.** A captured F19 displays
  as "F19", a captured Backslash displays as "Backslash". Pretty
  per-keyboard labeling (e.g. "Right Mouse Button 4") is a follow-up.

## Behavior contract (V1)

### B1 — Caps Lock as a first-class option (platform-specific semantics)

> **Correction (review #10127):** earlier drafts described Caps Lock
> as hold-to-talk on every platform while tech.md required macOS
> press-to-toggle. Resolved to a single source of truth below.

Caps Lock appears in the dropdown on all platforms (macOS, Windows,
Linux). The activation semantics are **platform-specific** and
documented to the user in the dropdown's tooltip:

- **macOS — press-to-toggle:** macOS reports Caps Lock as a single
  key event when the lock state changes, not as a sustained press.
  Holding-to-talk is impossible without OS-level event interception
  (out of V1 scope). One tap starts voice input recording; the next
  tap stops it. The dropdown tooltip on macOS reads
  *"Tap to start, tap again to stop."* This matches Discord and
  Krisp.
- **Windows — hold-to-talk:** standard behavior. Holding Caps Lock
  activates voice input; releasing it deactivates. The OS-level Caps
  Lock toggle (the LED, the uppercase-letters effect) still fires —
  V1 does not suppress it. The dropdown tooltip on Windows reads
  *"Hold to talk."*
- **Linux — hold-to-talk:** same as Windows. Tooltip reads
  *"Hold to talk."*

Acceptance criteria A4 is split accordingly:
- A4-mac: tap, then tap again, on macOS — voice activates then
  deactivates.
- A4-win-lin: hold and release on Windows or Linux — voice activates
  while held, deactivates on release.

Users on macOS who want hold-to-talk semantics can remap Caps Lock
to F19 at the OS level and bind F19; this is the current best
practice in Discord/Krisp.

### B2 — F1 through F12 as first-class options

F1–F12 each appear as their own dropdown entry on all platforms.
F13–F24 (and beyond, up to whatever `KeyCode` supports) are reachable
via the press-to-capture flow (B3) but are not pre-listed in the
dropdown to keep it scannable. The dropdown groups F-keys under a
single "Function key…" sub-section so the list does not balloon.

### B3 — Press-to-capture escape hatch

A new dropdown entry "Custom key…" opens a modal that prompts
"Press the key you want to use for voice input." The user presses
any single key. The modal:
- Confirms the captured key by name and `KeyCode` (e.g.
  "Selected: F19 (KeyCode::F19)").
- Has Cancel and Confirm buttons.
- On Confirm, persists the captured `KeyCode` to the setting.
- On Escape during capture, closes without saving.

The modal blocks the rest of the settings UI while open. It does not
record modifier-only chords; it captures the first non-modifier key
press, OR if only modifiers are held for >800ms before release, it
captures the held modifier (so the user can still capture a bare
Shift via this flow if for some reason the dropdown options are
hidden).

### B4 — Backwards compatibility for existing settings

Users who have already set their voice toggle key to one of the
existing 10 enum variants see no change. The TOML key
`agents.voice.voice_input_toggle_key`
([app/src/settings/ai.rs:148](app/src/settings/ai.rs)) preserves
its existing values. The new variants extend the enum; they do not
replace or rename any existing variant.

### B5 — Keys not safely bindable are rejected

The press-to-capture flow rejects (with an inline error message and
no save) any key that would brick the user's keyboard:
- The currently-bound app-quit shortcut.
- Enter, Escape, Tab, Backspace, Space (would block the agent input).
- Modifier-only without the >800ms held-modifier rule (B3 carve-out).

> **Correction (re-review #10127):** the previous draft included
> "the currently-bound voice-input shortcut itself" as a reject-list
> entry. That would have prevented users from changing an existing
> custom hotkey — the literal opposite of the modal's purpose. The
> tech spec already removed the rule (capture is accepted; same-key
> re-confirm is a no-op short-circuit). Product B5 now matches.

### B6 — "None" remains the disable option

The existing `VoiceInputToggleKey::None` variant remains the
"disable voice toggle" entry. The press-to-capture flow does not
offer a way to bind to "no key"; users must select "None" from the
dropdown. This keeps the disable path obvious.

### B7 — Display name reflects user perception, not the protocol

Captured keys display by their user-visible label, not their `KeyCode`
spelling. E.g. `KeyCode::Backquote` displays as "` (Backquote)" or
"\\\\` (Tilde key on US layout)" — whichever string already exists in
Warp's keymap display layer. Falls back to `KeyCode::Debug` only if
no display string is available; in that case a `log::warn!` fires (a
key the user successfully pressed should always have a display
string).

### B8 — Telemetry: which key was bound

When the user changes the voice input hotkey, fire the existing
settings-change telemetry event with the new value (enum variant name
for known variants, `"custom:<KeyCode>"` for press-to-capture
variants). This lets the team see whether the new options actually
solve the user demand or whether further additions are needed.

**Privacy guardrails (security review #10127):**
- The event fires **only after Confirm/save** in the press-to-capture
  modal, never during capture.
- Raw key events captured during the modal session are not logged or
  emitted — they exist only in the modal's local state.
- Rejected captures (B5 reject-list hits) are NOT emitted. The user
  pressing keys that fail validation is the user's private interaction
  with the modal.
- Cancelled captures (Escape, Cancel button) are NOT emitted.
- The emitted payload is `{ setting:
  "agents.voice.voice_input_toggle_key", new_value: <enum-name-or-custom-keycode> }`.
  No timestamps, no sequence of attempted keys, no modal session id.
- The event respects Warp's existing global telemetry opt-out.

## Acceptance criteria

A1. Caps Lock appears in the voice hotkey dropdown on macOS, Windows,
    and Linux.

A2. F1–F12 appear in the dropdown as individual entries, grouped
    under a "Function keys" sub-section.

A3. Selecting "Custom key…" opens a press-to-capture modal; pressing
    F19 binds F19; pressing the app-quit shortcut shows an inline
    error and does not save.

A4 is split per platform per B1's split semantics:

- **A4-win-lin (hold-to-talk).** On Windows and Linux, holding the
  bound key activates voice input; releasing deactivates it.
  Pixel-equivalent to today's behavior for the existing modifier
  keys.
- **A4-mac (press-to-toggle).** On macOS, when the bound key is
  Caps Lock, tapping starts voice input recording and tapping
  again stops it. For all other macOS-bound keys (F-keys, modifier
  keys, custom non-CapsLock), behavior is hold-to-talk identical
  to Windows/Linux. Caps Lock is the only key that uses press-to-
  toggle, because macOS reports it as a toggle event rather than a
  sustained press.

> **Correction (re-review #10127):** the previous A4 still said
> "holding the bound key … releasing deactivates" for every key,
> which contradicted B1's macOS Caps Lock tap-to-toggle. Resolved
> by splitting A4 above.

A5. A user upgrading from the current build with
    `agents.voice.voice_input_toggle_key = "alt_left"` in their
    settings file sees no change in behavior. The dropdown shows
    "Option (Left)" selected (macOS) / "Alt (Left)" (Windows/Linux).

A6. With an unbindable key bound from a corrupted settings file
    (e.g. someone hand-edited TOML to set `voice_input_toggle_key =
    "enter"`), the dropdown shows "None" and a one-time toast
    explains the setting was reset for safety. The TOML is
    rewritten with the corrected value so the user is not stuck in
    a permanent toast loop on subsequent launches.

> **Correction (re-review #10127):** the previous A6 said the TOML
> was left unchanged. tech.md's reset path rewrites the TOML so
> the user-facing toast fires once, not every launch. Updated A6
> here to match.

## Risks and decisions for tech.md

1. **Enum vs. `KeyCode`.** The current setting is a Rust enum with
   `SettingsValue`-derived TOML serialization. Adding F1–F12 as
   variants is fine. But "Custom key" must persist a `KeyCode` value
   that is not in the curated enum. Choices:
   (a) Make `VoiceInputToggleKey` carry a `Custom(KeyCode)` variant.
   (b) Keep the curated enum for the dropdown and add a separate
       `voice_input_toggle_keycode_override: Option<KeyCode>` setting
       that, if set, overrides the enum.
   The TECH spec must pick one and justify; (b) is less invasive but
   splits the source of truth.

2. **TOML serialization stability.** `SettingsValue` derive uses
   `rename_all = "snake_case"`. New variants `CapsLock` → `caps_lock`,
   `F1` → `f1`. For (a) above, `Custom(KeyCode)` needs custom
   serialization; the natural choice is `{ custom = "f19" }` matching
   `KeyCode`'s lowercase serialization.

3. **macOS Caps Lock special-case.** On macOS, Caps Lock generates a
   single keydown event when toggled on and a single keyup when
   toggled off, NOT a press-and-hold stream. This is OS-level. Our
   "hold to talk" semantics break for Caps Lock unless we either:
   (a) Use Caps Lock as a press-to-toggle (one tap = recording on,
       next tap = off);
   (b) Document the difference in the dropdown tooltip and let users
       decide whether to remap Caps Lock at the OS level first.
   The TECH spec must pick one and justify. Recommendation: (a),
   with a tooltip explaining the difference. Discord and Krisp both
   take approach (a).

4. **Dropdown UX with 20+ entries.** Adding Caps Lock + 12 F-keys +
   "Custom…" pushes the dropdown to 22 entries. The TECH spec must
   address whether to use a sub-menu, a search filter, or sectioned
   headings. Recommendation: sectioned headings ("Modifier keys" /
   "Function keys" / "Other") with the current modifiers at the top
   so existing users see no reorder.

5. **Press-to-capture cancellation.** What happens if the user
   presses Escape (which is on the B5 reject-list)? The TECH spec
   must define: Escape ALWAYS dismisses the modal without saving
   (not "rejected with error"), because Escape is the universal
   "cancel modal" key.

## Reporter-supplied detail (preserved)

The reporter explicitly cited Caps Lock and F1–F12 as the most-wanted
additions, with F-keys justified by ergonomic isolation from the
typing flow and Caps Lock justified by its commonness as a
push-to-talk key in voice/video apps. The reporter also requested
"any other bindable key" which B3 satisfies.
