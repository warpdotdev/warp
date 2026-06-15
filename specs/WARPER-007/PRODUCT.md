# WARPER-007: terminal rendering and input correctness

## Summary

Warper should port upstream terminal fixes that improve local startup reliability, shell path detection, crash resistance, key encoding, and IME behavior for Warper's retained desktop targets. It should defer terminal visual polish and niche protocol compatibility until the local-only baseline is stable.

## Why this matters for Warper

WARPER-001 says the product must launch and remain useful as a local terminal with hosted services amputated. That makes shell startup, local tool resolution, crash resistance, and native input correctness relevant. It does not make every upstream terminal feature worth porting. Warper currently supports iTerm and Kitty images, but the startup inline-image fix is about rendering image completions before preexec output, not about Warper startup reliability. Windows-specific fixes from upstream are not part of this spec because the current WARPER specs do not define Windows as a target.

## Source commits

| Commit | Resolution | Scope |
| --- | --- | --- |
| `ae832ff6` | Port manually | Fix zsh prompt grid glitches in hidden prompt mode. |
| `0902e973` | Port manually | Harden interactive shell `PATH` capture against startup rc output. |
| `fb3cb0e9` | Port | Fix legacy Meta+Enter, Meta+Tab, and Meta+Escape encoding. |
| `388f5dc1` | Port | Fix flat-storage row iterator underflow after a clear. |
| `fc1157e0` | Port | Skip macOS key-equivalent priority path while IME is composing. |
| `3ff78d29` | Port | Do not submit forms when Enter confirms Japanese IME conversion. |
| `ab081528` | Port | Refresh IME cursor area after redraw. |
| `6d4201ba` | Port | Enable IME on Linux X11. Warper has active Linux packaging and desktop resources. |

## Deferred upstream commits

| Commit | Decision | Why not in this spec |
| --- | --- | --- |
| `09be9c1f` | Defer | Warper does render iTerm and Kitty images, but this fix is for startup image ordering from terminal programs such as `fastfetch`; it is not a startup reliability, security, or OpenRouter-local-agent fix. |
| `e59c7a49` | Defer | The public PR body frames this as a Windows PTY issue. The current WARPER specs do not define Windows as a target. |
| `1df6ff13` | Defer | The public PR body frames this as a Windows Shift+Backspace issue. The current WARPER specs do not define Windows as a target. |
| `71edcac8` | Defer | Local terminal ergonomics, not current recovery work. |
| `b7dd0ef8` | Defer | Local selection UX, not current recovery work. |
| `d426c045` | Defer | Windows non-IME input is not grounded in the current Warper product specs. |
| `03ef4d05` | Defer | Windows non-Latin keyboard shortcuts are not grounded in the current Warper product specs. |
| `2992d02e` | Defer | Windows `CreateProcessW` environment handling is not grounded in the current Warper product specs. |

## Goals / Non-goals

- Goal: preserve local terminal correctness under shell startup, local path detection, crash-prone grid state, and native input edge cases.
- Goal: improve local keyboard, IME, and selection behavior on Warper's retained desktop targets.
- Goal: keep shell compatibility values only when needed for terminal behavior.
- Non-goal: port upstream IAP, gcloud, hosted SSH, remote-session, Sentry, autoupdate, or cloud crash-reporting code touched near these commits.
- Non-goal: add Windows support or unsupported platform validation.
- Non-goal: expand Warper platform support beyond the platforms Warper intentionally builds and tests.

## Behavior

1. Zsh prompt width-glitch constructs do not corrupt visible command-grid layout.
2. Shell `PATH` capture tolerates user rc files that print startup output before Warper's sentinel messages.
3. Legacy Meta-key encodings match expected terminal behavior.
4. Terminal grid storage remains stable after clear operations and cannot underflow row iteration.
5. macOS IME composition does not trigger app shortcuts, form submission, or stale cursor-area positioning.
6. Linux X11 users can use IME input in terminal and editor surfaces only if Warper keeps Linux UI builds in scope.
7. Any upstream changes to hosted, remote-only, visual-polish-only, or Windows-only files are omitted unless Warper has a matching retained target and a reproduced local bug.

## Validation

- Add targeted unit or integration coverage for zsh prompt rendering and shell `PATH` sentinel parsing.
- Add input tests for Meta-key legacy encodings.
- Add IME tests or manual validation notes for macOS. Add Linux X11 validation only if the Linux UI build remains a supported Warper target.
- Run the terminal-session smoke path after porting and confirm no network attempts are added.
