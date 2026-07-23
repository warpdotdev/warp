# Mid-session file attachment for Cloud Claude Code follow-ups

## Context

Users can send follow-up prompts to an active cloud CC session via the ambient agent pane, but currently there is no way to attach a file to those follow-up messages. This is a pure Warp client change — no server or oz binary changes are needed.

**Why no server changes are needed:** each follow-up triggers a new execution on the remote worker. The oz binary calls [`fetch_and_download_attachments`](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/attachments.rs#L36) at the start of every execution, which fetches all current task attachments via GraphQL and downloads them to `attachments_dir`. The server's `POST /api/v1/agent/runs/:runId/attachments/prepare` endpoint already works on in-progress tasks with no state guard — so uploading a new file between follow-ups is sufficient. The next execution picks it up automatically.

**Relevant code:**

- [`submit_cloud_followup` (L1101)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/view/ambient_agent/model.rs#L1101) — current follow-up entry point; takes only `prompt: String`, no attachments
- [`spawn_agent` (L1248)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/view/ambient_agent/model.rs#L1248) — the initial-run equivalent; already accepts `Vec<AttachmentInput>`
- [`prepare_attachments_for_upload` (L1415)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/server/server_api/ai.rs#L1415) — calls `POST .../attachments/prepare`, returns presigned GCS upload URLs
- [`process_attachment` (L279)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/attachments.rs#L279) — encodes a local file as `AttachmentInput`; used today for initial-run attachments
- [`MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY` (L25)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/ai/agent_sdk/driver/attachments.rs#L25) — limit of 25 attachments per task (cumulative across all follow-ups)
- [`request_attachments` building (L4438)](https://github.com/warpdotdev/warp/blob/b018f09de24a091db686d656a2adb7f3b797dc9f/app/src/terminal/input.rs#L4438) — how the initial-run UI processes files into `AttachmentInput` before send

## Proposed changes

All changes are in `app/src/terminal/` and its dependencies.

### 1. Follow-up input UI

Add a file-attach button (paperclip or similar) to the follow-up prompt composer in the cloud agent pane — the input that appears after a CC session is running. This mirrors the file picker already present on the initial-run prompt (`FileAttachmentInput` pattern). Single-file or multi-file selection; respect `MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY`.

### 2. Upload before send

When the user hits send with a file attached:
1. Call `prepare_attachments_for_upload` for each file — gets presigned GCS upload URLs
2. `PUT` each file's bytes to its GCS URL (same presigned-upload path used for initial-run attachments)
3. Block the send button and show an upload progress indicator during this step
4. Only after all uploads complete, call `submit_cloud_followup`

The upload must finish before the follow-up is submitted, otherwise the new execution may start before the attachment metadata is written to the task definition. `prepare_attachments_for_upload` writes the attachment metadata to the task definition synchronously before returning the presigned URL, so the race window is only between the GCS PUT completing and the follow-up being submitted — blocking send until PUT completes eliminates it.

### 3. `submit_cloud_followup` — no signature change needed

The function itself does not need to change. The upload is a client-side pre-step; once the files are in GCS and the metadata is in the task definition, the follow-up can be submitted with the existing text-only API. The next execution's `fetch_and_download_attachments` call picks up the new files.

The only optional improvement: pass the uploaded attachment IDs alongside the follow-up message text as a hint to the user prompt (e.g. "I've attached: `filename.py`"). This is a UX nicety, not a requirement — the agent will reference the file via the "Attached files:" block in its system prompt regardless.

## Testing and validation

**Manual:**
1. Start a cloud CC run.
2. Once the session is running (shared session visible), attach a Python file to the follow-up input and send "Summarize this file."
3. Confirm CC reads the file and produces a summary — the "Attached files:" block should appear in the new execution's prompt.
4. Attach a second file in a third follow-up; confirm both the first and second files are still available (cumulative).
5. Confirm the send button is disabled while the upload is in progress and re-enables on failure.

**Error path:**
- Upload fails (network error): show an error toast, do not submit the follow-up, allow retry.
- Attachment limit exceeded: disable the attach button or show an inline error when the 25-file task limit is reached.

**Existing behavior:**
- Follow-up sends without attachments still work (no regression).
- Initial-run file attachment behavior is unchanged.

## Parallelization

Not warranted — this is a focused UI change in one module with a clear sequential flow (UI → upload → submit). Single developer, single PR.
