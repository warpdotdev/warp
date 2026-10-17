# Technical spec: Voice input hotkey picker (GH-9869)

This spec is the implementation companion to `product.md`. It picks
the data-model approach, the UI changes, and the testable invariants.

## Current implementation

- **Setting enum:** `VoiceInputToggleKey` at
  [app/src/settings/ai.rs:110-138](app/src/settings/ai.rs). 10
  variants. Derives `SettingsValue` (TOML `snake_case`).
- **Persistence:** `agents.voice.voice_input_toggle_key` TOML path,
  `SyncToCloud::Never` (intentional — see product spec §non-goals).
- **Display:** `VoiceInputToggleKey::display_name`
  ([app/src/settings/ai.rs:169-198](app/src/settings/ai.rs)) returns
  platform-aware strings ("Option (Left)" on macOS,
  "Alt (Left)" on Windows/Linux).
- **Mapping to `KeyCode`:** `VoiceInputToggleKey::to_key_code`
  ([app/src/settings/ai.rs:200-213](app/src/settings/ai.rs)) maps
  variants to `warpui_core::platform::KeyCode`.
- **Dropdown UI:** [app/src/settings_view/ai_page.rs:498-532](app/src/settings_view/ai_page.rs)
  builds a `Dropdown<AISettingsPageAction>` from
  `VoiceInputToggleKey::all_possible_values()`.
- **Underlying `KeyCode` enum:** [crates/warpui_core/src/platform/keyboard.rs:85](crates/warpui_core/src/platform/keyboard.rs)
  already supports `CapsLock` (line 210), `F1`–`F35` (lines 437+),
  and the long tail.
- **Action dispatch:** `block_list_element.rs:3072` and
  `alt_screen_element.rs:620` compare incoming `key_code` to the
  configured one. No code changes needed there for new variants —
  they speak `KeyCode`, not the enum.

## Data model: `Custom(KeyCode)` variant on the existing enum

Choice (a) from product.md §risks: extend `VoiceInputToggleKey` with
a `Custom(KeyCode)` variant.

```rust
pub enum VoiceInputToggleKey {
    None,
    Fn,
    AltLeft, AltRight,
    ControlLeft, ControlRight,
    SuperLeft, SuperRight,
    ShiftLeft, ShiftRight,
    CapsLock,                          // NEW
    F1, F2, F3, F4, F5, F6,            // NEW
    F7, F8, F9, F10, F11, F12,         // NEW
    Custom(KeyCode),                   // NEW — non-iter variant
}
```

Why (a) over (b) (separate `Option<KeyCode>` override field):
- Single source of truth for "what key fires voice input."
- The dropdown selection state cleanly maps to one enum value.
- Avoids a "two settings disagree" failure mode where a user changes
  the dropdown but the override is stale.

### Replacing the EnumIter / `all_possible_values()` flow

