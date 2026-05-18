# Technical Spec: Oz CLI Named-Agent CRUD

## Context
`specs/REMOTE-1696/PRODUCT.md` defines the user-visible behavior.

The CLI command definitions live in `crates/warp_cli/src/agent.rs`. The old skill-discovery command has moved to `AgentCommand::Skills(ListAgentSkillsArgs)` with an optional `--repo` flag. The app-side dispatcher in `app/src/ai/agent_sdk/mod.rs` sends that variant to `agent_config::list_skills`.

The existing skill-discovery implementation is in `app/src/ai/agent_sdk/agent_config.rs`. It calls `AIClient::list_skills(repo)` and renders repository-discovered `AgentSkillItem` values.

Run listing/getting in `app/src/ai/agent_sdk/ambient.rs (60-116)` is the output model to follow. It accepts `JsonOutput`, fetches raw API JSON for JSON or `--jq`, and uses `output::print_raw_json`. The reusable output helpers are in `app/src/ai/agent_sdk/output.rs (1-258)`.

The public API methods and types flow through `app/src/server/server_api/ai.rs`. The `AIClient` trait exposes skill listing as `list_skills(repo)` and named-agent CRUD as `list_agents`, `list_agents_raw`, `get_agent`, `get_agent_raw`, `create_agent`, `create_agent_raw`, `update_agent`, `update_agent_raw`, and `delete_agent`. The underlying `ServerApi` exposes authenticated GET/POST/PUT/DELETE helpers for public API commands.

The named-agent API reference is in `../warp-server/public_api/openapi.yaml`:
- `POST /agent/identities` and `GET /agent/identities` at `../warp-server/public_api/openapi.yaml (2709-2781)`.
- `GET /agent/identities/{uid}`, `PUT /agent/identities/{uid}`, and `DELETE /agent/identities/{uid}` at `../warp-server/public_api/openapi.yaml (2783-2917)`.
- `CreateAgentRequest`, `UpdateAgentRequest`, `AgentResponse`, and `ListAgentsResponse` at `../warp-server/public_api/openapi.yaml (5399-5539)`.

Related named-agent environment work for `REMOTE-1695` adds an optional `environment_id` field to create/update/response models. The current checked-out OpenAPI schema does not yet show that field; the implementation should keep optional deserialization tolerant so the CLI can display it when the server begins returning it.

## Proposed changes
Add a new named-agent CLI surface and move the old skill-discovery command:
- Rename the old skill-discovery command to `AgentCommand::Skills(ListAgentSkillsArgs)` with `#[command(name = "skills")]`.
- Add `AgentCommand::List(AgentListArgs)`, `Get(AgentGetArgs)`, `Create(AgentCreateArgs)`, `Update(AgentUpdateArgs)`, and `Delete(AgentDeleteArgs)`.
- Keep the old skill-discovery behavior and `--repo` intact under the renamed skills command.
- Keep parser tests focused on custom conflict behavior rather than retesting clap's ordinary subcommand and repeated-flag parsing.

Command arguments:
- `AgentListArgs`: `--sort-by <name|created-at>`, `--sort-order <asc|desc>`, and flattened `JsonOutput`.
- `AgentGetArgs`: positional `uid` and flattened `JsonOutput`.
- `AgentCreateArgs`: required `--name`; optional `--description`; repeatable `--secret <NAME>`; repeatable `--skill <SPEC>`; optional `--base-model <MODEL_ID>`; optional `--environment <ENVIRONMENT_ID>` once the API field is available; flattened `JsonOutput`.
- `AgentUpdateArgs`: positional `uid`; optional `--name`; optional `--description` conflicting with `--remove-description`; repeatable `--add-secret`; repeatable `--remove-secret`; `--remove-all-secrets` conflicting with individual secret add/remove flags; repeatable `--add-skill`; repeatable `--remove-skill`; `--remove-all-skills` conflicting with individual skill add/remove flags; optional `--base-model` conflicting with `--remove-base-model`; optional `--environment` conflicting with `--remove-environment`; flattened `JsonOutput`.
- `AgentDeleteArgs`: positional `uid`. It intentionally does not flatten `JsonOutput` because delete has no API response body to filter with `--jq`.

