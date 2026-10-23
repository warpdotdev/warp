# Product Spec: Section-aware file-link resolution for multi-directory and recursive listings

**Issue:** [warpdotdev/warp#9908](https://github.com/warpdotdev/warp/issues/9908) (follow-up to V1 spec)
**Depends on:** V1 spec (single-directory listing resolution)

## Summary

V1 resolves clickable filenames against a single listed directory. V2 extends this to multi-directory (`ls DIR1/ DIR2/`) and recursive (`ls -R DIR/`) listings by parsing section headers from the command output to determine which directory each filename belongs to.

## Problem

After V1 ships, two common listing patterns still fall back to pwd-only resolution:

1. **Multi-directory:** `ls ~/projects/ ~/downloads/` outputs files from both directories, separated by `dirname:` headers.
2. **Recursive:** `ls -R ~/projects/` outputs files from the root and all subdirectories, each preceded by a `./subdir:` header.

Both produce structured output with clear section boundaries. Users expect clicking any filename to open the correct file regardless of which section it appears in.

## Goals

- Filenames in multi-directory and recursive listing output resolve against their section's directory.
- Section detection uses the standard `ls` header format (`path:` on its own line, followed by entries).
- V1's single-directory behavior is preserved as a fast path (no output scanning needed).
- Performance: output scanning is bounded and lazy (scan backward from hovered line only).

## Non-goals

- Custom/non-standard output formats (e.g. `ls --format=commas` where entries aren't one-per-line).
- `tree` output (different header format, blocked on #9909/#10004).
- Piped output (`ls -R | grep foo`) — cannot attribute lines to the listing command.

## Behavior Invariants

1. **Section header detection:** A line matching the pattern `<path>:` (path followed by colon, with no other content after ANSI stripping) immediately preceding one or more filename lines is treated as a section header.
2. **Multi-directory resolution:** For `ls DIR1/ DIR2/`, each section's filenames resolve against that section's header path (joined with the block's pwd if relative).
3. **Recursive resolution:** For `ls -R DIR/`, section headers like `./subdir:` resolve relative to the listing root (the directory argument from the command). Filenames resolve against the fully-resolved section path.
4. **Backward scan:** When the user hovers a filename, the system scans backward from that line to find the nearest section header. If no header is found, resolution falls back to V1 behavior (single-dir or pwd).
5. **Empty line separator:** `ls` separates sections with a blank line before the header. The scanner treats blank lines as potential section boundaries but does not require them.
6. **V1 fast path preserved:** If the command has exactly one positional directory arg and no `-R` flag, V1's direct resolution is used (no output scanning).
7. **Ambiguous headers:** If a line looks like a section header but the path doesn't resolve to a directory, it's not treated as a header (fall through to V1/pwd resolution).
8. **Relative headers:** Resolution base depends on the listing mode:
   - **Recursive (`ls -R DIR/`):** Headers like `./src:` or `subdir:` resolve relative to the listing root (the `-R` directory argument). If no directory argument, resolve relative to pwd.
   - **Multi-directory (`ls DIR1/ DIR2/`):** Headers like `DIR2/:` or `~/b/:` are already rooted paths as printed by `ls`. Resolve against pwd directly (they contain the full relative or absolute path).
9. **No regression:** If section parsing fails or produces no match, the system falls back to V1 -> pwd, in that order. No currently-working links break.
10. **Scope gate:** Section-aware resolution only activates when the command is a recognized listing command AND has either multiple directory positionals OR the `-R`/`--recursive` flag.

## Edge Cases

| Input | Behavior |
|---|---|
| `ls -R ~/projects/` with `./src:` header | Filenames under `./src:` resolve against `~/projects/src/` |
| `ls -R` (no dir arg, recursive from pwd) | Headers like `./src:` resolve against pwd |
| `ls ~/a/ ~/b/` where both contain `README.md` | Each `README.md` resolves against its own section's directory |
| `ls -R ~/projects/` with deeply nested `./a/b/c:` | Resolves against `~/projects/a/b/c/` |
| Header-like line in file content (e.g. `config:`) | Ambiguity check: `config` is not a directory -> not treated as header -> falls back |
| `ls --color=auto -R DIR/` | ANSI codes stripped before header matching |
| Very long output (10000+ lines) | Backward scan bounded (max 500 lines back) to avoid perf issues |

## Success Criteria

1. `ls -R ~/projects/` -> clicking `main.rs` under `./src:` header opens `~/projects/src/main.rs`.
2. `ls ~/a/ ~/b/` -> clicking `file.txt` under `~/b/:` header opens `~/b/file.txt`.
3. `ls -R` (no arg) -> clicking `helper.rs` under `./utils:` opens `$PWD/utils/helper.rs`.
4. V1 single-dir case still works without output scanning (fast path).
5. No performance regression on hover for non-listing commands.
6. Fallback chain: section-dir -> V1-dir -> pwd. No broken links.

## Resolved Questions

1. **`eza`/`lsd` header format:** Confirmed — all three tools (`ls`, `eza -R`, `lsd -R`) use the same `path:` section header format for recursive and multi-directory output. No regex variants needed. `eza -T`/`--tree` uses tree-drawing characters (different format, out of scope — blocked on #9909/#10004).

## Open Questions

1. **Scan depth limit:** How far back should we scan for a header? 500 lines? Configurable? Recommendation: 500 lines, hardcoded initially.
2. **Should V2 ship with V1 or as a follow-up PR?** Recommendation: follow-up PR. V1 establishes infrastructure, V2 adds output parsing on top.
