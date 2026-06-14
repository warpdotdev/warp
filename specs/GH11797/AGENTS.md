# AGENTS.md — Jujutsu (jj) prompt chips

> Implementation guide for [warpdotdev/warp#11797](https://github.com/warpdotdev/warp/issues/11797)

## Overview

Add two new VCS prompt context chips (`JjBookmark`, `JjDirtyItems`) to Warp's context chip system, mirroring the existing `SvnBranch` / `SvnDirtyItems` pattern.

## Key files

Only two files need changes:

| File | What goes there |
|------|-----------------|
| `app/src/context_chips/mod.rs` | Enum variants (`ContextChipKind`), `to_chip()`, `placeholder_value()`, `default_styles()`, `display_value()`, `udi_icon()`, `render_text_from_kind()`, `available_chips()`, `chips_to_string()` |
| `app/src/context_chips/builtins.rs` | Shell command generator functions (`jj_bookmark()`, `jj_dirty_items()`) returning `ShellCommandGenerator` |

No other files require changes — the chip system handles everything generically through `to_chip()` and `ContextChip::shell_builtin()`.

## ContextChipKind enum (`mod.rs` line 158)

The enum is matched exhaustively in ~8 different functions. Adding a new variant means adding arms to **all** of them. Missing one causes a compile error, so the compiler guides you.

### All match locations (in order they appear in the file)

1. **`to_chip()`** (line 188) — maps variant to `ContextChip::shell_builtin()`
2. **`placeholder_value()`** (line 368) — static placeholder for settings UI
3. **`default_styles()`** (line 392) — maps to `PromptColors` field
4. **`display_value()`** (line 441) — wraps value with prefix/suffix (e.g., `jj:(...)`)
5. **`udi_icon()`** (line 498) — maps to an `Icon` variant
6. **`render_text_from_kind()`** (line 616) — styled text with colored prefix/suffix
7. **`available_chips()`** (line 527) — adds to the chip picker list
8. **`chips_to_string()`** (line 580) — adjacent-chip spacing logic

### Pattern: follow SvnBranch/SvnDirtyItems exactly

For every match arm, copy the Svn equivalent and adjust:
- `SvnBranch` → `JjBookmark` (uses `input_prompt_branch` color, `Icon::GitBranch`)
- `SvnDirtyItems` → `JjDirtyItems` (uses `input_prompt_svn` color, `Icon::File`)

### Important gotchas

- **`default_styles()`**: `JjBookmark` uses `input_prompt_branch` (same as `ShellGitBranch`/`SvnBranch`), `JjDirtyItems` uses `input_prompt_svn` (same as `SvnDirtyItems`). Weight is always `Semibold`.
- **`display_value()`**: `JjBookmark` → `jj:(...)`, `JjDirtyItems` → `±...`
- **`udi_icon()`**: Extend existing arms — `ShellGitBranch | SvnBranch | JjBookmark` share `Icon::GitBranch`, `GitDiffStats | SvnDirtyItems | JjDirtyItems` share `Icon::File`.
- **`render_text_from_kind()`**: `JjBookmark` prefix `jj:(` and suffix `)` use `input_prompt_branch` color. `JjDirtyItems` prefix `±` uses `input_prompt_svn` color.
- **`chips_to_string()`**: Adjacent `JjBookmark`/`JjDirtyItems` chips (in either order) should not have a space between them, same as the Svn pair.

## Shell command generators (`builtins.rs`)

Two new functions, each returning `ShellCommandGenerator::new(command, Some(vec!["jj".to_owned()]))`.

### `jj_bookmark()`

Shell command output:
- Bookmarked change → `main` (or space-separated list)
- Anonymous with bookmarked ancestor → `f3a2b1c0 on main`
- Anonymous, no bookmarks → `f3a2b1c0` (short 8-char change ID)
- Not in a jj repo → empty string (chip hidden)

Three shell variants needed (bash/zsh, fish, PowerShell). See `tech.md` for exact scripts.

**Pattern**: Use `ShellCommand::shell_specific([...])` with `ShellType::Bash/Zsh/Fish/PowerShell` entries, matching `svn_branch_context()` / `svn_dirty_items()` pattern.

### `jj_dirty_items()`

Shell command output: integer count of changed files (e.g., `3`), or empty for clean workspace.

Uses `jj diff --summary` piped to `wc -l`. Three shell variants needed.

**Pattern**: Same as `svn_dirty_items()` — `ShellCommand::shell_specific([...])` with per-shell implementations.

## Testing

Test file: `app/src/context_chips/builtins_tests.rs`

Key test patterns:
- `available_chips()` inclusion
- `udi_icon()` returns correct icon
- Shell chip disabled when `jj` executable missing (see `test_shell_chip_disabled_when_executable_is_missing` in `current_prompt_tests.rs`)
- `render_text_from_kind()` produces correct styled spans (see `renderer_tests.rs`)
- `chips_to_string()` omits space between adjacent Jj chips (see `prompt_tests.rs`)

## Build commands

```bash
# Check specific module
cargo check -p warp --lib

# Run context_chips tests
cargo test -p warp -- context_chips

# Run specific test
cargo test -p warp -- builtins_tests
```

## Architecture notes

- `ContextChipKind` is `Serialize`/`Deserialize` — new variants must handle migration from configs that lack them. Derive behavior: unknown variants become `None` on deserialization (no crash), but **order matters** for backward compat with existing serialized configs. Add new variants at the **end** of the enum, before `AgentPlanAndTodoList`.
- `ShellCommandGenerator` dependencies (`vec!["jj".to_owned()]`) trigger the `RequiresExecutable` disabled state when `jj` is not in `$PATH`. This is handled automatically by `ChipRuntimePolicy::for_shell_generator()`.
- Shell commands run in the user's current working directory. Output is captured as the chip value text.

## Shell-specific considerations

- Bash/Zsh: share the same POSIX command string
- Fish: needs its own syntax (`set`, `test`, `string trim`)
- PowerShell: needs its own syntax (`$variables`, `Measure-Object`)
- Multi-line commands are fine (see `svn_dirty_items()` precedent)