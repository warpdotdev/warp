# Separate Preview Config Directory on macOS

## Summary

On macOS, give the Preview channel its own config directory (`~/.warp-preview`) instead of sharing `~/.warp` with Stable. On first launch, migrate existing config by symlinking each top-level entry from `~/.warp` into `~/.warp-preview`, so existing Preview users keep all their customizations.

## Problem

Stable and Preview currently share the `~/.warp` directory on macOS. This means:

- Settings changes made in one channel affect the other.
- Preview experiments (themes, keybindings, tab configs, etc.) can break the Stable experience.
- There is no safe way to test config-level changes on Preview without risking Stable user data.

Other channels (Dev, Integration, Local) already use separate directories (`.warp-dev`, `.warp-integration`, `.warp-local`). Preview is the only non-Stable channel that still shares with Stable.

## Goals

1. Preview uses `~/.warp-preview` as its config directory on macOS.
2. Existing Preview users retain all their customizations after the change.
3. Stable users see no change whatsoever.
4. The migration is automatic and requires no user action.

## Non-goals

- Changing behavior on Linux or Windows (these platforms already use separate directories via `ProjectDirs`).
- Migrating the SQLite database (already stored separately in `~/Library/Application Support/` or the App Group container).
- Changing Dev, Integration, or Local channel directories.
- Achieving full bidirectional sync between Stable and Preview configs going forward.

## Figma

Figma: none needed (no UI changes).

## User Experience

### First launch after upgrade (Preview channel)

When a Preview user launches Warp for the first time after this change:

1. Warp detects that `~/.warp-preview` does not exist.
2. Warp creates `~/.warp-preview`.
3. For each top-level entry in `~/.warp` (files and directories), Warp creates a symbolic link in `~/.warp-preview` pointing to the corresponding entry in `~/.warp`.
4. Preview proceeds using `~/.warp-preview` as its config directory.

After migration, the directory looks like:

```
~/.warp-preview/
  keybindings.yaml  -> ~/.warp/keybindings.yaml
  themes/           -> ~/.warp/themes/
  workflows/        -> ~/.warp/workflows/
  tab_configs/      -> ~/.warp/tab_configs/
  ...etc
```

### Subsequent launches (Preview)

No migration occurs. Preview reads and writes from `~/.warp-preview`.

### Stable users

Zero change. Stable continues to use `~/.warp`.

### settings.toml is not symlinked

The `settings.toml` file (used by the Settings File feature) is intentionally kept separate between Stable and Preview. The migration uses an explicit exclude list (`MIGRATION_EXCLUDED_FILES`) to skip `settings.toml` during symlinking. This handles the case where a user runs Stable first (which creates `settings.toml` in `~/.warp`) and later updates Preview — without the filter, the migration would symlink the file, causing both channels to share settings.

### Behavior after migration

- **Reading config**: Preview reads through symlinks, so existing Stable config is visible.
- **Modifying a shared file**: If Preview modifies a symlinked file (e.g., `keybindings.yaml`), the change affects Stable too, since the symlink points to the same file. This is the intended tradeoff — users who customize on Preview are likely the same user customizing on Stable.
- **Creating new entries in a symlinked directory**: If Preview creates a new file inside a symlinked directory (e.g., a new tab config in `tab_configs/`), the file is created in `~/.warp/tab_configs/` because the directory symlink resolves there. This is an acceptable limitation of the symlink approach.
- **Breaking the symlink**: If the user wants full independence for a specific config, they can delete the symlink in `~/.warp-preview/` and replace it with a real file or directory. Preview will use the local copy from that point on.

### Edge cases

- **`~/.warp` does not exist**: No migration needed. Preview creates `~/.warp-preview` fresh, and Warp's existing directory-creation logic populates it.
- **`~/.warp-preview` already exists**: Migration is skipped entirely. This handles the case where the user has manually created the directory or is launching for a second time.
- **Entries in `~/.warp` are themselves symlinks** (e.g., `keybindings.yaml -> ~/dotfiles/...`): The migration creates a symlink to the symlink. The chain resolves correctly. No special handling needed.
- **`.DS_Store` and hidden OS files**: These should be skipped during migration to avoid unnecessary clutter.
- **Simultaneous Stable and Preview launch**: The directory creation (`create_dir`) fails atomically if the directory already exists, so concurrent launches are safe. The migration will run in one process and be skipped in the other.
- **Permissions errors**: If symlink creation fails for a specific entry, log a warning and continue with the remaining entries. Partial migration is better than no migration.

## Alternatives Considered

### 1. Full copy instead of symlinks
Copy all files and directories from `~/.warp` to `~/.warp-preview`.

- Pro: Full independence from day one.
- Con: Doubles disk usage for themes, workflows, etc. Changes in Stable no longer propagate to Preview, which could confuse users who expect their config to be shared.

### 2. Symlink the entire directory
Create `~/.warp-preview` as a symlink to `~/.warp`.

- Pro: Simplest implementation.
- Con: Not actually a separate directory. Any new entries created by Preview (e.g., new directories Warp adds in future versions) would appear in `~/.warp`. Defeats the purpose of separation.

### 3. No migration (clean start)
Simply start using `~/.warp-preview` with no migration.

- Pro: Simplest. No migration code.
- Con: Users lose all custom keybindings, themes, workflows, launch configs, tab configs, and settings. This is a bad experience.

### 4. Deep symlink (per-file in each subdirectory)
For each directory like `themes/`, create the directory in `.warp-preview` and symlink each individual file.

- Pro: New files created inside directories stay in `.warp-preview` rather than leaking into `.warp`.
- Con: Significantly more complex. The improvement is marginal — most users don't create new config files frequently, and those who do are likely sophisticated enough to understand the symlink behavior.

**Decision**: Symlink top-level contents (option in the main proposal) is the best balance of simplicity, backwards compatibility, and user experience.

## Success Criteria

1. After upgrading, a Preview user's keybindings, themes, workflows, launch configs, tab configs, MCP config, and settings are all accessible in Preview without manual action.
2. A Stable user's `~/.warp` directory is unchanged after the upgrade.
3. `~/.warp-preview` is created on first launch of Preview and contains symlinks to each top-level entry in `~/.warp`.
4. If `~/.warp` does not exist, `~/.warp-preview` is created empty (normal startup logic applies).
5. If `~/.warp-preview` already exists, no migration runs.
6. The migration only runs on macOS.
7. Partial migration failures (e.g., one symlink fails) do not prevent Warp from launching.

## Validation

- **Manual testing on macOS Preview build**:
  - Fresh install with no `~/.warp`: Preview creates `~/.warp-preview`, normal setup.
  - Existing `~/.warp` with custom keybindings/themes: After upgrade, verify symlinks exist and config loads correctly.
  - Launch Stable after Preview migration: Stable still uses `~/.warp` and is unaffected.
- **Integration test**: Verify the migration function creates expected symlinks given a mock `~/.warp` directory.
