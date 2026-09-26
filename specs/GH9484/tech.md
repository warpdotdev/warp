# Custom and Per-Event Notification Sounds: Tech Spec
Product spec: `specs/GH9484/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/9484

## Context
Warp has one notification sound, the macOS default, gated by one boolean. The event that caused a notification is known where the notification is created but is dropped before it reaches the platform layer, so nothing downstream can pick a sound per event. All references below are pinned to `e865a74`.

Settings:
- [`app/src/terminal/session_settings.rs (56-109) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/session_settings.rs#L56-L109): `NotificationsSettings`, a `#[serde(default)]` struct holding `mode`, the per-event toggles, and `play_notification_sound: bool`. `#[serde(default)]` is what makes adding fields backward compatible (see the comment at L57-L66).
- [`app/src/terminal/session_settings.rs (344-354) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/session_settings.rs#L344-L354): registers it as `notifications`, `SupportedPlatforms::ALL`, `SyncToCloud::Globally`, TOML path `notifications.preferences`, `max_table_depth: 1`.
- [`crates/settings/src/lib.rs (157-166) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/settings/src/lib.rs#L157-L166): `SyncToCloud::{Globally, PerPlatform, Never}`. [`app/src/settings/app_icon.rs (124-147) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/settings/app_icon.rs#L124-L147) is the precedent for a macOS-only setting.
- [`crates/settings_value_derive/src/lib.rs (263-320) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/settings_value_derive/src/lib.rs#L263-L320): the `SettingsValue` derive falls back to `Default` for absent fields when the struct has `#[serde(default)]`.
- [`app/src/settings/init_tests.rs (420-458) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/settings/init_tests.rs#L420-L458): existing file-format tests for `NotificationsSettings`.

Settings UI:
- [`app/src/settings_view/features_page.rs (5269-5313) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/settings_view/features_page.rs#L5269-L5313): the notifications group. The "Play notification sounds" toggle is `#[cfg(target_os = "macos")]` (L5299).
- [`app/src/settings_view/features_page.rs (1841-1855) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/settings_view/features_page.rs#L1841-L1855): `ToggleNotificationSound` handler. L389-L400 registers its command palette toggle.
- [`crates/warpui_core/src/platform/file_picker.rs (22-36) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui_core/src/platform/file_picker.rs#L22-L36): `FileType` has `Image`, `Yaml`, `Markdown`; there is no audio type yet.

Where notifications are created (all in `app/src/terminal/view.rs @ e865a74`):
- [`L818-L831`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L818-L831): `BlockNotification { title, body }` and `NotificationsTrigger { LongRunningCommand, AgentTaskCompleted(bool), NeedsAttention, PasswordPrompt }`. `create_notification_content` (L865) turns a trigger into a `BlockNotification` and discards the trigger.
- [`L16505-L16545`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L16505-L16545): long-running command.
- [`L16591-L16625`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L16591-L16625): `send_agent_desktop_notification_or_show_banner`, shared by Warp's agent and CLI agents. `AgentTaskCompleted(true)` is gated by `is_agent_task_completed_enabled`; failures and `NeedsAttention` by `is_needs_attention_enabled` (L16615).
- [`L14133-L14167`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L14133-L14167): structured CLI agent events (the OSC 777 `warp://cli-agent` path used by the Claude Code plugin) map Blocked, Failed, and Success to the triggers above.
- [`L22183-L22210`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L22183-L22210): password prompts are sent as `NeedsAttention`. `PasswordPrompt` is a legacy variant that is no longer emitted.
- [`L13456-L13497`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L13456-L13497): plain OSC 9 and OSC 777 notifications from terminal programs, with no trigger at all.
- [`L22162-L22165`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/view.rs#L22162-L22165): `is_navigated_away_from_window`, which gates every desktop notification.

Where the sound is chosen:
- [`app/src/workspace/view.rs (16278-16305) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/workspace/view.rs#L16278-L16305): the only place that reads `play_notification_sound`. It builds `UserNotification::new_with_sound(title, body, data, play_sound)`.
- [`crates/warpui_core/src/notification.rs (10-70) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui_core/src/notification.rs#L10-L70): `UserNotification { title, body, data, play_sound: bool }`.
- [`crates/warpui/src/platform/mac/delegate.rs (327-351) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui/src/platform/mac/delegate.rs#L327-L351) and the FFI declaration at L34-L41 pass a `BOOL play_sound` to Objective-C.
- [`crates/warpui/src/platform/mac/objc/notifications/notifications.m (46-90) @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui/src/platform/mac/objc/notifications/notifications.m#L46-L90): builds a `UNMutableNotificationContent` and sets `content.sound = [UNNotificationSound defaultSound]` when `playSound` is true (L63-L65).
- Linux ([`crates/warpui/src/windowing/winit/notifications/linux.rs (9-24)`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui/src/windowing/winit/notifications/linux.rs#L9-L24), `notify-rust` 4.11) and Windows ([`windows.rs (9-35)`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui/src/windowing/winit/notifications/windows.rs#L9-L35), `tauri-winrt-notification` 0.7) ignore `play_sound` entirely.

Related but separate: the terminal bell ([`app/src/terminal/audible_bell/macos.rs @ e865a74`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/app/src/terminal/audible_bell/macos.rs)) calls `NSBeep()`, which plays the user's Alert sound. It is out of scope per the product spec.

Platform facts this design depends on:
- `UNNotificationSound` custom sounds are searched only in the `Library/Sounds` directory of the app's container, the `Library/Sounds` directory of one of the app's shared group containers, and the main bundle. Files must be Linear PCM, IMA4, µLaw, or aLaw in an AIFF, WAV, or CAF container, and shorter than 30 seconds; longer files play the default sound instead ([UNNotificationSound](https://developer.apple.com/documentation/usernotifications/unnotificationsound), [init(named:)](https://developer.apple.com/documentation/usernotifications/unnotificationsound/init(named:))).
- Warp is not sandboxed and already has the application group `2BBY89MBSN.dev.warp` ([`script/Entitlements.plist`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/script/Entitlements.plist)), so `~/Library/Group Containers/2BBY89MBSN.dev.warp/Library/Sounds` is a documented search location Warp can write to.
- Only Critical Alert sounds accept a volume, and those need an Apple-issued entitlement ([defaultCriticalSound(withAudioVolume:)](https://developer.apple.com/documentation/usernotifications/unnotificationsound/defaultcriticalsound(withaudiovolume:))).
- There are developer reports of custom `UNNotificationSound` files not playing on macOS 12 (FB11642483, [forum thread](https://developer.apple.com/forums/thread/716650)). This is the main technical risk and is handled below.

## Proposed changes

### 1. A new macOS-only setting for sound choices
Add a setting to the `SessionSettings` group next to `notifications` instead of adding fields to `NotificationsSettings`:

```rust
notification_sounds: NotificationSounds {
    type: NotificationSoundsSettings,
    default: NotificationSoundsSettings::default(),
    supported_platforms: SupportedPlatforms::MAC,
    sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "notifications.sounds",
    max_table_depth: 0,
    description: "Sound played for each kind of desktop notification.",
}
```

```rust
#[serde(default)]
pub struct NotificationSoundsSettings {
    pub agent_task_completed: NotificationSoundChoice,
    pub long_running_command: NotificationSoundChoice,
    pub needs_attention: NotificationSoundChoice,
    pub other: NotificationSoundChoice,
}

pub enum NotificationSoundChoice {
    Default,        // [UNNotificationSound defaultSound]
    NoSound,        // the notification is posted silently
    System(String), // a sound name from /System/Library/Sounds, e.g. "Glass"
    User(String),   // a sound name from ~/Library/Sounds
    File(String),   // any file path the user picked
}
```

Why a separate setting rather than new fields on `NotificationsSettings`: that struct is `SupportedPlatforms::ALL` and synced globally, so a macOS file path would sync to Linux and Windows. A separate `PerPlatform` setting keeps choices on Macs only (product invariants 24 and 25) and leaves `NotificationsSettings` and its existing tests untouched. `play_notification_sound` stays the master switch (invariant 5). Because every field defaults to `Default`, no migration is needed (invariants 1 to 3). An unknown or malformed value decodes as `Default` for that field only (invariant 23); `SettingsValue` gets a hand-written `from_file_value` for `NotificationSoundChoice` so a bad entry does not fail the whole struct. TOML shape:

```toml
[notifications.sounds]
agent_task_completed = { system = "Glass" }
long_running_command = "default"
needs_attention = { file = "~/Sounds/alarm.m4a" }
other = "no_sound"
```

### 2. Carry the event from creation to the platform layer
- Add `sound_event: NotificationSoundEvent` to `BlockNotification` (`view.rs` L818). `NotificationSoundEvent` is `{ AgentTaskCompleted, LongRunningCommand, NeedsAttention, Other }`.
- `NotificationsTrigger::create_notification_content` sets it with an exhaustive match: `AgentTaskCompleted(true)` to `AgentTaskCompleted`; `AgentTaskCompleted(false)`, `NeedsAttention`, and the legacy `PasswordPrompt` to `NeedsAttention`; `LongRunningCommand` to `LongRunningCommand`. This mirrors the toggle that gates each trigger at L16615 (invariant 18).
- The OSC 9 and OSC 777 path (L13476-L13484) sets `Other`.
- In `workspace/view.rs` L16293, replace the `play_sound` read with a resolver: if `play_notification_sound` is false, `NotificationSound::None`; otherwise look up `NotificationSoundsSettings` for the event and ask the staging module (section 3) for a playable name, falling back to `Default`.

### 3. Stage sound files where macOS will find them
New module `app/src/terminal/notification_sounds/` (`mod.rs`, `macos.rs`, `noop.rs`, following the `audible_bell` layout):
- `stage(choice) -> Result<StagedSound>` converts the source (a `/System/Library/Sounds` file, a `~/Library/Sounds` file, or a user path) into a CAF file with Linear PCM audio using AudioToolbox `ExtAudioFile`. It writes the file to `<group container>/Library/Sounds/warp-<sha256 of source bytes, first 16 hex>.caf`, where the group container comes from `NSFileManager containerURLForSecurityApplicationGroupIdentifier:`. Content-addressed names make staging idempotent and make edits to the source produce a new file.
- `stage` rejects files that cannot be decoded and files 30 seconds or longer, with typed errors the picker shows inline (invariant 12). Converting on import is what lets users pick MP3 and M4A even though notification sounds must be PCM-family audio.
- Staging runs on a background executor when a choice is saved, never on the notification path (invariant 22). The resolver only checks that the staged file exists: if it does, it returns `Named(file_name)`; if not, it returns `Default`, schedules a re-stage from the source, and marks the choice as broken so the picker shows the warning (invariants 13 to 15, 25).
- Warp keeps the staged copy, not the original file, so moving or deleting the original after import does not break the sound (invariant 13). Staged files no longer referenced by any choice are deleted after each settings change.

Tradeoff: staging `System` sounds too, instead of passing their names directly, costs one small copy each. It keeps every non-default sound on the documented search path, so the design does not rely on undocumented lookups in `/System/Library/Sounds`.

### 4. Platform notification API
- `crates/warpui_core/src/notification.rs`: replace `play_sound: bool` with `sound: NotificationSound { Default, None, Named(String) }`. Keep `new_with_sound(.., play_sound: bool)` as a thin wrapper that maps to `Default` or `None`, so the other caller (`app/src/uri/mod.rs` L704, via `UserNotification::new`) is unchanged.
- `mac/delegate.rs`: pass `sound_name: id` (nil for `Default`) alongside `play_sound`.
- `notifications.m`: `content.sound = soundName ? [UNNotificationSound soundNamed:soundName] : [UNNotificationSound defaultSound];` when `playSound` is set. Posting through `UNUserNotificationCenter` is what keeps Focus, Do Not Disturb, and the per-app sound switch in charge (invariants 19 to 21), with no new permission prompts.
- Linux and Windows accept the new field and keep ignoring it.

### 5. Settings UI
- In the notifications group (`features_page.rs` L5269-L5313), when notifications are enabled and `play_notification_sound` is on, render a sound dropdown and a preview button next to each event row, plus an "Other notifications" row (invariants 4 to 8). Reuse `Dropdown` as the other features-page dropdowns do (for example around L2414).
- The dropdown lists Default, None, `/System/Library/Sounds` entries, `~/Library/Sounds` entries (both read when the page opens), and "Choose file...". Add `FileType::Audio` (`aiff`, `aif`, `wav`, `caf`, `mp3`, `m4a`) to `file_picker.rs` and open the picker with it.
- Preview and confirm-on-select (invariants 9 to 11) play the staged file through `NSSound`, which is fine for a user-initiated action inside Settings. A single `NSSound` instance per settings view gives the stop-previous behavior of invariant 10. macOS has no public API that plays the default notification sound on its own, so **Default** has no preview button (invariant 9).
- Error and warning states (invariants 12, 14, 25) render as inline secondary text under the row.
- Accessibility labels per invariant 26. No command palette entry is added: `AGENTS.md` asks for palette entries for toggleable settings, and these are pickers.
- Gate the new UI and resolver behind `FeatureFlag::CustomNotificationSounds` (dogfood first). With the flag off, every event resolves to `Default` and behavior equals today.

### Alternative considered: play sounds in-process with `NSSound`
Post the notification with `content.sound = nil` and play the chosen file with `NSSound`. Benefits: any Core Audio format and path with no staging, per-sound volume (`NSSound.volume`), and no dependence on the custom-sound behavior reported in FB11642483. Costs: Warp, not the system, must decide whether to play. `UNNotificationSettings.soundSetting` exposes the per-app sound switch, but Focus status is only available through `INFocusStatusCenter` after an extra user authorization prompt, so Focus would be ignored or require a new permission. That breaks invariants 19 and 21 and could make Warp audible during Focus, which is a regression for every user. Recommendation: ship the `UNNotificationSound` design and keep this as the fallback if the prototype below fails, or as a separate opt-in "Play sounds through Warp" mode if the volume open question is accepted.

## Testing and validation

Unit tests (new `_tests.rs` files per `AGENTS.md`):
- `app/src/terminal/session_settings_tests.rs`: `NotificationSoundsSettings::default()` is all `Default`; file-format round trip for every `NotificationSoundChoice` variant; an absent `[notifications.sounds]` table decodes to the default (invariants 1 to 3); one malformed field decodes as `Default` while the others keep their values (invariant 23).
- `app/src/settings/init_tests.rs`: existing `NotificationsSettings` tests keep passing unchanged, proving the master toggle and its storage format did not move.
- `app/src/terminal/view_tests.rs` or a small new module test: the trigger to `NotificationSoundEvent` mapping for every `NotificationsTrigger` variant, including failed agent tasks and the legacy password variant (invariants 4 and 18); OSC notifications map to `Other`.
- Resolver tests with a fake staging store: master toggle off gives `None` (invariant 5); flag off gives `Default`; missing staged file gives `Default` plus a re-stage request and a broken marker (invariants 14, 15, 25).
- `notification_sounds/macos_tests.rs` using a temp directory in place of the group container: identical input produces the same staged name; changed input produces a new name; undecodable input and a 31-second input return the typed errors (invariant 12); unreferenced files are removed.

Prototype gate (before UI work): in a bundled build (`script/user_notifications`, see [`notifications/README.md`](https://github.com/warpdotdev/warp/blob/e865a743785403d86bf93b12471e4ef192c99572/crates/warpui/src/platform/mac/objc/notifications/README.md)), post notifications with staged sounds on the oldest and newest supported macOS versions, with "Play user interface sound effects" both on and off. If staged sounds do not play reliably, switch to the `NSSound` alternative and revise invariants 19 to 21 before continuing.

Manual validation (bundled build, Warp window in the background):
- Fresh profile: every notification plays the default sound (invariants 1 and 2).
- Pick Glass for Agent task completed, Submarine for Needs attention, and an MP3 for Long-running command. Trigger each: Warp agent finishes, Claude Code plugin finishes and blocks, `sleep 35` finishes. Each plays its own sound (invariants 4, 17, 18).
- Pick None: the banner appears silently (invariants 7 and 27).
- Turn off "Play notification sounds": silent; turn on: previous choices return (invariant 5).
- Enable a Focus mode that silences Warp: no sound (invariant 19). Turn off "Play sound for notifications" for Warp in System Settings: no sound (invariant 20).
- Choose a text file and a 45-second file: inline errors, previous choice kept (invariant 12).
- Pick a file, then delete the original: the sound still plays (invariant 13). Delete the staged file too: the default plays and the warning shows (invariant 14).
- Edit `settings.toml` by hand with an unknown sound name: that event plays the default and other settings are intact (invariant 23).
- Sync to a second Mac without the custom file: that event plays the default with the warning (invariant 25).
- VoiceOver: pickers and preview buttons are reachable and announce their labels and values (invariant 26).
- Before and after screenshots of the notifications settings group, plus a narrated screen recording of the manual pass, per `CONTRIBUTING.md`.

## Risks and mitigations
- **Custom `UNNotificationSound` unreliability on macOS (FB11642483).** Mitigated by the prototype gate and the `NSSound` fallback design.
- **Group container path.** `containerURLForSecurityApplicationGroupIdentifier:` returns nil if the entitlement is missing (for example in unsigned local builds). The resolver then returns `Default`, so development builds degrade to today's behavior instead of failing.
- **Disk use.** Staged files are short PCM clips, at most 30 seconds each and at most four referenced at once, and unreferenced files are removed.

## Follow-ups
- Linux: `notify-rust` supports `Hint::SoundFile` and `Hint::SoundName` ([docs](https://docs.rs/notify-rust/4.11.7/notify_rust/enum.Hint.html)); whether they are honored depends on the notification server.
- Windows: `tauri-winrt-notification` exposes only built-in sounds (`Sound::Default`, `IM`, `Mail`, `Reminder`, `SMS`, looping variants). Custom audio in toasts is limited to `ms-appx:///` and `ms-resource` sources, and local file paths are unsupported ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-custom-audio)), so Windows could offer the built-in list only.
- Terminal bell: let `AudibleBell` play a chosen sound instead of `NSBeep`.
- Volume: the opt-in in-process playback mode described above, if the product open question is accepted.
