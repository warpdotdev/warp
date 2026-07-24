*Proposed change: CloudMode recording artifacts in the Artifacts sidebar*

*Summary:* Fix the three reported recording-artifact behaviors in the Warp
client: a missing WASM sidebar pill, a native pill that loses the recording
title, and a native click that opens a save-file picker instead of the
recording viewer. The implementation is two coordinated PRs: this Warp PR
(client/WASM behavior) and a small warp-server PR (v2 GraphQL title exposure);
the server change must land and deploy before the client relies on that field
for the GraphQL-backed conversation path.

*Key design choices:* Keep recordings represented as `Artifact::File`, use the
persisted title as the preferred label with filename fallbacks, and classify
recordings by `mime_type` (`video/*`). Route the sidebar action through the
authenticated run-page URL helper being added by QUALITY-1193, with the
existing signed-download URL as the no-task-id fallback. Fix the WASM issue
upstream in artifact population, not by adding a rendering-only WASM gate.

*Design alternatives:*
- Add a `RECORDING`/`VIDEO` artifact union variant — rejected. Uploads,
  GraphQL, the protobuf event, and Oz web all currently model recordings as
  `FILE`; introducing a new variant would require a compatibility migration
  without fixing the missing-data path.
- Infer the title from the filename or description — retained only as a
  backward-compatible fallback. The short title is already persisted and is
  the value Oz web displays, so inference cannot satisfy title parity.
- Add a server-provided `view_url` field like QUALITY-1193 PR #13205 —
  rejected for this change. QUALITY-1193 PR #14210 builds the
  environment-correct authenticated URL in the client; extract/reuse that
  helper rather than creating a second server dependency. If #13205 is
  merged first, do not add a second competing route; preserve one canonical
  open-recording behavior.
- Open every file in a browser — rejected. Only video recordings open in the
  recording viewer; non-video files retain their download behavior and
  screenshots retain their lightbox behavior.

*Root cause / approach:*

