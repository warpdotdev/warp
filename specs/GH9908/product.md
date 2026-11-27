# GH9908: Product Spec — Listing-Aware File Links for `ls DIR/` Output
## 1. Summary
Warp should resolve clickable filenames in directory listing output against the directory that was listed, not only against the block's current working directory. When a user runs commands such as `ls subdir/`, `ls -la subdir/`, or `ls /tmp/example/`, clicking `README.md` in that block should open the listed file inside the listed directory.
This spec focuses on the `ls`-style directory listing case where output rows contain bare entry names. The goal is to eliminate silent wrong-file opens, make listed files and directories clickable when they exist under the listed directory, and preserve existing behavior for commands whose output already includes rooted or relative paths.
## 2. Problem
Warp currently treats bare filenames in BlockList output as relative to the block's stored working directory. That is correct for `ls` with no path argument, but incorrect when the command lists another directory. The most harmful result is silent misresolution: if both `CWD/README.md` and `CWD/subdir/README.md` exist, clicking `README.md` in `ls subdir/` output opens `CWD/README.md`.
Users expect clicked filenames in listing output to refer to the files that were just listed. A silent wrong-file open is worse than an obvious failure because users may edit, inspect, or trust the wrong file without noticing.
## 3. Goals
- Resolve bare entry names in supported `ls`-style listing output against the listed directory when the command has exactly one supported directory-listing context.
- Prefer the listed directory over the block CWD when both locations contain an entry with the same name.
- Make files that only exist in the listed directory clickable.
- Make directories listed by `ls DIR/` clickable and open them the same way directories listed by plain `ls` open today.
- Preserve scrollback/session-restore behavior by deriving the listed directory from data stored with the block.
- Avoid regressions for non-listing commands and for output that already includes absolute or explicit relative paths.
- Keep hover/link detection responsive and avoid adding noticeable per-hover latency.
## 4. Non-goals
- Recursive listings such as `ls -R DIR/`; each output section has a different root and requires section-aware resolution.
- Multi-directory listings such as `ls DIR1/ DIR2/`; this requires per-section root tracking and should not be guessed from a single clicked filename.
- Piped or redirected output such as `ls DIR/ > file; cat file`; the original listing command context is no longer attached to the output block.
- `tree` or tree-shaped `eza --tree` output; these are blocked by separate tokenization behavior around box-drawing characters.
- Changing how URLs are detected or opened.
- Adding a new visual affordance, menu, tooltip copy, or confirmation dialog for file links.
- Implementing a broad shell command parser for every program that can print filenames.
## 5. Figma / Design References
Figma: none provided.
No visual design changes are expected. Existing file-link hover styling, cursor behavior, tooltips, and open-file routing should remain unchanged.
## 6. User Experience
### Supported commands
- The primary supported command is `ls`.
- The behavior should also apply to common `ls`-style replacement commands that Warp recognizes as listing commands, including `exa`, `eza`, and `lsd`.
- Aliases whose resolved top-level command is a supported listing command should behave like the supported command when Warp can resolve the alias from the block/session context.
- Commands outside the supported listing-command set, such as `find`, `cat`, `git`, and `tree`, should retain current file-link behavior.
### Single listed directory behavior
When a completed local BlockList block was produced by a supported listing command with one directory argument:
- Bare output entry `NAME` resolves first as `LISTED_DIR/NAME`.
- If `LISTED_DIR/NAME` exists and is a file, hovering/clicking `NAME` behaves as an existing file link and opens that file.
- If `LISTED_DIR/NAME` exists and is a directory, hovering/clicking `NAME` behaves as an existing folder link and opens the folder using the same routing as plain `ls` directory entries.
- If both `CWD/NAME` and `LISTED_DIR/NAME` exist, `LISTED_DIR/NAME` wins.
- If `LISTED_DIR/NAME` does not exist, Warp may fall back to the existing CWD-based resolver so unrelated existing behavior is preserved.
### Directory argument forms
The listed directory may be:
- Relative to the block's stored working directory, such as `ls subdir`, `ls subdir/`, or `ls ./subdir`.
- Absolute, such as `ls /tmp/warp-repro`.
- Tilde-prefixed when Warp can expand it consistently with existing file-link behavior, such as `ls ~/Downloads`.
- Quoted or escaped when the path contains spaces or shell-special characters, such as `ls "project files"` or `ls project\ files`.
- Passed after flags, such as `ls -la subdir/` or `ls --color=always subdir/`.
- Used after leading environment variable assignments, such as `LS_COLORS=auto ls subdir/`.
### Cases that should not become listing-aware
Warp should keep current behavior and avoid guessing a listed root when:
- The supported listing command has no directory argument, such as `ls` or `ls -la`.
- The first path-like positional is not an existing directory.
- The command uses multiple directory operands in one invocation.
- The command is recursive, such as `ls -R subdir/`.
- The output is in a different block from the listing command because of pipes, redirects, scripts, or `cat` of saved output.
- The detected output candidate already contains an absolute path or a rooted relative path such as `subdir/README.md`; Warp should not double-join it as `subdir/subdir/README.md`.
### Scrollback and restored sessions
- Clicking file links in older blocks should use the command and working directory associated with that block, not the terminal's current working directory.
- Blocks restored from a previous session should behave the same as live blocks when their stored command and `pwd` are available.
- Moving to another directory after running `ls subdir/` should not affect clicks in the earlier block.
### Failure behavior
- If Warp cannot parse a safe single listed directory from the command, the user should see no new failure state; existing file-link behavior should apply.
- If a listed entry no longer exists by the time the user hovers/clicks it, Warp should not link it unless the existing resolver would link another valid candidate.
- The fix should reduce silent wrong-file opens; it should not introduce a prompt or warning before opening files.
## 7. Success Criteria
1. In a block with `pwd = /tmp/warp-repro` and command `ls -la subdir/`, clicking `README.md` opens `/tmp/warp-repro/subdir/README.md`.
2. If both `/tmp/warp-repro/README.md` and `/tmp/warp-repro/subdir/README.md` exist, the listed directory file wins.
3. If only `/tmp/warp-repro/subdir/README.md` exists, `README.md` is clickable in `ls subdir/` output.
4. A directory entry listed by `ls subdir/` is clickable and opens as a folder, matching current plain `ls` behavior.
5. `ls`, `ls -la`, and other supported listing commands without a directory argument preserve current CWD-based link behavior.
6. Non-listing command output, including `find subdir -name '*.md'`, preserves current path-link behavior.
7. Output candidates that already include `subdir/README.md`, `/tmp/warp-repro/subdir/README.md`, or another explicit path are not double-prefixed.
8. Scrollback clicks in an older block continue to resolve using that block's command and stored `pwd`, even after the active shell has changed directories.
9. Restored blocks with stored command and `pwd` behave the same as live blocks.
10. The listing-aware path check does not make hover scanning visibly slower in blocks with many output rows.
11. Unsupported or ambiguous cases, including `ls -R subdir/` and `ls dir1/ dir2/`, do not silently choose an incorrect root.
## 8. Validation
- Manual: Create `/tmp/warp-repro/README.md` and `/tmp/warp-repro/subdir/README.md`, run `ls -la subdir/`, click `README.md`, and verify the subdirectory file opens.
- Manual: Remove the root `README.md`, run `ls subdir/`, and verify `README.md` remains clickable and opens the subdirectory file.
- Manual: Add a nested folder inside the listed directory, run `ls -la subdir/`, and verify clicking the folder entry opens it in Finder or the platform-equivalent folder route.
- Manual: Run `ls`, `ls -la`, `find subdir -name '*.md'`, and `cat saved-ls-output.txt` to verify existing behavior is unchanged for non-target cases.
- Manual: Run `ls -la subdir/`, `cd /tmp`, scroll back, and verify the older block still resolves against `/tmp/warp-repro/subdir/`.
- Automated: Add pure unit coverage for command-to-listed-directory parsing, including flags, quotes, escaped spaces, env var prefixes, absolute paths, tilde paths, malformed input, unknown commands, recursive listings, and multi-argument listings.
- Automated: Add link-resolution coverage showing that listed-directory resolution is attempted before CWD resolution and falls back safely when no listed directory applies.
- Performance: Validate that the command parsing and existence checks are bounded to the hovered candidate path and do not scan the whole block output on each hover.
## 9. Open Questions
- Should the supported listing-command set be user-configurable in the first implementation, or should the first implementation ship a conservative built-in set and add settings support later?
- Should multi-directory listings be completely disabled for listing-aware resolution, or is a temporary first-directory fallback acceptable? This spec recommends disabling ambiguous multi-directory listings to avoid new silent misresolution.
