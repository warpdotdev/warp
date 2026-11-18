---
name: wait-for-events
description: Wait for messages from other agent runs when no independent work remains and you expect a reply. Use after spawning another agent run whose report you are waiting for, or after asking the agent a question when you expect its reply. Do not use when your assigned task is complete or cannot be completed; use oz-finish-task instead.
---
# Wait for events

Wait for inbound lifecycle or message events from other agents.

## When to use
- Use this skill when you have spawned a child agent run through the `oz-child-agent-orchestration` skill and cannot make further progress until they reply.
- As a child, wait only when you expect a response from your parent, for example immediately after sending it a question. Do not wait merely to remain available after completing your assigned task.
- Drain your inbox first. Do not wait while there are unread messages or lifecycle events. Follow `oz-child-agent-orchestration` for listing, reading, and acknowledging messages.
- Do not use this skill as a substitute for `oz-finish-task`. Use `oz-finish-task` when your overall assigned task is complete, or when you are stopping with a blocker rather than waiting for an expected response.

## Register the wait

`OZ_CLI` is the CLI for this environment. `OZ_RUN_ID` identifies your run. If either is missing, treat that as a blocker; do not guess.

Run this command with the IDs of the agents whose work or reply you are waiting on:

```sh
"$OZ_CLI" harness-support --run-id "$OZ_RUN_ID" wait-for-events --run-ids "<run-id-1>" "<run-id-2>"
```

For an expected parent reply, use `--run-ids "$OZ_PARENT_RUN_ID"`. Do not guess the parent ID if it is missing.

- The command updates your run's state to waiting after validation. Its success response contains `waiting_on_run_ids`.
- The server rejects impossible or cyclic waits, and will return an informative error message on failed validation. You cannot wait on terminal runs - send them a message with what you need to resume them first.
- IMPORTANT: On success, ALWAYS end your current turn IMMEDIATELY. When a reply or event is available, the harness will send it to you as part of a new turn. Do NOT call `oz-finish-task`, poll for new messages, sleep, or issue other commands or tool calls after registering the wait.
- Run the wait command alone. Never run it in parallel with another tool or command, including `oz-finish-task`.