1. *WASM population (#1).* At Warp `0b07f7c2e371f6ced4287ffbdaa17f50fb129dde`,
   `ArtifactButtonsRow::collect_buttons` handles `Artifact::File` without a
   WASM cfg gate (`app/src/ai/artifacts/buttons.rs:127-194`). Therefore a
   missing recording pill means the recording is absent from the data supplied
   to the row, not filtered while rendering. The WASM details-panel updater
   chooses `ConversationDetailsData::from_task` when the terminal has an
   ambient task id and otherwise falls back to
   `ConversationDetailsData::from_conversation`
   (`app/src/workspace/view/wasm_view.rs:166-221`; task/conversation sources
   at `app/src/ai/conversation_details_panel.rs:279-450`). The implementation
   must first reproduce in a real WASM/web run and compare both source lists
   with the server/task payload and live `ArtifactEvent` stream. It must then
   fix the minimal upstream drop point (task artifact hydration/deserialization
   or conversation-event/storage merge) so every source selected by the
   updater contains the confirmed `FILE` artifact. No `collect_buttons` gate
   or special rendering branch should be added.
2. *Title (#2).* Recording finalization sends `recording.summary` as
   `FileArtifactUploadRequest.title`
   (`app/src/ai/blocklist/action_model/recording_finalize.rs:168-175`), and
   the title is persisted in warp-server `FileArtifactData.Title`
   (`warp-server/model/types/ai_conversation_artifacts.go:80-89`).
   Oz web gets the title from the public Agent API's existing
   `FileArtifactData.Title` and prefers it for video badges
   (`warp-server/public_api/types/types.gen.go:1645-1667`;
   `warp-server/client/packages/agents/src/components/badges/ArtifactBadge.tsx:330-376`);
   that public API surface does not need a new field. The v2 GraphQL
   `FileArtifact` omits the field (`warp-server/graphql/v2/ai_conversation.graphqls:111-117`
   at `1c6eec98230bb84afe4a2690e2407e3a16fca7b9`), and the Warp client
   GraphQL/task artifact models consequently discard it
   (`crates/graphql/src/api/ai.rs:105-112`,
   `app/src/ai/artifacts/mod.rs:239-272`; the task JSON helper also needs to
   retain the already-public `title`). Add nullable `title` to the server v2
   GraphQL type and `ConvertFileArtifactToGraphQL`, regenerate server/client
   GraphQL types, add optional title to the client `Artifact::File` and task
   deserializer, and make `file_button_label` choose `title`, then filename,
   then filepath basename, then `File`.
3. *Open behavior (#3).* The sidebar currently emits
   `ArtifactButtonsRowEvent::DownloadFile`
   (`app/src/ai/artifacts/buttons.rs:54-76`) and native
   `open_file_download_result` enters the `local_fs` save-file picker
   (`app/src/ai/artifacts/mod.rs:398-459`). The existing
   `AIBlockAction::OpenRecordingArtifact` path is the product precedent
   (`app/src/ai/blocklist/block.rs:6924-6955`). Reuse/extract the
   `recording_artifact_view_url` helper from QUALITY-1193 PR #14210 so both
   the blocklist and sidebar use
   `{ChannelState::oz_root_url()}/runs/{task_id}?artifact={artifact_uid}`.
   The sidebar video action obtains the task id from the current
   `ConversationDetailsData::PanelMode::Task`; if no task id exists, it
   falls back to fetching the artifact and opening its existing signed
   `download_url()`. Non-video `FILE` artifacts continue through the
   download picker; screenshots and PR/plan actions are unchanged.

*Affected files:*

- Warp client: `app/src/ai/artifacts/buttons.rs`,
  `app/src/ai/artifacts/mod.rs`, `app/src/ai/conversation_details_panel.rs`,
  `app/src/workspace/view/wasm_view.rs`, `app/src/ai/agent/conversation.rs`,
  `app/src/ai/ambient_agents/task.rs`,
  `app/src/ai/blocklist/block.rs` (shared helper integration with
  QUALITY-1193), `app/src/ai/artifacts/mod_tests.rs`, the details-panel/WASM
  tests, and `crates/graphql/src/api/ai.rs`.
- warp-server implementation PR (separate from this spec PR):
  `graphql/v2/ai_conversation.graphqls`,
  `model/types/ai_conversation_artifacts.go`,
  generated `model/types/v2`/GraphQL files, and focused resolver/model tests.
  Do not change the already-correct public Agent API title schema unless
  implementation finds a regression in its existing title serialization.

*Open questions resolved:*

- There is no dedicated recording artifact kind; `video/*` `FILE` artifacts
  are the supported representation.
- “Title” means the persisted short upload title (`recording.summary`), not
  the filepath or long description. For old artifacts with no title, retain
  filename-derived labels.
- The canonical post-QUALITY-1193 behavior is the authenticated Oz run-page
  lightbox URL. The signed GCS URL is a compatibility fallback only when the
  client has no task id. Do not depend on or duplicate the abandoned
  server-side `view_url` approach from PR #13205.
- The exact WASM drop point cannot be hands-on reproduced in this headless
  environment; the implementation's first action is the required WASM repro
  comparing task hydration with the conversation/event fallback. Whichever
  source is selected must contain the same confirmed file artifact before
  rendering.
- Implementation has two PRs: Warp reuses this branch/PR for client changes;
  warp-server carries the GraphQL schema/model/generated-code change. The
  server PR must be deployed before the client's GraphQL title path is
  expected to work, while older clients remain safe because `title` is
  nullable.

*Risks / blast radius:* A shared artifact model change can affect task
deserialization, GraphQL conversation restore, and live event updates; keep
the new field optional and preserve filename fallbacks. The viewer URL must use
`ChannelState::oz_root_url()` and URL-encode the artifact UID so staging,
dogfood, and production do not cross-link. Do not route screenshots, plans,
PRs, or ordinary files through the recording action.

*Validation & verification criteria* (must ALL pass before merge):

1. Before changing code, reproduce the reported recording run in a native Warp
   build and the WASM/web build. Record the source selected by
   `update_transcript_details_panel_data`, the artifact list before
   `ArtifactButtonsRow`, and the observed pill/click behavior. The WASM repro
   must identify whether the file is dropped during task REST hydration or
   during the conversation `ArtifactEvent`/history fallback; if the client
   cannot be launched, record the concrete environment mismatch and keep the
   UI criterion outstanding.
2. Add a regression test that fails before the fix and passes after it for
   WASM/task-or-conversation artifact population: a `FILE` recording present in
   the selected source must survive into `ConversationDetailsData.artifacts`
   and produce a non-empty artifact row. The test must cover both a task with
   `AmbientAgentTask.artifacts` and the no-task conversation fallback, or
   explicitly prove the fallback is unreachable for the affected CloudMode
   viewer.
3. Re-run the original WASM repro after the fix. A confirmed video `FILE`
   artifact appears under *Artifacts* in the sidebar, with no rendering-only
   WASM exception; native and web show the same artifact count.
4. Add client/server regression coverage for the title round trip. The
   server test must assert `FileArtifactData.Title` is returned by
   `ConvertFileArtifactToGraphQL` and the generated v2 `FileArtifact.title`
   field. The client tests must assert GraphQL/task deserialization preserves
   an optional title and `file_button_label` chooses title > filename >
   filepath basename > `File`; an artifact with no title retains its current
   filename label. The native repro must show the reported recording title,
   not `warp-recording-<uuid>.mp4`.
5. Add action-mapping coverage that a `video/*` `Artifact::File` emits the
   recording-open action and a plain non-video file emits the existing
   download action. The recording action must reuse the QUALITY-1193 helper,
   use the configured Oz origin, URL-encode the artifact UID, and use the task
   id from the details-panel task mode.
6. Re-run the native recording click repro. With a task id, clicking the pill
   opens the authenticated `/runs/{task_id}?artifact={artifact_uid}` viewer
   and does not open `DownloadFile`/the save-file picker. Without a task id,
   the signed five-minute `download_url()` fallback opens. Clicking a
   non-video file still opens the native download picker; screenshot,
   pull-request, branch, and plan actions remain unchanged.
7. Run the relevant Warp checks unconditionally: the focused artifact/details
   tests (including the new regression tests), `cargo check -p warp --lib`,
   and the repository's documented `./script/presubmit` gate. Run the
   relevant warp-server tests for GraphQL/model conversion and
   `go tool gqlgen generate --config graphql/v2/gqlgen.yaml`, then the
   repository's documented `./script/presubmit` gate (plus client GraphQL
   generation if the generated client package is touched).
8. Complete user-facing verification with computer use against both the
   native Warp build and the WASM/web build, capturing screenshots that show:
   (a) the recording pill present in WASM, (b) the native title label, and
   (c) the recording click opening the canonical viewer (or the explicit
   signed-URL fallback case). Attach the visual proof to the task and the
   implementation PR; do not commit media to either repository.
