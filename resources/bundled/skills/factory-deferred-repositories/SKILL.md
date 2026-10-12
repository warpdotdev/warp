---
name: factory-deferred-repositories
description: Check out a Factory's deferred repository on demand with environment-checkout, without overwriting existing checkouts or exposing credentials
---

# Deferred Factory repositories

The run's initial repository inventory supplies a checkout runtime and typed checkout requests for repositories attached to this Factory but not cloned yet. Use that inventory, not the current working directory or a guessed forge.

1. Select only repositories the task needs. Copy their inventory `checkout request` objects, preserving each `source` identity. If the inventory marks a target-name collision, change `checkout_name` to an unused single directory name under the workspace root. Never replace or write inside an existing checkout, directory, file, or symlink.
2. Create a private scratch directory outside the checkout targets. Write a fresh JSON requests file with exactly `{"working_dir": "<inventory runtime working_dir>", "repositories": [<selected request objects>]}`. Use the inventory's absolute `working_dir`, even when the shell has automatically changed into the sole eager repository. Do not invent extra fields.
3. Invoke the inventory's shell-quoted absolute executable using its `command_template`. Replace each entire quoted placeholder argument (`'<requests-file>'` and `'<report-file>'`, including its quotes) with a shell-quoted absolute path in the private scratch directory. The command is top-level `environment-checkout`, not `agent-driver environment-checkout`. Keep `--fail-if-target-exists`; do not use `--remove-origins-only` or substitute another executable found on PATH.
4. Run in the existing agent shell, retaining its HOME, environment and preconfigured Git credential helper. Never replace credentials, disable authentication, or put an access token in a prompt, command, URL, log, or file. Do not fall back to raw `git clone` if the helper is unavailable; report the blocked checkout.
5. Capture the command's exit status and inspect the complete fresh report before treating any checkout as successful. Require one `outcomes` entry for every submitted `request_index`, no missing or duplicate indices, and `failure: null` for each successful checkout. On nonzero exit, missing/malformed report, or a `Clone`, `Checkout`, or `RemoveOrigin` failure, identify the affected request and its redacted diagnostics. A failed batch may still have created successful checkouts; inspect them rather than replaying the entire batch.
6. Never retry into a partially created target or automatically delete, reset, or remove an origin from a checkout target. Inspect a failed target first; choose an unused `checkout_name` if a retry is needed. Remove only the private scratch request/report files you created, after inspecting the report.

An on-demand checkout creates a Git working tree only. It does not automatically load repository skills, discover file-based MCP servers, run environment setup commands, initialize the build cache, or register the repository with Oz codebase indexing.
