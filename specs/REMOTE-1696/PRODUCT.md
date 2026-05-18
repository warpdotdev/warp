# Product Spec: Oz CLI Named-Agent CRUD

## Summary
Add first-class Oz CLI commands for managing named agents. Developers can list, inspect, create, update, and delete named agents from the terminal with output modes that match existing Oz run commands.

## Problem
Named agents are manageable through the public API and related Oz surfaces, but the Oz CLI does not expose the full CRUD surface. The existing `oz agent list` command lists repository-discovered agent skills, so the CLI needs a clear split between named agents and skills before adding CRUD commands.

## Goals
- Make `oz agent list` mean "list named agents".
- Preserve skill discovery by renaming the existing command to `oz agent skills`.
- Provide terminal-friendly pretty output, script-friendly plain text, and JSON output with `--jq` filtering.
- Preserve server patch semantics for updates so users only pass fields they want to modify.
- Make list sorting predictable and client-side.

## Non-goals
- Changing the public API behavior or authorization model for named agents.
- Adding interactive prompts for create or update.
- Managing API keys for named agents.
- Changing `oz agent run` or `oz agent run-cloud` semantics beyond continuing to accept an agent UID where already supported.

## Behavior
1. `oz agent skills` lists available agent skills using the current `oz agent list` behavior, including the existing `--repo` option and repository authorization flow.
2. `oz agent list` lists named agents accessible to the authenticated user's team.
3. `oz agent list` supports `--sort-by name` and `--sort-by created-at`.
4. `oz agent list` supports `--sort-order asc` and `--sort-order desc`. If omitted, name sorting defaults to ascending, and timestamp sorting defaults to descending.
5. Sorting is performed client-side after the API response is fetched and before rendering human-readable output formats.
6. `oz agent get <uid>` retrieves a single named agent by UID.
7. `oz agent create` creates a named agent. `--name` is required. Optional fields are accepted for the fields supported by the API: description, secrets, skills, base model, and default environment when that field is available.
8. `oz agent update <uid>` partially updates a named agent. Users only pass fields they want to change.
9. `oz agent update` preserves omitted fields. Passing an empty value through an explicit remove flag clears fields where the API supports clearing.
10. `oz agent update` supports add/remove operations for list fields without requiring users to hand-write replacement arrays. Secrets use `--add-secret <NAME>`, `--remove-secret <NAME>`, and `--remove-all-secrets`; skills use `--add-skill <SPEC>`, `--remove-skill <SPEC>`, and `--remove-all-skills`.
11. `oz agent delete <uid>` deletes a named agent and reports success. If the server rejects deletion, such as for the default agent, the CLI surfaces the server error.
12. Pretty output for list uses a readable table with the core fields needed to identify and choose agents: UID, name, availability, created time, base model, default environment when present, secret count, and skill count.
13. Pretty output for `oz agent list` also includes a hint for users looking for the old skill-discovery behavior: `Looking for agent skills? Use <binary> agent skills instead.`
14. Pretty output for get/create/update uses a detail view that shows the same core fields plus full lists of secret names and skill specs.
15. Plain-text output is stable and script-friendly. List output is tabular text with headers. Single-agent output is line-oriented key/value text.
16. JSON output for list/get/create/update preserves the API response shape so scripts can consume fields exactly as returned by the public API.
17. Sort flags are rejected for JSON output, including `--jq`, so raw API output remains raw.
18. `--jq` implies JSON output even when the global output format is omitted or set to a human-readable format.
19. `--jq` filters list/get/create/update JSON output using the same jq behavior as `oz run list` and `oz run get`, including unquoted scalar output.
20. Delete has no JSON API response body. For JSON output, the CLI emits a minimal operation result containing the deleted UID and `deleted: true`; delete does not support `--jq`.
21. All named-agent CRUD commands require authentication and reuse the existing CLI authentication and API-key behavior.
22. If the server returns authorization, validation, conflict, or not-found errors, the CLI prints the server-facing error through the existing fatal-error path.
23. The command hierarchy remains backward-compatible except for the intentional rename: users who want the previous skill list behavior must use `oz agent skills`.
