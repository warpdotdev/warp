# WARPER-007: terminal rendering and input correctness

## Summary

Warper should port upstream terminal fixes that improve local startup reliability, shell path detection, crash resistance, key encoding, and IME behavior for Warper's retained desktop targets. It should defer terminal visual polish and niche protocol compatibility until the local-only baseline is stable.

## Why this matters for Warper

WARPER-001 says the product must launch and remain useful as a local terminal with hosted services amputated. That makes shell startup, local tool resolution, crash resistance, and native input correctness relevant. It does not make every upstream terminal feature worth porting. Warper currently supports iTerm and Kitty images, but the startup inline-image fix is about rendering image completions before preexec output, not about Warper startup reliability. Windows-specific fixes from upstream are not part of this spec because the current WARPER specs do not define Windows as a target.

## Source commits

| Commit | Resolution | Scope |
| --- | --- | --- |
| `ae832ff6` | Port | Prevent duplicated or missing command characters in zsh hidden-prompt mode with explicit-width prompt constructs. |
| `0902e973` | Port manually | Harden terminal `LocalShellState` interactive `PATH` sentinel emission/parsing against startup rc output. |
| `fb3cb0e9` | Port | Fix legacy Meta+Enter, Meta+Tab, and Meta+Escape encoding. |
| `388f5dc1` | Port | Fix flat-storage row iterator underflow after a clear. |
| `fc1157e0` | Port | Skip macOS key-equivalent priority path while IME is composing. |
| `3ff78d29` | Port | Do not submit forms when Enter confirms Japanese IME conversion. |
| `ab081528` | Port | Refresh IME cursor area after redraw. |
| `6d4201ba` | Port | Enable IME on Linux X11, grounded in retained Linux desktop packaging and X11/Wayland split code. |

## Deferred upstream commits

| Commit | Decision | Why not in this spec |
| --- | --- | --- |
| `09be9c1f` | Defer | Warper does render iTerm and Kitty images, but this fix is for startup image ordering from terminal programs such as `fastfetch`; it is not a startup reliability, security, or OpenRouter-local-agent fix. |
| `71edcac8` | Defer | Local terminal ergonomics, not current recovery work. |
| `b7dd0ef8` | Defer | Local selection UX, not current recovery work. |

## Skipped upstream commits

| Commit | Decision | Why skipped |
| --- | --- | --- |
| `e59c7a49` | Skip | Public PR `#11906` fixes Windows PTY backpressure response loss. The bug is real, and Warper has Windows PTY code, but current WARPER specs do not target Windows. |
| `1df6ff13` | Skip | Public PR `#11563` fixes Windows Shift+Backspace behavior. Windows is not a current Warper target. |
| `d426c045` | Skip | Public PR `#10442` fixes Vietnamese IME/non-IME Windows input. Windows is not a current Warper target. |
| `03ef4d05` | Skip | Public PR `#9476` fixes Windows non-Latin chord shortcuts. Windows is not a current Warper target. |
| `2992d02e` | Skip | Public PR `#11714` fixes Windows `CreateProcessW` env handling. Windows is not a current Warper target. |
| `ebedb9fd` | Skip | Public PR `#11203` fixes localized PowerShell executable discovery. Windows is not a current Warper target. |

## Goals / Non-goals

- Goal: preserve local terminal correctness under shell startup, local path detection, crash-prone grid state, and native input edge cases.
- Goal: improve local keyboard and IME behavior on Warper's retained desktop targets.
- Goal: keep shell compatibility values only when needed for terminal behavior.
- Non-goal: port upstream IAP, gcloud, hosted SSH, remote-session, Sentry, autoupdate, or cloud crash-reporting code touched near these commits.
- Non-goal: add Windows support or unsupported platform validation. Windows-only upstream fixes are skipped until a Warper spec explicitly adds Windows desktop support.
- Non-goal: expand Warper platform support beyond the platforms Warper intentionally builds and tests.

## Behavior

1. Zsh explicit-width prompt constructs do not produce duplicated, missing, or shifted command characters in hidden-prompt mode.
2. Shell `PATH` capture tolerates user rc files that print startup output before Warper's sentinel messages. The port is limited to terminal/local-shell sentinel emission, parsing, and tests; IAP, gcloud, and server log changes are excluded.
3. Legacy Meta-key encoding sends Meta+Enter as `ESC CR`, Meta+Tab as `ESC HT`, and Meta+Escape as `ESC ESC`, not literal key names.
4. Terminal grid storage remains stable after clear operations and cannot underflow row iteration.
5. macOS IME composition does not trigger app shortcuts, form submission, or stale cursor-area positioning.
6. Linux X11 users can use IME input in terminal and editor surfaces on the retained Linux desktop target.
7. Any upstream changes to hosted, remote-only, visual-polish-only, or Windows-only files are omitted unless Warper has a matching retained target and a Warper spec that explicitly owns the platform or feature.

## Validation

- Add targeted unit or integration coverage for zsh prompt rendering and shell `PATH` sentinel parsing.
- Add input tests for Meta+Enter, Meta+Tab, and Meta+Escape legacy byte sequences.
- Add IME tests or manual validation notes for macOS and Linux X11.
- Run the terminal-session smoke path after porting and confirm no network attempts are added.
