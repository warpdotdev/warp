# TECH.md — Select microphone input device for Voice commands
Issue: https://github.com/warpdotdev/warp/issues/10931
Product spec: `specs/GH10931/product.md`

## Context
Warp already has a Voice settings section and a Voice recording pipeline, but the recording pipeline has no concept of a user-selected input device.

Relevant current code:

- `app/src/settings/ai.rs (79-151)` defines `VoiceInputToggleKey`, its settings storage, display names, and conversion to physical key codes.
- `app/src/settings/ai.rs:854` defines `voice_input_enabled_internal`, and `app/src/settings/ai.rs:883` stores the existing local-only `voice_input_toggle_key`.
- `app/src/settings/ai.rs (1875-1903)` implements first-time Voice setup and sets the default activation key.
- `app/src/settings_view/ai_page.rs:521` constructs `voice_input_toggle_key_dropdown`; `app/src/settings_view/ai_page.rs:945` and `app/src/settings_view/ai_page.rs:961` refresh that dropdown when AI/Voice settings change.
- `app/src/settings_view/ai_page.rs (5957-6019)` renders the Voice section, including the Voice Input toggle, description, and "Key for Activating Voice Input" dropdown.
- `app/src/settings_view/ai_page.rs:2709` handles `SetVoiceInputToggleKey` by writing to `AISettings`.
- `crates/voice_input/src/lib.rs:136` starts a Voice recording session. `crates/voice_input/src/lib.rs:153` currently calls `cpal::default_host().default_input_device()`, so every session uses the current system default.
- `app/src/editor/view/voice.rs:278` calls `VoiceInput::start_listening` when the user starts Voice from the microphone button or configured activation key.
- `app/src/root_view.rs:3181` handles stopping key-triggered active Voice input when the configured key is released.
- `crates/warpui_core/src/platform/mod.rs:272` exposes only microphone authorization state on the platform delegate; `crates/warpui/src/platform/mac/delegate.rs:418` implements that state with AVFoundation authorization.
- `app/src/server/voice_transcriber.rs (20-33)` sends the recorded WAV to the existing Wispr transcription provider and does not need to know which microphone produced the audio.

The change spans settings persistence, Settings UI, audio device enumeration/resolution, and the Voice session start path. The transcription service, quota checks, activation-key semantics, and text insertion behavior should remain unchanged.

## Proposed changes

### 1. Add a local-only Voice microphone setting
Add a serializable setting to `AISettings` next to the existing Voice settings:

- Suggested public setting name: `voice_input_microphone`.
- Suggested TOML path: `agents.voice.microphone`.
- Supported platforms: `SupportedPlatforms::DESKTOP`.
- Sync behavior: `SyncToCloud::Never`, matching the per-device rationale used by `voice_input_toggle_key` and microphone permission state.
- Default: `VoiceInputMicrophone::SystemDefault`.

Represent the value as a small settings-value type rather than a raw string:

- `VoiceInputMicrophone::SystemDefault`
- `VoiceInputMicrophone::Device { id: String, name: String }`

`id` is the stable backend identifier used to resolve the device. `name` is a display fallback and lets Settings show the unavailable selected device after the device is disconnected or renamed. Do not use `name` alone as the persisted identity because duplicate device names are possible and because the product spec requires the selected physical device to remain independent of the system default.

Add helpers on the setting type:

- `display_name(&self) -> String`
- `is_system_default(&self) -> bool`
- `storage_key()` and generated metadata through the existing settings macros

The generated `AISettingsChangedEvent` variant should be used by Settings UI to keep the dropdown selected value in sync across windows.

### 2. Add audio input device enumeration and resolution APIs to `voice_input`
Keep audio-device logic in `crates/voice_input` so the app layer does not depend on low-level audio APIs.

Introduce public types:

- `VoiceInputDeviceId(String)` or a type alias if the settings type remains app-owned.
- `VoiceInputDeviceInfo { id: String, name: String, is_system_default: bool }`
- `VoiceInputDeviceSelection`, or accept the app-owned `VoiceInputMicrophone` by passing primitive selection data into `start_listening`.

Add APIs:

- `VoiceInput::available_input_devices() -> Result<Vec<VoiceInputDeviceInfo>, DeviceEnumerationError>`
- `VoiceInput::default_input_device_info() -> Option<VoiceInputDeviceInfo>`
- an internal resolver that maps `SystemDefault` to `host.default_input_device()` and `Device { id, .. }` to a matching `cpal::Device`.

Implementation notes:

1. Use `cpal::default_host().input_devices()` for enumeration and `DeviceTrait::name()` for user-facing labels.
2. On macOS, use a stable CoreAudio device UID for `id`. `cpal` already depends on CoreAudio crates, but the implementation should choose the smallest direct dependency or platform helper needed to retrieve a UID from the enumerated input device. If `cpal` exposes a stable backend identifier by implementation time, prefer that over adding native glue.
3. Keep non-macOS compiling. If stable IDs are not available on Windows/Linux in the first pass, use a clearly documented fallback ID and avoid promising cross-reboot stability there. The issue is macOS-scoped, but the setting and code should not break desktop builds.
4. Deduplicate display names in the UI layer or return already-disambiguated labels from a helper so Settings can show two same-named devices distinctly.
5. Do not request microphone permission just to enumerate devices. Permission behavior should remain tied to recording start and the existing authorization checks.

