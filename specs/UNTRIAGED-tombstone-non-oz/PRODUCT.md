# Tombstone metadata for cloud agent runs — Product Spec

Linear: placeholder. Figma: none.

## Summary
For cloud agent runs (Oz and non-Oz harnesses like Claude Code), make the conversation-ended tombstone show `Run time`, `Credits used`, and the artifact buttons row (plans, PRs, files, screenshots) from the `AmbientAgentTask` when one is present, matching the conversation details panel for the same run.

Today the tombstone reads run time, credits, and artifacts off the in-memory `AIConversation`. For non-Oz cloud runs there is no `AIConversation`, so all three render empty. For cloud Oz runs the conversation values diverge from the details panel: run time excludes queue/sandbox lifecycle, credits exclude compute cost, and artifacts can lag the task-side copy.

## Goals
- Non-Oz cloud tombstones render `Run time`, `Credits used`, and the artifact buttons row populated from the `AmbientAgentTask`.
- Cloud Oz tombstones converge on the same task-derived values, matching `ConversationDetailsData::from_task` numerically and on artifacts.
- Hide the `Continue locally` button (tombstone + details panel) for tasks whose harness is non-Oz, since forking a non-Oz cloud run into a local Warp conversation is unsupported.
- Client-only change: no harness, server, schema, or endpoint work.

## Non-goals
- `Directory:` / `Branch:` segments on non-Oz tombstones.
- Implementing `Continue locally` for third-party harnesses (button is hidden, not wired up).
- Splitting credits into Inference + Compute sub-rows like the details panel does. The tombstone keeps a single aggregated `Credits used: X.X` segment because the `•`-separated row has no room.

## Behavior
The tombstone metadata row segments behave as follows. Source-of-truth rules mirror `ConversationDetailsData::from_task`.

- **Run time** (`Run time: <human-readable>`):
  - Cloud (task present): `AmbientAgentTask::run_time()`. Omitted if `started_at` missing.
  - Local (no task): unchanged. Exchange-derived.
- **Credits used** (`Credits used: <formatted via format_credits>`):
  - Cloud (task present): `AmbientAgentTask::credits_used()` (`inference + compute`). Omitted if `request_usage` missing.
  - Local (no task): unchanged. `conversation.credits_spent()`.
- **Source / Skill**: unchanged (already enriched from the task).
- **Working directory**: Oz unchanged (`AIConversation::initial_working_directory()`); non-Oz not rendered.
- **Artifacts row** (plan, branch, PR, screenshot, file buttons):
  - Cloud (task present, non-empty `task.artifacts`): `task.artifacts`. Wins over conversation artifacts.
  - Cloud (task present, empty `task.artifacts`): falls back to whatever `from_conversation` set (avoids blanking Oz when the task is partially populated).
  - Local (no task): unchanged. `conversation.artifacts()`.
- **`Continue locally` button** (tombstone desktop + details panel):
  - Hide rule is purely on harness: hide iff `harness == Some(non-Oz)`. `None` (unknown / not yet loaded / plain conversation) and `Some(Oz)` both show the button.
  - Local (no task): unchanged — harness stays `None`, shown when AI is enabled.
  - Cloud Oz task: shown both before and after task load.
  - Cloud non-Oz task (Claude, Gemini): button shows briefly until task fetch resolves, then hides.
  - Tombstone wasm `Open in Warp` button: unchanged. Opens the same conversation in the desktop client, where the same hide rules then apply.

Empty-row, error, and snapshot/transcript-viewer behaviors are unchanged.

## Success criteria
- For any cloud run with an `AmbientAgentTask` (Oz or non-Oz), the tombstone's `Run time` and `Credits used` strings equal what `ConversationDetailsData::from_task` produces for the same task.
- Non-Oz cloud tombstones with a populated `task.artifacts` render the same artifact buttons as the details panel for the same run.
- Cloud Oz tombstone numbers will visibly change: both increase. New values match the details panel and Oz task list.
- Cloud non-Oz tombstones and details panels do not render `Continue locally` once the task is loaded.
- Cloud Oz tombstones and details panels still render `Continue locally`.
- Local Oz tombstone behavior is unchanged.
- Non-Oz tombstones remain feature-flagged behind `FeatureFlag::AgentHarness`.

## Validation
- Unit tests on `enrich_from_task` covering: task wins for run time / credits / artifacts when present; falls back to `from_conversation` values when the corresponding task field is missing.
- Unit tests on `enrich_from_task` covering harness extraction: snapshot absent → harness stays `None`; snapshot without explicit harness → defaults to Oz; snapshot with explicit harness propagates.
- Manual: cloud Claude Code run shows non-empty `Run time`, `Credits used`, and artifact buttons matching the details panel, and `Continue locally` is hidden in both surfaces; cloud Oz run shows updated numbers and artifacts also matching the details panel, with `Continue locally` shown; local Oz run unchanged.

## Follow-ups
- Wasm parity (cloud tombstones on web don't currently fetch the task; pre-existing gap).
- `Directory:` / `Branch:` on non-Oz, via a future `POST /harness-support/report-context` sibling endpoint that piggy-backs in parallel with the existing transcript upload.
- Possible `Download conversation` button surfacing the harness JSON.
