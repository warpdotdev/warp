# Code Review: Case-Insensitive CWD Matching on macOS

## Context
When sending review comments, Warp checks whether a terminal's session CWD is inside the selected repo. This check uses `PathBuf::starts_with`, which does byte-level component comparison — case-sensitive even on macOS's case-insensitive APFS.

The two paths come from different sources with different casing guarantees:
- **Repo path** — from `DetectedRepositories::detect_possible_git_repo`, which discovers via git and returns a canonicalized path with on-disk casing (e.g. `/Users/kevin/Documents/GitHub/warp-2`).
- **Session CWD** — from shell-reported `$PWD` via `BlockMetadata::current_working_directory`, which preserves whatever casing the user typed when `cd`'ing (e.g. `/Users/kevin/Documents/Github/warp-2`).

When these differ only in case, the terminal is marked unavailable with "session cwd is not inside selected repo", and the send button shows "All terminals are busy" despite no command executing.

The comparison is in `review_terminal_status` at `app/src/workspace/view/right_panel.rs:1293`:
```rust
Some(cwd) if cwd.starts_with(repo_path) => {}
```

The session CWD is produced by `active_session_path_if_local` at `app/src/terminal/view.rs (6229-6251)`, which already performs filesystem I/O (`.filter(|path| path.is_dir())`), so the path is known to exist.

## Proposed changes
Canonicalize the path returned by `TerminalView::active_session_path_if_local` using `dunce::canonicalize` before returning it. This resolves the shell-reported CWD to its on-disk casing, ensuring it matches the git-discovered repo path.

`active_session_path_if_local` (`app/src/terminal/view.rs:6229`) currently ends with:
```rust
.filter(|path| path.is_dir())
```

Replace this with:
```rust
.and_then(|path| dunce::canonicalize(&path).ok())
```

`dunce::canonicalize` implies existence (it fails for non-existent paths), so it subsumes the `.is_dir()` filter. It also resolves symlinks and normalizes casing to match the on-disk representation, which is exactly what `DetectedRepositories` returns for repo paths.

`dunce` is already a dependency of the `app` crate (`app/Cargo.toml:95`).

### Why fix in `active_session_path_if_local` rather than at the comparison site
- All consumers of this method get a canonicalized path, preventing the same class of bug elsewhere.
- The method already does filesystem I/O, so `dunce::canonicalize` adds negligible overhead.
- The alternative (fixing only the comparison in `review_terminal_status`) would leave the underlying mismatch for any other code that compares session CWD against a canonical path.

### Note on `input.rs` variant
There is a second `active_session_path_if_local` in `app/src/terminal/input.rs:13099` that returns `Option<&Path>` (a borrow from the block metadata). This variant does not canonicalize and returns raw shell-reported paths. It is used only for prompt display and input context, not for path-equality checks against repo paths, so it does not need the same fix.

## Testing and validation
1. **Manual repro**: `cd` into a repo using a different-cased path (e.g. `cd ~/documents/github/repo` when the on-disk path is `~/Documents/GitHub/repo`). Open code review with a pending comment. Verify the send button is enabled and the comment can be sent.
2. **Unit test**: Add a test in `right_panel` that constructs a `ReviewTerminalStatus` where the session CWD differs only in case from the repo path, and assert the terminal is available. This requires mocking `active_session_path_if_local` to return a non-canonical path — if that's impractical, a targeted integration test is preferable.
3. **Regression check**: Verify that the existing code review send flow still works when the paths already match in case.
