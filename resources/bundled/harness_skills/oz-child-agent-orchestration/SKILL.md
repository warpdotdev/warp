---
name: oz-child-agent-orchestration
description: Coordinate with the parent of an Oz cloud child run. Read before starting delegated work, and when checking messages, reporting a blocker, or handing off results.
---
# Child coordination
Use the CLI in `OZ_CLI`, your current run ID in `OZ_RUN_ID`, and your parent's run ID in `OZ_PARENT_RUN_ID`. Do not guess missing IDs. If any is absent, report the missing coordination context instead of sending to an inferred recipient.

## Receive instructions
Before substantive work, between milestones, and immediately before finishing or waiting, list unread messages:

```sh
"$OZ_CLI" run message list "$OZ_RUN_ID" --unread
```

For each returned message ID, read its body, then mark it delivered only after reading it:

```sh
"$OZ_CLI" run message read <message-id>
"$OZ_CLI" run message mark-delivered <message-id>
```

Continue from the latest parent instructions. To wait for new messages, use the event stream rather than a polling loop:

```sh
"$OZ_CLI" run message watch "$OZ_RUN_ID" --since-sequence <last-sequence>
```

Resume from the last event sequence after reconnecting; then list and read unread messages. The watch stream is a notification, not a substitute for reading the full body.

## Report and hand off
Send blockers and questions to the parent before stopping, and report material results without narrating every command:

```sh
"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" \
  --to "$OZ_PARENT_RUN_ID" --subject '<subject>' --body '<message>'
```

Include exact plan IDs when referencing plans. Use plan content supplied by the parent when document tools are unavailable. You are in an isolated cloud sandbox: use the parent's requested handoff and push durable branches or draft PRs for code changes, reporting their references and validation results. Do not leave important work only on the sandbox filesystem.

When complete, perform the final unread-message check, then use `oz-finish-task` as your last action. Its summary is the completion handoff to the parent; include findings or deliverables themselves, not merely a description of having produced them. Do not claim completion while blocked waiting for an answer.
