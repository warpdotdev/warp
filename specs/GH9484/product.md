# Custom and Per-Event Notification Sounds: Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/9484
Figma: none provided

## Summary
Let users choose the sound Warp plays for each kind of desktop notification (agent task completed, long-running command finished, needs attention, and other terminal notifications) instead of always playing the macOS default notification sound. Each event can use the system default, no sound, a built-in or user-installed macOS sound, or a custom audio file. Today's single "Play notification sounds" toggle remains the master switch, and users who change nothing hear exactly what they hear today.

## Problem
Warp posts desktop notifications with the macOS default notification sound. That sound is fixed: it does not follow the Alert sound chosen in System Settings > Sound, and macOS offers no per-app override. Users cannot tell an agent finishing apart from a long command finishing without looking, and the default sound is easy to miss over music or calls. Warp's own settings expose only an on/off toggle.

## Goals
- Per-event sound selection for the notification types Warp already distinguishes.
- Choose from system sounds, sounds the user has installed in `~/Library/Sounds`, or any audio file on disk.
- Preview a sound from the picker before committing to it.
- Zero behavior change for users who never open the picker.
- Keep the system's notification controls (Focus, Do Not Disturb, the per-app "Play sound for notifications" switch) in charge of whether a sound plays.

## Non-goals
- Custom sounds on Linux and Windows in this iteration. The existing sound toggle is already macOS-only; see Follow-ups in the tech spec.
- Changing the terminal bell (`use_audible_bell`). The bell is not a notification and uses a separate system path. It can reuse this picker in a follow-up.
- Lowering ("ducking") other apps' audio while a notification sound plays. macOS has no public API to set another process's volume; doing it would require capturing and replaying every app's audio through Core Audio process taps, which is invasive and out of proportion for this feature.
- Per-sound volume control in this iteration. System-delivered notification sounds cannot carry a volume (only Critical Alerts can, and those require a special Apple entitlement). See Open questions.
- Changing when notifications fire (thresholds, focus rules, which events exist).

## Behavior

### Defaults and backward compatibility
1. A user who has never changed a sound setting hears the macOS default notification sound for every notification, exactly as today. Existing settings files, synced settings, and the existing "Play notification sounds" toggle keep working with no migration step and no prompt.
2. Every event's sound choice defaults to **Default** (the macOS default notification sound).
3. Upgrading Warp never changes which sound plays, and downgrading to a build without this feature falls back to today's behavior without errors or lost notification settings.

### Settings surface
4. In Settings > Features > Notifications, when desktop notifications are enabled and "Play notification sounds" is on, each notification toggle gains a sound picker for its event:
   - "Notify when an agent completes a task" > **Agent task completed** sound.
   - Long-running command notification > **Long-running command** sound.
   - "Notify when a command or agent needs your attention to continue" > **Needs attention** sound. This covers blocked agents, failed agent tasks, and password prompts, which already share this toggle.
   - An additional row, **Other notifications**, covers notifications that programs in the terminal send with OSC 9 or OSC 777.
5. When "Play notification sounds" is off, the pickers are hidden or disabled, the stored choices are preserved, and no notification plays a sound. Turning the toggle back on restores the previous choices.
6. When desktop notifications are disabled, no picker is shown and no sound plays, regardless of stored choices.
7. Each picker offers, in order:
   - **Default** (macOS default notification sound).
   - **None** (the notification is shown silently).
   - The sounds in `/System/Library/Sounds` (Basso, Blow, Bottle, Frog, Funk, Glass, Hero, Morse, Ping, Pop, Purr, Sosumi, Submarine, Tink, and any others present), listed by display name.
   - Sounds found in `~/Library/Sounds`, listed by display name in a separate section, if any exist.
   - **Choose file...**, which opens a file picker filtered to audio files.
