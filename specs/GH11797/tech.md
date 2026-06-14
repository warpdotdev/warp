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

Produces the chip value text using `jj log` with output templates. `change_id.shortest(8)`
returns the shortest unique prefix of the change ID, up to 8 characters (e.g., `qquqvwzk`).
All `jj log` and `jj diff` invocations include `--ignore-working-copy` to prevent
unintentional auto-snapshots during prompt rendering. Logic:

1. Query bookmarks on `@` — if found, output them directly.
2. Otherwise, get the change ID via `change_id.shortest(8)`.
3. Query the nearest bookmarked ancestor via `latest(ancestors(@) & bookmarks() ~ @)`.
4. If an ancestor bookmark exists, output `"<change_id> on <bookmarks>"`; otherwise output just `<change_id>`.

```rust
// Bash/Zsh:
const SH_BOOKMARK_CMD: &str = "bookmarks=$(jj log -r '@' --no-graph --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>/dev/null | head -1) \
    && if [ -n \"$bookmarks\" ]; then echo \"$bookmarks\"; \
    else cid=$(jj log -r '@' --no-graph --ignore-working-copy \
    -T 'change_id.shortest(8)' 2>/dev/null); \
    ancestor_bookmarks=$(jj log -r 'latest(ancestors(@) & bookmarks() ~ @)' --no-graph \
    --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>/dev/null | head -1); \
    if [ -n \"$ancestor_bookmarks\" ]; then echo \"${cid} on ${ancestor_bookmarks}\"; \
    else echo \"$cid\"; fi; fi";
```

```fish
// Fish:
const FISH_BOOKMARK_CMD: &str = "set bookmarks (jj log -r '@' --no-graph --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>/dev/null | head -1) \
    && if test -n \"$bookmarks\"; echo $bookmarks; \
    else; set cid (jj log -r '@' --no-graph --ignore-working-copy \
    -T 'change_id.shortest(8)' 2>/dev/null); \
    set ancestor_bookmarks (jj log -r 'latest(ancestors(@) & bookmarks() ~ @)' --no-graph \
    --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>/dev/null | head -1); \
    if test -n \"$ancestor_bookmarks\"; echo \"${cid} on ${ancestor_bookmarks}\"; \
    else; echo $cid; end; end";
```

```powershell
// PowerShell:
const PWSH_BOOKMARK_CMD: &str = "$bookmarks = jj log -r '@' --no-graph --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>$null; \
    if ($bookmarks) { $bookmarks } \
    else { $cid = jj log -r '@' --no-graph --ignore-working-copy \
    -T 'change_id.shortest(8)' 2>$null; \
    $ancestorBookmarks = jj log -r 'latest(ancestors(@) & bookmarks() ~ @)' --no-graph \
    --ignore-working-copy \
    -T 'separate(\" \", bookmarks.map(|x| x.name()))' 2>$null; \
    if ($ancestorBookmarks) { \"${cid} on ${ancestorBookmarks}\" } else { $cid } }";
```

All three shell variants use `ShellCommand::shell_specific([...])` with per-shell entries,
following the `svn_branch_context()` / `svn_dirty_items()` pattern.

### `jj_dirty_items()`

Produces the count of changed files in the working copy:

```rust
// Bash/Zsh:
const SH_DIRTY_CMD: &str =
    "count=$(jj diff --summary --ignore-working-copy 2>/dev/null | wc -l) && [ \"$count\" -gt 0 ] && echo \"$count\"";
```

```rust
// Fish:
const FISH_DIRTY_CMD: &str = "set count (jj diff --summary --ignore-working-copy 2>/dev/null | wc -l) \
    && test $count -gt 0 && string trim $count";
```

```rust
// PowerShell:
const PWSH_DIRTY_CMD: &str = "jj diff --summary --ignore-working-copy 2>$null | Measure-Object -Line | \
    Where-Object { $_.Lines -gt 0 } | ForEach-Object { $_.Lines }";
```

`jj diff --summary` outputs one line per changed file. The `--ignore-working-copy` flag prevents
auto-snapshotting during prompt rendering while still showing accurate file counts. The pipe to
`wc -l` / `Measure-Object` gives an accurate count. When the workspace is clean (zero changes),
the chip value is empty and the chip is hidden.

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
- `JjBookmark`: prefix `jj:(` and suffix `)` use `input_prompt_branch`.
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

- `display_chip.rs`: No changes — `JjBookmark` and `JjDirtyItems` produce plain `ChipValue::Text`, which renders via the generic text path. Click-to-copy is not supported, matching the existing `SvnBranch`/`SvnDirtyItems` behavior (which are also plain-text chips with no click handler).
- `current_prompt.rs`: No changes — `CurrentPrompt` uses `to_chip()` and `runtime_policy` generically.
- `prompt.rs`: No changes — `Prompt` singleton handles all `ContextChipKind` variants uniformly.
- `theme.rs`: No new colors needed — both chips reuse existing `input_prompt_branch` and `input_prompt_svn` colors.

## Testing and validation

Each shell-specific command variant (Bash/Zsh, Fish, PowerShell) should be exercised to ensure
the template string escaping is correct across all platforms.

| Product invariant | Test approach |
|---|---|
| 1. Chips appear in picker UI | Unit test: `available_chips()` includes `JjBookmark` and `JjDirtyItems` |
| 2. Runtime disable when `jj` absent | Unit test: `dependencies()` returns `["jj"]`; ChipRuntimePolicy disables chip |
| 3–5. JjBookmark display text | Shell command test: run `jj log` in a `jj` repo with various states and verify output format |
| 6. Empty when no jj repo | Manual: `cd /tmp && warp` — chip should not render |
| 7. JjDirtyItems count | Shell command test: run `jj diff --summary` in a repo with known changes and verify count |
| 8–9. Rendering | Unit test: verify `render_text_from_kind` produces correct prefix/suffix styled spans |
| 10. Adjacent spacing | Unit test: verify `chips_to_string` omits space between `JjBookmark`/`JjDirtyItems` in both orders |
| 11. Placeholders | Unit test: `placeholder_value()` returns `jj-feature-bookmark` and `3` |
| 12. Icons | Unit test: `udi_icon()` returns `Icon::GitBranch` / `Icon::File` |
