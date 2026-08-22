# TECH.md — Secure runtime secrets for headless agent and skill execution

Issue: https://github.com/warpdotdev/warp/issues/9621
Product spec: `specs/GH9621/product.md`

## Problem

Warp has several related systems, but none currently provide a complete command-scoped secret execution model for agents and skills:

- Warp Drive environment variable collections serialize values into shell initialization commands, which intentionally exports values into the session.
- External secret-manager integrations can generate shell commands such as `op item get --reveal`, but headless execution still depends on the external CLI being authenticated and often interactive.
- Warp-managed secrets exist and can be attached to ambient tasks, but generic raw secrets are currently converted into environment variables for the agent terminal session, making them available to the whole run rather than to a single requested command.

The implementation should introduce explicit runtime secret grants and command-scoped injection while reusing the managed-secret encryption, task-secret fetch, and agent task infrastructure already present in the repository.

## Relevant code

- `app/src/env_vars/mod.rs:47` — `EnvVar` stores `name`, `value`, and `description`; `EnvVarValue` supports `Constant`, `Command`, and external `Secret` values.
- `app/src/env_vars/mod.rs:77` — `EnvVar::get_initialization_string` renders shell exports such as `export NAME=value;`.
- `app/src/env_vars/mod.rs:99` — `EnvVarValue::Command` and `EnvVarValue::Secret` become command substitutions in the shell initialization string.
- `app/src/env_vars/view/env_var_collection.rs:100` — env-var collections show “Add secret or command. Warp never stores external secrets,” which reflects current external-secret behavior rather than managed runtime secrets.
- `app/src/env_vars/view/env_var_collection.rs:1021` — the invoke/load button is disabled until the collection is saved, then loading runs the generated initialization command.
- `app/src/env_vars/view/secrets.rs:74` — selecting an external secret stores an `EnvVarValue::Secret` row and later resolves it by invoking the external CLI.
- `app/src/external_secrets/mod.rs:54` — external secrets generate shell commands such as `op item get --fields credential --reveal` or `lpass show --password`.
- `app/src/ai/agent_sdk/secret.rs:71` — the CLI exposes `secret create/delete/update/list` behind the `WarpManagedSecrets` feature flag.
- `crates/managed_secrets/src/manager.rs:39` — managed secret creation encrypts values client-side with the owner public key before sending them to the server.
- `crates/managed_secrets/src/manager.rs:159` — `ManagedSecretManager::get_task_secrets` fetches task-scoped secrets with a short-lived workload token.
- `crates/graphql/src/api/queries/task_secrets.rs:4` — GraphQL task-secret fetch returns secret values for a specific task.
- `app/src/server/server_api/managed_secrets.rs:225` — `ServerApi` implements `get_task_secrets` by sending task ID and workload token to the server.
- `app/src/ai/ambient_agents/task.rs:18` — `AgentConfigSnapshot` is the persisted runtime configuration for agent execution.
- `app/src/ai/agent_sdk/config_file.rs:14` — agent config files currently support name, environment, model, prompt, MCP servers, host, and computer-use fields, but not runtime secret declarations.
- `app/src/ai/agent_sdk/ambient.rs:423` — cloud runs build an `AgentConfigSnapshot` and send it in `SpawnAgentRequest`.
- `app/src/ai/agent_sdk/mod.rs:885` — local/worker task initialization sends `merged_config` to `create_agent_task`.
- `app/src/ai/agent_sdk/mod.rs:925` — starting from an existing task fetches task secrets, task metadata, and attachments before building the driver.
- `app/src/ai/agent_sdk/driver.rs:511` — managed secrets are converted into environment variables for the terminal session today.
- `app/src/ai/agent_sdk/driver/terminal.rs:72` — `TerminalDriverOptions` accepts `env_vars` that are installed when the terminal view is opened.
- `crates/ai/src/agent/action/mod.rs:34` — `AIAgentActionType::RequestCommandOutput` stores command metadata but has no secret-binding field.
- `crates/ai/src/agent/action/convert.rs:24` — server `RunShellCommand` tool calls are converted into `RequestCommandOutput` actions.
- `app/src/ai/blocklist/action_model/execute/shell_command.rs:118` — shell-command autoexecution uses command risk/read-only metadata and does not reason about secret grants.
- `app/src/ai/blocklist/action_model/execute/shell_command.rs:850` — command results include full unobfuscated block output, relying on existing redaction elsewhere.
- `app/src/ai/blocklist/permissions.rs:833` — command execution permissions evaluate risk, read-only status, allowlists, denylists, and redirection, but not secret usage.
- `app/src/ai/blocklist/action_model/execute/start_agent.rs:307` — remote child-agent starts pass environment ID and skill references but no explicit secret delegations.

