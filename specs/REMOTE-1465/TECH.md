# Oz File-Edit Hooks for Snapshotting Non-Git-Tracked Files — Tech Spec
Product spec: `specs/REMOTE-1465/PRODUCT.md`
Linear: [REMOTE-1465](https://linear.app/warpdotdev/issue/REMOTE-1465)

## Context

This feature extends the REMOTE-1332 end-of-run snapshot pipeline (`specs/REMOTE-1332/TECH.md:408` explicitly records it as a follow-up). The declarations file already supports two entry kinds (`repo` and `file`), but no code path ever emits `file` entries automatically — the docker-image `snapshot-declarations.sh` script only emits `repo` lines, and operator-authored `file` entries are the only current producer. That means files the agent creates or edits outside any declared git repo never make it into the snapshot upload.

The Oz SDK driver already has every hook point we need. It subscribes to `BlocklistAIHistoryEvent` for the driver's own terminal view, observes every `AIAgentExchange` as it completes, and already runs the end-of-run snapshot pipeline with the correct gates. The work here is to observe successful file-edit tool results inside that subscription and append matching `file` lines to the declarations file the pipeline already reads, then filter out any `file` entries that would double-upload against the scanned `repo` entries.

### Relevant code
- `app/src/ai/agent_sdk/driver/snapshot.rs` — the end-of-run snapshot pipeline. The JSONL declarations format is documented in the module-level doc, and `DeclarationEntry` (in the `Declarations file parsing` section) already understands both `Repo` and `File` kinds. `resolve_declarations_path` resolves the per-run declarations path (env override → `/tmp/oz/<task-id>/snapshot-declarations.jsonl` → `/tmp/oz/snapshot-declarations.jsonl`). `upload_snapshot_from_declarations_file` is where the parsed entries are consumed.
- `build_repo_patch` in the same file generates the per-repo patch via `git diff --binary HEAD` plus `git ls-files --others --exclude-standard`. This is the path that already covers tracked changes and untracked non-gitignored files inside a declared repo.
- `app/src/ai/agent_sdk/driver.rs::AgentDriver::execute_run` — the existing `BlocklistAIHistoryEvent` subscription. The `AppendedExchange` arm already calls `write_exchange_inputs` once a new exchange is appended. Every `AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Success { updated_files, deleted_files, .. })` reaches the next exchange as an `AIAgentInput::ActionResult` input, and the paths in `updated_files[i].file_context.file_name` are the absolute paths we need.
- `AgentDriverOptions` and the `AgentDriver` struct in the same file already own `working_dir: PathBuf`, `task_id: Option<AmbientAgentTaskId>`, and `snapshot_disabled: bool`. These are the inputs the writer needs.
- `AgentDriver::run_snapshot_upload` defines the existing gate (`FeatureFlag::OzHandoff.is_enabled()`, `task_id.is_some()`, `!snapshot_disabled`). The writer reuses this gate verbatim.
- `crates/ai/src/agent/action_result/mod.rs::RequestFileEditsResult` is the variant we pattern-match. `UpdatedFileContext.file_context.file_name` carries the absolute file path populated by `apply_create_file` / `apply_search_replace` / `apply_v4a_update` in `app/src/ai/blocklist/action_model/execute/request_file_edits/diff_application.rs` (they call `host_native_absolute_path` before constructing diffs, so the path reaching the executor is already absolute in practice).
- `warp-agent-docker/snapshot-declarations.sh` — the existing script's dedup step reads only `repo` JSONL lines into its seen-set, so `file` lines written by the driver are left intact across repeated script invocations.

## Proposed changes

### 1. Observe successful file edits in the SDK driver

Extend the existing `BlocklistAIHistoryEvent` handler in `AgentDriver::execute_run` so that on every `AppendedExchange` event, the driver walks `exchange.input` for `AIAgentInput::ActionResult` entries and extracts every `AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Success { updated_files, .. })` result's paths. Action results from the prior exchange flow back as inputs on the next exchange, so scanning inputs on each newly-appended exchange captures every completed file edit. `deleted_files` is deliberately ignored (see product invariant 6).

Gate the observer at construction time: `AgentDriver::new` only constructs a `DeclarationsWriterHandle` when `FeatureFlag::OzHandoff.is_enabled() && task_id.is_some() && !snapshot_disabled`. When that gate fails the field stays `None` and the observer's `if let Some(writer) = me.snapshot_file_writer.as_ref()` short-circuits before touching any exchange data.

The subscription closure runs on the driver's model-context thread and must not touch the filesystem inline. It only collects path strings from each successful `RequestFileEdits` result and hands the resulting `Vec<String>` off to the writer task introduced in section 2 via a non-blocking `DeclarationsWriterHandle::append` call. Path normalization (joining relative paths against `working_dir`, dropping non-absolute or non-UTF-8 paths), the on-write repo preempt, directory creation, and JSONL append writes all happen on the writer task — never on the subscription thread.

Any error surfaced by the writer task is logged and absorbed. The observer never fails the subscription, the exchange, or the overall run.

### 2. Dedicated async writer task for `file` declarations

Introduce a `DeclarationsWriterHandle` in `app/src/ai/agent_sdk/driver/snapshot.rs`, owned by `AgentDriver` for the lifetime of the run:

```rust
enum WriterCommand {
    Append(Vec<String>),
    Flush(oneshot::Sender<()>),
}

pub(super) struct DeclarationsWriterHandle {
    tx: tokio::sync::mpsc::UnboundedSender<WriterCommand>,
}

impl DeclarationsWriterHandle {
    pub(super) fn new(
        task_id: AmbientAgentTaskId,
        working_dir: PathBuf,
        background: &Background,
    ) -> Self;
    pub(super) fn append(&self, paths: Vec<String>);
    pub(super) async fn flush(&self);
}
```

`new` spawns a single long-lived task on the application's `Background` tokio runtime that owns the `seen: HashSet<String>` plus the resolved declarations path. The task loops on `rx.recv()` and processes commands sequentially, which gives us in-process serialization for free — two subscription events cannot race on either the set or the file because there is exactly one writer. `AgentDriver::new` runs in a sync `add_singleton_model` callback, so the writer is spawned via `ctx.background_executor()` rather than `tokio::spawn` (the foreground executor has no current tokio runtime).

`WriterCommand::Append` handling, per path:
1. Normalize: join non-absolute paths against `working_dir`, then drop the path with a `log::warn!` if it is still not absolute or is not valid UTF-8.
2. Skip if the normalized path is already in `seen` (in-memory dedup, product invariant 4).
3. Walk the path's ancestors and `tokio::fs::try_exists` each `<ancestor>/.git`. If any ancestor is already a repo at enqueue time, skip with `log::debug!` and add the path to `seen` so future appends short-circuit. This is the cheap on-write preempt for the common "agent edits inside an existing repo" case and avoids populating the declarations file with entries the gather-time filter would just drop anyway.
4. Otherwise `tokio::fs::create_dir_all` the declarations file's parent, open the file with `tokio::fs::OpenOptions::new().append(true).create(true).open(...)`, write one JSONL line built from the `FileDeclaration` struct via `serde_json::to_string`, `flush().await`, and then `seen.insert(path)`.
5. Per-path write failures are logged at `log::warn!` (the `seen` set is not advanced) and the task continues with the next path and the next command.

`WriterCommand::Flush` simply acks the provided oneshot. Because the writer task drains the mpsc queue in order, the ack fires only after every previously-enqueued `Append` has finished its fs writes — callers that await `flush()` can rely on every prior `append()` being on disk by the time `flush` returns.

`DeclarationsWriterHandle::append` is a sync, non-blocking send from the subscription thread. `DeclarationsWriterHandle::flush` is called once from `run_snapshot_upload` immediately before `snapshot::run_declarations_script`, so no driver-side write is in flight when the script starts its own appends.

Visibility is the minimum needed: the handle, its command enum, and `new`/`append`/`flush` are `pub(super)` so the driver module can construct and drive them; the writer task's internal helpers stay `fn`.

### 3. Drop `file` entries covered by declared repos at gather time

Add a pure helper in `app/src/ai/agent_sdk/driver/snapshot.rs`:

```rust
fn drop_files_covered_by_repos(entries: Vec<DeclarationEntry>) -> Vec<DeclarationEntry>;
```

It:
- Collects every `EntryKind::Repo` path.
- For each `EntryKind::File` entry, drops it if the file path, treated as a filesystem path, `starts_with` any repo path (per `std::path::Path::starts_with` on normalized components). Logs an `info!` noting which repo covers which file so operators can trace elided entries.
- Leaves every other entry untouched.

Invocation site: `upload_snapshot_from_declarations_file`, immediately after `read_and_parse_declarations` returns and before the `repo_count` / `file_count` log line so the count reflects what will actually be uploaded.

This gather-time filter and the on-write ancestor-repo check in section 2 are two layers with different jobs:
- The on-write check preempts `file` declarations when the path already sits inside an existing repo at edit time, so common-case edits inside `/workspace/existing-repo/` never hit the declarations file at all. This is the fast path for the majority of agent edits and keeps the end-of-run gather step cheap.
- The gather-time filter covers the "agent writes files, then later `git init`s the parent directory" case: at write time no `.git` existed yet so the path was recorded as a `file` entry, but once the end-of-run script emits the new `repo` entry, this filter elides the earlier `file` entries so the file is not uploaded twice (once as a raw blob, once inside the patch's untracked-files section).

### 4. No changes to `snapshot-declarations.sh` or `warp-agent-docker`

The existing script's dedup step only tracks `repo` lines (`warp-agent-docker/snapshot-declarations.sh:42`), so driver-written `file` lines survive re-invocation. Nothing in the Docker image or `entrypoint.sh` needs to change.

### 5. Feature flag and rollout

No new flags. Reuses `FeatureFlag::OzHandoff` so the whole mechanism is in lockstep with REMOTE-1332's rollout.

## Testing and validation

Product-spec invariants in `specs/REMOTE-1465/PRODUCT.md` map to tests as follows. New unit tests live in `app/src/ai/agent_sdk/driver/snapshot_tests.rs` next to the existing REMOTE-1332 coverage.

- Invariant 1, 2, 4, 5 — `DeclarationsWriterHandle` unit test: construct a handle against a synthetic task id and a tmp `working_dir`, `append` a sequence of absolute paths that includes a repeat, `flush().await`, and assert the handle created the parent directory, wrote one JSONL line per unique path, and a second `append+flush` of the same paths is a no-op. Assert exact JSONL shape by round-tripping through `serde_json::from_str::<DeclarationLine>` so any schema drift surfaces here.
- Invariant 3 — same handle test extended: relative paths passed to `append` are resolved against `working_dir`; paths that cannot be made absolute are dropped with a WARN log and not written.
- On-write repo preempt (section 2 step 2) — writer unit test: set up a tmp dir containing a `.git` subdirectory, `append` a path under it, `flush().await`, and assert no JSONL line was written. Repeat with a path outside any `.git` ancestor and assert the line is written.
- Flush semantics (section 2) — writer unit test: `append` many paths followed immediately by `flush().await`, and assert every expected JSONL line is on disk by the time `flush` returns; a follow-up `append+flush` after the first flush still works.
- Invariant 6 — covered by structural observation: the observer pattern-matches `RequestFileEditsResult::Success { updated_files, .. }` and never reads `deleted_files`. Any future regression here would surface in the driver-side observer code rather than as a behavioral difference, so no dedicated unit test is added.
- Invariant 7, 8 — end-to-end pipeline test in `snapshot_tests.rs` (`e2e_repo_plus_inside_and_outside_files_filters_overlap`): pre-seed a declarations file with one `repo` path and two `file` paths (one inside the repo, one outside). Run `upload_snapshot_from_declarations_file` against a `mockito::Server` harness and assert the inside-repo file is not uploaded and does not appear in the manifest's `files` list, while the outside-repo file is uploaded normally.
- Invariant 9 — covered structurally: `AgentDriver::new` returns `snapshot_file_writer: None` whenever any of the three gate conditions fail, and the observer closure no-ops on `None`. Driver-level harness is not exercised here because the gate logic is co-located with the construction site and is small enough to verify by inspection.
- Invariant 10 — no explicit test; enforced structurally because the observer lives on `AgentDriver`, which only runs for Oz SDK runs. Third-party harnesses don't subscribe to `BlocklistAIHistoryEvent` for file edits.
- Invariant 11 — error-path test (`declarations_writer_continues_after_per_path_write_failures`): pre-create a directory at the declarations file path so the first `append`'s open call fails, then remove it and assert the next `append` succeeds. Verifies the writer task absorbs the failure and keeps servicing commands.
- Invariant 12, 13 — documented as product-level limitations; no explicit tests. The existing REMOTE-1332 untracked-files coverage continues to validate the underlying git path.

Manual validation:
- Run a cloud Oz run (`./script/oz-local` per Warp Drive notebook `zOJarbIZgXHJDXS7dF9u82`) that asks the agent to create a file at `/tmp/oz-handoff-check.txt`. Confirm the declarations file picks up a `file` line, and confirm the end-of-run snapshot manifest includes the file with `"status": "uploaded"`.
- Run a cloud Oz run where the agent edits a file inside a pre-existing git repo under the workspace. Confirm the end-of-run pipeline logs `drop_files_covered_by_repos` electing not to upload the file as a standalone blob, and confirm the manifest still shows the repo's patch containing the change.
- Run a cloud Oz run where the agent creates files first, then runs `git init`. Confirm the manifest shows one `repo` entry for the initialized directory, and no separate `file` entries for the pre-existing files under it.
- Repeat the first manual case with `--no-snapshot`. Confirm no declarations file is written even though the agent edits a file.

## Risks and mitigations

- **Double-uploading when the overlap filter misses a nested repo root.** `Path::starts_with` is strict, so `/workspace/my-proj` and `/workspace/my-proj/sub/.git` both get their own `repo` entry after scanning; a `file` under `sub/` is correctly caught by the `sub` repo. The only real failure mode is comparing paths with differing trailing separators or symlinks; normalize both sides with a helper that strips trailing `/` and canonicalizes where possible before comparing.
- **Writer contention on a single declarations file.** In-process serialization comes from the single writer task owning both the `seen` set and the file handle — two subscription events cannot race on either, because `mpsc::UnboundedSender::send` from the subscription is non-blocking and the writer task processes commands sequentially. Cross-process serialization against `snapshot-declarations.sh` relies on two pieces: (1) `run_snapshot_upload` awaits `DeclarationsWriterHandle::flush` before spawning the script, so no driver-side write is queued when the script starts; and (2) `O_APPEND` atomicity on POSIX guarantees that any write smaller than `PIPE_BUF` (4096 on Linux, 512 on macOS, safely larger than our JSONL lines) cannot interleave with another writer's output. `flock(2)` is not added; the drain plus atomic appends are sufficient for correctness.
- **Gitignored files inside a declared repo are silently dropped.** Documented in product invariant 13 and the Open questions section. Mitigation deferred to a follow-up: fold `git check-ignore -q <path>` into `drop_files_covered_by_repos` so `file` entries whose paths git would ignore are preserved.
- **Exchange input walker missing new tool variants.** The observer explicitly pattern-matches `AIAgentInput::ActionResult` carrying `AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Success { .. })` and ignores other variants. If future file-writing tools (e.g. `CreateDocuments`, `EditDocuments`) start writing to the local filesystem, they must either go through `RequestFileEdits` or get their own observer branch. The existing `CreateDocuments` / `EditDocuments` variants target Warp Drive documents rather than local files, so this is not a regression today.
- **Last-exchange edits without a follow-up.** Action results from the most recent exchange only land in the next exchange's inputs, so file edits performed in the very last exchange of a run (e.g. when the agent stops without producing a follow-up) are not observed. In practice the agent always emits a final response after tool calls, but if this becomes a measurable gap we can also poll `exchange.input` from the `UpdatedConversationStatus` arm before the run completes.

## Follow-ups
- Use `git check-ignore -q` (or `git ls-files --error-unmatch`) to keep `file` entries whose paths fall inside a declared repo but would not be carried by the repo's diff.
- Add a tombstone-style `deleted_file` declaration kind plus manifest support so the snapshot can represent deletion of files outside any declared repo.
- Wire equivalent hooks for third-party harnesses (Claude Code) via their hook system so this mechanism also works for non-Oz runs, per the Linear issue's explicit follow-up line.
- Surface a WARN earlier (e.g. at 75% of `MAX_SNAPSHOT_FILES_PER_RUN`) when the tool-call writer approaches the per-run cap.