8. The picker shows the current choice by name. For a custom file it shows the file name, with the full path in a tooltip.
9. Each picker has a preview (play) button that plays the currently selected sound once, immediately, on this Mac. Previewing never changes the stored setting. The preview button is hidden for **None** and for **Default** (there is no public macOS API for playing the default notification sound outside a notification).
10. Starting a preview while another preview is playing stops the first one. Closing Settings stops any preview in progress.
11. Selecting any sound other than **Default** or **None** plays it once as confirmation, the way choosing an Alert sound in macOS System Settings does.

### Custom files
12. **Choose file...** accepts common audio files (at least AIFF, WAV, CAF, MP3, and M4A). After a file is chosen, Warp validates it before saving the choice:
    - If the file cannot be decoded as audio, the picker shows an inline error ("This file isn't a playable audio file") and the previous choice is kept.
    - If the file is 30 seconds or longer, the picker shows an inline error ("Notification sounds must be shorter than 30 seconds") and the previous choice is kept. macOS silently substitutes the default sound for longer files, so Warp rejects them up front.
    - Otherwise the choice is saved and the sound plays once as confirmation (invariant 11).
13. After a custom file is accepted, the notification keeps playing that sound, as it was when chosen, even if the original file is later moved, edited, or deleted. Warp keeps its own copy of the audio it needs.
14. If Warp cannot play a stored custom sound when a notification fires (for example its copy is missing and the original is gone), the notification still appears and plays the **Default** sound. The notification is never dropped or silenced because of a sound problem. The picker for that event then shows a warning ("Sound file not found, using Default") until the user picks a new sound.
15. If a sound in `~/Library/Sounds` that the user selected is later deleted, behavior matches invariant 14.

### When sounds play
16. A sound plays only when Warp actually posts a desktop notification. The rules for when notifications are posted (only when Warp's window is not the active window, per-event toggles, the long-running threshold) do not change.
17. The sound that plays is the one selected for the notification's event (invariant 4). Each notification plays the sound for its own event, never the sound of an earlier notification.
18. Structured CLI agent notifications (for example from the Claude Code Warp plugin) use the **Agent task completed** sound when the agent finishes successfully, and the **Needs attention** sound when it is blocked or fails, matching the toggle that already gates each one.
19. Focus and Do Not Disturb: whether a custom sound plays follows exactly the same system rules as the default sound today. If a Focus mode silences Warp's notifications, no custom sound plays either.
20. The per-app **Play sound for notifications** switch in System Settings > Notifications > Warp silences custom sounds exactly as it silences the default sound today.
21. A custom sound plays at the same volume the system uses for the default notification sound today. Warp does not change any system or per-app volume.
22. Choosing a sound never delays the notification banner. If a sound cannot be prepared in time, the banner is posted with the Default sound (invariant 14).

### Sync and multiple machines
23. Sound choices are stored in the settings file under the notifications section and are human-editable. Invalid values in the file (unknown sound name, malformed entry) are treated as **Default** for that event and do not reset any other notification setting.
24. Sound choices sync between the user's Macs through settings sync when sync is enabled. They are not applied on Linux or Windows.
25. On a Mac where a synced custom file does not exist, that event plays **Default** and its picker shows the warning from invariant 14. Other Macs are unaffected.

### Accessibility
26. Every picker and preview button is reachable and operable by keyboard, has an accessible label naming its event (for example "Agent task completed sound", "Preview agent task completed sound"), and announces the selected sound name to screen readers.
27. Sound is never the only signal: the banner, and the in-app notification where enabled, still appear with the same content whether the sound is Default, a custom sound, or None.

## Open questions
- **Volume and loudness.** The motivating complaint is that the default sound gets lost under music. Choosing a louder or more distinctive file solves this within the system's alert volume. Should a later iteration add an opt-in "Play sounds through Warp" mode that allows a per-sound volume, at the cost of Warp (not the system) deciding whether to honor Focus? See the tech spec's playback tradeoff.
- **Sync scope.** Should custom-file choices sync at all, given that the file may not exist on the user's other Macs (invariant 25), or should only system and `~/Library/Sounds` choices sync?
- **Terminal bell.** Should the audible bell reuse this picker in a follow-up, so users can replace the Alert sound it uses today?
