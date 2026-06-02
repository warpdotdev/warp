# TECH.md — Jujutsu (jj) prompt chips

**GitHub Issue:** [warpdotdev/warp#11797](https://github.com/warpdotdev/warp/issues/11797)
**Product Spec:** `specs/GH11797/product.md`

## Context

Warp's prompt context chip system is organized around four key files:

- **`app/src/context_chips/mod.rs`**: Defines the `ContextChipKind` enum (line 158) with variants for every chip type, and `to_chip()` (line 188) which maps each variant to a `ContextChip` struct (generator + policy). Also contains `available_chips()` (line 527), `display_value()` (line 445), `render_text_from_kind()` (line 616), `chips_to_string()` (line 580), and `placeholder_value()` (line 368).

- **`app/src/context_chips/builtins.rs`**: Contains the shell command generator functions (`ShellCommandGenerator`) for VCS-backed chips (git, svn, kubectl). Each function returns a `ShellCommandGenerator` with a shell command and optional dependency list.

- **`app/src/context_chips/context_chip.rs`**: Defines `ContextChip`, `ShellCommandGenerator`, `ChipRuntimePolicy`, and `ShellCommand` types.

- **`app/src/themes/theme.rs`**: Defines `PromptColors` struct with per-chip color fields.

The SVN chips (`SvnBranch`, `SvnDirtyItems`) are the closest analogue: shell-based, on-demand refresh, `jj:()` / `±N` rendering, and adjacent-chip spacing logic. The implementation will mirror their pattern exactly.

## New JJ shell commands

### `jj_bookmark()`

Produces the chip value text using the prototype command from the issue body:

```rust
const SH_COMMAND: &str = concat!(
    "_jj_bm=$(jj log -r @ --no-graph --ignore-working-copy",
    " --template 'if(bookmarks,bookmarks,\"\")' 2>/dev/null);",
    " if [ -n \"$_jj_bm\" ]; then echo \"$_jj_bm\";",
    " else _jj_cid=$(jj log -r @ --no-graph --ignore-working-copy",
    " --template 'change_id.short(8)' 2>/dev/null);",
    " _jj_abm=$(jj log -r 'latest(ancestors(@) & bookmarks() ~ @)'",
    " --no-graph --ignore-working-copy --template 'bookmarks' 2>/dev/null);",
    " [ -n \"$_jj_abm\" ] && echo \"$_jj_cid on $_jj_abm\" || echo \"$_jj_cid\"; fi",
);
```

This single shell command handles all five bookmark/change states listed in the issue description. The PowerShell variant wraps the same logic in a multi-line script.

### `jj_dirty_items()`

Produces the count of changed files in the working copy:

```bash
# Bash/Zsh:
count=$(jj diff --summary 2>/dev/null | wc -l) && (( $count > 0 )) && echo $(( $count ))

# Fish:
set count (jj diff --summary 2>/dev/null | wc -l); test $count -gt 0 && string trim $count

# PowerShell:
$count = (jj diff --summary 2>$null | Measure-Object -line).Lines; if ($count -gt 0) { $count }
```

`jj diff --summary` outputs one line per changed file and produces no output when the workspace is clean, so `wc -l` gives an accurate count without header noise.

## Proposed changes

### 1. `app/src/context_chips/mod.rs` — Add `JjBookmark` and `JjDirtyItems` variants

**1a. Enum definition (~line 176):** Add `JjBookmark,` and `JjDirtyItems,` after `SvnDirtyItems`.

**1b. `to_chip()` (~line 320):** Add match arms mirroring `SvnBranch`/`SvnDirtyItems`:

```rust
Self::JjBookmark => Some(ContextChip::shell_builtin(
    "Jj Bookmark",
    builtins::jj_bookmark(),
    None,
    RefreshConfig::OnDemandOnly,
)),
Self::JjDirtyItems => Some(ContextChip::shell_builtin(
    "Jj Uncommitted File Count",
    builtins::jj_dirty_items(),
    None,
    RefreshConfig::OnDemandOnly,
)),
```

**1c. `placeholder_value()` (~line 384):** Add:

```rust
Self::JjBookmark => ChipValue::Text("jj-feature-bookmark".to_string()),
Self::JjDirtyItems => ChipValue::Text("3".to_string()),
```

**1d. `default_styles()` (~line 416):** Map to theme colors:

```rust
Self::JjBookmark => prompt_colors.input_prompt_branch,
Self::JjDirtyItems => prompt_colors.input_prompt_svn,
```

**1e. `display_value()` (~line 451):** Add wrappers:

```rust
Self::JjBookmark => format!("jj:({text})"),
Self::JjDirtyItems => format!("±{text}"),
```

**1f. `udi_icon()` (~line 509):** Update existing arm to include JjBookmark:

```rust
Self::ShellGitBranch | Self::SvnBranch | Self::JjBookmark => Some(Icon::GitBranch),
Self::GitDiffStats | Self::SvnDirtyItems | Self::JjDirtyItems => Some(Icon::File),
```

**1g. `render_text_from_kind()` (~line 627):** Add match arms for both variants, mirroring `SvnBranch`/`SvnDirtyItems` rendering:
- `JjBookmark`: prefix `jj:(` and suffix `)` use `input_prompt_branch` (matching `ShellGitBranch`'s use of `input_prompt_git`).
- `JjDirtyItems`: prefix `±` uses `input_prompt_svn`.

**1h. `available_chips()` (~line 539):** Add `ContextChipKind::JjBookmark,` and `ContextChipKind::JjDirtyItems,` to the list.

**1i. `chips_to_string()` (~line 593):** Extend the adjacent-chip spacing arm:

```rust
(ContextChipKind::SvnBranch, Some(ContextChipKind::SvnDirtyItems))
| (ContextChipKind::JjBookmark, Some(ContextChipKind::JjDirtyItems))
| (ContextChipKind::JjDirtyItems, Some(ContextChipKind::JjBookmark)) => (),
```

### 2. `app/src/context_chips/builtins.rs` — Add generator functions

Add two new public functions: `jj_bookmark()` and `jj_dirty_items()`, each returning a `ShellCommandGenerator` with dependencies `vec!["jj".to_owned()]`.

For PowerShell variants, port the bash logic to PowerShell using `jj` commands directly (the prototype bash script is POSIX-only; PowerShell needs its own). Multi-line scripts are acceptable per the existing `svn_dirty_items()` precedent.

### 3. No other files require changes

- `display_chip.rs`: No changes — `JjBookmark` and `JjDirtyItems` produce plain `ChipValue::Text`, which renders via the generic text path.
- `current_prompt.rs`: No changes — `CurrentPrompt` uses `to_chip()` and `runtime_policy` generically.
- `prompt.rs`: No changes — `Prompt` singleton handles all `ContextChipKind` variants uniformly.
- `theme.rs`: No new colors needed — both chips reuse existing `input_prompt_branch` and `input_prompt_svn` colors.

## Testing and validation

| Product invariant | Test approach |
|---|---|
| 1. Chips appear in picker UI | Unit test in `builtins_tests.rs`: verify `available_chips()` includes both new variants |
| 2. Runtime disable when `jj` absent | Unit test in `current_prompt_tests.rs`: mirror `test_shell_chip_disabled_when_executable_is_missing` for `jj` |
| 3–5. JjBookmark display text | Shell command test: run `jj log` in a `jj` repo with various states and verify output format |
| 6. Empty when no jj repo | Manual: `cd /tmp && warp` — chip should not render |
| 7. JjDirtyItems count | Shell command test: run `jj diff --summary` in a repo with known changes and verify count |
| 8–9. Rendering | Unit test in `display_chip_tests.rs` or `renderer_tests.rs`: verify `render_text_from_kind` produces correct styled spans |
| 10. Adjacent spacing | Unit test in `prompt_tests.rs` or `mod.rs` test: verify `chips_to_string` omits space between `JjBookmark`/`JjDirtyItems` |
| 13. Icons | Unit test: verify `udi_icon()` returns correct icon for both variants |
