# GH9908: Tech Spec — Listing-Aware File Link Resolution
## 1. Problem
File-link hover detection for terminal BlockList output currently resolves relative candidate paths against the block's stored working directory. That loses command context for `ls DIR/` output, where bare filenames in the output are entries under `DIR`, not entries under the block CWD.
The implementation should add a narrow listing-aware resolution root for supported `ls`-style commands. It should prefer the listed directory only when the block command and stored `pwd` make that root unambiguous, then fall back to the existing resolver for all other cases.
## 2. Relevant code
- `app/src/terminal/view/link_detection.rs:438` — `scan_for_file_path` selects the working directory used for file-link resolution. For BlockList links it currently reads `block.pwd()` and does not inspect the command.
- `app/src/terminal/view/link_detection.rs:500` — `compute_valid_paths` validates hovered candidate paths by calling `absolute_path_if_valid` against the selected working directory.
- `app/src/util/file.rs:32` — `absolute_path_if_valid` joins a candidate path against the supplied working directory and validates that the resolved path is a file or a directory without line/column metadata.
- `app/src/terminal/model/grid/grid_handler.rs:91` — `FILE_LINK_SEPARATORS` defines token separators for file-link candidates.
- `app/src/terminal/model/grid/grid_handler.rs:1097` — `possible_file_paths_at_point` returns candidate path fragments around the hovered cell, ordered longest to shortest.
- `app/src/terminal/model/terminal_model.rs:1826` and `app/src/terminal/model/blocks.rs:2439` — model/block wrappers route candidate extraction for AltScreen and BlockList.
- `app/src/terminal/model/block.rs:2172` — `Block::top_level_command` returns the block's top-level command and resolves a single alias level through the session.
- `app/src/terminal/model/block/serialized_block.rs:145` — serialized blocks store `stylized_command` and `pwd`, which allows restored blocks to retain the data needed for listing-aware resolution.
- `app/src/terminal/view/open_in_warp.rs:292` — `check_openable_in_warp` shows an existing pattern for parsing command positionals from completed commands.
- `crates/warp_completer/src/parsers/simple/mod.rs:44` — `top_level_command` parses the command name while skipping leading environment variable assignments.
## 3. Current state
On hover, `TerminalView::maybe_link_hover` queues a file-link scan when the hovered fragment changes. `scan_for_file_path` gathers:
- The resolver root: active local `pwd` for AltScreen, or `block.pwd()` for BlockList.
- Candidate path substrings from `possible_file_paths_at_point`.
- Shell launch data for shell-native path conversion.
The expensive work runs on a background thread via `compute_valid_paths`. For each candidate, Warp calls `absolute_path_if_valid(candidate, ShellPathType::ShellNative(working_directory), shell_launch_data)`. Prefix and suffix cleanup for git-diff-style `a/`, `b/`, and symlink `@` runs only after the first direct validation attempt fails.
This design is already asynchronous and existence-based, but it has only one root. For `ls subdir/` output, the root should be `block.pwd()/subdir`, while the current code always uses `block.pwd()`. Tokenization is not the primary problem for ordinary `ls` output because bare filenames like `README.md` are already valid candidates.
## 4. Proposed changes
### Add a pure listing-command resolver helper
Add a small helper module under `crates/warp_util/src/listing_command.rs` and export it from `crates/warp_util/src/lib.rs`.
Recommended API:
- `pub const DEFAULT_LISTING_COMMANDS: &[&str] = &["ls", "exa", "eza", "lsd"];`
- `pub fn listing_command_argument_dir(command: &str, pwd: &Path, listing_commands: &[&str]) -> Option<PathBuf>;`
- Optionally add an internal variant that accepts a resolved top-level command override when the caller already resolved an alias.
The helper should:
- Parse shell-like tokens with existing dependency support where possible. `shlex` is already available in the app crate; if the helper lives in `warp_util`, add `shlex` to `crates/warp_util/Cargo.toml` or use an existing parser that is already available to `warp_util`.
- Skip leading `KEY=VALUE` environment variable assignments before checking the command name.
- Check the command name against `listing_commands`.
- Skip flags and flag values that are known not to be directory operands.
- Reject recursive mode for `ls -R` and `ls --recursive`.
- Return a directory only when there is exactly one usable directory operand for V1.
- Resolve relative operands against `pwd`, preserve absolute operands, and support tilde expansion if implemented consistently with existing path-link expansion.
- Return `None` for malformed input, unknown commands, missing operands, non-directory operands, recursive listings, and multi-directory listings.
The helper should not inspect terminal output. It should be pure command/pwd logic plus bounded metadata checks for candidate directory operands.
### Thread listing root through BlockList scanning
Update `scan_for_file_path` so BlockList scanning can collect both:
- The existing `working_directory` root from `block.pwd()`.
- An optional `listing_directory` root derived from the block command and `pwd`.
For AltScreen scanning, keep the current active-pwd behavior and do not attempt listing-aware resolution. AltScreen content is live screen state rather than durable completed-block output, and it does not have the same completed block command boundary.
For BlockList scanning:
- Skip all file-link detection for remote blocks exactly as the current code does.
- Read the block command using the non-secret display command path already used by the block model, such as `block.command_to_string()` or the same source used by `Block::top_level_command`.
- Use `Block::top_level_command(self.sessions.as_ref(ctx))` to support simple aliases where the user-typed command resolves to `ls`, `exa`, `eza`, or `lsd`.
- Compute `listing_directory` only when `block.pwd()` exists and the helper returns a valid directory.
- Move the optional `listing_directory` into the same background thread as `compute_valid_paths` so the UI thread does not do repeated candidate validation.
### Prefer listed-directory resolution before existing CWD resolution
Change `compute_valid_paths` to accept an optional listed-directory root:
- Existing root: `working_directory: &str`.
- New root: `listing_directory: Option<PathBuf>`.
For each candidate path:
1. If `listing_directory` exists and the candidate is not already absolute and does not already include a path component that should resolve correctly against `working_directory`, try `absolute_path_if_valid(candidate, ShellPathType::PlatformNative(listing_directory.clone()), shell_launch_data)`.
2. If that succeeds, create the link with the original candidate range and return it.
3. Otherwise run the existing `ShellNative(working_directory)` validation and prefix/suffix cleanup unchanged.
The double-join guard is important. A candidate such as `subdir/README.md`, `./subdir/README.md`, `../other/file.txt`, or `/tmp/warp-repro/subdir/README.md` should not become `subdir/subdir/README.md`. A conservative V1 rule is:
- Use `listing_directory` only for bare output entry names with no path separators after cleanup.
- Continue using the existing resolver for candidates that include `/`, `\`, `./`, `../`, `~`, drive prefixes, or absolute-path syntax.
This conservative rule matches ordinary `ls DIR/` output and avoids altering rooted output from `find`, `grep`, compiler diagnostics, and tools that already print paths.
### Preserve prefix/suffix behavior
The existing `a/` and `b/` prefix cleanup and `@` symlink suffix cleanup should remain intact. Apply listed-directory resolution only to the same candidate forms that are safe to treat as bare entry names. If suffix cleanup turns `name@` into bare `name`, listed-directory resolution can be attempted before falling back to CWD resolution.
Do not apply listed-directory resolution to `a/name` or `b/name`; those are path-like and should remain covered by existing git-diff behavior.
### Settings and extensibility
V1 can ship with `DEFAULT_LISTING_COMMANDS` as the supported set. If reviewers require user configurability in the first implementation, introduce a settings-backed command list that defaults to the same values and pass that list into the helper from `TerminalView`.
If settings support is deferred, keep the helper signature list-based rather than hard-coding internally. That preserves an easy path to a user-configurable command list without changing link-detection semantics later.
## 5. End-to-end flow
1. User hovers `README.md` in a completed BlockList block produced by `ls -la subdir/`.
2. `maybe_link_hover` queues `FindLinkArg` because the hovered fragment changed.
3. `scan_for_file_path` identifies the hovered block, verifies it is local, reads `block.pwd()`, and reads the block command.
4. `listing_command_argument_dir("ls -la subdir/", Path::new(block_pwd), DEFAULT_LISTING_COMMANDS)` returns `block_pwd/subdir`.
5. `possible_file_paths_at_point` returns candidates such as `README.md`.
6. The background thread calls `compute_valid_paths` with both `working_directory = block_pwd` and `listing_directory = block_pwd/subdir`.
7. `compute_valid_paths` sees that `README.md` is a bare candidate, validates `block_pwd/subdir/README.md`, and creates a `FileLink` for the original output range.
8. `handle_file_link_completed` installs the highlighted file link and cursor state exactly as it does today.
9. Opening the highlighted link uses the existing `open_file_path` or folder route based on the resolved absolute path.
## 6. Risks and mitigations
- Risk: New silent misresolution for multi-directory listings. Mitigation: return `None` when more than one directory operand is present in V1.
- Risk: Recursive listings use changing roots per section. Mitigation: detect `-R` and `--recursive` and opt out.
- Risk: Double-joining paths that already include a directory. Mitigation: only apply listed-directory resolution to bare names and fall back to the current resolver for path-like candidates.
- Risk: Per-hover performance regression from command parsing or filesystem checks. Mitigation: parse once per hover, only check the command operands and hovered candidate, keep work off the UI thread, and avoid scanning block output.
- Risk: Moving command parsing into `warp_util` adds dependencies or duplicates shell parsing behavior. Mitigation: prefer the smallest dependency surface, add focused tests, and keep parsing logic limited to listing-command operands.
- Risk: Alias handling can be incomplete. Mitigation: support the existing single-level alias resolution from `Block::top_level_command`; document transitive or positional alias limitations as follow-ups.
- Risk: Remote sessions may have shell-native path conversion needs that differ from local files. Mitigation: preserve the existing remote-block skip behavior and only apply this to local BlockList scanning.
## 7. Testing and validation
### Unit tests for the helper
Add tests in `crates/warp_util` for:
- `ls subdir`, `ls subdir/`, `ls ./subdir`, `ls /absolute/path`.
- `ls -la subdir`, `ls --color=always subdir`, and flags before operands.
- Quoted and escaped directory names with spaces.
- Leading environment assignments such as `LS_COLORS=auto ls subdir`.
- Tilde-prefixed directory operands if tilde support is implemented.
- Unknown commands such as `find`, `cat`, and `git`.
- Missing operands, non-directory operands, malformed shell input, `ls -R subdir`, `ls --recursive subdir`, and `ls dir1 dir2`.
- Custom listing-command lists if the helper accepts caller-provided command sets.
### Unit tests for resolution order
Add targeted tests around `compute_valid_paths` or a new pure helper extracted from it:
- Listed-directory match beats CWD match for a same-named file.
- Listed-directory-only file becomes linkable.
- Listed-directory-only directory becomes linkable when there is no line/column suffix.
- Existing CWD behavior remains for no listed directory.
- Path-like candidates with slashes are not resolved through the listed directory.
- Symlink suffix cleanup with `name@` can resolve under the listed directory.
### Integration/manual validation
- Run the repro from the issue on macOS and verify the clicked Markdown file is the subdirectory copy.
- Verify directories listed by `ls DIR/` open in Finder.
- Verify scrollback after `cd` and restored sessions still use the original block command and `pwd`.
- Verify `find DIR -name '*.md'`, compiler diagnostics, git diff paths, and plain `ls` still behave as before.
### Repository checks
Recommended checks after implementation:
- `cargo test -p warp_util --lib listing_command`
- Targeted app tests covering link detection if available.
- `cargo fmt --all --check`
- `cargo clippy -p warp --all-targets -- -D warnings` if feasible in the development environment.
## 8. Follow-ups
- Add a user-facing setting for listing-aware commands if it is not included in V1.
- Add section-aware support for `ls -R`.
- Add section-aware support for multi-directory listings.
- Address the separate tokenizer issue for `tree` and tree-shaped output using box-drawing characters.
- Consider caching parsed listing roots per block if profiling shows command parsing is measurable, though the current proposed work should be bounded enough to avoid needing a cache.
