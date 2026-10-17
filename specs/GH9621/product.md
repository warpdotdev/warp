# PRODUCT.md — Secure runtime secrets for headless agent and skill execution

Issue: https://github.com/warpdotdev/warp/issues/9621

## Summary

Warp should let users use sensitive values in automated agents, skills, scheduled runs, workflows, and cloud environments without saving those values to plaintext files or exporting them into the broader shell session. Users should be able to store a static API key or service account token in Warp-managed secure storage, grant it to a specific automation, and have Warp inject it only at the moment a command or tool needs it.

The desired outcome is a headless-safe secret execution model: no biometric prompt is required once the user has intentionally stored and granted the secret, but the secret value is not written to disk, added to shell profiles, included in the prompt, or made available to unrelated background processes in the terminal session.

## Problem

Warp has user-facing support for environment variable collections and has internal managed-secret infrastructure for agent tasks, but the common automation workflow still pushes users toward insecure patterns:

- save a token in `.env` or another plaintext configuration file;
- export it globally in a shell profile or agent environment;
- rely on an external secret manager that prompts for interactive approval, which blocks headless execution;
- expose the secret to every process in a terminal session rather than only the command that needs it.

This undermines the security value of storing a secret in Warp because using the secret safely is not as easy as storing it safely.

## Goals

- Let users store static secrets, such as third-party API keys and service account tokens, in Warp-managed secure storage and use them in headless automations.
- Let a skill, cloud agent, scheduled agent, or local agent run declare which secrets it needs without embedding secret values in prompts, config files, shell history, or source control.
- Inject secret values only into the specific command or runtime boundary that requested them, not into the parent shell session or all commands in an agent run.
- Make secret usage visible and auditable by secret name, scope, and consuming automation while never showing the secret value.
- Support team-owned and personal secrets with predictable permission checks, revocation, and child-agent delegation rules.
- Preserve current environment variable collection behavior for non-secret values while providing a safer path for secrets.

## Non-goals

- This spec does not replace all existing environment variable collection workflows. Loading a collection into the interactive shell may continue to export values for compatibility.
- This spec does not attempt to make secrets invisible to the process that legitimately receives them. A command that is explicitly granted a secret can read that secret and pass it to child processes it spawns.
- This spec does not provide a generic OS sandbox that prevents a malicious command from exfiltrating a secret it was granted.
- This spec does not require 1Password, LastPass, or another external secret manager. External managers can remain supported as secret sources, but the headless path must work with Warp-managed static secrets.
- This spec does not define a complete visual redesign of Warp Drive or settings beyond the minimum UI needed to create, grant, inspect, and revoke runtime secrets.
- This spec does not make every MCP server or third-party CLI automatically secret-safe. MCP and harness-specific support can be phased in once the core secret-grant model exists.

## Figma / design references

Figma: none provided.

## User experience

### Secret storage and discovery

1. Users can create a Warp-managed secret with a name, optional description, owner scope, and raw value. The value is accepted through a secure input path and is not echoed after creation.
2. Secrets are discoverable by name, description, owner, created time, and updated time. Secret values are never displayed in list, details, search, logs, command output, telemetry, or PR-style artifacts.
3. A secret can be personal or team-owned. Team-owned secrets are available only to users and automations authorized for that team scope.
4. Updating a secret changes future uses but does not expose the previous value. Deleting or revoking a secret causes future automations that depend on it to fail with an actionable missing-secret message.

### Declaring secret requirements

5. Skills can declare required or optional runtime secrets by logical alias, human-readable purpose, and preferred environment variable name. For example, a Monday.com skill can request a `monday_api_token` alias that should be injected as `MONDAY_API_TOKEN` only when a command needs it.
6. Agent configuration files, scheduled agents, cloud-agent runs, and local-agent runs can reference secret names or aliases without including the underlying value. These references are safe to commit when they contain only secret names and scopes.
7. When an automation references a secret that has not been granted, Warp shows a consent step before the run can start. In non-interactive or scheduled contexts, the run fails early with a clear setup error instead of silently falling back to plaintext environment variables.
8. Users can persist a grant for a scheduled agent or reusable agent configuration. Persisted grants allow future headless runs without Touch ID or other interactive prompts, provided the user or team still has permission to the secret.

### Command-scoped injection

9. A secret is injected only for an explicit command or tool execution that requests the secret alias. The parent shell, future commands, unrelated background processes, and the agent prompt do not receive the plaintext value.
10. When a command uses a secret, Warp displays the command with secret placeholders or badges, such as `MONDAY_API_TOKEN=<secret:monday_api_token>`, rather than the raw value.
11. The command result reports which secret aliases were used, but never includes secret values. If a command prints the secret or a known derived token, Warp redacts it before the output is sent back to the model, shown in shared-session viewers, persisted in transcripts, or uploaded as an artifact.
12. If the requested secret is unavailable, revoked, out of scope, or denied by policy, the command does not run. Warp returns an error that names the missing alias and explains how to grant or update it without revealing candidate secret values.
13. If a command requests a secret alias that was not declared by the skill or run configuration, Warp requires explicit user approval before allowing the command to run. In fully autonomous cloud execution, undeclared secret requests are denied by default.
14. Child agents do not inherit a parent run's secrets automatically. Parent agents must explicitly delegate a declared alias when starting a child agent. The child sees only the delegated alias and can use it only according to the same command-scoped rules.