## Current state

Environment variable collections are Warp Drive cloud objects that can hold constant values, command values, or external secret-manager references. Invoking a collection produces shell initialization strings. This is compatible with interactive shell setup, but it means sensitive values or value-producing commands are installed into the session-level environment.

Warp-managed secrets are a separate system. Users can create encrypted managed secrets through the CLI, and ambient tasks can fetch task-scoped secret values via a workload token. Once fetched in the agent SDK path, `AgentDriver::new` currently maps raw managed secrets to environment variables named after the secret and maps provider-specific secrets to fixed variables such as `ANTHROPIC_API_KEY` and AWS Bedrock variables. Those variables are passed to `TerminalDriverOptions` and become part of the terminal session environment.

Agent and skill configuration currently has no first-class way to declare “this automation needs this secret alias.” Shell-command tool calls also have no first-class way to request a secret for one command. The model can only ask to run a command string, and any secret use must be encoded into that command or already present in the environment.

## Proposed changes

### 1. Add runtime secret grants to agent configuration

Extend `AgentConfigSnapshot` with a new optional field, for example:

- `runtime_secrets: Option<Vec<RuntimeSecretGrant>>`

Define `RuntimeSecretGrant` in `app/src/ai/ambient_agents/task.rs` with fields similar to:

- `alias`: stable model/tool-facing name, such as `monday_api_token`.
- `secret_name`: managed secret name stored in Warp.
- `owner`: personal or team owner scope, using the same ownership concepts as managed secrets.
- `env`: optional preferred environment variable name, such as `MONDAY_API_TOKEN`.
- `required`: whether the run should fail early if the grant cannot be resolved.
- `delegable_to_child_agents`: default false.

Update these config paths:

- `app/src/ai/agent_sdk/config_file.rs` to parse a safe `runtime_secrets` or `secrets` key from JSON/YAML config files. Config files must contain only references and aliases, never values.
- `app/src/ai/agent_sdk/ambient.rs` to pass CLI/config-provided grants into cloud-agent `SpawnAgentRequest` config.
- `app/src/ai/agent_sdk/mod.rs` to include grants in local/worker task creation through `create_agent_task`.
- `app/src/ai/agent_sdk/schedule.rs` to persist grants on scheduled agents and support update/removal semantics.
- `app/src/ai/blocklist/action_model/execute/start_agent.rs` and the multi-agent API schema to allow explicit child-agent secret delegation in `StartAgentExecutionMode::Remote`, while defaulting to no delegation.

Server-side work is required to store the references on the task or schedule, validate access at grant time and run time, and ensure `task_secrets` returns only secrets granted to the task. Client-side code should treat server validation as authoritative and fail closed if a required grant is absent.

### 2. Represent secret bindings in shell-command tool calls

Add a secret-binding field to the multi-agent `RunShellCommand` tool schema. A suggested shape is:

- `secret_env`: list of `{ env_name, alias }` entries.

Then update:

- `crates/ai/src/agent/action/mod.rs` to add `secret_env: Vec<RunCommandSecretEnv>` to `AIAgentActionType::RequestCommandOutput`.
- `crates/ai/src/agent/action/convert.rs` to convert API tool-call secret bindings into the action type.
- `app/src/ai/blocklist/inline_action/requested_command.rs` and related rendering code to display secret badges/placeholders and make the secret use reviewable without exposing values.
- `app/src/ai/blocklist/permissions.rs` so autoexecution considers secret usage. If an action requests undeclared or ungranted secrets, deny autoexecution. If all requested aliases are declared and granted, continue applying existing risk/read-only/allowlist checks.

The prompt/tool contract should instruct the model to request secret aliases through `secret_env` instead of embedding secret values or shell-specific secret-fetch snippets into `command`.

### 3. Introduce a runtime secret broker

Add a client-side broker module, for example `app/src/ai/runtime_secrets/`, responsible for:

- loading the task's granted secret metadata and values when available;
- mapping aliases to secret values for a specific task/conversation;
- refusing undeclared aliases;
- producing redaction matchers for exact values and important derived forms;
- clearing in-memory values at run end;
- exposing audit metadata without exposing values.

