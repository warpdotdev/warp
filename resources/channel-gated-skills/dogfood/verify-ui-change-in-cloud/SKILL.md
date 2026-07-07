---
name: verify-ui-change-in-cloud
description: >
  Invoke this automatically after completing any user-facing client change, ONLY in non-sandboxed environments and local environments.
  Verifies changes by spawning a cloud agent with computer use to test Warp.
  Triggers a cloud agent that runs the test-warp-ui skill.
user-invocable: true
---

# Verify UI Change in Cloud

After making a user-facing client change, use this workflow to verify it by spawning a cloud agent with computer use capabilities. This applies to any change that affects what the user sees or experiences in the running app — not just visual/UI changes, but also startup behavior, config handling, migration flows, and other client-side logic.

## Workflow

### 1. Push Your Changes

The cloud agent runs in a fresh environment that clones the repo. Your changes must be pushed to a branch so the cloud agent can access them.

### 2. Detect the Repository

Before spawning the cloud agent, detect which repository you are running in. Check the Git remote URL to determine the repo:

```bash
git remote get-url origin
```

Verify the remote URL contains `warpdotdev/warp`. If it does not, warn the user that this skill only supports the warp repository and stop.

The environment ID for the warp Dev Environment is `SVhg783GBFQHk1OfdPfFU9`.

### 3. Spawn the Cloud Agent

Use the `run_agents` tool to spawn a remote cloud agent. A single-child batch (one entry in `agent_run_configs`) is valid.

- `summary`: a brief declarative explanation, e.g. `"Spawning a cloud agent with computer use to verify the UI change."`
- `base_prompt`: include an instruction to read and follow the `test-warp-ui` skill, followed by the verification task (see the next section)
- `remote.environment_id`: `SVhg783GBFQHk1OfdPfFU9`
- `remote.computer_use_enabled`: `true`
- `agent_run_configs`: a single entry with `name` set to a short display name such as `"verify-ui-change"`. The per-agent `prompt` can be empty since `base_prompt` covers the task.

The `test-warp-ui` skill is bundled, so the cloud agent has it automatically. Tell the agent to invoke it by name in the `base_prompt` (e.g. "Read and follow the test-warp-ui skill.").

### 4. Write an Effective Prompt

The prompt should tell the cloud agent:
- Which element, flow, or behavior to test
- What hardcoding or mocking is needed (see below and the test-warp-ui skill for details on sandbox constraints)
- What filesystem or app state to pre-seed before launching (e.g., creating directories, writing config files)
- What specific observations to report back

**Example prompts:**

```
I changed the settings dialog header to use a larger font and blue color.
Hardcode the settings dialog to open on launch, then describe the header text,
font size relative to other text, and color.
```

```
I added a migration that symlinks config from ~/.warp into ~/.warp-preview on first launch.
The migration is gated on Channel::Preview. Before building, hardcode the migration to run
regardless of channel by removing the channel check. Also create a fake ~/.warp directory
with test files. After launching Warp, verify the symlinks were created in ~/.warp-preview.
```

### Hardcoding to reach the code path under test

The cloud agent builds Warp with `cargo run`, which may not match the exact runtime conditions of your change (e.g., different channel, missing feature flags, absent preconditions). When this happens, instruct the cloud agent to temporarily hardcode the code so the build exercises the path you need to test. Common examples:

- **Gated code paths**: If the change is behind a channel check, feature flag, or experiment, tell the agent to remove or bypass the gate before building.
- **Pre-existing state**: If the change depends on filesystem state that wouldn't exist in a clean environment (e.g., a config directory from a prior install), tell the agent to create it before launching.
- **Startup behavior**: If the change affects something that only happens on first launch or migration, make sure the agent sets up the preconditions that trigger it.

Be explicit in the prompt about what to hardcode and why — the cloud agent won't infer this on its own.

### 5. Surface the Cloud Agent Link

No extra surfacing step is needed — the Warp client displays the cloud agent run automatically.
