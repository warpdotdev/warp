# WARPER-007: terminal crash and corruption port

## Summary

Warper should port upstream terminal fixes only when the retained terminal can crash or corrupt local terminal state during normal use. Under the XP bar, the current upstream terminal candidate is the flat-storage underflow fix; zsh width, PATH capture, key encoding, IME, image ordering, and Windows fixes are not implementation work without a failing Warper smoke test or user report.

## Why this matters for Warper

WARPER-001 requires a local terminal that can run without hosted services. A terminal grid underflow after ordinary clear/resize/write operations threatens that baseline because it can crash or corrupt the active terminal display. Cosmetic terminal compatibility and platform polish do not meet the same bar.

## What goes wrong without this

1. A normal terminal session can build enough scrollback that Warper discards old rows to stay within the scrollback limit. This is not exotic input: long command output, test logs, build logs, package-manager logs, or repeated command output can all push rows out of retained scrollback.
2. After old rows are discarded, the retained flat-storage content no longer starts at absolute offset zero. The visible rows still render because the row index and content buffer agree about the current offset.
3. A clear operation can remove every remaining flat-storage row while preserving the fact that earlier rows were already discarded. In current Warper, clear paths exist for "clear above", "clear saved history", reset-and-clear, active-block clear, and full terminal reset. From the user's side this can be a shell clear, terminal reset, app-triggered clear, or another program emitting the matching escape sequence.
4. If the user then resizes the terminal pane or window, the terminal reflows storage to the new column count. The broken behavior is that the empty row index is rebuilt as if content starts at offset zero, while the backing content buffer still has a later absolute offset from the discarded scrollback.
5. When the next output arrives, Warper pushes a new row into flat storage. The terminal can still appear normal at this moment because the failure is latent in the row index/content offset mismatch.
6. The crash happens when Warper materializes a row after that push: repainting, scrolling, copying, searching, resizing again, or any path that asks flat storage for rows can enter `RowIterator::next`. The iterator computes a content slice from the mismatched offsets and underflows, producing an out-of-range slice panic instead of terminal output.
7. The user-visible failure is not a minor rendering glitch. The active local terminal can crash after a common sequence: produce lots of output, clear, resize, then receive more output. That breaks WARPER-001 because local terminal reliability no longer depends only on the user's shell command; it depends on avoiding a normal clear/resize sequence.
8. If the underflow does not immediately panic in a given build mode, the same mismatch can materialize the wrong backing content for rows. That means the terminal can show corrupted scrollback or active output, which is worse than dropping old scrollback deliberately because the user can no longer trust what the terminal displays.

## Source commits

| Commit | Upstream why | Current Warper evidence | Resolution |
| --- | --- | --- | --- |
| `388f5dc1` | PR `#12085` fixes flat-storage underflow after clear/resize/write. | `app/src/terminal/model/grid/grid_handler.rs:310` owns flat storage; `app/src/terminal/model/grid/ansi_handler.rs:805`, `:853`, and `:893` clear it. | Port. |

## Behavior

1. Clear, resize, and subsequent writes cannot underflow flat terminal storage.
2. The fix must not add hosted startup work, telemetry, remote-session behavior, or Windows-only validation.

## Deferred Terminal Rows

| Commits | Reason |
| --- | --- |
| `ae832ff6`, `0902e973`, `fb3cb0e9`, `fc1157e0`, `3ff78d29`, `ab081528`, `6d4201ba` | Real upstream terminal/input bugs, but no current Warper stop-ship evidence. |
| `09be9c1f` | Startup inline images are visual compatibility, not Warper survival. |
| `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `ebedb9fd` | Windows-only fixes; current WARPER specs do not target Windows. |

## Validation

- Add or port the upstream flat-storage regression test for clear/resize/write underflow.
- Run the local terminal smoke path after the port.
