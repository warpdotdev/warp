# PRODUCT.md — CLI agent rich input visibility notifications

Issue: https://github.com/warpdotdev/warp/issues/15977

## Summary

Notify a running third-party CLI agent when Warp's CLI agent rich input composer opens or closes in the same terminal pane. Agents that opt in can hide their own prompt bar while Warp's composer is visible, then restore it when the composer is no longer visible, so users do not see two competing input boxes.

The feature is a host capability exposed to the CLI agent process. It does not change how users open, close, type in, or submit from Warp rich input. Agents that do not support the capability continue to behave exactly as they do today.

## Problem

Warp can render a rich input composer for detected CLI agents such as Claude Code, Codex, Gemini, OpenCode, and Grok Build. Many CLI agents also render their own terminal-native prompt bar. When Warp rich input opens, the agent has no host signal that lets it hide its native prompt bar, so the user can see two text entry surfaces at once. This makes it unclear where typing should happen and makes the CLI agent integration feel less native.

## Goals

- Expose rich input visibility as a pane-scoped capability to local CLI agent processes started inside Warp.
- Send one state update when rich input opens and one when it closes, including all close paths: manual close, submit-driven close, auto-toggle close, session replacement, and pane teardown.
- Send the current visibility state immediately when an agent connects, so agents that start or reconnect while rich input is already open can synchronize without waiting for the next transition.
- Support multiple clients per pane. Each connected client receives the same state updates; a failed client must not block updates to other clients or the terminal.
- Make unsupported agents and unsupported transports degrade silently. If no agent connects, if an agent ignores the capability, or if the capability cannot be advertised, Warp behavior remains unchanged.
- Avoid sending user prompt text, terminal output, credentials, or agent identity over this channel.

## Non-goals

- Replacing the existing OSC 777 CLI-agent notification protocol. OSC 777 remains the agent-to-Warp status channel; this feature is a Warp-to-agent visibility channel.
- Adding a visible setting, button, or new rich input UI.
- Requiring all CLI agents to hide their prompt bars.
- Forwarding the local control endpoint through SSH, WSL, containers, or remote Warp sessions in v1.
- Accepting commands or configuration from the CLI agent. Version 1 is one-way: Warp writes state, the agent reads it.
- Changing rich input submit behavior, image paste, prompt routing, toolbar chips, or agent session detection.

## Figma / design context

Figma: none provided. No new Warp UI is required; the user-visible improvement is that cooperating CLI agents can remove their duplicate prompt bar while Warp rich input is active.

## User experience

### When the agent supports the capability

1. The user starts a supported CLI agent in a Warp pane.
2. The CLI agent discovers Warp's pane-scoped control endpoint from its environment.
3. The user opens Warp's CLI agent rich input using an existing entry point, such as the toolbar button, Ctrl-G, or auto-open behavior.
4. Warp sends `rich_input active=true` for that pane.
5. The CLI agent hides or suppresses its own prompt bar while keeping the agent session otherwise unchanged.
6. The user composes and submits through Warp rich input.
7. When Warp rich input closes, Warp sends `rich_input active=false`.
8. The CLI agent restores its native prompt bar.

### When the agent starts while rich input is already open

If a CLI agent client connects after Warp rich input is already open for the pane, the first message it receives is the current `active=true` state. The agent can hide its prompt bar immediately instead of waiting for another open/close transition.

### When the agent or channel is unsupported

If the environment variable is absent, the endpoint cannot be opened, the transport is unsupported, or the agent ignores the event, nothing visible changes in Warp. The CLI agent may continue showing its own prompt bar.

### Multiple panes

Each terminal pane has an independent visibility state and endpoint. Opening rich input in one pane must not notify or affect CLI agents running in another pane.

### Session end and pane close

When a CLI agent session ends or is replaced while rich input was active, cooperating clients should receive a final inactive state when practical. When the pane closes, the control endpoint closes and readers observe EOF.

## Behavior requirements

1. Warp advertises the capability only when it has successfully created a control endpoint for the pane and can write visibility messages to clients.
2. The advertised value is scoped to the terminal pane, not globally shared across the app.
3. Message framing is UTF-8 JSON Lines: one JSON object per line, terminated by `\n`.
4. Version 1 messages include an integer protocol version, an event name, and a boolean active state.
5. The version 1 event name is `rich_input`.
6. Opening rich input sends `active: true` exactly once per closed-to-open transition.
7. Closing rich input sends `active: false` exactly once per open-to-closed transition.
8. New clients receive the current state immediately after connecting, even if there has not been a transition since the endpoint was created.
9. Unknown future event names and fields are ignored by agents. Warp only increments the version for breaking changes.
10. Failed writes or disconnected clients are dropped without user-visible errors.
11. The channel never carries prompt text, terminal output, auth tokens, file paths from user prompts, or other sensitive session content.

## Success criteria

1. A test client connected to a pane's advertised endpoint receives `active: false` when connecting while rich input is closed.
2. Opening rich input in that pane sends a `rich_input` message with `active: true`.
3. Closing rich input in that pane sends a `rich_input` message with `active: false`.
4. A test client connecting after rich input is already open receives `active: true` as its first message.
5. Two clients connected to the same pane both receive the same open and close messages.
6. Disconnecting one client does not prevent remaining clients from receiving future messages.
7. Opening rich input in a different pane does not notify clients connected to the first pane.
8. If the endpoint cannot be created, Warp still starts the terminal session and rich input works normally; the environment variable is not set.
9. Existing CLI agent rich input flows continue to pass: open, close, submit, auto-dismiss, auto-toggle, image attachments, and text routing.

## Validation

- Unit tests for the control endpoint's JSON-line framing, initial-state-on-connect behavior, multi-client broadcast, and disconnected-client cleanup.
- Unit tests or integration tests that verify terminal startup advertises the endpoint only when endpoint creation succeeds.
- UI/model tests that verify `CLIAgentSessionsModel` open/close transitions publish the correct active state.
- Regression tests for manual close, submit close, auto-toggle close, session replacement, and pane teardown.
- Manual validation with a minimal client that connects to the advertised endpoint and logs messages while rich input is opened and closed.
- Manual validation with at least one cooperating CLI agent once agent-side support exists.

## Open questions

- Final environment variable name: the issue proposes `WARP_CLI_AGENT_CONTROL_SOCKET`. This is suitable for Unix-domain sockets, but Windows named pipes may benefit from a more transport-neutral name.
- Should the capability be gated by the existing `HOANotifications` feature flag, a new rollout flag, or the existing CLI agent rich input flag alone?
- Should WSL ever receive a translated or proxied endpoint, or should v1 explicitly omit the variable from WSL sessions?
- Should SSH/remote forwarding be designed as a follow-up for warpified remote sessions?
