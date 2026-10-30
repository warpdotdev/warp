# Product Spec: Resolve clickable filenames in listing-command output against the listed directory

**Issue:** [warpdotdev/warp#9908](https://github.com/warpdotdev/warp/issues/9908)

## Summary

When a user runs `ls DIR/` (or `eza DIR/`, `lsd DIR/`, etc.), the terminal output contains bare filenames relative to `DIR`, not to the block's working directory. Clicking these filenames currently resolves them against `pwd`, which silently opens wrong files (if a same-named file exists in `pwd`) or fails to resolve at all.

This spec defines the behavior for resolving clickable filenames against the listed directory when the block's command is a recognized directory-listing command.

## Problem

Warp's file-link detection scans terminal output for path-like strings and resolves them against the block's `pwd`. This works for most commands, but breaks for listing commands with a directory argument:

- `ls ~/projects/` outputs `README.md`, `src/`, etc. — these are relative to `~/projects/`, not `pwd`.
- If `pwd` also contains a `README.md`, clicking opens the wrong file with no indication of the error.
- If `pwd` does not contain the file, the link simply doesn't resolve — a false negative.

This is the most common case of file-link misresolution because `ls DIR/` is one of the most frequent terminal commands.

## Goals

- Bare filenames in listing-command output resolve against the listed directory, not just `pwd`.
- Existing resolution behavior is preserved for all non-listing commands.
- The feature works with common aliases (`ll`, `la`, `l`) via Warp's existing alias resolution.
- The set of recognized listing commands is extensible (future: user setting).

## Non-goals

- Recursive listing (`ls -R DIR/`) — output interleaves multiple subdirectory headers; per-section tracking is a separate feature.
- Multi-operand listing (`ls DIR1/ DIR2/`) — output sections are not reliably separable without parsing column headers.
- Piped output (`ls DIR/ | grep foo`) — Warp cannot determine which command produced the output in a pipeline.
- `tree` output — blocked on box-drawing character tokenization (#9909).
- Shell variable expansion in arguments (`ls $MYDIR`) — the terminal sees the expanded command string, so this works if the shell expands before execution; unexpanded variables are out of scope.

## Behavior Invariants

1. **Single directory operand:** When a listing command has exactly one positional argument that resolves to an existing directory, bare filenames in the output resolve against that directory first, then fall back to `pwd`.
2. **No directory operand:** When a listing command has no positional arguments (e.g. bare `ls`), resolution uses `pwd` only (unchanged behavior).
3. **Multiple positional arguments:** When a listing command has more than one positional argument, resolution uses `pwd` only (unchanged behavior — we cannot determine which section a filename belongs to).
4. **`-d` / `--directory` flag:** When present, the listing command lists the directory entry itself, not its contents. Resolution uses `pwd` only (the output names the operand, not entries inside it).
5. **Alias resolution:** Shell aliases are resolved via `Block::top_level_command`. If `ll` resolves to `ls`, the feature activates. Aliases with baked-in positional arguments (e.g. `alias lt='ls /tmp'`) are not expanded — only the alias-to-command mapping is used.
6. **Fallback to pwd:** If a filename does not resolve against the listed directory, resolution falls back to `pwd`. This ensures no currently-working links are broken.
7. **Non-listing commands:** Commands not in the recognized set are completely unaffected.
8. **Recognized commands:** `ls`, `eza`, `exa`, `lsd` (configurable in future).
9. **Flag-argument consumption:** Flags that take arguments (e.g. `--color=auto`, `-I PATTERN`) do not consume the directory operand. Argument classification uses the existing command-signatures infrastructure.
10. **Env-var prefixes:** Leading `KEY=VALUE` assignments (e.g. `LANG=C ls DIR/`) are stripped before command identification (existing parser behavior).

## Edge Cases

| Input | Behavior |
|---|---|
| `ls` | No directory arg → pwd-only resolution (unchanged) |
| `ls -la ~/projects/` | Single dir arg → resolve against `~/projects/` |
| `ls file1.txt file2.txt` | Multiple positional args, none are dirs → pwd-only (unchanged) |
| `ls ~/projects/ ~/other/` | Multiple dir args → pwd-only (cannot attribute output lines) |
| `ls -d ~/projects/` | `-d` flag present → pwd-only (output names the operand itself) |
| `ll ~/projects/` | `ll` resolves to `ls` via alias → feature activates |
| `PAGER=cat ls ~/projects/` | Env prefix stripped → `ls` identified → feature activates |
| `ls "path with spaces/"` | Quoted arg handled by parser → resolves correctly |
| `ls $(echo dir)` | Subshell in arg → cannot statically resolve → pwd-only fallback |

## Success Criteria

1. `ls ~/projects/` → clicking `README.md` in output opens `~/projects/README.md`.
2. `ls -la ~/projects/` → same behavior (flags don't interfere).
3. `ll ~/projects/` (where `ll` is aliased to `ls -l`) → feature activates.
4. `ls` (no args) → behavior unchanged from today.
5. `ls -d ~/projects/` → clicking `projects` resolves against `pwd` (not `~/projects/`).
6. `ls ~/a/ ~/b/` → behavior unchanged (multi-dir fallback to pwd).
7. No regression: commands that are not listing commands behave exactly as before.

## Open Questions

1. **Should this be a user-configurable setting?** zachlloyd suggested making the command list configurable. Recommendation: ship with a hardcoded list first, add a setting if there's demand.
2. **Should multi-operand be attempted?** If both args are dirs, we could try each. Risk: false positives if dirs share filenames. Recommendation: defer to follow-up.