Extend `StartListeningError` with a distinct unavailable-selection case:

- `SelectedDeviceUnavailable { name: String }`

This lets `EditorView` show a specific toast for product Behavior #12 instead of logging a generic stream creation error.

### 3. Pass the selected microphone into Voice session start
Change the Voice start path so the app chooses a selection and the audio layer resolves it:

1. In `app/src/editor/view/voice.rs`, read `AISettings::as_ref(ctx).voice_input_microphone` immediately before calling `VoiceInput::start_listening`.
2. Change `VoiceInput::start_listening(ctx, source)` to accept a microphone selection, for example `start_listening(ctx, source, microphone_selection)`.
3. In `crates/voice_input/src/lib.rs`, replace the unconditional `host.default_input_device()` call with:
   - default device resolution for `SystemDefault`
   - selected-device lookup for `Device { id, name }`
4. Keep all existing stream configuration, resampling, WAV conversion, session duration, abort, and transcription state transitions unchanged after the `cpal::Device` is chosen.

Changing the Settings dropdown while a session is listening should not mutate `VoiceInputState::Listening`; the opened `cpal::Stream` remains the source for that session.

### 4. Render the microphone dropdown in Settings
Update `AISettingsPageView` in `app/src/settings_view/ai_page.rs`:

- Add `voice_input_microphone_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>`.
- Add `AISettingsPageAction::SetVoiceInputMicrophone(VoiceInputMicrophone)`.
- Construct the dropdown in `AISettingsPageView::new` using the same width constants as the activation-key dropdown.
- Add a helper such as `refresh_voice_input_microphone_dropdown(&ViewHandle<Dropdown<_>>, ctx)` that:
  - enumerates devices from `VoiceInput::available_input_devices()`
  - inserts "System Default" first
  - inserts available device entries after it
  - inserts an unavailable selected-device entry when the saved `Device { id, name }` does not appear in the current enumeration
  - selects the saved setting value
  - disables or empty-states the menu when enumeration fails
- Subscribe to `AISettingsChangedEvent::VoiceInputMicrophone { .. }` and update the selected item, matching the `VoiceInputToggleKey` event handling pattern at `app/src/settings_view/ai_page.rs:945`.
- Refresh the list when the view is created and when the Voice settings section is re-rendered or the dropdown is about to open. If the existing dropdown component does not expose an open callback, refreshing on settings view construction plus on relevant settings changes is acceptable for the first implementation; document any known limitation in the PR.

In `VoiceWidget::render_voice_section`, render a new `render_dropdown_item` below the Voice description and before or after "Key for Activating Voice Input":

- Label: "Microphone" unless design chooses "Input device".
- Description: "Choose which microphone Warp uses for Voice input." or shorter copy confirmed by design.
- Local-only icon: use `LocalOnlyIconState::for_setting` with the new setting's storage key and sync behavior.
- Dropdown handle: `&view.voice_input_microphone_dropdown`.

Add search terms to `VoiceWidget::search_terms`: `microphone mic input device audio input`.

### 5. User-facing errors
In `app/src/editor/view/voice.rs`, handle the new `StartListeningError::SelectedDeviceUnavailable { name }` separately:

- Show a toast such as `Selected microphone "External USB Microphone" is unavailable. Choose another microphone in Settings > Agents > Warp Agent > Voice.`
- Do not start listening, do not send a "start" Voice telemetry event, and do not transcribe.

Keep `StartListeningError::AccessDenied` mapped to the existing microphone-access toast. Other stream/config errors can continue to log and use the existing generic Voice error behavior.

### 6. Telemetry and privacy
No telemetry is required for this spec. If implementation adds telemetry, do not send raw device IDs or exact microphone names. Acceptable fields are high-level values such as:

- `selection_type: "system_default" | "specific_device" | "unavailable_specific_device"`
- whether start failed because the selected device was unavailable

The saved device ID is local settings data and should not sync to cloud preferences.

### 7. Feature flag and rollout
This can ship behind the existing `voice_input` compile-time feature because the Settings section already renders only when `cfg!(feature = "voice_input") && UserWorkspaces::as_ref(app).is_voice_enabled()`. A new runtime feature flag is optional but not required unless product wants staged rollout independent of Voice input itself.

If a runtime flag is added, gate both the Settings picker and the selected-device recording behavior behind the same flag. Do not expose a selectable device UI that the recording path ignores.

## End-to-end flow
1. Settings opens the Voice section and populates the microphone dropdown from `voice_input` device enumeration.
2. The user selects `Device { id, name }`.
3. `AISettingsPageAction::SetVoiceInputMicrophone` writes the local-only `AISettings` value.
4. The user starts Voice from the microphone button or activation key.
5. `EditorView::toggle_voice_input` reads the saved selection and passes it to `VoiceInput::start_listening`.
6. `voice_input` resolves the selection to a `cpal::Device`.
7. If resolution succeeds, the existing stream/resample/WAV/transcription flow runs unchanged.
8. If resolution fails for a specific selected device, `SelectedDeviceUnavailable` is returned and the app shows the targeted error toast without recording from a different microphone.

