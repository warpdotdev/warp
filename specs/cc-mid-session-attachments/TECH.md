# File attachment for Cloud Claude Code runs

## Context

Cloud Warp (Oz) sessions support file attachment in two ways: while the session is live (via session sharing), and when the VM is down (via follow-up). Cloud Claude Code sessions support neither. This spec covers both gaps.

**Why Oz works but CC doesn't — the fundamental difference:**

Oz runs in-process inside the Warp application on the worker VM. When a user sends a message with an attachment, it arrives as a structured `AgentPromptRequest { prompt, attachments }` over the session-sharing WebSocket. On the VM, Oz downloads the file references from GCS to VM-local paths and feeds them as typed `AIAgentAttachment::FilePathReference` objects into `AIAgentInput::UserQuery`. This is a structured, typed protocol.

CC is an external PTY process. The session-sharing message arrives with the same structured payload, but at [`terminal_view_adaptor.rs:1357`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/local_tty/terminal_view_adaptor.rs#L1357), the CC branch does:

```rust
if cli_agent_active {
    submit_text_to_cli_agent_pty(request.prompt.clone(), ...);
    return;  // request.attachments is never read
}
// Oz branch follows — handles request.attachments
```

Attachments are silently discarded. CC's only input interface is raw PTY bytes.

**For follow-ups (VM down):**

[`RunFollowupRequest`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/server/server_api/ai.rs#L327) is `{ pub message: String }` — no attachment field. The `NewCloudVm` routing branch at [`input.rs:4118`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/input.rs#L4118) only collects the prompt text and never reads pending attachments — they are simply never included, not explicitly dropped. Each follow-up spawns a new execution which runs `fetch_and_download_attachments` at startup — so attachments just need to be in the task definition before the new execution starts.

**Relevant code:**

- [`terminal_view_adaptor.rs:1357–1401`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/local_tty/terminal_view_adaptor.rs#L1357) — the decisive branch: CC takes PTY-only path, Oz takes structured path with full attachment download
- [`shared_session.rs:669–815`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/blocklist/controller/shared_session.rs#L669) — Oz: downloads FileReferences, builds `AIAgentAttachment::FilePathReference`, calls `send_user_query_in_conversation_with_attachments`
- [`input.rs:14474–14839`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/input.rs#L14474) — viewer-side: uploads files via `prepare` endpoint, creates `AgentAttachment::FileReference { attachment_id, file_name }`
- [`input.rs:4118–4130`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/input.rs#L4118) — `NewCloudVm` routing branch: collects prompt text only, never touches pending attachments
- [`model.rs:1101`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/view/ambient_agent/model.rs#L1101) — `submit_cloud_followup(prompt: String)` — text only
- [`fetch_and_download_attachments` (L36)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/attachments.rs#L36) — called at every new execution startup; downloads all task attachments fresh via GraphQL
- [`terminal.rs:266–273`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/terminal.rs#L266) — `set_attachments_download_dir` set on VM controller at session start
- [`MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY` (L25)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/attachments.rs#L25) — 25 attachments per task cumulative

## Proposed changes

### Fix 1: Follow-up (VM down)

Each follow-up triggers a new execution, and `fetch_and_download_attachments` runs at every new execution startup. So the fix is: upload the file to the task definition before submitting the follow-up text. No changes to `RunFollowupRequest` or the server are needed.

**a. File picker on follow-up input UI**

The file picker already exists. `PendingAttachment::File(PendingFile)` is a live type in `context_model.rs`, `attach_files()` opens the OS file dialog, and `collect_cloud_launch_attachments` at `input.rs:4441` already reads `pending_files()` for the initial cloud run. The follow-up input uses the same `ai_context_model`, so no new UI component is needed — the attachment button is already wired up; the gap is only in the submission path.

**b. Upload before send**

When the user sends a follow-up with a file:
1. In the `NewCloudVm` branch at `input.rs:4118`, collect `self.ai_context_model.as_ref(ctx).pending_files()` (same source as the initial-run path)
2. Call `prepare_attachments_for_upload` — writes attachment metadata to task definition, returns presigned GCS URL. Unlike the initial-run path (which base64-encodes inline), use the presigned-upload path so the file lands in GCS via the task definition
3. `PUT` file bytes to GCS via an async `ctx.spawn`; block the send button and show a progress indicator while in flight
4. On upload completion, emit `Event::SubmitCloudFollowup { prompt }` normally

Also remove the explicit warn-and-drop at [`input.rs:13928`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/input.rs#L13928) in the queued-prompt cloud follow-up path, which currently logs a warning and discards queued attachments rather than uploading them.

The file is in the task definition before the follow-up is submitted. The new execution's `fetch_and_download_attachments` picks it up automatically. Race-free: GCS PUT completes before submit; new execution starts after submit with seconds of container-startup delay.

**c. No changes to `submit_cloud_followup` or `RunFollowupRequest`**

The function signature and server API stay text-only. The attachment is delivered via the task definition, not the follow-up message itself.

---

### Fix 2: Live session (VM running)

The viewer-side upload + session-sharing transport already works correctly: files are uploaded to GCS, `AgentAttachment::FileReference { attachment_id, file_name }` is built at `input.rs:14474`, and `AgentPromptRequest { prompt, attachments }` is sent over the WebSocket. The gap is entirely on the VM/sharer side in `terminal_view_adaptor.rs`.

**Change `terminal_view_adaptor.rs:1357–1381`:**

Currently the CC branch calls `submit_text_to_cli_agent_pty(request.prompt.clone())` and returns, discarding `request.attachments`. Replace with:

1. If `request.attachments` is non-empty, for each `FileReference { attachment_id, file_name }`:
   - Call `download_task_attachments` to get a fresh presigned download URL
   - Download the file to a deterministic VM-local path using the `attachments_dir` already set on the controller at `terminal.rs:266–273`
   - Append a file-path note to the prompt text: `\n\nThe following file(s) have been placed on the filesystem:\n- {file_name} → {path}`
2. Send the augmented prompt text to CC via PTY

**`HarnessRunner` does not need a new method.** The augmented text goes through the same `submit_text_to_cli_agent_pty` path — file path injection is string concatenation before the PTY write. The CC plugin's `on-prompt-submit` hook fires normally.

---

## Testing and validation

**Fix 1 (follow-up):**
1. Start a cloud CC run and wait for execution to end.
2. Attach a Python file to the follow-up input and send "Summarize this file."
3. Confirm the new execution's "Attached files:" block includes the file and CC produces a summary.
4. Attach a second file in a subsequent follow-up; confirm both files are available (cumulative).
5. Confirm send is blocked during upload; on failure, an error toast appears and the follow-up is not submitted.

**Fix 2 (live session):**
1. Start a cloud CC run and wait for the session to be live (CC actively running).
2. Attach a Python file to the active session input and send "Summarize this file."
3. Confirm CC receives a message with the local file path appended and reads the file.
4. Confirm the file is present at the deterministic path on the VM.
5. Confirm no raw attachment ID or marker text appears in the CC conversation.

**Regressions:**
- Follow-up without attachments still works.
- Initial-run attachments unchanged.
- Oz live session and Oz follow-up attachment behavior unchanged.

## Risks and mitigations

**PTY injection for live session:** File paths injected as text work for any file type — CC reads the file via its `Read` tool using the path. Same mechanism as pre-session attachments already in prod.

**Attachment download latency on live session:** Downloading from GCS on the VM before sending to PTY adds latency. The viewer UI should show "uploading…" so the user knows the message is in flight.

**Cumulative 25-file limit:** Attachments from live-session messages persist in the task definition. The UI should track and enforce the cumulative limit.

## Parallelization

Two PRs, can be developed in parallel since they touch different code paths:
- **PR 1** — Follow-up: async upload-before-submit in `NewCloudVm` branch + fix queued-prompt drop at `input.rs:13928` (file picker UI already exists)
- **PR 2** — Live session: CC attachment download + PTY injection (`terminal_view_adaptor.rs`)
