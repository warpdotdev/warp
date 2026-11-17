---
name: wait-for-events
description: Wait for inbound lifecycle or message events from other Oz agents when no independent work remains. Use after dispatching work, or after asking your parent a question when you expect its reply. Do not use when your assigned task is complete; use oz-finish-task instead.
---
# Wait for events

Wait for inbound lifecycle or message events from other agents.

## When to use
- Use this skill when you have dispatched work to other agents and cannot make further progress until they reply or emit a lifecycle event.
- As a child, wait only when you expect a response from your parent, for example immediately after sending it a question. Do not wait merely to remain available after completing your assigned task.
- Drain your inbox first. Do not wait while there are unread messages or lifecycle events. Follow `oz-child-agent-orchestration` for listing, reading, and acknowledging messages.
- Do not use this skill as a substitute for `oz-finish-task`. Use `oz-finish-task` when your overall assigned task is complete, or when you are stopping with a blocker rather than waiting for an expected parent response.

## Register the wait

`OZ_CLI` is the CLI for this environment. `OZ_RUN_ID` identifies your run. If either is missing, treat that as a blocker; do not guess.

Run this command with the IDs of the agents whose work or reply you are waiting on:

```sh
"$OZ_CLI" harness-support --run-id "$OZ_RUN_ID" wait-for-events --run-ids "<run-id-1>" "<run-id-2>"
```

For an expected parent reply, use `--run-ids "$OZ_PARENT_RUN_ID"`. Do not guess the parent ID if it is missing.

- The command registers the wait and returns immediately. Its success response contains `waiting_on_run_ids`; it does not contain delivered events.
- Only wait on runs in your orchestration tree that are still working or validly waiting. The server rejects impossible or cyclic waits.
- On success, end your current agent turn immediately. The conversation resumes automatically when a new event arrives. Do not call `oz-finish-task`, exit the harness process, poll, sleep, or issue another command after registering the wait.
- Run the wait command alone. Never run it in parallel with another tool or command, including `oz-finish-task`.
- If registration fails, the wait was not established. Read the error and correct the targets or report the blocker; do not assume the conversation will resume.

## After resumption

- Read the inbound messages and lifecycle events before deciding what to do next. Any inbound event can resume you, not only an event from a listed run.
- If a wait times out and you resume with an empty event result, no event arrived; do not treat that empty result as event delivery.
- Ordinary progress clears the registered target list. If you still cannot proceed, drain the inbox and register a new wait for the remaining runs.
