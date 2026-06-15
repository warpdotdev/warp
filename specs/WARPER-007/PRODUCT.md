# WARPER-007: terminal crash and corruption port

## Summary

Warper should port upstream terminal fixes only when the retained terminal can crash or corrupt local terminal state during normal use. Under the XP bar, the current upstream terminal candidate is the flat-storage underflow fix; zsh width, PATH capture, key encoding, IME, image ordering, and Windows fixes are not implementation work without a failing Warper smoke test or user report.

## Why this matters for Warper

WARPER-001 requires a local terminal that can run without hosted services. A terminal grid underflow after ordinary clear/resize/write operations threatens that baseline because it can crash or corrupt the active terminal display. Cosmetic terminal compatibility and platform polish do not meet the same bar.

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
