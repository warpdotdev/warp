# PRODUCT.md — Select microphone input device for Voice commands
GitHub issue: https://github.com/warpdotdev/warp/issues/10931
Figma: none provided. The issue is tagged `needs-mocks`, so visual layout details should be confirmed before implementation if a design mock becomes available.

## Summary
Warp should let users choose which microphone device Voice input records from in Settings > Agents > Warp Agent > Voice. The default remains the current system-default behavior, but users with multiple microphones can pin Warp Voice to a specific connected input device without changing the operating system's global audio input.

## Problem
Voice input currently records from the system default microphone. Users with a built-in microphone, external USB microphone, AirPods, headset, or virtual audio input cannot choose the microphone that gives the best quality or matches their workflow unless they change the system-wide default, which affects every other app.

## Goals
- Add a user-visible microphone/input-device picker to the existing Voice settings section.
- Preserve the current behavior for users who do not make an explicit selection.
- Use the selected microphone for every Warp Voice recording path, independent of later system default changes, while that selected device remains available.
- Keep the selection local to the current machine because audio device identities are machine-specific.
- Make unavailable-device and no-device states understandable before the user starts recording.

## Non-goals
- Selecting speakers, output devices, monitoring devices, or transcription providers.
- Per-workspace, per-project, per-pane, or per-profile microphone preferences.
- Changing Voice transcription quotas, the Wispr Flow provider, audio processing, hotkey behavior, or the Voice enablement toggle.
- Automatically changing the operating system's global input device.
- Guaranteeing a selected physical device can be found after the OS reports it as removed, renamed, or replaced with a different hardware identity.

## Behavior
1. In Settings > Agents > Warp Agent > Voice, when Voice Input is enabled, Warp shows a new dropdown labeled "Microphone" next to the existing "Key for Activating Voice Input" control.

2. When Voice Input is disabled, the Voice section preserves its current disabled state: Voice cannot be started, and the microphone picker is hidden or disabled consistently with the existing hotkey control. Disabling Voice Input does not clear the saved microphone selection.

3. The dropdown always includes a "System Default" option as the first option. This is the default value for all existing and new users.

4. When "System Default" is selected, Warp records from the operating system's default input device at the time each Voice recording session starts. If the user later changes the system default, the next Warp Voice session follows the new system default.

5. When the user selects a specific connected microphone, Warp records future Voice sessions from that selected microphone instead of the system default. This remains true even if the operating system's default input device changes.

6. The selected microphone applies to every existing Warp Voice entry point that starts a Voice recording session, including the microphone button and the configured Voice activation key. The feature does not create separate selections for different input fields or windows.

7. Device options show human-readable names reported by the operating system, for example "MacBook Pro Microphone", "External USB Microphone", or "AirPods Pro".

8. If two available input devices have the same display name, Warp disambiguates them in the dropdown with a stable suffix such as "(1)" and "(2)" or equivalent detail. The user must be able to distinguish and reselect the intended device within the current device list.

9. The selected dropdown value updates immediately after the user chooses an option. The next Voice session uses the new selection without requiring an app restart or settings reload.

10. Changing the microphone selection while a Voice session is actively listening does not switch the microphone mid-recording. The active session continues with the device it opened at start, and the new selection applies to the next session.

11. If the selected device is no longer available, Settings keeps showing the saved device as unavailable, for example "External USB Microphone (Unavailable)", rather than silently resetting the user's preference.

12. If the user starts Voice while a specific selected device is unavailable, Warp does not silently fall back to another microphone. The recording does not start, and Warp shows an error toast explaining that the selected microphone is unavailable and can be changed in Settings.

13. If "System Default" is selected and the OS reports no default input device, Warp preserves the existing no-microphone failure behavior and shows an error instead of starting a recording.

14. If no input devices can be enumerated, the dropdown shows "System Default" plus a disabled "No microphones found" state, or an equivalent empty state. The Voice recording path still fails gracefully if there is no usable system default device.

15. If microphone permission is denied or restricted, the device selection remains visible when Settings can enumerate devices, but Voice recording cannot start. Warp continues to show the existing microphone-access error when the user tries to record.

16. If the device list changes while Settings is open, Warp refreshes the dropdown by the next time the user opens the dropdown, returns to the Voice settings section, or otherwise triggers a settings refresh. A full app restart must not be required to see a newly connected microphone.

17. If a previously unavailable selected device becomes available again with the same stable device identity, Warp automatically treats it as selected again for future Voice sessions.

18. The microphone selection is stored locally on the device and is not synced through cloud settings. A user's laptop and desktop can have different selected microphones.

19. Settings search finds the Voice section for relevant microphone terms such as "microphone", "mic", "input device", "audio input", and existing Voice search terms.

20. Keyboard and accessibility behavior matches existing Settings dropdowns: the microphone dropdown is reachable by keyboard, announces its label and selected value to assistive technologies, and supports the same focus, open, navigate, select, and escape behaviors as the hotkey dropdown.

21. The new picker uses the active Warp theme and existing Settings layout patterns. It must not introduce hard-coded colors or nonstandard dropdown behavior.

22. The Voice description, enable toggle, Wispr Flow link, hotkey dropdown, first-time Voice toast, and transcription result insertion behavior are unchanged except for the recording device used.

23. If the selected device fails while opening or streaming, Warp reports a Voice recording error and does not submit empty or partial audio for transcription as if recording succeeded.

24. The selected microphone is not exposed in telemetry or logs as a raw stable device identifier. If telemetry is added for the setting, it should record only high-level state such as "system default" versus "specific device selected".

## Success criteria
1. A user with multiple connected microphones can open Settings > Agents > Warp Agent > Voice and select a specific microphone.
2. After selecting a specific microphone, changing the OS default input device does not change which microphone Warp Voice records from.
3. Selecting "System Default" preserves the current behavior and follows future OS default changes.
4. Disconnecting the selected microphone does not silently record from a different microphone.
5. Reconnecting the same selected microphone makes future Voice sessions use it again without reselecting it.
6. The selected microphone persists across Warp restarts on the same machine but does not sync to other machines.
7. Existing Voice enablement, activation-key, quota, permission, and transcription behaviors remain unchanged except for the chosen recording device.

## Validation
- Manually validate on macOS with at least two input devices: select an external microphone, change the OS default to the built-in microphone, record in Warp, and confirm audio comes from the external microphone.
- Manually validate the "System Default" option by switching the OS default and confirming subsequent Warp recordings follow the new default.
- Manually validate disconnect and reconnect behavior for the selected device.
- Manually validate the Settings UI with Voice Input enabled and disabled.
- Manually validate keyboard navigation and screen-reader-visible labels for the new dropdown.
- Add automated coverage for settings persistence, local-only sync behavior, selected-device resolution, unavailable-device fallback prevention, and Settings action handling.

## Open questions
- Should the final label be "Microphone" or "Input device"? The issue uses both. This spec uses "Microphone" because it is shorter and user-facing, but design should confirm the exact label.
- Should unavailable selected devices appear in the dropdown only as the selected value, or also as an item in the open menu? The product requirement is that the selection remains visible and recoverable; the exact dropdown menu presentation can follow the implementation's dropdown constraints.
