# TECH.md — Notify for Codex permission requests only when a human decision is required

Issue: https://github.com/warpdotdev/warp/issues/15933
Product spec: [`specs/GH15933/product.md`](./product.md)

## Context

Warp learns about a Codex permission request through three hops, and the behavior [`product.md`](./product.md) asks for is not expressible at the first one.

```text
Codex CLI  --PermissionRequest hook-->  codex-warp script  --OSC 777-->  Warp client
```

**Warp client.** The client is not where the problem lives, and it already does the right thing with the events it is given. References pinned to `a0627971`:

- [`app/src/terminal/cli_agent_sessions/event/v1.rs:15-26 @ a0627971`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/event/v1.rs#L15-L26) — the OSC 777 event names are parsed here. `tool_complete`, `permission_request`, and `permission_replied` are all already in the protocol.
- [`app/src/terminal/cli_agent_sessions/mod.rs:238-245 @ a0627971`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/mod.rs#L238-L245) — `PermissionRequest` sets `CLIAgentSessionStatus::Blocked` with the request summary.
- [`app/src/terminal/cli_agent_sessions/mod.rs:253-258 @ a0627971`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/mod.rs#L253-L258) — `PermissionReplied` returns a blocked session to `InProgress`. [`mod.rs:216-222`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/mod.rs#L216-L222) does the same for `ToolComplete`.
- [`app/src/terminal/cli_agent_sessions/mod.rs:184-194 @ a0627971`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/mod.rs#L184-L194) — `clear_permission_scoped_state` drops the request summary whenever the session leaves the permission flow.
- [`app/src/terminal/view.rs:13804-13835 @ a0627971`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/view.rs#L13804-L13835) — a status change to `Blocked`, while the user is navigated away, becomes a `NotificationsTrigger::NeedsAttention` desktop notification. `view.rs:13783` closes rich input on the same transition.

Two consequences follow from that code. Warp's *status* is already reversible: `permission_replied` and `tool_complete` both clear `Blocked`. Warp's *notification* is not, because a delivered desktop notification cannot be withdrawn. So the only place the decision can be made correctly is before the event is emitted, which puts it in the plugin, which puts it in what Codex tells the plugin.

Worth noting: `permission_replied` is parsed and handled by the client but **no plugin script emits it today** — neither `warpdotdev/codex-warp` nor `warpdotdev/claude-code-warp` has a producer for it. It is a protocol slot with no writer.

**Plugin.** [`plugins/warp/hooks/hooks.json`](https://github.com/warpdotdev/codex-warp/blob/main/plugins/warp/hooks/hooks.json) registers five events — `SessionStart`, `Stop`, `PermissionRequest`, `UserPromptSubmit`, `PostToolUse` — and [`plugins/warp/scripts/on-permission-request.sh`](https://github.com/warpdotdev/codex-warp/blob/main/plugins/warp/scripts/on-permission-request.sh) reads the hook payload from stdin, builds a summary from `tool_name` and `tool_input`, and emits `permission_request` unconditionally. It inspects no decision because the payload carries none.

**Codex contract.** Evidence below is from Codex CLI 0.154.0 as installed on the development machine, one patch after the 0.153.4 the issue reports. The binary embeds draft-07 JSON Schema documents for every hook payload, which is a primary source for the contract's shape; it is not a substitute for Codex's published hook documentation, and each claim should be restated against that documentation before this spec is treated as settled.

1. The hook catalogue has twelve events: `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`, `Interrupt`. **None of them fires when a permission request is resolved.**

2. The `permission-request.command.input` schema carries exactly `agent_id`, `agent_type`, `cwd`, `hook_event_name`, `model`, `permission_mode`, `session_id`, `tool_input`, `tool_name`, `transcript_path`, `turn_id`. **There is no `approval_policy` and no `approvals_reviewer`.** The one adjacent field, `permission_mode`, does not substitute for them. Its schema enum is Claude Code's — `default`, `acceptEdits`, `plan`, `dontAsk`, `bypassPermissions` — rather than Codex's own `AskForApproval` (`on-failure`, `on-request`, `granular`, `never`, `untrusted`), and the only two values the hook runtime itself carries are `default` and `bypassPermissions`. Under the issue's configuration it reads `default` whether or not a reviewer will answer.

3. The `permission-request.command.output` schema lets the hook return `behavior: allow | deny`, so the hook is an input to the approval chain rather than an observer of its outcome. Three fields in it — `interrupt`, `updatedInput`, `updatedPermissions` — are documented as reserved and currently fail closed, so the surface is still being built out.

4. Automatic review is real and lives on a different surface: `ApprovalsReviewer` is `user | auto_review | guardian_subagent`, and the app-server side carries `AutoReviewRequired`, `StrictReviewRequiredNotification`, `RequestPermissionsEvent`, and a `disableAutoReview` requirement. None of this reaches a hook.

So the answer to the question this spec exists to settle is **no**: Codex offers no event that separates "a reviewer is deciding" from "a human is being asked". The work this issue needs starts with a contract change, not with a plugin edit.

One point remains observational rather than measured. That the `PermissionRequest` hook fires *before* the reviewer answers is reported by the issue author against 0.153.4 and accepted in triage (`repro:high`), and it is consistent with a hook that returns a decision into the chain — but it was not reproduced under instrumentation for this spec. It does not change the conclusion: whichever order the two run in, the payload described above carries nothing that distinguishes the outcome, and the catalogue means nothing fires afterward to correct it.

## Proposed changes

### 1. Propose a resolution-time hook event, and a way to detect it, upstream — `openai/codex`

Two things are needed, and asking for only the first leaves the plugin unable to use it safely.

**The event.** A hook event that fires at the moment Codex determines a permission request requires a human decision, carrying at least the `session_id`, `turn_id`, and `tool_name` of the request it refers to, so a consumer can correlate it with the `PermissionRequest` that opened it.

**A capability signal the hook process can read.** Without one, a plugin cannot tell "this Codex will fire the escalation event, so stay quiet at request time" from "this Codex never will, so notify now" — and guessing wrong in the quiet direction is the silent miss invariant 12 forbids. The absence of an event is not observable at the moment the decision has to be made.

The shape has a precedent in this same integration, running the other way. `plugins/warp/scripts/should-use-structured.sh` gates every structured emission on `WARP_CLI_AGENT_PROTOCOL_VERSION` and `WARP_CLIENT_VERSION`, environment variables Warp exports into the Codex process, and `build-payload.sh` negotiates `min(plugin_current, warp_declared)` from them. What is missing is the symmetric direction: Codex advertising its own hook capability to the hook process it spawns.

Either form works, in this order of preference:

1. A capability or hook-protocol version in the hook environment or payload, which lets a plugin ask "does this build fire the escalation event" rather than inferring it from a release number.
2. Codex's own version, which reduces the gate to a version comparison. `CODEX_VERSION` appears in the binary in the same symbol run as `CODEX_PLUGIN_METRICS_OUTPUT` and `CODEX_PERMISSION_PROFILE`, which suggests the plugin subprocess environment — but it was **not confirmed to reach a hook process**, because hooks do not fire in an unauthenticated `codex exec` (see Risks). If it already does reach hooks, this half of the request costs upstream nothing and only the event remains.

This is the only proposal here that satisfies the product spec, and the reason is worth stating because the cheaper alternative looks adequate until invariant 12 is applied to it:

- **Adding the reviewer configuration to the `PermissionRequest` payload is not sufficient on its own.** It would tell the plugin that *a reviewer will look at this first*, never that *the reviewer declined*. A plugin that suppressed on it would go silent on exactly the requests that decline into a human decision — trading a false notification for a missed one, which invariant 12 forbids.
- **Delaying the emission and retracting it is not available.** Invariant 4 rules out emitting and withdrawing, because a desktop notification cannot be withdrawn, and invariant 5 rules out waiting a fixed interval before emitting.
- **`PostToolUse` cannot substitute.** It fires after the tool ran, so it can tell Warp a request was resolved but cannot prevent the notification that was already sent.

The event's name and exact payload are upstream's to choose. What this spec asks for is its timing: it must fire when Codex decides to ask the user, not when the request is created.

### 2. Emit on the new event instead — `warpdotdev/codex-warp`

Once (1) exists, `hooks.json` registers the escalation event **in addition to** `PermissionRequest`, never in place of it, and a new `on-permission-escalated.sh` emits the `permission_request` OSC 777 event that `on-permission-request.sh` emits today.

Which of the two scripts actually emits is decided at runtime by the capability signal from (1), not by which hooks are registered:

- **Capability present.** `on-permission-request.sh` emits nothing, and `on-permission-escalated.sh` emits `permission_request`. This is the behavior the issue asks for.
- **Capability absent.** `on-permission-request.sh` emits `permission_request` exactly as it does today, and `on-permission-escalated.sh` never runs because the event never fires. The user gets today's over-notification rather than silence, satisfying invariant 12.

The gate belongs in `should-use-structured.sh` alongside the checks already there, so both directions of capability negotiation live in one file and a script that forgets to consult it fails toward emitting.

**Without the capability signal from (1), this change cannot be made safely at all.** A plugin that simply moved the emission would go quiet on every Codex build that does not fire the escalation event, which is a worse regression than the bug being fixed. That dependency is the reason (1) asks for two things rather than one.

The existing OSC 777 protocol needs no new event name. `permission_request` already means "blocked on the user" to the client, which is precisely what it would now mean.

### 3. Warp client — no protocol change required

With (1) and (2) in place, the client needs no change to satisfy invariants 1 through 8, 11, and 13 through 15: `permission_request` continues to mean `Blocked`, the navigated-away gate continues to apply, and the existing clearing paths continue to work.

Two product invariants may need client work, and both are deliberately left open rather than designed here:

- Invariant 9's open question, whether a request under automatic review should present as a distinct third state. A new state would touch `CLIAgentSessionStatus` at [`mod.rs:24-39`](https://github.com/warpdotdev/warp/blob/a06279712f838d01295575b0dc0f14b7a34ba049/app/src/terminal/cli_agent_sessions/mod.rs#L24-L39) and every surface that matches on it. Under (2) the smaller answer comes for free: with no event emitted, the session simply stays `InProgress`.
- Invariant 12's version skew is handled in the plugin under (2), so the client needs no version gate. If that placement turns out to be wrong, the client already receives `plugin_version` on `session_start` and could gate on it.

## Testing and validation

Product invariants are numbered in [`product.md`](./product.md); each is listed here against what would prove it.

**Warp client, `cargo nextest run -p warp -E 'test(cli_agent_sessions)'`.** The client's mapping is unchanged, so its existing coverage is the regression suite for invariants 1, 2, 14, and 15: `permission_request` maps to `Blocked`, `permission_replied` and `tool_complete` clear it, and `stop` and `stop_failure` are untouched. If invariant 9 resolves toward a distinct state, that state needs its own cases for the status mapping and for each surface that matches on `CLIAgentSessionStatus`.

**Plugin scripts, `warpdotdev/codex-warp`.** Shell-level tests over the emission decision, driving each script with a recorded hook payload on stdin and asserting the OSC 777 body:

The capability gate from (2) is what these tests are really pinning, so each case names which side of it it drives.

- Invariant 1: capability present, `PermissionRequest` alone — `on-permission-request.sh` emits nothing.
- Invariants 2 and 6: capability present, the escalation event fires — `on-permission-escalated.sh` emits `permission_request` once, with the summary built from `tool_name` and `tool_input`.
- Invariant 12: capability absent, `PermissionRequest` alone — `on-permission-request.sh` still emits `permission_request`. This is the case most worth pinning, because its failure mode is silence rather than a visible error.
- The gate's own falsifiability: a case that removes the capability signal and asserts the emission comes back. A gate that is never observed failing is not evidence, and this one decides between over-notifying and going quiet.
- Exactly one emission per request across both scripts, so a build where both paths fire does not double-notify.

**Manual validation.** The distinction under test only exists in a real Codex session, so this cannot be reduced to unit tests. On macOS, with Warp navigated away from the Codex pane:

- `approvals_reviewer = "user"` with `approval_policy = "on-request"` — every permission request notifies. Invariant 13.
- `approvals_reviewer = "auto_review"` with a request the reviewer approves — no notification, and the session keeps running. Invariant 1.
- `approvals_reviewer = "auto_review"` with a request the reviewer declines into a human decision — one notification, arriving when Codex asks. Invariants 2, 4, 6.
- Two Codex sessions in one window, one under review and one waiting on the user — exactly one notification. Invariant 11.
- The same passes against a Codex build without the new event, to confirm today's behavior survives. Invariant 12.

A screen recording of the second and third cases is the evidence worth attaching to the implementation PR, since the difference between them is a notification that does and does not appear.

## Risks and mitigations

**Registering an unknown event name in `hooks.json`.** (2) asks the plugin to register an event that older Codex builds do not know, which would matter a great deal if an unrecognized name took the rest of the file down with it. Measured against 0.154.0 with an isolated `CODEX_HOME`, it does not:

- A `hooks.json` containing an unknown event name alongside a known one starts a session with no warning and no failure.
- The negative control fired, so that silence is meaningful: a `hooks.json` whose `SessionStart` value is a string rather than a sequence produces `warning: failed to parse hooks config <path>: invalid type: string ..., expected a sequence at line 4 column 58`. The file is read, and parse problems do surface.
- That warning is a warning. Codex continued and printed its session header, so even a `hooks.json` that fails to deserialize does not stop Codex from starting.

Since the unknown-name file deserialized without a warning, the known entries in that same map were retained. Whether the sibling hook then *executes* was not observable here: the positive control did not fire either, with an omitted matcher and again with an explicit `startup|resume|clear` one, because `SessionStart` hooks do not run in an unauthenticated `codex exec` at all. That same limit is why the `CODEX_VERSION` question in (1) is left open rather than answered — a hook that never runs cannot report the environment it was given. Confirm both in an authenticated session before (2) ships, and repeat the whole probe against the oldest Codex the plugin supports rather than only against 0.154.0.

**Silence is worse than noise.** Every proposal here is shaped by invariant 12, and the review of any implementation should check the failure direction first: a change that removes notifications for requests it cannot classify is a regression even if it fixes the reported case.

**This spec depends entirely on an upstream change, and nothing here ships without it.** Nothing in (1) is Warp's to land, (2) is unsafe to attempt before (1) exists, and (3) is the observation that the client needs no work rather than work of its own. If upstream declines, the issue has no correct fix under the current contract, and the honest outcome is to say so on the issue rather than to ship a heuristic.

**The contract evidence is read from a binary.** Every claim in Context is from schemas and symbols embedded in Codex 0.154.0, not from published documentation. The shape is unlikely to be wrong, but the wording of an upstream proposal should be checked against Codex's own hook documentation first.

## Follow-ups

- Whether the other CLI agent plugins that emit `permission_request` have the same gap against their own upstreams, tracked separately per the product spec's open questions.
- `permission_replied` having no producer in either plugin. An earlier draft of this spec proposed filling that slot as an independently shippable improvement; that was wrong, and it is recorded here rather than dropped because the reasoning generalizes. A producer would need to fire on the approved, denied, abandoned, timed-out, and user-answered paths, and Codex exposes an observable for exactly one of them: the tool running, which `on-post-tool-use.sh` already reports as `tool_complete`, and which the client already uses to clear `Blocked`. So a `permission_replied` producer built on today's contract would be redundant where it worked and silent everywhere else. It becomes worth doing once (1) lands, and not before.