### Warp Drive environment variable collections

15. Existing environment variable collections remain available for users who want to load variables into an interactive shell.
16. Warp provides a secure alternative for secrets in collections: users can attach a managed-secret reference to a variable name and use it in command-scoped execution instead of exporting the value into the session.
17. If a user tries to load a collection containing secret-backed variables into the broader shell environment, Warp warns that this will expose the secret to the session and offers the safer command-scoped path.
18. Non-secret constant variables can still be exported as they are today.

### Headless and cloud behavior

19. Cloud agents and scheduled agents can run with persisted secret grants without a biometric or desktop prompt.
20. Local agents can use the same declaration and command-scoped injection model when Warp can resolve the secret locally or through the task-scoped managed-secret API.
21. If a cloud run starts in an environment with no granted secrets, Warp does not inject any static user secrets by default.
22. Secrets must not be included in model prompts, skill instructions, conversation summaries, handoff snapshots, source-code context, or agent-visible tool schemas except as aliases and metadata.

### Administration, audit, and revocation

23. Users can review which automations are allowed to use each secret, including scheduled agents and reusable configs.
24. Users can revoke a grant without deleting the secret. Revocation prevents future runs from using the secret and does not alter historical transcripts except for already-redacted usage metadata.
25. Team administrators can disable team-secret usage in agents or require manual approval before a team secret is granted to an automation.
26. Audit events record secret name, owner scope, consuming automation, run ID, command/tool category, timestamp, and success/failure state. Audit events never include plaintext secret values or command output containing unredacted secrets.

## Success criteria

- A user can create a raw Warp-managed secret for a third-party API token and grant it to a skill without writing a `.env` file or editing a shell profile.
- A skill can run a command that receives the secret as a process-local environment variable while the parent terminal session does not retain the variable after the command completes.
- A scheduled or cloud agent can reuse a previously granted secret without a Touch ID prompt and without a plaintext secret in the agent config, environment definition, or prompt.
- A command that uses a secret shows a stable placeholder in the UI and transcript, not the secret value.
- A command that prints the exact secret value has that value redacted before it reaches the model-facing tool result and persisted transcript.
- Attempting to use an undeclared or ungranted secret in an autonomous run fails closed.
- Deleting or revoking a secret prevents future runs from using it and produces an actionable error for dependent automations.
- Child agents receive no secrets unless the parent explicitly delegates a declared alias.
- Existing environment variable collection loading for non-secret variables continues to work.
- A user can identify where a secret is used and revoke those grants from Warp-managed metadata.

## Validation

- Product walkthrough: create a personal secret, grant it to a skill, run the skill, and verify the command succeeds without `.env`, shell-profile edits, or visible secret values.
- Headless walkthrough: create a scheduled or cloud run using the grant and verify it runs without interactive prompts.
- Negative test: revoke the grant and verify the next run fails before command execution with a clear missing-grant error.
- Negative test: request an undeclared secret from an autonomous agent and verify the request is denied.
- Redaction test: run a command that intentionally prints the secret and confirm the UI, model-facing result, transcript, shared session, and artifact paths show a redaction marker.
- Session-scope test: after a command-scoped injection completes, run `env` or equivalent in the same session and confirm the secret variable is absent.
- Child-agent test: start a child agent without delegation and verify it cannot use the secret; start one with explicit delegation and verify only the delegated alias works.
- Warp Drive compatibility test: load a collection with non-secret values and confirm existing behavior; attempt to load one with secret-backed values and confirm the warning plus secure-run alternative.

## Open questions

- Should the first implementation expose a new UI for managed secrets, or should it begin with CLI-backed secret creation plus UI for grants and warnings?
- Should command-scoped injection be available to all shell commands through a visible `warp secret exec` wrapper, an agent-only tool field, or both?
- What is the minimum viable secret declaration format for skills: frontmatter, a dedicated `secrets` section, or a separate manifest file?
- Which existing secret types should be supported at launch beyond raw values: Anthropic API keys, AWS Bedrock credentials, OAuth tokens, and external secret-manager references?
- What team policy controls are required before allowing team-owned secrets in autonomous agents?
- How long should secret values remain cached in memory within a run, and should users be able to force a stricter per-command fetch mode?
- Should existing environment variable collections be migrated to managed-secret references automatically when values look sensitive, or should migration always be explicit?
