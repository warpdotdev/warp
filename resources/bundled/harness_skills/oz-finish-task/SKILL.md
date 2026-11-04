---
name: oz-finish-task
description: Reports final task status and sends a child run's summary to its parent. Required before stopping after completing or failing a task, including informational tasks.
---
# Finish Task
You MUST use this skill before stopping when your task is complete or has ended unsuccessfully, even if it was purely informational or no code changed. A plain-text final response, CLI exit, or harness stop notification does not replace reporting completion.

This command updates your task status. For a child run, it also sends the summary as a message to the parent agent's inbox, so do not send a separate completion message with `oz run message send`.

Waiting for input is not completion. If work is still pending a reply, continue the orchestration skill's inbox polling workflow rather than reporting a final outcome.

Finish all work and artifact reporting first. For a child run, check your unread inbox and address any parent instructions before finishing. Then run the following as your final action:

```sh
"$OZ_CLI" harness-support finish-task --status <success|failure> --summary '<summary>'
```

- Use `success` when the assigned work is complete, or `failure` when it could not be completed.
- Put the full answer or actionable handoff in `<summary>`: findings or decisions, artifact paths or links, validation results, and any blockers or follow-up needed. For an informational task, include the answer itself, not just "research completed."
- Pass the summary as one shell argument, quoting or escaping its contents correctly.
- Wait for the command to succeed before stopping. If it fails, resolve the error and retry; do not claim completion or fall back to a plain-text final response.
- After a successful finish, stop without repeating the summary as ordinary assistant text.
