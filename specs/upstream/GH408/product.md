# Product Spec: Support ~ expansion in /open-file slash command

**Issue:** [warpdotdev/warp-external#408](https://github.com/warpdotdev/warp-external/issues/408)
**Figma:** none provided

## Summary

The `/open-file` slash command should expand `~` to the user's home directory, matching the behavior of the Cmd-O open file dialog. Currently, typing `/open-file ~/foo.txt` incorrectly prepends the working directory, producing a path like `/current/dir/~/foo.txt` instead of `/home/user/foo.txt`.

## Problem

When a user types `/open-file ~/some/file.txt` and presses Enter, the `~` is treated as a literal directory name. The slash command handler joins the raw argument with the current working directory, resulting in a broken path like `/Users/me/project/~/some/file.txt`. This always fails with "File not found" because no literal `~` directory exists.

The Cmd-O (open file palette) already handles `~` correctly by calling `shellexpand::tilde()` before resolving the path. Users who discover `~` works in Cmd-O reasonably expect it to work in `/open-file` as well.

## Goals

- `~` at the start of a path in `/open-file` expands to the user's home directory.
- Behavior is consistent with the Cmd-O open file palette.
- Existing relative and absolute path handling is unaffected.

## Non-goals

- Supporting `$HOME` or other environment variable expansion in `/open-file` (follow-up if needed).
- Supporting `~otheruser` syntax (non-standard and not supported in Cmd-O either).
- Changing how autosuggestions/completions populate paths (that is a separate concern).

## User experience

### Current behavior (broken)

1. User types `/open-file ~/Documents/notes.txt` and presses Enter.
2. The handler joins `~` literally with the working directory, producing e.g. `/Users/me/project/~/Documents/notes.txt`.
3. A "File not found" error toast appears.

### Expected behavior (after fix)

1. User types `/open-file ~/Documents/notes.txt` and presses Enter.
2. `~` is expanded to the user's home directory (e.g. `/Users/me`).
3. The resulting absolute path `/Users/me/Documents/notes.txt` is used directly (not joined with the working directory).
4. If the file exists, it opens in Warp's code editor.
5. If the file does not exist, the "File not found" toast shows the expanded path (e.g. `File not found: /Users/me/Documents/notes.txt`), not the literal `~` form.

### Edge cases

- **`~` alone:** `/open-file ~` should show the "only works for files, not directories" error (since `~` expands to the home directory, which is a directory).
- **`~/` prefix:** `/open-file ~/foo.txt` expands correctly.
- **No tilde:** `/open-file foo.txt` continues to resolve relative to the current working directory (unchanged).
- **Absolute paths:** `/open-file /etc/hosts` continues to work (unchanged — `PathBuf::join` with an absolute path already replaces the base).
- **Escaped tilde from autosuggestion:** If the path comes from shell autosuggestion with escape characters (e.g. `\~`), the unescape step happens first, then tilde expansion applies to the unescaped result.
- **Line/column suffix:** `/open-file ~/foo.txt:10:5` should expand `~` and preserve the line/column argument.

## Success criteria

1. `/open-file ~/path/to/file.txt` opens the file at `$HOME/path/to/file.txt`.
2. `/open-file ~/path/to/file.txt:10` opens the file at line 10.
3. `/open-file relative/path.txt` still resolves relative to the working directory.
4. `/open-file /absolute/path.txt` still works as an absolute path.
5. The "File not found" error toast displays the expanded path, not the literal `~`.
6. `/open-file ~` shows the "only works for files, not directories" error.
7. Behavior matches the Cmd-O file palette for `~` paths.

## Validation

- **Unit test:** Add a test that verifies `/open-file ~/somefile` expands `~` to the home directory and resolves to the correct absolute path.
- **Manual test:** Type `/open-file ~/.bashrc` (or any file known to exist in the home directory) and confirm it opens correctly.
- **Regression test:** Confirm `/open-file relative.txt` and `/open-file /absolute/path.txt` continue to work.

## Open questions

None.
