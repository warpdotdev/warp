# TECH.md — Preseed Gemini CLI config to skip onboarding

## 1. Context
When a cloud agent run uses the Gemini harness, we launch `gemini --yolo -i "$(cat <prompt>)"` in a non-interactive terminal. On a fresh machine Gemini CLI prompts twice before accepting input:

1. An auth-type picker (`Login with Google`, `Gemini API key`, `Vertex AI`, …), driven by `security.auth.selectedType` in `~/.gemini/settings.json`.
2. A folder-trust dialog for the current working dir, driven by `~/.gemini/trustedFolders.json`.

Both prompts block the run since no one is at the TUI. We need to preseed both files so Gemini starts straight into the prompt, while leaving other user-owned state in those files alone.

The harness layer already has the hook we need: `ThirdPartyHarness::prepare_environment_config` runs in `AgentDriver::prepare_harness` (`app/src/ai/agent_sdk/driver.rs:1476`) right before `build_runner`, and errors are wrapped into `AgentDriverError::HarnessConfigSetupFailed { harness, error }` (`driver.rs:367`). The Claude harness already implements this hook (`app/src/ai/agent_sdk/driver/harness/claude_code.rs:45`) using two private helpers, `read_json_file_or_default` and `write_json_file`, to patch Claude's own JSON config. Gemini's impl was a no-op default (`harness/mod.rs:55-61`, `harness/gemini.rs` pre-change).

Relevant files:
- `app/src/ai/agent_sdk/driver/harness/mod.rs:55-61` — default `prepare_environment_config` no-op.
- `app/src/ai/agent_sdk/driver/harness/gemini.rs` — Gemini harness; gets the new impl.
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs (45-56, 688-723 pre-change)` — existing Claude impl + JSON helpers.
- `app/src/ai/agent_sdk/driver.rs:1471-1476` — where `prepare_environment_config` is invoked.

## 2. Proposed changes
### 2a. Extract shared JSON helpers
Move `read_json_file_or_default` and `write_json_file` out of `claude_code.rs` into a new `app/src/ai/agent_sdk/driver/harness/json_utils.rs`, declared from `harness/mod.rs`. Claude's impl already uses the exact same read → parse-or-default → pretty-write idiom Gemini needs; a second per-harness copy would drift. Visibility stays `pub(super)` so the helpers are shared by sibling harness modules but not public API.

### 2b. Implement `GeminiHarness::prepare_environment_config`
Take `working_dir` (was `_working_dir`), resolve `~/.gemini` via `dirs::home_dir()`, and call two focused helpers:

- `prepare_gemini_settings(path)` — read `settings.json` into a typed `GeminiSettings { security: Option<GeminiSecurity { auth: Option<GeminiAuth { selected_type, .. }> }>, .. }`, set `security.auth.selectedType = "gemini-api-key"`, write back pretty JSON. Each level uses `#[serde(flatten)] extra: Map<String, Value>` so unrelated keys at every nesting level round-trip verbatim.
- `prepare_gemini_trusted_folders(path, working_dir)` — read `trustedFolders.json` into `HashMap<String, String>`, insert `working_dir → "TRUST_FOLDER"`, write back. The flat map shape matches Gemini's on-disk schema exactly, so no wrapper struct is needed.

Constants (`GEMINI_API_KEY_AUTH_TYPE = "gemini-api-key"`, `GEMINI_TRUST_LEVEL_FOLDER = "TRUST_FOLDER"`, file/dir names) live at the bottom of `gemini.rs` and link to the upstream Gemini source (`packages/core/src/core/contentGenerator.ts`, `packages/cli/src/config/trustedFolders.ts`) in doc comments so we know what to update if Gemini changes their discriminants.

Errors bubble up through `anyhow::Result` and are converted at the `ThirdPartyHarness` boundary into `HarnessConfigSetupFailed { harness: "gemini", error }`, matching Claude.

### 2c. Design notes
- **Typed structs vs `serde_json::Value`** for `settings.json`: typed wins because we only touch one field and want compile-time safety on the path; the `flatten` extras map gives us lossless round-trip of everything else without hand-written merge logic.
- **Why not `HashMap<String, Value>` for settings too**: Gemini's config is nested; a flat map would require either walking by string key at each level or silently clobbering siblings. The typed path makes the invariant "only `security.auth.selectedType` is ours" explicit.
- **Surfacing malformed JSON as an error** (rather than overwriting) is deliberate: the file is user-owned and may contain hand edits; silently clobbering it on a parse error would be a footgun. Tests lock this in.

## 3. Testing and validation
All tests live in `app/src/ai/agent_sdk/driver/harness/gemini_tests.rs`. Each invariant below maps to one or more tests against a `TempDir`-backed path.

- **Fresh install produces the expected settings** — `prepare_gemini_settings_creates_file_with_api_key_auth`: from a missing file, the helper writes `security.auth.selectedType = "gemini-api-key"`.
- **User edits survive** — `prepare_gemini_settings_preserves_unrelated_keys`: top-level (`ui.theme`), sibling-branch (`security.folderTrust.enabled`), and sibling-of-target (`security.auth.enforcedType`) keys all round-trip unchanged while `selectedType` is added.
- **Malformed config is surfaced, not clobbered** — `prepare_gemini_settings_surfaces_malformed_json_as_error`: a wrong-typed `security` causes `Err`, leaving the file on disk untouched.
- **Idempotent** — `prepare_gemini_settings_is_idempotent`: two back-to-back calls produce byte-identical output.
- **Trusted folder is written for the working dir** — `prepare_gemini_trusted_folders_creates_file_with_working_dir`: the new entry maps `working_dir → "TRUST_FOLDER"`.
- **Other trust entries survive** — `prepare_gemini_trusted_folders_preserves_existing_entries`: `TRUST_PARENT` / `DO_NOT_TRUST` entries for unrelated paths are preserved alongside the new entry.
- **Re-trusting overrides prior level** — `prepare_gemini_trusted_folders_overwrites_prior_level_for_same_path`: a previous `DO_NOT_TRUST` on the same path becomes `TRUST_FOLDER`.

Unit tests cover the helpers directly; `prepare_environment_config` itself is a thin wrapper and doesn't need its own test.

Manual validation (one-off, not automated): run a cloud agent with the Gemini harness on a fresh environment and confirm the TUI boots directly into the prompt with no auth / trust prompts. Presubmit (`./script/presubmit`) covers fmt + clippy + the new tests.

## 4. Risks and mitigations
- **Gemini changes the discriminant strings** (`"gemini-api-key"`, `"TRUST_FOLDER"`). Mitigation: constants are documented with links to the upstream files so a Gemini CLI upgrade that changes them is a grep away; a broken run would surface as the prompt returning, which is noisy and catchable during bring-up.
- **`trustedFolders.json` uses a flat `HashMap<String, String>`**, so a future schema change to an object per entry would silently fail parse. We'd see `HarnessConfigSetupFailed` immediately — same error path as malformed user JSON — which is the right signal.
- **`~/.gemini` is shared with a user's local Gemini CLI**. The selected-type patch is harmless (users can change it back in-UI) and the trusted-folder write is strictly additive for the current working dir; no existing keys are removed.

## 5. Follow-ups
- REMOTE-1407 — pipe the agent system prompt into Gemini (TODO in `GeminiHarnessRunner::new`).
- REMOTE-1408 — upload the Gemini conversation transcript in `save_conversation`, alongside the block snapshot we already upload.
