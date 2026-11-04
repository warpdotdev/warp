---
name: oz-finish-task
description: Reports a task as succeeded or blocked. This MUST be used before stopping. For child agents, also sends the summary to the parent.
---
# Finish Task
You MUST use run the command below before stopping when your task is complete or has ended unsuccessfully, even if it was purely informational. A plain-text final response does not replace reporting completion through this command:

```sh
"$OZ_CLI" harness-support finish-task --status <success|failure> --summary '<summary>'
```

- Use `--status success` when the assigned work is complete; this reports `SUCCEEDED`.
- Use `--status failure` when you cannot complete the assigned work or require input and are stopping the run; this reports `BLOCKED`, not `FAILED`, which is reserved for infrastructure failures. The CLI accepts `success` or `failure`, not `blocked`.
- If you are a child, `<summary>` is sent as a message to your parent, so should contain your full reply or actionable handoff to the parent
- If you are not a child, `<summary>` should be a brief description of what was accomplished (or what went wrong).
