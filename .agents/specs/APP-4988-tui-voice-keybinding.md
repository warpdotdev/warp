# Spec: TUI configurable hold key for voice input

Linear: APP-4988 — https://linear.app/warpdotdev/issue/APP-4988/tui-configurable-keybinding-to-start-voice-mode
Originating thread: https://warpdev.slack.com/archives/C0BDQDW8V5E/p1784988760470699
Relevant repo: `warp`

== PRODUCT ==

*Summary:* Add a TUI-specific setting for a push-to-talk modifier key. Pressing and holding the configured modifier starts voice recording; releasing the same physical modifier stops recording and begins transcription. The existing `ctrl-s` tap-to-start binding and `/voice` command remain independent and unchanged.

*Behavior:*
1. **Default remains unchanged.** `agents.voice.voice_input_hold_key` defaults to `none`. With that value, no hold-to-record interaction is installed and `ctrl-s` remains the default voice keybinding.
2. **Press starts recording.** When the session composer owns input and the terminal supports keyboard enhancement, pressing the configured modifier starts voice recording. Repeated presses and a press while another voice session is active do nothing.
3. **Release stops only the matching hold session.** Releasing the same left/right modifier that successfully started the hold session stops recording and begins transcription. Unrelated modifier releases do nothing.
4. **Other voice entry points stay independent.** Releasing the configured modifier must not stop voice started by `ctrl-s`, `/voice`, or another interaction. If the hold-started recording ends through Escape, Enter, failure, cancellation, or completion, the held-key marker is cleared before any later release can affect another recording.
5. **Existing stop controls remain available.** Escape and Enter continue to stop or cancel voice according to the existing TUI voice lifecycle. They remain a fallback if a terminal fails to deliver a modifier release.
6. **Supported values reflect TUI input capabilities.** Values are `none`, `alt_left`, `alt_right`, `control_left`, `control_right`, `shift_left`, `shift_right`, `super_left`, and `super_right`. `fn` is not accepted because crossterm cannot report it. Super/Command remains best-effort because many host terminals or operating systems intercept it.
7. **Left and right remain distinct.** A configured left modifier does not match the corresponding right modifier, and vice versa.
8. **Physical modifier semantics match the GUI.** If the configured modifier is held as part of a chord, voice is active for the duration that modifier is held. The chord's existing action still fires independently. For example, configuring `control_left` means a left-Control `ctrl-c` press both activates push-to-talk while Control is held and dispatches the existing `ctrl-c` action.
9. **Graceful degradation.** On terminals without Kitty keyboard enhancement support, the configured hold interaction is inactive. It does not swallow input or change ordinary keybindings.
10. **Settings apply live.** The session view reads `TuiVoiceSettings` while rendering and subscribes to changes. A changed hold key applies when the view rebuilds without re-registering the keymap.

== TECH ==

*Settings:*
- `app/src/settings/tui_voice.rs:19` defines the TUI-only `TuiVoiceInputHoldKey` enum. It intentionally excludes `Fn`.
- `app/src/settings/tui_voice.rs:49` registers `TuiVoiceSettings.voice_input_hold_key` at `agents.voice.voice_input_hold_key`, with `SettingSurfaces::TUI`, `SyncToCloud::Never`, and default `none`.
- The GUI keeps its existing `agents.voice.voice_input_toggle_key` setting and `VoiceInputToggleKey` type. The two settings have different accepted value sets and separate schema entries, so the schema generator requires no duplicate-path merge behavior.

*Terminal input protocol:*
- The lower `factory/tui-key-lifecycle` branch enables `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES` on enhancement-capable terminals, and adds `REPORT_ALTERNATE_KEYS | REPORT_ALL_KEYS_AS_ESCAPE_CODES` only when standalone modifier reporting is requested. It exposes repeat-aware `TuiEvent::KeyDown` plus `TuiEvent::ModifierKeyChanged { key_code, state }`.
- This branch derives that request from the hold-key setting: `crates/warp_tui/src/voice_input.rs` (`configured_hold_key`, `requires_modifier_key_reporting`) maps the setting to a physical `KeyCode` and to that request, `crates/warp_tui/src/session.rs` applies it at mount, and `TuiDriverHandle::set_modifier_key_lifecycle_enabled` re-applies it to a live session using the protocol's set form, which leaves the mode-stack entry restored at exit untouched.
- `ModifierKeyChanged` carries a platform-neutral `KeyCode`, so modifier events preserve left/right identity without a voice-specific core enum.
- Repeats remain key-down events for existing keymap, editor, and PTY behavior. Non-modifier releases are dropped before dispatch; modifier press/release reaches the foreground process only when that process requested all-keys reporting itself.
- Requesting modifier reporting also makes the terminal report the layout's shifted character, so symbol chords are spelled as the produced character (`ctrl-!`) rather than the base key (`ctrl-shift-1`). No TUI binding uses punctuation today.

