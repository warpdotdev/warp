# Tech Spec: Resolve clickable filenames in listing-command output against the listed directory

**Issue:** [warpdotdev/warp#9908](https://github.com/warpdotdev/warp/issues/9908)

## Context

### Current system

File-link detection lives in `app/src/terminal/view/link_detection.rs`. The `scan_for_file_path` method:

1. Extracts the block's `pwd`, `command_to_string()`, and `top_level_command(sessions)` (lines 463–477).
2. Calls `possible_file_paths_at_point` to get candidate path strings from the terminal grid.
3. Resolves each candidate against `pwd` using `std::fs::metadata` on a background thread.

`Block::top_level_command` delegates to `warp_completer::parsers::simple::top_level_command`, which strips env-var prefixes and returns the first literal token. It also resolves shell aliases via the session's alias table.

### Existing parser infrastructure

- **`warp_completer::parsers::simple::top_level_command`** — Extracts the command name from a command string, stripping env-var prefixes and handling quoting/subshells.
- **`warp_completer::parsers::classify_command`** — Full argument classification using `CommandRegistry`. Separates env vars, flags (with their consumed arguments), and positional arguments. Handles `--flag=value` syntax, short flag stacking, and variadic flag arguments.
- **`CommandRegistry`** — Loaded from the [command-signatures](https://github.com/warpdotdev/command-signatures) repository. Contains typed signatures for common commands including `ls` (with options like `-a`, `--color`, `-d` and a variadic `filepaths` positional argument).
- **`warp_completer::parsers::simple::parse_for_completions`** — Produces a `LiteCommand` from a string, handling the lexer/parser pipeline.

### Relevant files

| File | Role |
|---|---|
| `app/src/terminal/view/link_detection.rs:445–510` | `scan_for_file_path` — entry point for file-link resolution |
| `crates/warp_completer/src/parsers/simple/mod.rs:44` | `top_level_command` — command name extraction |
| `crates/warp_completer/src/parsers/mod.rs:117` | `classify_command` — full arg classification |
| `crates/warp_completer/src/signatures/` | `CommandRegistry` and signature loading |
| `crates/warp_util/src/listing_command.rs` | Current PR's custom parser (to be replaced) |

## Proposed Changes

### Architecture

Replace the custom `shlex`-based parser in `listing_command.rs` with a thin adapter over `classify_command` + `CommandRegistry`. The adapter:

1. Receives the block's command string and resolved alias name.
2. Uses `parse_for_completions` to produce a `LiteCommand`.
3. Calls `classify_command` with the `CommandRegistry` to get a `ClassifiedCommand`.
4. Inspects the classified result: extracts positional arguments and checks for the `-d`/`--directory` flag.
5. Returns `Option<PathBuf>` — the single directory argument if conditions are met.

### Module: `crates/warp_completer/src/listing_dir.rs` (new)

```rust
/// Given a classified command, determines if it is a single-directory listing
/// command and returns the directory path to resolve filenames against.
pub fn listing_directory_from_classified(
    command_str: &str,
    resolved_command_name: Option<&str>,
    pwd: &Path,
    command_registry: &CommandRegistry,
    listing_commands: &[&str],
) -> Option<PathBuf>
```

**Logic:**

1. Parse `command_str` via `simple::parse_for_completions` → `LiteCommand`.
1b. **Pipeline rejection:** Use `decompose_command(command_str, escape_char)` to check if the command contains a pipeline. If more than one top-level command is present, return `None`. This prevents `ls DIR/ | grep foo` from misclassifying `grep`'s arguments — `parse_for_completions` returns the last unclosed command, which would be `grep`, not `ls`.
2. Tokenize into `Vec<&str>` for `classify_command`.
3. Call `classify_command(lite_command, &mut tokens, command_registry, case_sensitivity)`.
4. Check if the resolved command name (or first token) is in `listing_commands`.
5. If the command was classified (signature found in registry): inspect `command.flags` for `-d`/`--directory` → if present, return `None`.
6. Extract positional arguments from `ClassifiedCommand`.
7. If exactly one positional: resolve it against `pwd`, check `is_dir()` → return `Some(resolved_path)`.
8. Otherwise (zero or multiple positionals): return `None`.

**Alias handling note:** `classify_command` looks up signatures by the literal first token. If the user typed `ll` (aliased to `ls`), the registry won't have a signature for `ll`, so the command is returned as unclassified. In this case:
- The feature still activates (step 4 uses `resolved_command_name` from `Block::top_level_command`).
- Flag-argument classification is best-effort: without a signature, all non-flag-looking tokens are treated as positionals. This means `ll --color=auto DIR/` works (flag with `=` is self-contained), but `ll -I PATTERN DIR/` may miscount positionals (the `-I` argument `PATTERN` looks like a positional).
- This is acceptable for V1. If reviewers require full alias support, the enhancement is to substitute the resolved command name into the first token of the `LiteCommand` before calling `classify_command`, giving it access to the `ls` signature.

### Wire-up: `app/src/terminal/view/link_detection.rs`

Replace the current call to `listing_command_argument_dir` (from `warp_util`) with a call to `listing_directory_from_classified` (from `warp_completer`). The `CommandRegistry` is already available in the app context (used by the completion engine).

### Changes to `crates/warp_util/`

Remove `listing_command.rs` and `listing_command_test.rs` — the custom parser is no longer needed. The `shlex` dependency can also be removed from `warp_util/Cargo.toml`.

### Data flow

```
Block command string + alias resolution
    ↓
parse_for_completions (simple parser)
    ↓
LiteCommand
    ↓
classify_command (with CommandRegistry)
    ↓
ClassifiedCommand { env_vars, command, error }
    ↓
Check: is listing command? has -d? exactly 1 positional dir?
    ↓
Option<PathBuf> (the directory to resolve against)
```

### Handling lucieleblanc's concerns

| Concern | Resolution |
|---|---|
| Multi-arg exclusion drops currently-resolved entries | Invariant 3 + Invariant 6: multi-arg falls back to pwd-only, which is today's behavior. No regression. |
| `-d` exclusion unclear | Invariant 4: `ls -d DIR` lists the directory entry itself (name, permissions), not its contents. Output is the operand name, not children. Resolving against `DIR` would be incorrect. |
| Aliases with positional args | Invariant 5: We use `Block::top_level_command` for alias→command mapping only. Baked-in positional args in aliases are not expanded (would require shell-level alias expansion, which Warp doesn't do). The resolved command name triggers the feature; the positional arg comes from the actual command string. |
| Reuse existing parsers | This spec replaces the custom `shlex` parser with `classify_command` + `CommandRegistry`. Full reuse of existing infrastructure. |
| Leverage command-signatures | `classify_command` uses the `ls` signature from command-signatures to properly separate `-d`, `--color=auto`, `-I PATTERN` (flag args) from the directory positional. |

### Types

No new public types. The function signature is:

```rust
pub fn listing_directory_from_classified(
    command_str: &str,
    resolved_command_name: Option<&str>,
    pwd: &Path,
    command_registry: &CommandRegistry,
    listing_commands: &[&str],
) -> Option<PathBuf>
```

### Tradeoffs

- **Pro:** Full reuse of battle-tested parser infrastructure. Correct handling of quoting, subshells, env-var prefixes, and flag-argument consumption.
- **Pro:** Access to command-signatures means we correctly handle flags like `--color=auto` (which takes an argument) vs `-l` (which doesn't).
- **Con:** Requires `CommandRegistry` access at the call site. The registry is already loaded for completions, so this is available but adds a dependency from link-detection to the completer crate.
- **Con:** Commands not in the registry fall back to unclassified parsing (positionals only, no flag-arg consumption). For `ls`/`eza`/`lsd` this is fine — they're all in the registry.

## Testing and Validation

### Unit tests (in `crates/warp_completer/src/listing_dir.rs`)

| Test | Invariant |
|---|---|
| `ls DIR/` with existing dir → returns `Some(DIR)` | 1 |
| `ls` (no args) → returns `None` | 2 |
| `ls DIR1/ DIR2/` → returns `None` | 3 |
| `ls -d DIR/` → returns `None` | 4 |
| `ls --directory DIR/` → returns `None` | 4 |
| `ls -la DIR/` → returns `Some(DIR)` | 1, 9 |
| `ls --color=auto DIR/` → returns `Some(DIR)` | 9 |
| `LANG=C ls DIR/` → returns `Some(DIR)` | 10 |
| `ls "path with spaces/"` → returns `Some(path with spaces)` | 1 |
| `ls $(echo dir)` → returns `None` (subshell) | edge case |
| Resolved alias `ll` → `ls`: `ll DIR/` → returns `Some(DIR)` | 5 |
| `ls nonexistent/` → returns `None` (not a dir) | 1 |
| `ls file.txt` → returns `None` (not a dir) | 1 |

### Integration test (in `app/src/terminal/view/`)

- Verify that `scan_for_file_path` resolves a filename from `ls DIR/` output against `DIR`, not `pwd`.

### Manual validation

1. `ls ~/some-project/` → hover over a filename → tooltip shows path under `~/some-project/`.
2. Click → opens the correct file.
3. `ls` (no args) → behavior unchanged.
4. `ll ~/some-project/` → same as (1) if `ll` is aliased to `ls`.

## Risks

| Risk | Mitigation |
|---|---|
| `CommandRegistry` not available at link-detection call site | Registry is already loaded for completions; thread through from app context. |
| Performance: `classify_command` on every hover | Acceptable per-hover cost: pure string parse, no I/O, bounded by command length (typically <100 chars), runs on the background thread. Same call site as existing `top_level_command` which already parses the command string on each hover. No cache needed for V1 — if profiling shows otherwise, a `(block_id, command_hash) -> Option<PathBuf>` cache can be added without API changes. |
| Unclassified commands (not in registry) | Fall back to `None` — same as today's behavior. No regression. |

## Follow-ups

- User-configurable listing command list (setting).
- Multi-operand heuristic (try each dir, accept if unambiguous).
- `ls -R` recursive listing with per-section directory headers.
- `tree` support (blocked on #9909 / #10004).
