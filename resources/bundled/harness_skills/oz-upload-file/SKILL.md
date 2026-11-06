---
name: oz-upload-file
description: Upload a local file to the Oz platform as a conversation artifact.
---
Use this for supplemental files that should be attached to the task but should NOT be committed to the repo or included in the PR — e.g. screenshots, logs, generated reports, or other large or derived outputs.

Do NOT use this for source code, tests, docs, or any change that should be reviewed and merged through a PR. Those belong in a commit, not as an artifact.

Only call this after the file already exists on disk.

```sh
"$OZ_CLI" artifact upload '<path>' --run-id "$OZ_RUN_ID" --description '<description>'
```

Replace `<path>` with the absolute path to the file. Include a `--description` when it adds useful context about what the file is or why it is attached (e.g. "screenshot of failing login page", "profiler output for slow query"). Omit when the file name alone is self-explanatory.
