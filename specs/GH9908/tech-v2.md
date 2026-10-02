# Tech Spec: Section-aware file-link resolution for multi-directory and recursive listings

**Issue:** [warpdotdev/warp#9908](https://github.com/warpdotdev/warp/issues/9908) (follow-up to V1 spec)
**Depends on:** V1 tech spec (classify_command + CommandRegistry integration)

## Context

### Current system (after V1)

V1 introduces `listing_directory_from_classified` in `crates/warp_completer/src/listing_dir.rs`. It uses `classify_command` + `CommandRegistry` to extract the single directory positional from a listing command. When the command has multiple positional directories or the `-R`/`--recursive` flag, V1 returns `None` and resolution falls back to pwd.

V2 adds output-aware resolution for these cases by scanning the block's terminal grid for section headers.

### Relevant files

| File | Role |
|---|---|
| `app/src/terminal/view/link_detection.rs:445-510` | `scan_for_file_path` — entry point, calls V1's listing_directory_from_classified |
| `app/src/terminal/model/grid/grid_handler.rs:848` | `line_to_string` — extracts text content from any grid row |
| `app/src/terminal/model/grid/grid_handler.rs:1097` | `possible_file_paths_at_point` — provides hovered row context |
| `crates/warp_completer/src/listing_dir.rs` | V1's command classification (new in V1) |

### ls output format

Multi-directory (`ls ~/a/ ~/b/`):
```
~/a/:
file1.txt
file2.txt

~/b/:
fileA.txt
fileB.txt
```

Recursive (`ls -R ~/projects/`):
```
~/projects/:
README.md
src

~/projects/src:
main.rs
lib.rs

~/projects/src/utils:
helpers.rs
```

Header format: `<path>:` on its own line (possibly with ANSI color codes), followed by entries, separated by blank lines between sections.

## Proposed Changes

### New module: `app/src/terminal/view/listing_section_scanner.rs`

```rust
use std::path::{Path, PathBuf};

/// Scans backward from `hovered_row` in the block's output grid to find the
/// nearest section header (`path:` line). Returns the resolved directory path
/// if a valid header is found.
///
/// `listing_root` is the primary directory argument from the command (if any),
/// used to resolve relative headers like `./src:`. If None, relative headers
/// resolve against `pwd`.
pub fn resolve_via_section_header(
    grid: &GridHandler,
    hovered_row: usize,
    listing_root: Option<&Path>,
    pwd: &Path,
    max_scan_depth: usize,
) -> Option<PathBuf>
```

**Logic:**

1. Starting at `hovered_row - 1`, scan backward up to `max_scan_depth` rows.
2. For each row, call `grid.row_text(row)` to get the plain text content.
   - **Note:** `GridHandler::line_to_string` is currently `pub(super)` (private to the grid module). A new public accessor is needed:
   ```rust
   /// Returns the plain text content of a single row.
   pub fn row_text(&self, row: usize) -> Option<String> {
       self.line_to_string(row, 0..self.columns(), false, false, RespectObfuscatedSecrets::No, false)
   }
   ```
   This is minimal API surface and generally useful beyond this feature.
3. Strip ANSI escape sequences from the line.
4. Trim whitespace. If the line matches `^(.+):$` (non-empty path followed by colon, nothing else):
   a. Extract the path portion (everything before the trailing colon).
   b. Resolve the path — avoiding double-prefix:
      - If absolute or starts with `~/`: expand directly.
      - If starts with `./`: strip the `./` prefix, then join with `listing_root` (or `pwd` if no listing root). This handles `ls -R ./src` which emits `./src:`, `./src/utils:` headers.
      - Otherwise (bare relative like `src` or `src/subdir`): resolve against `pwd` directly. This handles `ls -R src` which emits `src:`, `src/subdir:` headers — these are already pwd-relative and must NOT be joined with listing_root (that would produce `src/src/...`).
   c. Check `resolved_path.is_dir()`. If true, return `Some(resolved_path)`.
   d. If not a directory, continue scanning (it's not a real header).
5. If scan exhausts `max_scan_depth` or reaches row 0 with no valid header, return `None`.

### Wire-up in `link_detection.rs`

In `scan_for_file_path`, after the V1 call:

```rust
let listing_dir = pwd.as_deref().and_then(|pwd_str| {
    listing_directory_from_classified(/* V1 args */)
});

// V2: if V1 returned None and command is a multi-dir or recursive listing,
// try section-header scanning.
//
// listing_root semantics differ by case:
// - Recursive (ls -R DIR/): headers like ./src: are relative to DIR.
//   Pass DIR as listing_root.
// - Multi-directory (ls DIR1/ DIR2/): headers like DIR2/: are already
//   rooted paths (relative to pwd or absolute). Pass None — resolve
//   all headers against pwd directly.
let effective_dir = listing_dir.or_else(|| {
    if is_multi_dir_or_recursive_listing(&classified_command) {
        let listing_root = if is_recursive(&classified_command) {
            first_dir_positional.as_deref() // ./subdir: resolves relative to this
        } else {
            None // multi-dir headers are already rooted paths
        };
        resolve_via_section_header(
            &grid_handler,
            hovered_point.row,
            listing_root,
            Path::new(pwd_str),
            500, // max scan depth
        )
    } else {
        None
    }
});
```

### Helper: ANSI stripping

Use the existing `strip_ansi_escapes` crate (already a dependency in the workspace) or a simple regex `\x1b\[[0-9;]*m` to strip color codes before header matching.

### Constants

```rust
const MAX_SECTION_SCAN_DEPTH: usize = 500;
const SECTION_HEADER_REGEX: &str = r"^(.+):$";
```

## Data Flow

```
Hover event on filename in block output
  |
  v
V1: classify_command -> single dir? -> resolve against it (fast path)
  |
  | (V1 returns None: multi-dir or -R case)
  v
V2: is_multi_dir_or_recursive?
  |
  | yes
  v
Scan backward from hovered_row in grid
  |
  v
Find line matching `path:` pattern
  |
  v
Strip ANSI, extract path, resolve (relative to listing_root or pwd)
  |
  v
is_dir()? -> return Some(resolved_dir)
  |
  | (no valid header found)
  v
Fall back to pwd (existing behavior)
```

## Resolution Priority (complete V1+V2 system)

```
1. Section header directory (V2 — multi-dir/recursive only, output scanning)
2. Single listing directory (V1 — single-dir fast path, command parsing only)
3. Block pwd (existing behavior — all other commands)
```

## Testing and Validation

### Unit tests (in `app/src/terminal/view/listing_section_scanner.rs`)

| Test | Invariant |
|---|---|
| Grid with `./src:` header + listing_root=project -> resolves to project/src | 1, 3 |
| Grid with `src:` header from `ls -R src` -> resolves against pwd (not listing_root) | double-prefix prevention |
| Grid with `src/subdir:` header from `ls -R src` -> resolves against pwd as pwd/src/subdir | double-prefix prevention |
| Grid with `./src/utils:` header + listing_root -> strips ./ and joins with listing_root | 3 |
| Grid with no header lines -> returns None | 4 |
| Grid with `~/absolute/path:` header -> resolves absolutely | 8 |
| Grid with `config:` where config is not a dir -> skips, returns None | 7 |
| Grid with ANSI-colored header `\x1b[1m./src:\x1b[0m` -> strips and matches | edge |
| Scan stops at max_scan_depth -> returns None | perf |
| Multiple headers: returns nearest one above hovered row | 4 |
| Blank line between sections doesn't break scanning | 5 |

### Integration tests

- Create a mock block with multi-section ls output, verify hover on different sections resolves to different directories.
- Verify V1 fast path still works (single-dir, no output scanning triggered).

### Manual validation

1. `ls -R ~/some-project/` -> hover filenames in different sections -> each resolves correctly.
2. `ls ~/dir1/ ~/dir2/` -> hover in each section -> correct resolution.
3. `ls ~/single-dir/` -> still uses V1 fast path (no output scanning).
4. Non-listing command -> completely unaffected.

## Risks

| Risk | Mitigation |
|---|---|
| Performance: scanning 500 rows on every hover | Only triggered for multi-dir/recursive listings (scope gate). Single-dir and non-listing commands skip entirely. Could cache result per (block_id, hovered_row). |
| False positive headers | `is_dir()` check eliminates most. Lines like `error:` or `warning:` won't match because those paths don't exist as directories. |
| Grid row access during model lock | `line_to_string` is already called in the same code path for `possible_file_paths_at_point`. No new locking concerns. |

## Follow-ups

- Cache section header lookups per block (avoid re-scanning on adjacent row hovers).
- User-configurable scan depth.
- `tree` output support (different header format: indented with box-drawing chars, blocked on #10004).