*Dispatch and state ownership:*
- The lower branch's `TuiEventHandler::on_modifier_key_changed` lets an element wrapper observe modifier press/release after its child declines them, with explicit propagation.
- `crates/warp_tui/src/terminal_session_view.rs` (`VOICE_INPUT_BINDING_NAME`) preserves the existing `tui:session:start_voice_input` `ctrl-s` binding and `StartVoiceInput` action.
- `crates/warp_tui/src/terminal_session_view.rs` (`with_voice_hold_handler`) wraps the rendered session tree and matches only the configured physical `KeyCode`. Press starts; release stops; a press while a hold is already active does not retrigger. The wrap lives at the session rather than the input view because the input view's element subtree is absent whenever something else owns the composer area, and `TuiView::render` applies it once for every session state — including the conversation-restore screens — so no state can swallow a release.
- `TuiTerminalSessionAction::VoiceHoldKeyChanged { key, state }` is separate from `StartVoiceInput`, so release handling remains independent of the ordinary keymap.
- `crates/warp_tui/src/voice_input.rs` (`TuiVoiceInputModel::hold_key`, `handle_hold_key`, `stop_hold`) owns hold state alongside the voice lifecycle it constrains: `start` records the `KeyCode` only for `VoiceInputStartSource::HoldKey`, and `set_state` drops it on every transition out of `Listening`, so a release can only stop the recording its own press started.
- `crates/warp_tui/src/input/view.rs` (`voice_hold_key`, `handle_voice_hold_key`, `stop_active_voice_hold`, `stop_voice_input`) exposes those model operations to the session owner. The recording, transcription, Escape, and Enter lifecycle remains owned by the TUI voice model and input view.

*Compatibility and blast radius:*
- The lower branch owns terminal-protocol and general key-lifecycle compatibility. This branch only consumes physical modifier key-down/key-up events.
- Modifier events bypass keymap matching, so chord actions such as `ctrl-s`, `ctrl-c`, and `ctrl-v` still dispatch through their existing key-down paths. As in the GUI, the configured physical modifier also activates push-to-talk while it is held for a chord.
- The session-level wrapper handles only the configured physical key; unrelated and repeated key events propagate.
- If a host terminal does not report the configured modifier or its release, the feature degrades without affecting other input. Escape and Enter remain available to end recording.

*Validation:*
1. `cargo check -p warpui_core --features tui --all-targets` and `cargo check -p warp_tui --all-targets` compile cleanly.
2. The lower branch's complete `warpui_core` and `warp_tui` suites cover Press/Repeat/Release conversion, keymap behavior, child-first lifecycle dispatch, PTY forwarding, and keyboard protocol setup.
3. `cargo nextest run -p warp_tui -E 'test(/voice/)'` covers setting-to-modifier mapping, exact-side matching, release-to-stop, stale-hold cleanup, release after composer ownership changes, `ctrl-s` independence, and user guidance.
4. `cargo nextest run -p warp -E 'test(/tui_voice_setting/)'` covers the setting default, round trip, TOML path, local-only sync policy, and TUI-only schema surface.
5. `cargo nextest run -p warpui_core --features tui` and `cargo nextest run -p warp_tui` pass as crate-level regressions.
6. `./script/format` and the repository clippy command pass.
7. Real-terminal verification on a Kitty-protocol-capable terminal confirms: the configured physical modifier starts on Press and stops on Release; the opposite side does nothing; `none` disables the hold interaction; `ctrl-s` still starts independently; Escape/Enter still end recording; and a chord using the configured modifier continues to dispatch its normal action while voice remains active for the physical hold.