> **Correction (review #10127):** the existing
> `VoiceInputToggleKey::all_possible_values()`
> ([app/src/settings/ai.rs:153](app/src/settings/ai.rs)) calls
> `Self::iter().collect()` from `strum::EnumIter`. Adding a payload
> variant like `Custom(KeyCode)` makes the enum non-iterable: there is
> no finite list of `KeyCode` payloads.

Replace `all_possible_values()` with a curated `predefined_options()`
that returns the unit variants for the dropdown:

```rust
impl VoiceInputToggleKey {
    pub fn predefined_options() -> Vec<VoiceInputToggleKey> {
        let mut out = vec![
            VoiceInputToggleKey::None,
            VoiceInputToggleKey::AltLeft, VoiceInputToggleKey::AltRight,
            VoiceInputToggleKey::ControlLeft, VoiceInputToggleKey::ControlRight,
            VoiceInputToggleKey::SuperLeft, VoiceInputToggleKey::SuperRight,
            VoiceInputToggleKey::ShiftLeft, VoiceInputToggleKey::ShiftRight,
            VoiceInputToggleKey::CapsLock,
            VoiceInputToggleKey::F1,  VoiceInputToggleKey::F2,
            VoiceInputToggleKey::F3,  VoiceInputToggleKey::F4,
            VoiceInputToggleKey::F5,  VoiceInputToggleKey::F6,
            VoiceInputToggleKey::F7,  VoiceInputToggleKey::F8,
            VoiceInputToggleKey::F9,  VoiceInputToggleKey::F10,
            VoiceInputToggleKey::F11, VoiceInputToggleKey::F12,
        ];
        if matches!(OperatingSystem::get(), OperatingSystem::Mac) {
            // Fn was macOS-only in the original list; preserve that.
            out.insert(1, VoiceInputToggleKey::Fn);
        }
        out
    }
}
```

`Custom(KeyCode)` is **not** part of `predefined_options()` — it is
constructed by the press-to-capture modal at runtime. The dropdown
shows a synthetic "Custom key…" entry that opens the modal; once a
key is captured, the resulting `Custom(KeyCode)` is set as the
current value but does not appear in the predefined list.

Drop the `EnumIter` derive (it would now be wrong for the payload
variant). Existing call sites of `iter()` are limited to
`all_possible_values()` itself, so this is a one-line removal.

### Displaying a persisted `Custom(KeyCode)` in the dropdown

> **Correction (re-review #10127):** the previous draft did not
> specify how the dropdown shows the *currently selected* value
> when that value is a `Custom(KeyCode)` outside
> `predefined_options()`. Resolved below.

When the user has previously bound a `Custom(KeyCode::F19)` (or
similar), the dropdown still uses `predefined_options()` for the
**list of choices**, but the **selected-row label** is rendered
from the live setting value:

1. The dropdown's `set_selected_by_index` call at
   [app/src/settings_view/ai_page.rs:529](app/src/settings_view/ai_page.rs)
   currently looks up the index of the current value in
   `predefined_options()`. For a `Custom` value this lookup
   returns `None`.
2. On `None`, instead of falling back to index 0 (which would
   *change* the persisted value to the first predefined option on
   render — a silent data-loss bug), the dropdown gets a virtual
   "current row" prepended to the list with the label
   `format!("Custom: {}", custom_display(key_code))` and gets
   selected.
3. The virtual row is **read-only as a list entry** — clicking it
   re-opens the press-to-capture modal pre-populated with the
   current `KeyCode` (so the user sees what they have and can
   confirm or change). It is removed from the list as soon as a
   different option is selected.

This keeps `predefined_options()` finite and small while
correctly displaying any `Custom(...)` already in the user's
TOML, including values from a future build that captured a key
the current build's predefined list doesn't surface.

`custom_display(KeyCode)` is the same friendly-name helper used
in the modal display (see "Display name for new variants" below)
so both surfaces show the same string for the same `KeyCode`.

### Settings serialization

The existing `implement_setting_for_enum!` macro
([app/src/settings/ai.rs:140](app/src/settings/ai.rs)) does **not**
support payload variants. Two options:

1. **Extend the macro** to allow a `Custom(T)` arm with a custom
   tag/serialize hook.
2. **Hand-write `Serialize`/`Deserialize` for `VoiceInputToggleKey`**
   and use the simpler `implement_setting!` (non-enum) macro for
   persistence.

Recommendation: **option 2.** The macro extension would be a one-off
for this single enum and the manual impl is ~30 lines of
straightforward code, fully testable.

### Canonical TOML format

> **Correction (review #10127):** earlier drafts mentioned both
> `{ custom = "F19" }` and `{ kind = "custom", key = "f19" }`. The
> canonical form is below; the tests assert it bit-for-bit.

For unit variants — existing snake_case format, no change:
```toml
voice_input_toggle_key = "alt_left"
voice_input_toggle_key = "caps_lock"
voice_input_toggle_key = "f1"
voice_input_toggle_key = "f12"
```

For `Custom(KeyCode)` — TOML inline-table with one key, `custom`,
and the `KeyCode` value as its lowercased Serde-default
representation:
```toml
voice_input_toggle_key = { custom = "f19" }
voice_input_toggle_key = { custom = "backslash" }
voice_input_toggle_key = { custom = "intl_yen" }   # snake_case for multi-word
```

The `KeyCode` payload uses `serde_with`-style snake_case lowercasing
to match the existing settings TOML convention. Round-trip:
- Serialize: `KeyCode::F19` → `"f19"`, `KeyCode::IntlYen` → `"intl_yen"`.
- Deserialize: case-insensitive match against the snake_case form.
The hand-written `Serialize`/`Deserialize` impl is the single
source of truth for this mapping; T2 in the test plan asserts the
exact strings.

### Migration

No migration needed. Existing TOML values continue to deserialize.
New variants only appear in TOML if the user picks them in the UI.

## UI changes (ai_page.rs)

### Dropdown sectioning

Replace the flat `dropdown.add_items(values.into_iter().map(...))`
loop at
[app/src/settings_view/ai_page.rs:517-528](app/src/settings_view/ai_page.rs)
with a sectioned builder:

```text
[Disabled]
  None

[Modifier keys]
  Fn (macOS only)
  Option (Left), Option (Right)
  Control (Left), Control (Right)
  Command (Left), Command (Right)
  Shift (Left), Shift (Right)
  Caps Lock                ← NEW

[Function keys]            ← NEW SECTION
  F1, F2, F3, F4, F5, F6,
  F7, F8, F9, F10, F11, F12

[Other]                    ← NEW SECTION
  Custom key…              ← opens press-to-capture modal
```

If `Dropdown` does not support section headings today, add a
non-clickable `DropdownItem` constructor (or use a thin separator
`DropdownItem`). The TECH spec recommends adding the heading
support; it is small and reusable.

### Press-to-capture modal

New module `app/src/settings_view/voice_hotkey_capture_modal.rs`.

- Triggered when the user clicks "Custom key…".
- Renders a focused modal: title "Press the key for voice input",
  body with the captured key shown by **friendly display name**
  (e.g. "F19", "Caps Lock", "Backslash") and a hover tooltip
  showing the protocol-level `KeyCode` spelling (e.g.
  `KeyCode::F19`); initially empty, Cancel + Confirm buttons,
  an inline error area.

  > **Correction (re-review #10127):** the previous draft showed
  > "Selected: F19 (KeyCode::F19)" inline in the body — that
  > inlines the protocol spelling. The resolved decision (Open
  > Question 4 in the previous round) is friendly-name-only
  > inline, with `KeyCode` spelling only in the hover tooltip.
  > Both surfaces (modal body and dropdown "Custom: ..." row)
  > use the same friendly name.
- Listens for raw key events at the top level (using whatever modal
  key-event hook the existing settings modals use — verify by
  inspecting one existing modal first).
- Filters per B5 (product.md): rejects unbindable keys with an
  inline error; accepts any other single key.
- On Confirm, dispatches
  `AISettingsPageAction::SetVoiceInputToggleKey(VoiceInputToggleKey::Custom(key_code))`.
- On Cancel or Escape, closes without dispatch.
- The modal's reject-list (B5) operates on **full keystrokes** (key
  + modifiers), not bare `KeyCode`s.

> **Correction (review #10127):** earlier drafts described the
> reject-list as a `KeyCode` set. App-quit (`Cmd+Q` on macOS,
> `Alt+F4` on Windows) is a chord — comparing only `KeyCode::KeyQ`
> would either ban a bare `Q` (wrong) or fail to reject Cmd+Q. The
> reject-list must compare full `Keystroke`s.

  Reject-list construction (at modal open time):

  - **Currently-bound app-quit shortcut:** read the full
    `Keystroke` from the key-binding registry (e.g. `Cmd+Q` on
    macOS, `Alt+F4` on Windows). The captured input is rejected
    only if its full `Keystroke` (key + modifiers) matches.
  - **Currently-bound voice-input shortcut:** the modal does NOT
    add the existing voice-input value to the reject list. The
    capture flow exists precisely to *change* the binding; rejecting
    the existing value would only matter for the "same key already"
    no-op confirm case, which we instead handle by accepting the
    capture and short-circuiting the dispatch when the captured
    `KeyCode` equals the current one. There is no infinite loop
    risk because the dispatch handler at `block_list_element.rs:3072`
    only fires voice activation, never re-binds the hotkey.

> **Correction (re-review #10127):** the previous draft said the
> reject-list "rejects if the user picks a different value than the
> existing one for any reason" — which would have prevented users
> from changing an existing custom hotkey. That was the literal
> opposite of the intended behavior. Resolved as above.
  - **Static structural rejects:** these are bare-key rejects (no
    modifier needed) because they would brick the input flow:
    `[Enter, Escape, Tab, Backspace, Space]`. The modifier
    state is ignored for these specifically. (Pressing
    `Cmd+Enter` is also rejected — Cmd+Enter is just as broken as
    bare Enter for our use case.)
  - **Bare modifier rule:** a press whose `KeyCode` is a modifier
    AND no other key is pressed can still be captured via the
    >800ms held-modifier rule (B3). The captured value is the
    bare modifier (`KeyCode::ShiftLeft`), not the
    chord, since voice activation tracks press/release of a
    single physical key.

  Because the captured value persisted into the setting is a
  single `KeyCode` (or one of the predefined enum variants), not a
  full `Keystroke`, the reject-list comparison happens **at
  capture time only**, before the value is saved. Once saved, the
  dispatch-side (`block_list_element.rs:3072` /
  `alt_screen_element.rs:620`) only sees the bare `KeyCode` and
  matches a single key event.

### Caps Lock toggle vs. hold semantics

Per product.md §risk 3, on macOS Caps Lock fires only on
toggle-on/toggle-off, not on press/release. The handler in
`block_list_element.rs:3072` uses press-and-release tracking. For
the `CapsLock` variant on macOS, switch the handler to
press-to-toggle: first event starts recording, second event stops.

Implementation: a new helper `VoiceInputToggleKey::is_toggle_mode()`
returns `true` for `CapsLock` on macOS, `false` otherwise. The
handler branches on this. The dropdown tooltip for the `CapsLock`
entry on macOS displays "Tap to start, tap again to stop" instead
of "Hold to talk."

## Display name for new variants

Extend `VoiceInputToggleKey::display_name`
([app/src/settings/ai.rs:169](app/src/settings/ai.rs)):

```rust
match self {
    // existing arms unchanged
    VoiceInputToggleKey::CapsLock => "Caps Lock",
    VoiceInputToggleKey::F1 => "F1",
    // ... F2..F12
    VoiceInputToggleKey::Custom(key_code) => {
        // Use a new helper that mirrors what the keymap layer shows
        // for this KeyCode, falling back to {key_code:?}.
        Box::leak(format_keycode_display(key_code).into_boxed_str())
    }
}
```

Note: `display_name` currently uses `Box::leak` for runtime-formatted
strings. The pattern is unchanged for `Custom`.

`tooltip_message` ([app/src/settings/ai.rs:247](app/src/settings/ai.rs))
gets matching arms.

## Mapping to `KeyCode`

Extend `VoiceInputToggleKey::to_key_code`
([app/src/settings/ai.rs:200](app/src/settings/ai.rs)):

```rust
VoiceInputToggleKey::CapsLock => Some(KeyCode::CapsLock),
VoiceInputToggleKey::F1 => Some(KeyCode::F1),
// ... F2..F12
VoiceInputToggleKey::Custom(key_code) => Some(*key_code),
```

`keystroke()` at line 218: the existing modifier-only path returns
a `Keystroke` with the modifier flag set. For non-modifier keys
(F-keys, CapsLock, custom), construct a `Keystroke { key:
keycode_string(key_code).to_string(), .. Default::default() }`.

## Settings reset on corrupted TOML (A6)

> **Correction (review #10127):** earlier drafts said the validation
> hook fires when "deserialization successfully produces an
> unbindable `KeyCode`." But TOML like `voice_input_toggle_key =
> "enter"` does NOT deserialize successfully into the curated unit-
> variant set — it fails parsing. A `validate()` hook running on the
> deserialized value never sees the bad value. Below is the corrected
> design that intercepts the raw parse failure.

> **Correction (re-review #10127):** the previous draft proposed
> mutating a `Mutex<Option<String>>` on `AISettings` *from inside*
> the `Deserialize` impl of `VoiceInputToggleKey`. A field
> deserializer doesn't have a reference to the containing settings
> struct — that design isn't feasible Serde. The corrected design
> below uses a wrapper-enum field type, which IS standard Serde,
> and post-processes after deserialization completes.
>
> Also corrected: product.md A6 said the TOML was left unchanged
> while tech.md said it was rewritten. Resolved to **rewrite** so
> the toast fires once, not on every launch.

The TOML field uses a wrapper enum that captures both successful
and failed parses without erroring out:

```rust
#[derive(Deserialize)]
#[serde(untagged)]
enum VoiceInputTomlValue {
    /// The TOML value parsed cleanly into one of the predefined
    /// variants OR the `{ custom = "..." }` form.
    Valid(VoiceInputToggleKey),
    /// Anything else — string, table, integer — is captured raw
    /// for diagnostic surfacing.
    Invalid(toml::Value),
}
```

> **Correction (re-review #10127):** the previous draft introduced
> the wrapper but didn't reconcile it with the rest of the spec
> (which still treats `voice_input_toggle_key` as if it had the
> bare `VoiceInputToggleKey` type — `set_value`, dropdown reads,
> tests, etc.). The reconciliation below makes the wrapper an
> internal implementation detail; all external surfaces continue
> to see the unwrapped type.

**API reconciliation:**

- The `AISettings` struct holds the field internally typed as
  `VoiceInputTomlValue`, deserialized directly from TOML.
- Immediately after `AISettings::initialize()` (the post-init pass
  described below), the field is normalized: any `Invalid` value
  is logged, the toast is fired, the TOML is rewritten with
  `VoiceInputToggleKey::None`, and the field is replaced with
  `Valid(VoiceInputToggleKey::None)`.
- After post-init, **the field is always `Valid(...)`**. All
  external accessors are typed as `VoiceInputToggleKey`:
  - `voice_input_toggle_key.value() -> VoiceInputToggleKey`
    unwraps the always-`Valid` variant.
  - `voice_input_toggle_key.set_value(VoiceInputToggleKey, ctx)`
    wraps in `Valid` before storing.
- Existing tests, dropdown rendering, and dispatch all continue
  to use `VoiceInputToggleKey` exactly as documented elsewhere in
  this spec. The wrapper is a 30-line implementation detail in
  the field's `Serialize`/`Deserialize` impls + the post-init
  pass, not a leak into the settings public API.

T1, T2, IT1–IT3 in the test plan continue to assert against
`VoiceInputToggleKey` values directly — they don't see the
wrapper.

This works with stock Serde (`#[serde(untagged)]` falls through to
`Invalid` when none of the `Valid` shapes match) and does not
require any side-channel mutation from inside a deserializer.

After `AISettings::initialize()` completes, a single post-init pass
walks the deserialized struct:

```rust
fn post_init_voice_input_toggle_key(settings: &mut AISettings, ctx: &mut AppContext) {
    let Some(invalid) = settings.voice_input_toggle_key.take_if_invalid() else {
        // Either Valid (no action) or already drained on a previous launch.
        return;
    };

    // Path 1 (raw parse failure) — `invalid` is the rejected toml::Value.
    let raw_repr = invalid.to_string();
    log::warn!(raw = ?raw_repr,
        "voice_input_toggle_key did not deserialize, resetting to None");
    settings.voice_input_toggle_key.set_value(VoiceInputToggleKey::None, ctx);
    settings.persist_to_disk(ctx);  // rewrite TOML so the toast fires once
    fire_toast(ctx, format!(
        "Voice input hotkey reset — could not parse `{raw_repr}`"
    ));
}
```

Path 2 (parsed-but-unbindable: e.g. a `Valid` value that fails the
runtime reject-list check) is folded into the same routine: after
unwrapping `Valid`, run `validate()`; if it fails, log + rewrite +
toast with the same code path as Path 1, but with the rejected
`KeyCode`'s display name as the raw representation.

This guarantees A6's toast fires **once** for any combination of
TOML-parse-failure and post-parse-validation-failure, then the
disk file is consistent and subsequent launches are quiet.

## Test plan

### Unit tests (`app/src/settings/ai.rs::tests`)

- T1: TOML round-trip for each new variant (`caps_lock`, `f1`–`f12`).
- T2: TOML round-trip for `Custom(KeyCode::F19)` and
  `Custom(KeyCode::Backslash)`.
- T3: `display_name` returns expected strings on each platform.
- T4: `to_key_code` returns the correct `KeyCode` for each variant.
- T5: `is_toggle_mode()` returns true for `CapsLock` on macOS,
  false otherwise.
- T6: A TOML file with an invalid `voice_input_toggle_key = "enter"`
  loads as `None` and triggers the validation reset path.

### Unit tests (`app/src/settings_view/voice_hotkey_capture_modal_test.rs`)

- T7: Pressing F19 in the modal captures `KeyCode::F19`.
- T8: Pressing the app-quit shortcut shows an inline error and does
  not dispatch.
- T9: Pressing Escape closes without dispatch.
- T10: Holding only Shift for >800ms then releasing captures
  `KeyCode::ShiftLeft` (B3 carve-out).

### Integration test (`app/src/integration_testing/`)

- IT1: Open the AI settings page, click "Custom key…", press F19,
  click Confirm. Verify `AISettings.voice_input_toggle_key.value() ==
  Custom(KeyCode::F19)`.
- IT2: Set the setting to `CapsLock` on macOS, press CapsLock once,
  verify voice input starts; press CapsLock again, verify it stops.
- IT3: Set the setting to `F5`, hold F5, verify voice input is
  active; release, verify it stops.

## Files touched

- `app/src/settings/ai.rs` — extend enum, `display_name`,
  `to_key_code`, `keystroke`, `tooltip_message`, add
  `is_toggle_mode()` and `validate()`.
- `app/src/settings_view/ai_page.rs` — sectioned dropdown,
  "Custom key…" entry, modal trigger.
- `app/src/settings_view/voice_hotkey_capture_modal.rs` (new) —
  press-to-capture modal.
- `app/src/terminal/block_list_element.rs:3072` — branch on
  `is_toggle_mode()` for press-to-toggle vs hold-to-talk.
- `app/src/terminal/alt_screen/alt_screen_element.rs:620` — same.
- `app/src/settings_view/voice_hotkey_capture_modal_test.rs` (new) —
  T7–T10.
- `app/src/integration_testing/settings/voice_hotkey_test.rs` (new)
  — IT1–IT3.

## Out-of-scope follow-ups

- Modifier-combination hotkeys (Cmd+Shift+V).
- Per-tab or per-profile voice hotkey.
- Cloud sync for voice hotkey (would conflict with cross-device
  keyboard variation).
- Remapping the OS-wide voice input shortcut.

## Open questions for maintainer review

1. ~~Does the `SettingsValue` derive macro support enum variants
   with payloads (`Custom(KeyCode)`), or do we need a manual
   `Serialize`/`Deserialize` impl?~~ **Resolved (re-review
   #10127):** committed to the manual impl path in the "Settings
   serialization" section above (option 2). The `VoiceInputTomlValue`
   wrapper enum and post-init pass on `AISettings::initialize`
   are the implementation. No macro extension needed.
2. For the dropdown sectioning, is there an existing
   `DropdownItem::heading()` constructor, or should we add one?
3. Confirm the macOS Caps Lock press-to-toggle decision (vs
   "document the difference and don't fix"). Discord and Krisp
   both go press-to-toggle.
4. ~~Should the press-to-capture modal expose the captured
   `KeyCode` spelling to the user?~~ **Resolved (re-review
   #10127):** the modal shows the friendly display name primarily
   (e.g. "F19", "Caps Lock"), with the protocol-level `KeyCode`
   spelling (e.g. `KeyCode::F19`) shown only in a hover tooltip
   on the captured-key label. This matches the dropdown's
   "Custom: F19" rendering above and keeps the same friendly name
   on both surfaces. Power users get the protocol spelling without
   noise for everyone else.