For ambient tasks, the broker should reuse `ManagedSecretManager::get_task_secrets`. For local interactive conversations, the first implementation can require a server-created task ID before command-scoped secret execution is enabled, matching existing task-scoped managed-secret infrastructure. If no task ID is available, return a clear unsupported/missing-task error rather than falling back to process-global environment variables.

The broker should store secrets in memory only and should never serialize values into `AgentConfigSnapshot`, conversation messages, snapshots, telemetry, logs, or persisted Warp Drive objects.

### 4. Replace generic run-wide secret injection with command-scoped injection

Change `AgentDriver::new` so generic raw managed secrets are no longer converted into terminal-session environment variables by default. Provider and harness authentication secrets that must be present at process startup, such as Claude Code and AWS Bedrock setup, can remain on the current path initially but should be documented as exceptions and migrated to narrower provider-specific setup where practical.

The command execution path should inject requested secrets only for the command being run. Prefer a visible wrapper command with no plaintext values, for example:

- displayed command: `curl ...` plus secret badges;
- executed command: `warp secret exec --run-id <id> --env MONDAY_API_TOKEN=monday_api_token -- <shell> -lc '<original command>'`.

The wrapper is responsible for fetching granted task secrets with the workload identity already available to the run, setting only the requested variables in its own process environment, and `exec`ing the target command. The parent shell receives only the wrapper command and never receives the secret value as an exported variable.

Implementation options:

- Add a `secret exec` or `run secret-exec` subcommand in `crates/warp_cli` and dispatch it from `app/src/ai/agent_sdk/mod.rs` without requiring interactive auth when running inside an isolated task.
- Extend `ShellCommandExecutor` in `app/src/ai/blocklist/action_model/execute/shell_command.rs` to transform a `RequestCommandOutput` with `secret_env` into the wrapper form before emitting `ShellCommandExecutorEvent::ExecuteCommand`.
- Render the original command and secret badges in UI while storing the executed wrapper command separately or redacted, so users understand what was run without seeing values.

If the terminal/session layer grows a safe API for command-specific environment overrides, the wrapper can be replaced later. The first version should avoid shell-string interpolation of plaintext secrets.

### 5. Redact command output and transcripts using granted secret values

The command result path currently reads block output with `output_with_secrets_unobfuscated`. For secret-bound commands, add a redaction pass before returning output to the model or persisting it in conversation/task artifacts.

Update or integrate with existing redaction code in:

- `app/src/ai/agent/redaction.rs`
- `app/src/ai/blocklist/block/secret_redaction.rs`
- `app/src/server/telemetry/secret_redaction_tests.rs`
- `app/src/integration_testing/secret_redaction/assertion.rs`

The runtime secret broker should provide exact value matchers for granted secrets. For structured secret values, redact all sensitive fields. Redaction must apply to:

- `RequestCommandOutputResult` and `ReadShellCommandOutputResult` sent back to the model;
- block snapshots uploaded by third-party harnesses;
- shared-session output visible to viewers when the output is generated by a secret-bound command;
- telemetry and logs that include command output snippets.

### 6. Add skill-level secret declarations

Add a shared parser for optional skill secret declarations. The exact format can be finalized with product, but a minimal YAML frontmatter shape is sufficient:

- `secrets:`
- `  - alias: monday_api_token`
- `    env: MONDAY_API_TOKEN`
- `    description: Monday.com API token`
- `    required: true`

The skill loader should expose these declarations to the agent run setup flow. User-facing behavior:

- if a skill declares a required alias and no grant exists, ask the user to select/create a managed secret before starting interactive runs;
- for non-interactive runs, fail early with a setup error;
- include declarations in the prompt/tool context as aliases only, never values.

Relevant starting points include `app/src/ai/skills/skill_manager.rs` and `crates/ai/src/skills/conversion.rs`.

### 7. Integrate with Warp Drive environment variable collections

Add managed-secret references to environment variable collections without breaking existing exports. A possible model extension is a new `EnvVarValue::ManagedSecretReference` containing secret owner, secret name, and optional alias. For compatibility:

- `get_initialization_string` should not silently expand a managed-secret reference to plaintext.
- Existing “Load” behavior should warn and require explicit confirmation if the user chooses to export a secret-backed value into the shell.
- Add a secure-run path that converts selected secret-backed variables into command-scoped `secret_env` bindings.

This lets Warp Drive remain the organization surface for variables while shifting sensitive values to managed secrets.

### 8. Audit and revocation

Add server-backed audit events and grant metadata. At minimum, the client should send or the server should derive:

- secret name and owner scope;
- run ID/task ID/schedule ID;
- consuming skill or agent config name;
- command/tool category;
- timestamp and success/failure status.

