---
name: oz-finish-task
description: Reports a task as succeeded or blocked before stopping, including informational tasks. For child agents, also sends the summary to the parent.
---
# Finish Task
You MUST use this skill before stopping when your task is complete or has ended unsuccessfully, even if it was purely informational or no code changed. A plain-text final response, CLI exit, or harness stop notification does not replace reporting completion.

Use this skill for both parent/root and child runs. It updates the task status to `SUCCEEDED` or `BLOCKED`. If you are a child agent, it also sends the summary as a message to the parent agent's inbox for either outcome; do not also use `oz run message send` to report that same outcome.

To send a message while keeping the task in progress, use `oz run message send` without reporting a final outcome. If you cannot proceed and are stopping the run, report `BLOCKED` using the failure option below. A blocked run can resume when it receives a follow-up.

Finish all work and artifact reporting first. For a child run, check your unread inbox and address any parent instructions before finishing. Then run the following as your final action:

```sh
"$OZ_CLI" harness-support finish-task --status <success|failure> --summary '<summary>'
```

- Use `--status success` when the assigned work is complete; this reports `SUCCEEDED`.
- Use `--status failure` when you cannot complete the assigned work or need input before continuing and are stopping the run; this reports `BLOCKED`, not `FAILED`. The CLI accepts `success` or `failure`, not `blocked`.
- Put the full answer or actionable handoff in `<summary>`: findings or decisions, artifact paths or links, validation results, and any blockers or follow-up needed. For an informational task, include the answer itself, not just "research completed."
- Pass the summary as one shell argument, quoting or escaping its contents correctly.
- Wait for the command to succeed before stopping. If it fails, resolve the error and retry; do not claim completion or fall back to a plain-text final response.
- After a successful finish, stop without repeating the summary as ordinary assistant text.
