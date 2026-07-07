# TUI Ctrl-C: Cancel / Clear on Single Press, Exit on Double Press

## Summary

Match the ctrl-c behavior of peer agent CLIs (Claude Code, Amp) in the Warp TUI: a single ctrl-c performs one contextual action — cancel the streaming agent response, or clear the input — and a second ctrl-c within a short window fully exits the TUI. A footer hint line below the input shows "ctrl-c again to exit" while the exit confirmation is armed.

## Problem

Previously a single ctrl-c force-terminated the entire TUI. This makes accidental exits easy and gives users no keyboard affordance to stop a streaming agent response or clear a half-typed prompt — both standard, muscle-memory behaviors in comparable agent CLIs.

## Goals

- Single ctrl-c cancels the in-progress agent conversation (token streaming and in-flight tool calls) when one is running.
- Single ctrl-c clears the input buffer when the agent is idle and the input is non-empty.
- Double ctrl-c (second press within 1 second) always exits the TUI, regardless of state.
- Surface a "ctrl-c again to exit" hint in a footer line below the input while the exit confirmation is armed.
- The TUI must always remain exitable, including on pre-login screens (which say "Press Ctrl-C to exit").

## Non-goals

- The full footer design (contextual key hints like "↑ to edit • Esc to stop • ← for conversations" and the right-hand model/cwd/branch section). This change only introduces the footer row and its left hint slot.
- `Esc` to stop streaming (shown in the Figma design) — a natural follow-up reusing the same cancel path.
- Interrupting an agent-requested shell command that is already executing in the PTY (`CancelExecution` wiring).

## Figma

- Footer hint line below the input: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-17252

## User Experience

### Single ctrl-c (one contextual action per press)

Evaluated in priority order:

1. **Agent running** (conversation in progress or blocked): cancel the conversation. Input content is preserved.
2. **Agent idle, input non-empty**: clear the input buffer.
3. **Agent idle, input empty**: no action beyond arming the exit confirmation.

### Double ctrl-c exits

Every ctrl-c press arms (or re-arms) a 1-second exit-confirmation window. A second ctrl-c while the window is armed exits the TUI unconditionally — this is what makes "double ctrl-c" exit even when the first press cancelled a stream or cleared the input. The window expiring silently disarms the confirmation.

### Footer hint

- A one-row, dim-styled footer renders below the input box.
- While the exit confirmation is armed, it shows `ctrl-c again to exit`; the hint disappears when the window lapses.
- Typing into the input (making it non-empty) disarms the confirmation and hides the hint.

### Pre-login screens

On the login placeholder / login-failed screens (no terminal session yet), a single ctrl-c exits immediately, matching their "Press Ctrl-C to exit." copy.

## Edge Cases

1. **Fresh conversation**: a brand-new conversation reports "in progress" before any exchange exists; it is not treated as cancellable work, so the first ctrl-c is not silently consumed by it.
2. **Cancel during tool execution**: TUI conversations auto-execute to completion, so "running" includes tool calls, not just token streaming; cancellation covers both.
3. **Ctrl-c while streaming with non-empty input**: only cancels the stream (input preserved); a later single ctrl-c (after the window lapses) clears the input.
4. **Exit mid-stream**: double ctrl-c exits even while the agent is streaming; process teardown ends in-flight work. (This intentionally differs from the GUI, which blocks exit while in progress — matching Claude Code/Amp per product decision.)
5. **Rapid re-presses after the window lapses**: each press re-runs the contextual action and re-arms a fresh window; only a press inside an armed window exits.
6. **Modified ctrl-c**: only a plain ctrl-c (no alt/shift) triggers this behavior.

## Success Criteria

1. Ctrl-c during an agent response stops the stream/tool calls and does not exit the TUI.
2. Ctrl-c with idle agent and non-empty input clears the input and does not exit.
3. A second ctrl-c within 1 second exits the TUI cleanly (terminal restored from raw mode / alternate screen).
4. The footer shows "ctrl-c again to exit" after the first press and hides it ~1 second later if no second press arrives.
5. Ctrl-c on pre-login screens exits immediately.

## Validation

- Run `script/run-tui`; send a prompt and press ctrl-c mid-stream — the response stops, the hint appears, the app stays open.
- Type text while idle and press ctrl-c — the input clears, the hint appears.
- Press ctrl-c twice quickly (from idle, mid-stream, and with text in the input) — the TUI exits with a clean terminal.
- Press ctrl-c once and wait — the hint disappears after ~1 second and a later ctrl-c does not exit.
- Press ctrl-c once, type a character — the hint disappears; ctrl-c again clears the input instead of exiting.
- On the login screen, press ctrl-c once — the TUI exits.

## Open Questions

(None outstanding — window duration of 1s and "double ctrl-c always exits, even with non-empty input" were confirmed as product decisions.)
