# Settings File Offline Edit Detection

Linear: QUALITY-474
Figma: none provided

## Summary

When a user edits `settings.toml` while Warp is closed (or changes a setting via the UI while offline), cloud sync currently overwrites their local changes on next startup. This feature detects when the local settings file has diverged from the last-known cloud-synced state and preserves the user's local changes instead of silently replacing them.

## Problem

Warp's cloud settings sync uses a "cloud wins" strategy on startup: for every synced setting that exists in the cloud, the cloud value overwrites the local value. This is correct when the user hasn't made local changes, but breaks down in two scenarios:

1. **Offline file edit**: The user edits `settings.toml` while Warp is closed. On next startup, cloud sync overwrites their edit with the (now stale) cloud value.
2. **Offline UI change**: The user changes a setting via the Warp UI while their internet is offline. The change is written to the file but never uploaded to cloud. On next startup (now online), cloud sync overwrites the local change.

In both cases the user's most recent intentional change is silently lost.

## Goals

- Detect when the settings file has local changes that cloud sync doesn't know about.
- Preserve local changes by syncing them *to* cloud instead of accepting cloud values *from* cloud.
- Maintain existing behavior (cloud wins) when there are no local changes.
- Handle the broken-file case correctly: when the TOML file can't be parsed, allow cloud to restore settings in memory without overwriting the broken file on disk.
- Handle the missing-file case correctly: when the file is absent or empty, treat it as "no local state" and let cloud restore normally rather than wiping cloud with local defaults.

## Non-goals

- Per-key conflict resolution or merge UI. This feature uses a blanket "local wins" or "cloud wins" decision, not per-setting granularity.
- Prompting the user to choose between local and cloud. The detection is automatic and silent.
- Handling conflicts between two devices that both made offline changes. The last device to come online wins (its local changes overwrite cloud, which then syncs to other devices).
- Changes to the settings sync protocol or server-side logic.

## User Experience

### Normal operation (no offline edits)

The user's experience is unchanged. Cloud sync works as it does today: cloud values are applied to local on startup, and local changes made while online are uploaded to cloud.

### Offline file edit

1. User closes Warp.
2. User edits `settings.toml` in a text editor.
3. User opens Warp.
4. Warp detects the file has changed since the last cloud sync.
5. Warp treats the local file as authoritative: local values are uploaded to cloud, cloud values do not overwrite local.
6. The user's edit is preserved and synced to other devices.

### Offline UI change

1. User has Warp open but is offline.
2. User changes a setting via the UI. The change is saved to the file.
3. User closes Warp (still offline — the change was never uploaded).
4. User opens Warp (now online).
5. Warp detects the file has changed since the last cloud sync.
6. Local wins — the user's change is preserved and uploaded to cloud.

### Broken settings file

1. User introduces a syntax error in `settings.toml`.
2. User opens Warp.
3. Warp cannot parse the file. Cloud sync is allowed to restore settings in memory so the app functions.
4. Flush suppression (already implemented) prevents the broken file from being overwritten on disk.
5. The settings error banner is shown so the user knows their file has errors.
6. The user fixes the file. Hot-reload picks up the fix and normal operation resumes.

### Missing or empty settings file

1. User deletes `settings.toml` (or empties its contents) to reset their settings.
2. User opens Warp.
3. Warp detects the file is missing/empty and treats it as "no local state".
4. Cloud sync restores settings from cloud into memory and recreates the file with those values.
5. The user's cloud settings are preserved on other devices — local defaults do not overwrite cloud.

### First launch / fresh install

No stored hash exists. Warp treats this as "no local changes" and cloud wins, which is the same as today's behavior.

## Success Criteria

1. When a user edits `settings.toml` while Warp is closed and reopens Warp, their edits are preserved and synced to cloud.
2. When a user changes a setting via the UI while offline, closes Warp, and reopens while online, their change is preserved and synced to cloud.
3. When no local changes have been made (file matches last-known cloud-synced state), cloud sync behavior is unchanged from today.
4. When the settings file is broken (unparsable TOML), cloud sync restores settings in memory. The broken file is not overwritten on disk. The settings error banner is shown.
5. When the settings file is missing or empty on startup, cloud wins — local defaults do not overwrite cloud values, and the file is recreated from cloud.
6. On first launch with no stored hash, behavior matches today (cloud wins).
7. The hash is updated after every successful cloud sync reconciliation (initial load and local→cloud uploads), so that subsequent startups correctly detect whether new local changes have occurred.

## Validation

- **Unit tests**: Verify hash computation produces consistent results for identical content and different results for different content.
- **Integration tests**:
  - Startup with file matching stored hash → cloud wins (existing behavior preserved).
  - Startup with file differing from stored hash → local wins.
  - Startup with broken file → cloud restores in memory, file untouched.
  - Startup with missing or empty file (with a stored hash present) → cloud wins, file recreated from cloud values.
  - First launch with no stored hash → cloud wins.
- **Manual testing**: Edit `settings.toml` while Warp is closed, reopen, verify the edit persists and syncs to another device.

## Open Questions

- Should we log or surface any indication to the user when local-wins is triggered? Currently it's silent. A log message at minimum seems warranted.