Add app-side named-agent command handling:
- Create a new module such as `app/src/ai/agent_sdk/agent_management.rs` to avoid mixing named-agent CRUD with skill discovery.
- Route the renamed skills command to the existing `agent_config::list_skills`.
- Route named-agent CRUD commands from `run_agent` to the new module.
- Update `command_requires_auth` so every named-agent command and the renamed skills command require authentication.
- Add telemetry variants for `AgentSkills`, `AgentList`, `AgentGet`, `AgentCreate`, `AgentUpdate`, and `AgentDelete`, or at minimum preserve the existing `AgentList` telemetry for the renamed skills command and add distinct variants for new CRUD commands if the telemetry enum supports adding them.

Add API types and methods in `app/src/server/server_api/ai.rs`:
- Define serde types for `SecretRef`, `CreateAgentRequest`, `UpdateAgentRequest`, `AgentResponse`, `ListAgentsResponse`, `AgentSkillItem`, and supporting skill response types.
- Model `created_at` as `DateTime<Utc>`.
- Model `environment_id` as an optional field to tolerate both the currently checked-in schema and the related server work.
- For create requests, skip absent optional fields and omit empty lists.
- For update requests, use nested `Option`/custom serde helpers as needed so omitted fields are not serialized, remove flags serialize empty string or empty array, and non-empty values serialize replacements.
- Add `AIClient` trait methods for typed named-agent CRUD plus raw JSON list/get/create/update methods used by JSON output and `--jq`, using the names `list_agents`, `list_agents_raw`, `get_agent`, `get_agent_raw`, `create_agent`, `create_agent_raw`, `update_agent`, `update_agent_raw`, and `delete_agent`.
- Implement the methods with `GET/POST/PUT/DELETE /api/v1/agent/identities`.
- Add reusable `put_public_api`, `put_public_api_response`, and `delete_public_api_unit` helpers to `ServerApi` if no existing helper covers those verbs.

Output behavior:
- Implement `TableFormat` for a list row wrapper around `AgentResponse`.
- For list, fetch typed responses for pretty/text/ndjson and raw JSON for JSON/`--jq`. Apply client-side sorting before output for pretty/text/ndjson only.
- Reject `--sort-by` or `--sort-order` when JSON output is requested or implied by `--jq`, because JSON mode should preserve raw API output rather than returning a client-mutated response.
- In pretty list output, print `Looking for agent skills? Use <binary> agent skills instead.` after the named-agent list, where `<binary>` is resolved through the existing CLI binary-name helper.
- For get/create/update, pretty output should render a detail table; text output should render stable key/value lines; JSON/`--jq` should process the raw API response.
- For delete, text/pretty should say the agent was deleted; JSON output should print `{ "uid": "<uid>", "deleted": true }`. Delete does not support `--jq`.

Update references and naming:
- Update user-facing help strings so "agents" means named agents and "skills" means repository-discovered skills.
- Update examples only if the CLI docs/examples mention `oz agent list` as skill discovery.

## Testing and validation
- Keep CLI parse tests only for custom conflicts:
  - `agent update <uid> --description <value> --remove-description` is rejected.
  - `agent update <uid> --add-secret <NAME> --remove-all-secrets` is rejected.
- Add unit tests for update request serialization to prove omitted fields are absent, remove flags serialize empty values, and replacement values serialize correctly.
- Add unit tests for client-side secret and skill delta helpers.
- Add unit tests for list sorting by name and created time.
- Add unit tests that list sort flags are rejected for JSON and `--jq` output.
- Add output tests for named-agent detail text/pretty helpers where they can be tested without a full app context.
- Run `cargo fmt`.
- Run targeted Rust tests for `warp_cli` and the new agent SDK module.
- Run a focused clippy command if compile/test scope reveals warnings.

## Parallelization
Parallel child agents are not necessary for the first implementation pass. The change is medium-sized but tightly coupled across command definitions, SDK dispatch, API methods, and output tests; parallel edits would likely collide in `agent.rs`, `mod.rs`, and `ai.rs`. If this grows beyond the planned scope, validation can be delegated later to a child agent in a separate worktree after the main API/CLI shape lands.

## Risks and mitigations
- **Patch semantics are easy to break:** use serialization tests for omitted, remove, and replacement cases.
- **The old `agent list` behavior is being renamed:** keep the renamed `agent skills` behavior otherwise unchanged and include the pretty-output hint for users looking for skills.
- **Raw JSON sorting can diverge from "raw API" expectations:** reject sort flags when producing JSON output.