Do not include command output or raw command strings if they may include user-provided sensitive data. Grant revocation should be enforced by the server before `task_secrets` returns values and by the client broker before command execution.

## End-to-end flow

1. User creates a managed secret named `monday_api_token` and stores the Monday.com API token value.
2. A skill declares `monday_api_token` as required and prefers injection as `MONDAY_API_TOKEN`.
3. User starts the skill. Warp sees the declaration, asks the user to grant a personal or team secret if no persisted grant exists, and creates/updates the task config with the alias-to-secret reference.
4. Server validates the grant and stores it on the task or schedule. The model receives only the alias and usage instructions.
5. The model calls `run_shell_command` with `command: "python scripts/sync_monday.py"` and `secret_env: [{ env_name: "MONDAY_API_TOKEN", alias: "monday_api_token" }]`.
6. `ShellCommandExecutor` checks permissions, asks the runtime secret broker to resolve `monday_api_token`, and transforms the execution into command-scoped secret execution.
7. The wrapper fetches the task-scoped secret, sets `MONDAY_API_TOKEN` only for the target process, and runs the command.
8. Output is redacted with the granted secret value before it is displayed in model-facing results or persisted artifacts.
9. After the command exits, the parent shell does not contain `MONDAY_API_TOKEN`. The broker clears cached values at run completion.

## Risks and mitigations

- Risk: command-scoped env vars are still visible to the command and its children. Mitigation: document that this is intentional least-privilege injection, not a malicious-command sandbox; preserve existing risky-command permission prompts.
- Risk: the wrapper command could leak secret aliases or grant names in shell history. Mitigation: aliases and names are not secret values; avoid writing plaintext values and consider redacting aliases if product deems names sensitive.
- Risk: existing agents depend on raw managed secrets being injected run-wide. Mitigation: gate the behavior behind a feature flag, migrate known callers to explicit secret bindings, and temporarily keep provider/harness-specific injection where required.
- Risk: redaction misses transformed secret values. Mitigation: start with exact-value redaction and structured fields; add provider-specific derived-token redaction only when known and testable.
- Risk: persistent grants may unintentionally authorize future automation after a secret changes. Mitigation: show grants on the secret detail page, support revocation independent of deletion, and audit each use.
- Risk: team secrets in autonomous agents can surprise admins. Mitigation: add team policy controls and fail closed when policy is unknown or disabled.
- Risk: environment variable collection migration could break users who expect “Load” to export everything. Mitigation: keep existing behavior behind confirmation, add secure-run as an additional path, and avoid automatic migration in the first implementation.

## Testing and validation

- Unit test `AgentConfigSnapshot` serialization/deserialization with runtime secret grants and verify unknown/plaintext value fields are rejected.
- Unit test config file parsing in `app/src/ai/agent_sdk/config_file_tests.rs` for `secrets`/`runtime_secrets` precedence and error messages.
- Unit test `RunShellCommand` conversion so `secret_env` reaches `AIAgentActionType::RequestCommandOutput` without modifying the command string.
- Unit test command permission evaluation: declared and granted aliases pass normal command checks; undeclared aliases are denied; revoked aliases produce a missing-grant result.
- Unit test runtime secret broker resolution using mocked `ManagedSecretsClient` and task-secret responses.
- Unit test wrapper command construction to verify no plaintext secret is present in the displayed or executed shell string.
- Integration test a cloud/ambient run that uses a raw managed secret through command-scoped injection and then verifies the parent session environment does not retain the variable.
- Redaction tests for exact secret output across command results, transcript restoration, block snapshots, and telemetry paths.
- Scheduled-agent validation: create a schedule with a persisted grant, run it headlessly, revoke the grant, and verify the next run fails before command execution.
- Warp Drive compatibility tests for constant-only collections, secret-backed collection warning, and secure-run binding generation.

## Follow-ups

- Add a richer UI for creating and managing managed secrets if the first implementation ships with CLI-first creation.
- Migrate Claude, Codex, Gemini, AWS Bedrock, and other harness/provider authentication paths away from run-wide env injection where each provider supports a narrower mechanism.
- Support external secret-manager references as managed-secret sources for users who still want 1Password or LastPass as the source of truth.
- Add secret usage analytics that count usage by alias and run type without collecting values, command output, or sensitive command arguments.
- Consider a future terminal/session API for command-specific environment overrides that avoids a visible wrapper command while preserving the same security boundary.
- Consider optional secret rotation reminders and stale-grant warnings for long-lived service account tokens.
