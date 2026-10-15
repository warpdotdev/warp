---
name: oz-finish-task
description: Report task completion or failure back to the Oz platform when done with a task.
---
When your task is complete, report the outcome by running:

```sh
"$OZ_CLI" harness-support finish-task --status <success|failure> --summary '<summary>'
```

Replace `<success|failure>` with either `success` or `failure`, and `<summary>` with a brief description of what was accomplished (or what went wrong).
