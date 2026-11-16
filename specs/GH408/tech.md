# Tech Spec: Support ~ expansion in /open-file slash command

**Issue:** [warpdotdev/warp-external#408](https://github.com/warpdotdev/warp-external/issues/408)

## Problem

The `/open-file` slash command handler does not expand `~` before resolving the file path. The argument is unescaped and then joined directly with the current working directory, so `~/foo.txt` becomes `/cwd/~/foo.txt`. The Cmd-O open file palette already handles this correctly by calling `shellexpand::tilde()`.

## Relevant code

- `app/src/terminal/input/slash_commands/mod.rs (445-450)` — The `/open-file` (`commands::EDIT`) handler that parses the path argument and resolves it. This is the code that needs to change.
- `app/src/search/command_palette/files/data_source.rs:196` — The Cmd-O palette's tilde expansion: `shellexpand::tilde(&query_file_content).into_owned()`. This is the pattern to follow.
- `crates/warp_util/src/path.rs (149-181)` — `CleanPathResult::with_line_and_column_number()`, which strips line/column suffixes from the path string. Called before path resolution.
- `app/src/terminal/input_test.rs (2553-2596)` — Existing test `test_open_slash_command_clears_buffer_on_success` that exercises the `/open-file` handler with a real file.

## Current state

The handler in `slash_commands/mod.rs` does this sequence:

1. Parses line/column suffix with `CleanPathResult::with_line_and_column_number(args.trim())`
2. Unescapes shell characters with `session.shell_family().unescape(&parsed_path.path)`
3. Joins with cwd: `current_dir.join(&*unescaped_path)`

Step 3 always prepends the working directory. When the path starts with `~`, this produces an invalid path. There is no tilde expansion step.

## Proposed changes

### Single change in `app/src/terminal/input/slash_commands/mod.rs`

After the unescape step (line 449) and before the `current_dir.join()` call (line 450), add a tilde expansion step using `shellexpand::tilde()`:

```rust
let parsed_path = CleanPathResult::with_line_and_column_number(args.trim());
let unescaped_path = session.shell_family().unescape(&parsed_path.path);
let expanded_path = shellexpand::tilde(&unescaped_path);
let file_path = current_dir.join(&*expanded_path);
```

This works correctly for all cases because of how `PathBuf::join` behaves:
- **Tilde path** (`~/foo.txt`): `shellexpand::tilde` converts to `/home/user/foo.txt` (absolute), and `join` with an absolute path replaces the base entirely.
- **Relative path** (`foo.txt`): `shellexpand::tilde` is a no-op, and `join` prepends `current_dir` as before.
- **Absolute path** (`/etc/hosts`): `shellexpand::tilde` is a no-op, and `join` replaces the base (existing behavior).

No other files need to change.

## End-to-end flow

1. User types `/open-file ~/Documents/notes.txt` and presses Enter.
2. `execute_slash_command` is called with argument `~/Documents/notes.txt`.
3. `CleanPathResult::with_line_and_column_number` strips any `:line:col` suffix → path = `~/Documents/notes.txt`.
4. `shell_family().unescape()` removes shell escape characters → path = `~/Documents/notes.txt` (no change unless escaped).
5. **New:** `shellexpand::tilde()` expands `~` → path = `/home/user/Documents/notes.txt`.
6. `current_dir.join()` sees an absolute path and uses it directly.
7. `std::fs::metadata` confirms the file exists and it opens in the editor.

## Risks and mitigations

**Risk:** `shellexpand::tilde` could fail if `$HOME` is not set.
**Mitigation:** `shellexpand::tilde` returns the input unchanged when the home directory cannot be determined, so it degrades gracefully to the current (broken) behavior. This matches how other call sites in the codebase use it.

**Risk:** Interaction with shell-escaped `~` from autosuggestions.
**Mitigation:** The unescape step runs before tilde expansion. If autosuggestion produces `\~`, it unescapes to `~`, then tilde expansion works correctly. If somehow `~` was already escaped to mean a literal `~` directory name, that's an extreme edge case not worth special-casing.

## Testing and validation

1. **New unit test:** Add a `#[cfg(feature = "local_fs")]` test similar to `test_open_slash_command_clears_buffer_on_success` that creates a temp file at a known location, simulates `/open-file ~/relative-to-home` using the temp file's home-relative path, and verifies the buffer clears (indicating success).
2. **Manual test:** Run Warp, type `/open-file ~/.bashrc`, confirm it opens.
3. **Regression:** Existing tests (`test_open_slash_command_clears_buffer_on_success`, `test_open_slash_command_requires_path`, etc.) continue to pass.

## Follow-ups

- Consider supporting `$HOME` and other environment variable expansion (via `shellexpand::full` or `shellexpand::env`) if users request it.
- Investigate whether the completions/autosuggestions engine should also offer `~`-prefixed path completions.