## Testing and validation
Automated tests:

- Add unit tests for `VoiceInputMicrophone` serialization/deserialization, default value, display fallback, and `SyncToCloud::Never`.
- Add settings action tests or view-level tests for `SetVoiceInputMicrophone` writing the setting and preserving `explicitly_interacted_with_voice` semantics for the activation-key path.
- Add device-list helper tests that:
  - put "System Default" first
  - disambiguate duplicate display names
  - preserve an unavailable selected device as the selected value
  - select "System Default" when the saved value is default
- Add `voice_input` resolver tests behind a test seam. Because `cpal` devices are hardware-dependent, introduce a small trait or helper abstraction around host/device enumeration so tests can simulate:
  - default-device selection
  - matching selected-device ID
  - selected-device unavailable
  - duplicate names
  - no default input device
- Add a unit test for the new `StartListeningError::SelectedDeviceUnavailable` path if `EditorView` toast behavior is testable in existing editor view tests.

Manual validation on macOS:

1. Connect at least two microphones.
2. Open Settings > Agents > Warp Agent > Voice and confirm "Microphone" appears with "System Default" and connected devices.
3. Select an external microphone, change the OS default to the built-in microphone, start Voice, and confirm the recorded audio comes from the external microphone.
4. Select "System Default", change the OS default, start Voice, and confirm Warp follows the OS default.
5. Disconnect the selected external microphone, confirm Settings shows it as unavailable, start Voice, and confirm Warp shows the selected-device-unavailable toast without recording from another microphone.
6. Reconnect the microphone and confirm the saved selection becomes usable again.
7. Confirm Voice Input disabled state, activation-key behavior, quota-limit behavior, microphone permission denied behavior, and transcription insertion still match the prior implementation.
8. Validate keyboard navigation and screen-reader-visible labels for the new dropdown.

Recommended commands before review:

- `cargo fmt`
- A focused test command for the modified crates once tests exist, for example `cargo nextest run -p voice_input -p warp_app --no-fail-fast`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` or `./script/presubmit` if runtime permits

## Parallelization
Do not split the initial implementation across child agents until the persisted setting type and `voice_input` device-resolution API are agreed on. The UI, settings, and recording path are tightly coupled through that contract, and the most important validation requires real macOS audio hardware.

After the contract is defined, limited parallel work is possible:

- `audio-backend` could own `crates/voice_input/src/lib.rs`, platform-specific stable IDs, and resolver tests in a local worktree such as `/workspace/warp-worktrees/GH10931-audio` on branch `oz-agent/GH10931-audio`.
- `settings-ui` could own `app/src/settings/ai.rs`, `app/src/settings_view/ai_page.rs`, and Settings tests in `/workspace/warp-worktrees/GH10931-settings-ui` on branch `oz-agent/GH10931-settings-ui`.
- Merge strategy would be a single combined implementation PR after the audio API lands, because `settings-ui` depends on the exact selection type and resolver error type.

For a first pass, a single implementer is preferable to avoid churn across shared types and because manual macOS validation is the gating step.

## Risks and mitigations

### Risk: unstable or duplicate device identity
`cpal` reliably exposes device names, but names are not enough to distinguish duplicate devices or preserve a selected physical device across default changes.

Mitigation: use a stable backend identifier for persisted `Device { id, name }`, with macOS CoreAudio UID as the first required implementation target. Keep `name` only as display fallback. Add duplicate-name tests for the dropdown presentation.

### Risk: silently using the wrong microphone
Falling back to the system default when a selected device is missing would violate the user's explicit choice and could record lower-quality or unintended audio.

Mitigation: return `SelectedDeviceUnavailable`, show a targeted toast, and require the user to choose "System Default" or another device before recording.

### Risk: device list staleness
Operating systems can add, remove, or rename devices while Settings is open.

Mitigation: refresh the list at practical UI boundaries such as view construction, settings refresh, and dropdown open if supported. Persist unavailable selected devices so stale enumeration does not erase user choice.

### Risk: permission and enumeration differ by platform
Some backends may restrict device names or device availability before microphone permission is granted.

Mitigation: keep permission errors on the recording path, preserve the existing access-denied toast, and make enumeration failures degrade to a disabled/empty dropdown state rather than crashing Settings.

### Risk: broad platform scope
The issue is macOS-specific, but `voice_input` is a desktop crate and must continue compiling on Windows and Linux.

Mitigation: implement stable IDs with `cfg(target_os = "macos")` first, keep cross-platform fallbacks compile-safe, and avoid exposing platform-specific details in the product UI.

## Follow-ups
- Consider live device-change notifications if users frequently plug microphones while Settings is already open and the current refresh triggers are not enough.
- Consider adding high-level telemetry for unavailable selected-device failures if support needs to understand how often users hit that state.
- Revisit Windows/Linux stable device IDs if product wants the same level of persistence guarantees outside macOS.
