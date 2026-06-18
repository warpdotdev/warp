# Skip third-party CLI onboarding in cloud harnesses — Tech Spec

Linear: [REMOTE-1247](https://linear.app/warpdotdev/issue/REMOTE-1247)

## Problem

When Oz runs third-party CLI agents like Claude Code or Codex in a fresh cloud environment (for example a Namespace-backed runner), the CLI may start an interactive first-run onboarding flow before executing the user's prompt. That breaks the harness contract that cloud agents should run autonomously and produce useful output without manual terminal interaction.

For Claude Code specifically, the first-run flow currently asks the user to choose a theme, approve `ANTHROPIC_API_KEY` auth, trust the current project directory, and confirm bypass-permissions mode.

## Relevant code

- `app/src/ai/agent_sdk/driver/harness/claude_code.rs` — Claude harness validation, command construction, runner startup, and transcript collection from `~/.claude`
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — shared `ThirdPartyHarness` / `HarnessRunner` traits and validation helpers used by CLI-specific harnesses
- `resources/bundled/skills/oz-platform/SKILL.md` — product-facing guidance for launching cloud agents with third-party CLIs
- `resources/bundled/skills/oz-platform/references/third-party-clis.md` — per-CLI auth and non-interactive invocation docs that should stay aligned with harness behavior

## Current state

`ClaudeHarness::validate()` requires `ANTHROPIC_API_KEY` to be present in the harness secrets map, and `ClaudeHarnessRunner::start()` launches Claude with:

```sh path=null start=null
claude --session-id <uuid> --dangerously-skip-permissions < '<prompt-file>'
```

That command bypasses tool permission prompts, but it does not guarantee Claude has already persisted first-run config. In a fresh home directory, Claude may still stop on onboarding and project-trust prompts before consuming the prompt file.

`ClaudeHarnessRunner::save_conversation()` already assumes Claude's state lives under `~/.claude` (or `$CLAUDE_CONFIG_DIR`) and reads transcript/todo artifacts from that tree. So the harness already has one CLI-specific config-root concept, but only for reading artifacts after execution, not for preparing config before execution.

The bundled Oz CLI docs also describe `ANTHROPIC_API_KEY` as automatic auth and do not document any harness-side onboarding suppression.

## Proposed changes

### 1. Add a harness preparation step for CLI-specific config files

Extend the third-party harness flow with an explicit pre-start step that can materialize config files in the target environment's home/config directories before the CLI process starts, but only when the harness is running in an isolated sandbox environment.

Add this as a new `ThirdPartyHarness` hook, e.g. `prepare_environment_config(&self, working_dir, secrets)`, with a default no-op implementation for harnesses that do not need config writes. The shared orchestration should call this hook only after verifying the current environment is an isolated sandbox via `warp_isolation_platform::detect().is_some()`. That treats any detected isolation platform as eligible for config prep, including future `IsolationPlatformType` variants, and avoids inferring sandbox-ness from harness type alone. That keeps the call site generic while leaving the file formats and merge logic CLI-specific.

For Claude Code, the prep step should create/update:

- `~/.claude.json`
- `~/.claude/settings.json`

Implement this as a read/merge/write operation over the existing JSON files, not as a blind overwrite:

1. Read `~/.claude.json` / `~/.claude/settings.json` if they exist; otherwise start from an empty JSON object.
2. Update only the harness-owned keys needed for onboarding, project trust, and bypass-permissions.
3. Preserve unrelated existing keys and nested values verbatim where possible.
4. Write the merged JSON files back to disk and create parent directories if they are missing.

Use `serde_json::Value` for this merge logic instead of serde structs. The harness only needs to patch a few known object paths while preserving unknown keys, and Claude owns the broader config schema. Direct JSON-object mutation keeps that contract explicit and avoids over-modeling third-party config fields.

### 2. Claude onboarding suppression via `~/.claude.json` and `~/.claude/settings.json`

For Claude, model and write the onboarding and permissions state needed to skip the known first-run prompts:

```json path=null start=null
{
  "hasCompletedOnboarding": true,
  "projects": {
    "~/workspace/": {
      "hasTrustDialogAccepted": true
    }
  }
}
```

and:

```json path=null start=null
{
  "skipDangerousModePermissionPrompt": true
}
```

The project trust key should be derived from the actual harness working directory instead of hardcoding `~/workspace/`. Use the full `working_dir` path string for the `projects[...]` key, because Claude's trust map stores the cwd with slashes intact rather than using the transcript directory encoding.

### 3. Leave API key onboarding out of the initial implementation

For this ticket, only preseed Claude's onboarding, project-trust, and bypass-permissions config. Do not implement `customApiKeyResponses` suffix merging or `apiKeyHelper` yet.

That keeps the first pass narrowly focused on removing the deterministic onboarding dialogs that block autonomous execution, while avoiding extra coupling to Claude auth internals before we know we need it. If Claude's API-key approval prompt still appears after the non-auth config writes above, handle auth config in a follow-up with a separate implementation decision.

### 4. Make config preparation extensible across harnesses

Do not bake Claude-specific file writes into a generic runner path in a way that makes Codex/Gemini support harder. Instead, push CLI-specific prep into each `ThirdPartyHarness` implementation and keep the shared orchestration generic.

A likely shape is:

- shared harness orchestration checks `warp_isolation_platform::detect().is_some()` and only calls a per-harness prep method before spawning the CLI command when any isolation platform is detected
- Claude prep writes Claude onboarding/trust/permissions config files
- future Codex prep can handle `~/.codex/*` or login bootstrap independently

This keeps the top-level operation stable (`prepare harness config files`) while allowing per-CLI implementation details to differ.

## End-to-end flow

```mermaid path=null start=null
autonomous cloud run
  participant Oz as Oz harness
  participant FS as Home/config files
  participant CLI as Claude Code
  participant API as Anthropic API

  Oz->>FS: read/merge/write ~/.claude.json and ~/.claude/settings.json
  Oz->>CLI: launch claude --session-id ... --dangerously-skip-permissions
  CLI->>FS: read onboarding, trust, and permission config
  CLI->>API: authenticate from existing ANTHROPIC_API_KEY handling
  CLI-->>Oz: execute prompt without interactive onboarding
  Oz->>FS: read ~/.claude transcript/todo artifacts
```

Main Claude flow:

1. Harness validates the CLI binary and required managed secret.
2. If `warp_isolation_platform::detect().is_some()`, harness prepares Claude config files in the environment home directory for the session's working directory. Otherwise it skips config preparation and preserves the user's existing local CLI state.
3. Harness launches Claude with the prompt file and bypass-permissions flag.
4. Claude starts in an already-onboarded, already-trusted project state and uses the existing `ANTHROPIC_API_KEY` auth path.
5. Harness periodically/finally reads transcript artifacts from `~/.claude` and uploads them.

## Risks and mitigations

- **Brittle coupling to Claude's private config schema.** Mitigate by keeping all Claude-specific assumptions isolated to `ClaudeHarness`, preserving unrelated JSON keys, adding version-aware tests where possible, and documenting the exact fields we rely on in this spec.
- **Accidental overwrite of user-supplied config.** Always read the existing Claude JSON first, merge only harness-owned keys into that parsed object, and preserve unrelated fields. Add unit tests that start from pre-populated `~/.claude.json` / `~/.claude/settings.json` fixtures and assert unrelated keys survive the write.
- **Unexpected local config mutation outside sandboxed runs.** Gate `prepare_environment_config` behind an explicit `warp_isolation_platform::detect().is_some()` check in shared harness orchestration, and add tests for the non-sandbox path to verify no Claude config files are created or modified.

## Testing and validation

- Unit-test Claude config preparation against temp HOME/CLAUDE_CONFIG_DIR directories:
  - creates `~/.claude.json` and `~/.claude/settings.json` when absent
  - reads pre-existing Claude config files, merges only harness-owned fields, and preserves unrelated top-level and nested keys
  - uses the expected project trust key derived from the harness working directory
  - leaves existing `customApiKeyResponses` and other auth-related fields unchanged in this first-pass implementation
- Test merging configs, as if there is an existing local config file.
- Test that shared harness orchestration skips `prepare_environment_config` when `warp_isolation_platform::detect()` returns `None`, and does call it when detection returns any isolation platform.
- Run a Claude harness in a fresh environment with no existing Claude config and verify the first terminal output is the agent executing the prompt, not onboarding UI.
- Verify transcript upload still works from `~/.claude` after config prep.

## Follow-ups

- Decide and implement Claude API-key prompt suppression if onboarding/trust/permission config alone does not fully remove first-run interactivity. Options are merging `customApiKeyResponses.approved` or using `apiKeyHelper`.
- Add Codex/Gemini-specific config prep if they still trigger first-run auth/onboarding in fresh Oz images.
- Consider supporting user-provided harness config overlays so advanced users can opt into selected local CLI settings in cloud runs.
- Update `resources/bundled/skills/oz-platform/SKILL.md` and `resources/bundled/skills/oz-platform/references/third-party-clis.md` once this behavior is implemented and verified.
- Add telemetry around whether harness prep succeeded and whether the CLI still emitted known onboarding prompts, if we need production visibility into drift.
