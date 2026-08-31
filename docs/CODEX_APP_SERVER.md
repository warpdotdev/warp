# Embedded Codex (ChatGPT) provider for Warp Agent

This fork makes Codex available as an inference provider inside Warp's embedded Agent—the
conversation opened with **Cmd+Return**. It does not require users to launch a separate Codex
terminal UI. Warp talks to the locally installed `codex app-server` over JSONL stdio and adapts
its streamed events into Warp's native multi-agent conversation protocol.

The implementation is based on upstream Warp commit
`86cfeb9006da7865d7f27f33228ed0f581d49f02`.

## User experience

1. Install a recent Codex CLI that includes `codex app-server`.
2. Open **Settings → AI → Warp Agent → Custom Inference**.
3. In **Codex with ChatGPT**, select **Connect ChatGPT** and complete browser login.
4. Open Warp Agent with **Cmd+Return**.
5. Choose **Codex (ChatGPT)** in the model picker.
6. Submit prompts in the normal embedded Warp Agent interface.

The selected model is intercepted at Warp's Agent API boundary before a hosted Warp request is
serialized. Requests using `Codex (ChatGPT)` are therefore sent to the local app-server instead
of Warp's hosted multi-agent inference endpoint.

## Embedded provider behaviour

- Registers `Codex (ChatGPT)` as a native Agent Mode model.
- Routes Cmd+Return Agent requests through `codex app-server`.
- Converts Codex thread and turn events into Warp `ResponseEvent` values.
- Streams assistant text into Warp through `AppendToMessageContent` actions.
- Creates and updates normal Warp conversation tasks and messages.
- Resumes follow-up prompts through Codex `thread/resume`.
- Preserves visible transcript context when switching an existing hosted Warp conversation to
  Codex.
- Uses namespaced local continuation tokens so a Codex thread identifier is never sent to Warp's
  hosted backend.
- Starts a fresh hosted continuation when the user switches from Codex back to a hosted model.
- Rejects use from Warp remote sessions because the app-server and credentials are local to the
  desktop running Warp.

## Authentication

The settings-page **Connect ChatGPT** button asks Codex app-server to start its ChatGPT browser
login flow. Warp opens the URL and waits for the app-server's completion event.

Codex remains the credential owner. Warp does not copy or persist ChatGPT access or refresh
tokens. It only displays non-secret account metadata returned by Codex, such as account type,
email, and plan name.

For a headless login flow, the supplementary command surface remains available:

```sh
warp codex login --device-code
```

Other account commands:

```sh
warp codex status
warp codex status --json
warp codex status --refresh
warp codex logout
```

To use a Codex executable outside `PATH`:

```sh
export WARP_CODEX_PATH=/absolute/path/to/codex
```

or:

```sh
warp codex --codex-path /absolute/path/to/codex status
```

## Permissions and isolation

The embedded provider runs Codex with `approvalPolicy = never` because Warp's embedded approval
protocol and Codex's bidirectional approval callbacks are not interchangeable. Normal embedded
Agent use is confined to Codex's `workspace-write` sandbox.

Only Warp's explicit combination of unsupervised autonomy and no isolation maps to Codex
`danger-full-access`. Unexpected approval, broad-permission, user-input, or dynamic-tool callbacks
fail closed rather than silently granting access.

## Conversation mapping

Warp normally stores a server-issued conversation token. For this provider the token has the
form:

```text
codex-app-server:<codex-thread-id>
```

The adapter strips the namespace only when calling Codex. Before any hosted Warp request is
constructed, a Codex-namespaced token is removed. This separation prevents accidental crossover
between local Codex threads and Warp-hosted conversation identifiers.

For a new conversation, the adapter emits:

1. `StreamInit`
2. `CreateTask`
3. `AddMessagesToTask` with an empty `AgentOutput`
4. zero or more `AppendToMessageContent`/`UpdateTaskMessage` actions
5. `StreamFinished`

Existing conversations retain their visible root task and receive subsequent assistant messages
on that task.

## Current scope

The integration is intentionally local-first:

- Cmd+Return embedded Agent is supported on native desktop builds.
- Warp web builds do not expose the local provider.
- Remote Warp sessions, cloud agents, shared-session handoff, and ambient cloud runs do not route
  through the desktop-local Codex process.
- Assistant text is rendered natively in Warp. Codex command/file activity is performed by Codex
  under its sandbox but is not yet reconstructed as Warp's proprietary tool cards.
- Codex chooses its effective underlying model from its own configuration; the Warp picker exposes
  the provider as one stable `Codex (ChatGPT)` entry.

## Supplementary CLI

The fork retains a direct command interface for diagnostics and scripting:

```sh
warp codex chat
warp codex chat --prompt 'Review this repository.'
warp codex chat --read-only --prompt 'Find correctness risks.'
```

This CLI is not the primary integration. The primary path is the embedded Cmd+Return Agent model
selection described above.

## Implementation map

| Area | Path |
|---|---|
| App-server process and JSONL protocol | `crates/codex_app_server` |
| Embedded response adapter | `app/src/ai/agent/api/codex.rs` |
| Embedded request interception | `app/src/ai/agent/api/impl.rs` |
| Agent model registration | `app/src/ai/llms.rs` |
| ChatGPT account settings | `app/src/settings_view/warp_agent_page.rs` |
| Supplementary command parser | `crates/warp_cli/src/codex.rs` |
| Supplementary command runner | `app/src/codex_app_server.rs` |

## Validation

The provider has focused tests for:

- native model registration;
- Codex continuation-token namespacing;
- hosted-to-Codex transcript transfer;
- Codex delta to Warp message-action conversion;
- sandbox/autonomy mapping;
- successful stream completion;
- app-server account parsing and login challenges;
- command-line argument parsing.

Recommended validation commands:

```sh
cargo fmt --all -- --check
cargo test -p codex_app_server
cargo test -p warp_cli --lib
cargo check -p warp --lib
cargo test -p warp --lib codex -- --nocapture
cargo build -p warp --bin warp-oss
```

On macOS, Warp's native build requires a full Xcode installation with the Metal compiler.
