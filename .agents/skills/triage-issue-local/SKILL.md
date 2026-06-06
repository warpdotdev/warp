---
name: triage-issue-local
specializes: triage-issue
description: Repo-specific triage guidance for warp. Only the categories declared overridable by the core triage-issue skill may be specialized here.
---

# Repo-specific triage guidance for `warp`

This file is a companion to the core `triage-issue` skill. It does not
redefine the triage output schema, safety rules, or follow-up-question
contract. It only specializes the override categories the core skill
marks as overridable.

## Heuristics

- `warp` is the public-facing Warp desktop client repository. Treat public issue reports as potentially incomplete and avoid asking for secrets, tokens, private workspace names, private repository names, or account identifiers in the public issue thread.
- Distinguish the user's observed Warp behavior from their guesses about Rust modules, UI components, server behavior, feature flags, or product intent.
- For issue reports that mention another terminal, editor, shell, or CLI tool, identify whether the problem is Warp-specific or generally reproducible outside Warp before assigning Warp ownership.
- When the issue includes screenshots, videos, logs, stack traces, or command output, use them as primary evidence and ask follow-up questions only for missing details that cannot be inferred from that evidence.
- Before asking any follow-up questions, check the Warp documentation and the repository's existing feature set to determine whether the desired behavior the reporter is describing is already supported. If an existing feature, setting, or workflow satisfies the request, recommend it to the reporter instead of treating the issue as a bug or feature gap.
- If the report is about billing (pricing, plans, subscriptions, payments, refunds, invoices, AI request quotas, charges) or about appeals (account suspensions, bans, takedowns, abuse decisions, or other account-status disputes), do not attempt to triage it as an actionable bug or feature request. Instead, notify the reporter that these requests must go through Warp's support channels (https://docs.warp.dev/support-and-community/troubleshooting-and-support/sending-us-feedback) and direct them there for resolution. Apply the relevant `area:billing` or `area:auth` label as appropriate so the issue is still routed correctly.

## Follow-up question limit

Ask **at most 2 follow-up questions** per triage response. Each question must be high-value: it should meaningfully change the label assignment, owner routing, or reproduction confidence if answered. Do not ask questions whose answers can be inferred from existing evidence, and do not bundle multiple sub-questions into a single bullet. If more than 2 unknowns exist, prioritize the two that are most likely to unblock triage.

## Label taxonomy

The label taxonomy for this repository is managed in `.github/issue-triage/config.json`. Prefer labels from that configuration, especially the `area:*`, `os:*`, `repro:*`, `accessibility`, `needs-info`, `duplicate`, and primary issue-type labels. Do not invent new labels unless the prompt explicitly allows it.

Evaluate `ready-to-implement` during triage instead of relying on issue-template defaults. For bug reports, apply `ready-to-implement` only when the issue is reproducible from the provided evidence or straightforward local verification and the likely fix appears narrow enough to implement without a product spec, design mocks, or substantial investigation. If the bug is not reproducible, lacks a clear fix path, requires product/design decisions, or needs deeper technical discovery, omit `ready-to-implement` and prefer `needs-info`, `ready-to-spec`, `needs-mocks`, or the appropriate `repro:*` label.

Use area labels based on the user's reported surface:

- `area:shell-terminal` for terminal output, block rendering, shell integration, prompt rendering, command execution display, and terminal-emulation behavior.
- `area:terminal-input` for command-line input editing, cursor movement, key handling, and typed text behavior.
- `area:window-tabs-panes` for window, tab, pane, split, layout, and focus behavior.
- `area:editor-notebooks` for editors, notebooks, markdown rendering, LSP, and code display.
- `area:agent` for agent conversations, agent mode, cloud/local agent execution, prompts, and AI-specific UI.
- `area:code-review` for git diff views, review UI, review comments, and PR-focused agent flows.
- `area:mcp` for MCP server connection, tool/resource discovery, OAuth, and integration issues.
- `area:settings-keybindings` for settings UI, preferences, keyboard shortcuts, and keybinding configuration.
- `area:warp-drive` for Warp Drive objects, sync, sharing, workflows, notebooks, tab configs, and persisted artifacts.
- `area:performance:*` when the report includes CPU, memory, GPU, startup, rendering, latency, or responsiveness symptoms. Add the more specific CPU, memory, or GPU label when the evidence points to that resource.

## Information to check for before asking follow-up questions

Before asking the reporter for more information, check the issue body, comments, attachments, logs, labels, and repository context for:

- Warp channel and version/build number, especially whether the report is for Dev, Canary, Preview, Beta, or Stable.
- OS and version, architecture, display setup, window manager or desktop environment on Linux, and whether the issue is platform-specific.
- Shell and terminal context: shell name/version, prompt framework, shell integration status, command being run, terminal mode, local vs SSH/remote/tmux, and whether the behavior reproduces in a fresh session.
- Clear reproduction steps, expected behavior, actual behavior, frequency, regression timing, and whether the user can reproduce outside Warp.
- Visual evidence for UI, rendering, layout, font, cursor, focus, window, pane, tab, and accessibility issues. Prefer a screenshot or short recording when the symptom is visual.
- Logs and diagnostics for crashes, hangs, startup failures, update failures, authentication failures, MCP failures, and agent execution failures. Ask for redacted logs only when the report lacks actionable evidence.
- For AI/agent reports: whether the agent is local or cloud, the model if known, relevant conversation/session link, repository context, tool or MCP server involved, and the exact user action that triggered the failure.
- For performance reports: approximate project/session size, command output size, CPU/memory/GPU observations, profile or diagnostics if provided, and whether the issue appears after long-running sessions.
- For keyboard or input reports: keyboard layout, custom keybindings, IME usage, conflicting OS shortcuts, focused surface, and whether the same keys work in other apps.
- For account, billing, or auth reports: account tier or authentication method only if the user already provided it. Do not ask for private identifiers in public; direct the user to support when private account details are required. For billing or appeals reports specifically, do not pursue further triage questions in the public thread—redirect the reporter to Warp's support channels per the heuristic above.

## Recurring follow-up patterns

- Visual UI/rendering issue with no media: ask for a screenshot or short screen recording first.
- Environment-sensitive terminal issue: ask for Warp version/channel, OS/version, shell, and whether it reproduces in a fresh local session.
- SSH/tmux/remote issue: ask for local OS, remote OS, shell, whether tmux is involved, and the minimal command or workflow that reproduces it.
- Agent/MCP issue: ask for the failing workflow, local vs cloud execution, relevant session link, MCP server/tool name, and any redacted error text.
- Performance issue: ask for approximate scale, how long Warp has been running, what action triggers the spike or hang, and whether logs or a profile are available.

## Owner-inference hints

Prefer `.github/STAKEHOLDERS` for owner inference. When no path-level match exists, use the label and issue surface to choose likely owners rather than defaulting to broad app ownership.
