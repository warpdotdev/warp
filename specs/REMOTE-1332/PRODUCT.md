# End-of-Run Snapshot Upload and Client-Side Snapshot Hydration — Product Spec
Linear: [REMOTE-1332](https://linear.app/warpdotdev/issue/REMOTE-1332)
Figma: none provided

## Summary
Enable workspace state handoff between cloud agent runs and the local Warp client. At the end of every cloud agent run, the Warp client automatically captures its workspace state (git repository diffs and arbitrary files) before signaling completion and uploads it to the server, and the Warp client can download and display the terminal output snapshot of a third-party harness conversation when a user opens it.

## Problem
When a cloud agent completes work using a third-party harness (e.g. Claude Code), the user has no way to see what the agent did or resume from the agent's workspace state. The conversation history only shows metadata — the terminal output and workspace changes are lost. This makes the handoff from cloud to local feel like starting over rather than continuing a unit of work.

Two capabilities are needed to close this gap:
1. A mechanism for the cloud agent to capture and upload its workspace state (repo diffs, files) at the end of a run so that future work can resume from where the agent left off.
2. A mechanism for the Warp client to download and display the terminal output of a third-party harness conversation so the user can see what happened during the cloud agent run.

## Goals
- Automatically capture workspace state (git repo diffs and arbitrary files) at the end of every cloud agent run and upload it to the server.
- Allow the Warp client to download and display the terminal output snapshot of a third-party harness conversation.
- Make opening a cloud agent conversation that used a third-party harness feel equivalent to opening an Oz-native cloud conversation for viewing — the user sees the terminal output inline.
- Anchor workspace snapshot capture and handoff download to the cloud agent run's assigned workspace. Snapshot upload and handoff download operate inside the run's workspace and do not perform local repo switching or directory reconciliation.

## Non-goals
- Automatically applying the uploaded workspace patch to the user's local checkout. The snapshot captures state; applying it is a separate follow-up.
- Rendering the raw transcript of a third-party harness conversation inline. Only the block snapshot (terminal TUI state) is displayed.
- Resuming a third-party harness conversation via `--conversation`. This spec covers displaying saved block snapshots in the client and handing off workspace snapshots; Claude Code `--conversation` resumption is separate follow-up work.
- Supporting harnesses other than Claude Code for block-snapshot hydration. The architecture is harness-agnostic, but only Claude Code is wired up on the hydration side. Snapshot upload itself runs for both Oz and third-party harness runs.
- Streaming snapshot uploads during a run. The snapshot is uploaded once, at the end of the driver's lifecycle.
- Tracking individual file mutations performed by the agent's tool calls. Workspace state is captured at the repo level via git diff. A follow-up may add per-tool-call tracking to catch files outside any repo.
- Reconciling snapshot paths against the user's local checkout. Snapshot upload and handoff download operate inside the cloud agent workspace; local repo switching and missing-directory hints are conversation-viewer concerns, not snapshot-handoff behavior.

## User Experience

### End-of-run snapshot upload
At the end of every cloud agent run, `AgentDriver` reads a declarations file describing workspace paths to snapshot and uploads the gathered state to the server. The upload runs automatically after agent execution finishes and before the caller is signaled, with no user or agent intervention.

**Trigger:** Invoked during the `AgentDriver` run tail on every driver shutdown. The upload is skipped when:
- `FeatureFlag::OzHandoff` is disabled, or
- The run has no associated task ID (purely local agent runs with no cloud task), or
- `--no-snapshot` was specified for `oz agent run` or `oz agent run-cloud`, or
- The declarations file is missing or empty.

The upload runs *before* the caller is signaled, so on cloud task runs the CLI waits for the snapshot pipeline to complete (or hit the upload/script timeouts) before the process exits. The defaults are 2 minutes for the upload pipeline and 1 minute for the declarations script, configurable via `--snapshot-upload-timeout <DURATION>` and `--snapshot-script-timeout <DURATION>` on `oz agent run` and `oz agent run-cloud`. This adds up to ~3 minutes of default worst-case tail latency on cloud runs but avoids abandoning in-flight uploads when termination tears down the event loop.

**Declarations file:** Scoped per run at `/tmp/oz/<task-id>/snapshot-declarations.jsonl` by default, so concurrent runs never clobber each other's declarations. Falls back to `/tmp/oz/snapshot-declarations.jsonl` when no task ID is available, and can be overridden entirely via the `OZ_SNAPSHOT_DECLARATIONS_FILE` environment variable for testability or operator use.

The file is generated by `snapshot-declarations.sh`, a bash script shipped alongside `entrypoint.sh` in the `warp-agent-docker` image. Immediately before the upload pipeline reads the file, the driver invokes the script from `$OZ_SNAPSHOT_DECLARATIONS_SCRIPT` (exported by `entrypoint.sh` in containerized runs, or provided via `oz-local --docker-dir <dir>` in local dev). The driver passes the agent's `working_dir` (the workspace root assigned to the run) as the script's CWD, so the script's `$PWD` anchors the scan to the workspace regardless of where the warp binary itself was launched. Operators can override the scan roots via the colon-separated `OZ_SNAPSHOT_SCAN_ROOTS` for unusual setups. The script finds every `.git` directory with `find -prune` and appends one JSONL `repo` declaration per newly-discovered repository to the declarations file. Repeated invocations within a single run accumulate instead of clobbering (dedup is seeded from JSONL repo declarations already emitted by the script), so the pipeline can stay additive as future callers trigger mid-run snapshots.

Script-invocation failures (missing `OZ_SNAPSHOT_DECLARATIONS_SCRIPT`, missing script file, non-zero exit status, timeout) are logged at ERROR level and the pipeline continues — if the file happens to already exist from a previous invocation it will still be read, and if not the upload is a no-op. Script runtime defaults to 1 minute and upload runtime defaults to 2 minutes; users can override them with `--snapshot-script-timeout <DURATION>` and `--snapshot-upload-timeout <DURATION>`. The script itself fails if `OZ_SNAPSHOT_DECLARATIONS_FILE` is not set, so standalone invocations cannot accidentally write to a shared fallback.

Format is one JSON object per non-empty line:
```
{\"version\":1,\"kind\":\"repo\",\"path\":\"/absolute/path/to/repo\"}
{\"version\":1,\"kind\":\"file\",\"path\":\"/absolute/path/to/file\"}
```
Blank lines are ignored. Malformed lines (invalid JSON, missing fields, missing or unsupported version, unknown kind, non-absolute path) are logged at WARN level and skipped; they do not abort the upload. Duplicate `(kind, path)` pairs are ignored. The `file` entry kind is not auto-emitted by the generator script today and is reserved for operator or future-extension use (see follow-ups).

**Repo snapshots:**
- For each `repo` entry, the driver generates a binary git diff against HEAD, including untracked files.
- For each `repo` entry, the manifest also records the repo name, current branch when HEAD is attached to a branch, and current HEAD SHA when available. Metadata collection is best-effort and does not block patch generation or upload.
- If the repo is clean (no diff), the manifest records `"status": "clean"` with no patch file.
- If the repo is dirty, the diff is uploaded as a `.patch` file and the manifest records the final patch outcome as `"status": "uploaded"`, `"failed"`, or `"skipped"`.
- If the git diff itself fails (e.g. missing git binary, corrupt repo), the manifest records `"status": "gather_failed"` with no patch file and an error message.
- `status` is separate from `patch_file` so the manifest can distinguish clean repos from incomplete snapshots, and so a resume agent can understand when expected changes may be missing from the handoff.

**File snapshots:**
- For each `file` entry, the driver reads the file contents and infers MIME type.
- If the file is read, the upload outcome is recorded as `"status": "uploaded"`, `"failed"`, or `"skipped"`.
- If a file cannot be read, the manifest records `"status": "read_failed"` with the error message and the driver continues with the remaining entries.

**Manifest:**
- A `snapshot_state.json` manifest is generated from protobuf-defined snapshot types (serialized as JSON) describing all repos and files.
- Each repo manifest entry includes `repo_name`, optional `branch`, and optional `head_sha` so a later rehydration agent can restore or verify the expected repo ref before applying the uploaded patch. The server stores and serves this manifest opaquely; it does not parse or version-check the manifest contents.
- The manifest is uploaded last, after every other file has had its upload attempted, and each repo/file entry records the actual outcome (`status`, `uploaded: true/false`, plus any error message). This makes the manifest the authoritative on-GCS record of what the snapshot contains vs. what was intended.
- Patches, files, and the manifest are uploaded to presigned URLs returned by the server.

**Reliability:**
- Transient upload errors (5xx responses, network timeouts, connection resets, 408/429) are automatically retried with exponential backoff and jitter. Permanent errors (other 4xx responses) fail fast on the first error.
- Retries are strictly bounded so the run tail cannot hang: every upload (patches, files, and the manifest) attempts at most 3 times total. After the cap, the entry is recorded as `failed` and the driver moves on.
- Non-manifest uploads run concurrently so one slow file does not block others; the manifest uploads sequentially after the batch completes.
- Snapshot upload failures are never fatal to the overall `AgentDriver` run. The run's success/failure is determined by the agent's work, not by the snapshot handoff.

**Observability:**
- There is no CLI stdout consumer, so output is via logs only.
- On success (all entries uploaded), an INFO log summarizes the outcome.
- On any partial failure (gather, read, upload, or manifest-upload failure), a WARN log summarizes counts plus the failing entries so operators can see partial state without parsing INFO noise.
- The uploaded manifest remains the authoritative on-GCS record regardless of what appears in the logs.

### Block snapshot upload during harness run
While a third-party harness is running, the Oz agent driver periodically captures a `SerializedBlock` snapshot of the terminal TUI state and uploads it to the server. This happens at save points (periodic, post-turn, final) and is transparent to the user.

### Client-side conversation viewer restoration and snapshot hydration
When a user opens a cloud agent conversation in the Warp client, the client uses shared restoration plumbing and dispatches by harness type. This path restores saved conversation output for viewing; it does not resume the third-party harness process or make `--conversation` continue a Claude Code session.

1. The client fetches conversation metadata and identifies the harness type (e.g. `ClaudeCode`).
2. Oz conversations restore the native AI conversation data.
3. Supported third-party harness conversations download the block snapshot via `GET /agent/conversations/{id}/block-snapshot`.
4. The block snapshot is deserialized and inserted into the terminal model as restored block content, displaying the agent's terminal output inline.
5. The conversation appears in the conversation history and can be navigated to from the agent mode homepage.

Existing conversation-viewer directory handling (e.g. `cd`-ing into the saved working directory when it exists) is unchanged and is not part of the snapshot handoff contract.

### Handoff snapshot attachment download
When a subsequent execution of the same cloud agent run starts, the embedded Warp client downloads the handoff snapshot files that the prior execution uploaded so the server-side rehydration prompt's `{attachments_dir}/handoff/<filename>` references resolve to real files on disk. This includes `snapshot_state.json` when present. The agent on the new execution reads the rehydration prompt, inspects the manifest for repo name, branch, and HEAD metadata, and then applies the downloaded patches (`git apply`), so download reliability directly affects whether the new run can pick up from where the old one left off.

The handoff download does not perform local directory reconciliation. The next execution is prepared in the same cloud-agent workspace shape, and the downloaded artifacts are placed under that run's attachments directory for the rehydration prompt to consume.

**Reliability:**
- Transient download errors (5xx responses, 408, 429, network timeouts, connection resets) are automatically retried with exponential backoff and jitter. Permanent errors (other 4xx responses) fail fast.
- Retries are strictly bounded to 3 attempts per file, matching the upload side.
- Per-file failures do not abort the remaining downloads — a failed patch is reported but the rest of the files still land on disk.
- Downloads run concurrently via `join_all`.

**Outcome surfacing:**
- Per-file failures are aggregated inside the download function and summarized in a single log line: INFO when every file downloaded, WARN when any failed (with filename + error for each failure).
- The download function returns the `attachments_dir` only when at least one file landed successfully, and `None` otherwise, so the rehydration prompt never references a phantom path.
- Callers never see a structured per-file error list — operators rely on the aggregated log line for visibility into partial state.

**Failures outside per-file downloads:**
- If the server call to list handoff snapshot attachments fails (e.g. 5xx at the API layer), the download step is aborted and the error is logged. The next execution still proceeds — rehydration from the snapshot is best-effort; the conversation history remains the authoritative record.

### Gating
All snapshot/handoff functionality is gated behind `FeatureFlag::OzHandoff`, which is independent of `FeatureFlag::AgentHarness` (which only gates third-party agent CLIs like the `--harness` flag and `harness-support` subcommand). When `OzHandoff` is disabled:
- End-of-run snapshot upload is skipped during the driver run tail.
- Handoff snapshot attachments are not downloaded on subsequent executions.
- Block snapshots are not fetched or displayed when opening a CLI agent conversation.

## Success Criteria
- At the end of every cloud agent run, `AgentDriver` reads the declarations file and uploads the manifest and all declared repos/files to presigned URLs before signaling completion.
- A repo with uncommitted changes produces a `.patch` file that captures both tracked and untracked changes.
- A clean repo is recorded in the manifest with `"status": "clean"` and no patch file.
- Repo entries in the manifest include `repo_name`, current branch when available, and current HEAD SHA when available.
- Malformed declarations lines (invalid JSON, missing fields, missing or unsupported version, unknown kind, non-absolute path) are logged at WARN level and skipped; they do not abort the upload.
- Individual upload, gather, and read failures are reported per-entry without aborting the remaining entries.
- Transient upload failures are automatically retried with a bounded attempt cap; the run tail cannot hang on a stuck connection.
- The uploaded manifest accurately reflects per-entry outcomes (`clean`, `uploaded`, `failed`, `skipped`, `gather_failed`, `read_failed`) rather than only declared intent.
- A missing or empty declarations file results in a WARN log and an early skip; the `AgentDriver` run itself still succeeds.
- The declarations-generation script is invoked automatically before the upload pipeline reads the file. A missing `OZ_SNAPSHOT_DECLARATIONS_SCRIPT`, a missing script file, a non-zero script exit, or a script runtime exceeding the configured timeout are each logged at ERROR level without aborting the upload.
- In local dev, `oz-local --docker-dir <dir>` sets `OZ_SNAPSHOT_DECLARATIONS_SCRIPT` for the oz process so the script is resolvable even when the Docker image isn't in play.
- Snapshot upload failures (gather, read, upload, manifest) never cause the `AgentDriver` run to fail — they are logged but the driver completes normally.
- The upload is skipped when `FeatureFlag::OzHandoff` is disabled, when the run has no task ID, or when `--no-snapshot` is specified.
- `--snapshot-script-timeout <DURATION>` and `--snapshot-upload-timeout <DURATION>` override the default script and upload timeouts.
- `OZ_SNAPSHOT_DECLARATIONS_FILE` overrides the default path, making the upload testable and operationally tunable. Without the override, per-run files at `/tmp/oz/<task-id>/snapshot-declarations.jsonl` keep concurrent runs isolated.
- Repeated invocations of `snapshot-declarations.sh` within a single run append to the declarations file without re-emitting repos that were already discovered earlier, keeping the pipeline additive.
- Handoff snapshot attachment downloads automatically retry transient errors with a bounded attempt cap.
- Partial download failures are logged at WARN level and do not abort the remaining downloads.
- When at least one handoff snapshot file downloads successfully, the rehydration prompt still receives the attachments directory so downstream work can apply what's available.
- When a user opens a third-party harness conversation, the terminal output snapshot is displayed inline in the terminal view.
- Snapshot upload and handoff download do not require local repo switching or directory reconciliation.
- The conversation appears in conversation history with the correct metadata (title, working directory, timestamps).
- All snapshot/handoff behavior is gated behind `FeatureFlag::OzHandoff` and is inert when the flag is disabled.

## Validation
- Manual validation that a cloud agent run with a dirty repo entry in the declarations file produces a patch and uploads it.
- Manual validation that a clean repo entry records `"status": "clean"` and no patch file in the manifest.
- Manual validation that repo entries in `snapshot_state.json` include `repo_name`, `branch` when available, and `head_sha` when available.
- Manual validation that a mix of `repo` and `file` entries produces the correct manifest and uploads all artifacts.
- Manual validation that malformed declarations lines (invalid JSON, missing or unsupported version, non-absolute path, unknown kind) are skipped with a WARN log without aborting the upload.
- Manual validation that a missing declarations file results in a WARN log and no upload; the AgentDriver run still completes normally.
- Manual validation that an unreadable `file` entry is recorded as `"status": "read_failed"` in the manifest and does not abort the remaining uploads.
- Manual validation that a simulated transient upload failure (e.g. forced 5xx) is retried and eventually succeeds, and that a permanent failure (e.g. 403) fails fast without retries.
- Manual validation that the aggregate log summary correctly reflects per-entry outcomes.
- Manual validation that `OZ_SNAPSHOT_DECLARATIONS_FILE` overrides the default declarations path.
- Manual validation that `snapshot-declarations.sh` in a containerized run produces a declarations file listing every git repository under the workspace.
- Manual validation that `oz-local --docker-dir <dir>` causes the local-dev flow to invoke the script from the provided directory and that the resulting declarations file is consumed by the upload pipeline.
- Manual validation that a missing `OZ_SNAPSHOT_DECLARATIONS_SCRIPT` (e.g. running the oz CLI without going through `entrypoint.sh` or `oz-local`) produces an ERROR log and no script invocation, and the AgentDriver run still completes normally.
- Manual validation that `--no-snapshot` skips snapshot generation and upload.
- Manual validation that `--snapshot-script-timeout <DURATION>` and `--snapshot-upload-timeout <DURATION>` override the default caps.
- Manual validation that snapshot upload failures never cause the overall AgentDriver run to fail.
- Manual validation that a simulated transient handoff download failure (5xx) is retried and eventually succeeds.
- Manual validation that a simulated permanent handoff download failure (e.g. 403) fails fast and is recorded without retries.
- Manual validation that a partial download failure is logged at WARN level and that the successfully-downloaded files are still on disk.
- Manual validation that opening a completed third-party harness conversation in Warp displays the terminal output inline.
- Manual validation that handoff snapshot files download under the attachments directory for the next execution without local repo switching.
- Manual validation that conversations from third-party harnesses appear in conversation history.
- Manual validation that disabling `FeatureFlag::OzHandoff` prevents all snapshot/handoff behavior (upload, download, hydration).

## Open Questions
- Should the client eventually apply the uploaded workspace patch automatically, or should that always be a user-initiated action?
- Should there be a size limit on individual files or the total snapshot payload?
- Should snapshot download failures surface a user-visible error, or silently fall back to showing metadata only?
