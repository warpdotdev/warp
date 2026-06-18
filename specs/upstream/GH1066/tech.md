# Update Onboarding Agent Autonomy Settings for Full and Partial — Tech Spec
Product spec: `specs/GH1066/product.md`
GitHub issue: https://github.com/warpdotdev/warp-external/issues/1066
## Context
Onboarding presents three autonomy options on the agent slide. When the user completes onboarding with Agent-Driven Development selected, `apply_onboarding_settings` translates the chosen `AgentAutonomy` into `ActionPermission` values and writes them onto the default `AIExecutionProfile`.
Investigating the original issue surfaced three related bugs in the plumbing that connects the onboarding UI to the default profile. The tech spec therefore covers four changes, summarized up front so the overall shape is easy to follow:
1. **Autonomy → permission mapping** in `app/src/settings/onboarding.rs` — the original scope of the issue.
2. **Partial subtitle copy** on the agent slide — the original scope of the issue.
3. **`edit_profile_internal` persistence when no personal drive is available** in `app/src/ai/execution_profiles/profiles.rs` — fixes onboarding-driven writes being silently dropped for logged-out users.
4. **`InitialLoadCompleted` reconciliation** in `app/src/ai/execution_profiles/profiles.rs` — fixes existing users' cloud default profile being missed after login, which had been causing onboarding to create a duplicate cloud object.
5. **Preserve existing cloud default profile in `apply_agent_settings`** in `app/src/settings/onboarding.rs` — ensures the user's stored cloud values survive when they log into an existing account at the end of onboarding.
Relevant code:
- `crates/onboarding/src/slides/agent_slide.rs` — `autonomy_options` array in `render_autonomy_section` (Full/Partial/None titles, subtitles, and the `AgentAutonomy` variants dispatched on click) and the `AgentAutonomy` enum / `AgentDevelopmentSettings::new` (keeps `Partial` as the default selected autonomy).
- `app/src/settings/onboarding.rs` — `apply_onboarding_settings`, `apply_agent_settings`, `OnboardingAutonomyPermissions`, `action_permissions_for_onboarding_autonomy`.
- `app/src/ai/execution_profiles/mod.rs` — `ActionPermission` / `WriteToPtyPermission` enums. Already expose `AlwaysAllow`, `AlwaysAsk`, `AgentDecides`; no new variants needed.
- `app/src/ai/execution_profiles/profiles.rs` — `AIExecutionProfilesModel`, `DefaultProfileState` (`Unsynced` / `Synced` / `Cli`), `edit_profile_internal`, `handle_cloud_model_event`, and the `set_*` setters called by `apply_agent_settings`.
- `app/src/workspaces/user_workspaces.rs` (via `UserWorkspaces::ai_autonomy_settings`) — the `has_override_for_*` helpers consumed by `apply_agent_settings`. Behavior is unchanged; the new defaults continue to respect overrides the same way.
- `app/src/cloud_object/model/persistence.rs` — `CloudModel::update_objects_from_initial_load` (the cloud bulk-load path that suppresses per-object events) and `CloudModelEvent::InitialLoadCompleted` (the single event fired at the end of that path).
- `app/src/auth/auth_state.rs` — `AuthStateProvider` and `AuthState::user_id()`, which `UserWorkspaces::personal_drive()` consults to decide whether a personal drive is available.
See product spec for user-visible behavior.
## Proposed changes
### 1. Update the Partial subtitle on the onboarding agent slide
In `crates/onboarding/src/slides/agent_slide.rs`, update the second entry of the `autonomy_options` array in `render_autonomy_section` so the Partial row's subtitle reads:
```
Can plan, read files, and execute low-risk commands. Asks before making any changes or executing sensitive commands.
```
This is a pure string swap inside `autonomy_options`. The Full row and None row are unchanged. No layout, height, or keyboard-navigation code changes — the row height constant `OPTION_HEIGHT = 72.` already accommodates two-line wrapping of comparable length strings, and `render_two_line_button` wraps subtitle text internally.
### 2. Update the autonomy → permission mapping
In `app/src/settings/onboarding.rs`, rewrite the `AgentAutonomy::Full` and `AgentAutonomy::Partial` arms of `action_permissions_for_onboarding_autonomy` so the returned `OnboardingAutonomyPermissions` match the values in invariants 5 and 6 of the product spec. The `AgentAutonomy::None` arm is unchanged.
```rust
match autonomy {
    AgentAutonomy::Full => OnboardingAutonomyPermissions {
        apply_code_diffs: ActionPermission::AlwaysAllow,
        read_files: ActionPermission::AlwaysAllow,
        execute_commands: ActionPermission::AlwaysAllow,
        mcp_permissions: ActionPermission::AlwaysAllow,
        write_to_pty: WriteToPtyPermission::AlwaysAllow,
    },
    AgentAutonomy::Partial => OnboardingAutonomyPermissions {
        apply_code_diffs: ActionPermission::AlwaysAsk,
        read_files: ActionPermission::AlwaysAllow,
        execute_commands: ActionPermission::AgentDecides,
        mcp_permissions: ActionPermission::AgentDecides,
        write_to_pty: WriteToPtyPermission::AlwaysAsk,
    },
    AgentAutonomy::None => OnboardingAutonomyPermissions {
        apply_code_diffs: ActionPermission::AlwaysAsk,
        read_files: ActionPermission::AlwaysAsk,
        execute_commands: ActionPermission::AlwaysAsk,
        mcp_permissions: ActionPermission::AlwaysAsk,
        write_to_pty: WriteToPtyPermission::AlwaysAsk,
    },
}
```
The caller (`apply_agent_settings`) continues to gate each `set_*` call on the workspace override flag and does not need to change shape for this mapping update: its logic is "if the workspace does not enforce this permission, write the onboarding-derived value." The caller *does* gain a preservation guard (change 5 below) but the guard does not affect the mapping itself.
### 3. MCP and `write_to_pty` defaults
The issue text does not mention MCP or `write_to_pty`, but per reviewer feedback the MCP defaults for Full and Partial are aligned with the updated autonomy semantics so the resulting profile is internally consistent with each option's subtitle:
- Full: `mcp_permissions = AlwaysAllow` (changed from `AgentDecides`) so "without asking" is true for MCP tool calls as well.
- Partial: `mcp_permissions = AgentDecides` (changed from `AlwaysAsk`) so low-risk MCP tool calls can proceed without a prompt, mirroring how `execute_commands` behaves for Partial.
- None: `mcp_permissions = AlwaysAsk` (unchanged).
- `write_to_pty` is unchanged for every variant: `AlwaysAllow` for Full, `AlwaysAsk` for Partial and None.
Calling this out explicitly here so reviewers don't read the MCP change as an accidental out-of-scope tweak.
### 4. Persist Unsynced edits locally when no personal drive is available
In `AIExecutionProfilesModel::edit_profile_internal` (`app/src/ai/execution_profiles/profiles.rs`), the `Unsynced` branch previously required a `personal_drive` to transition to `Synced` and silently returned otherwise — dropping the caller-supplied profile mutation. That dropping path is why the onboarding Full/Partial selections never showed up for logged-out users: every `set_*` on the default profile started from the original `Unsynced` clone, mutated it, then threw it away.
The fix splits the branch into two arms:
- `personal_drive` available → create a cloud object and transition to `Synced`, as before.
- `personal_drive` unavailable → write the mutated profile back into `DefaultProfileState::Unsynced` so subsequent reads (and subsequent edits after login) see the updated values.
Both arms emit `AIExecutionProfilesModelEvent::ProfileUpdated(profile_id)` so views re-render.
This keeps the invariant that "the default profile is never reverted" and guarantees onboarding-driven writes on a logged-out user accumulate locally. When that user later logs in and makes an edit, the first edit path will promote the accumulated local profile to a single cloud-backed default object (assuming no cloud default arrives from the account in the meantime — see change 5).
### 5. Reconcile execution-profile state on `InitialLoadCompleted`
The initial cloud load path (`CloudModel::update_objects_from_initial_load` → `upsert_from_server_object_internal`) intentionally inserts objects into `CloudModel` with `emit_events = false`. It emits a single `CloudModelEvent::InitialLoadCompleted` at the end rather than per-object `ObjectCreated` events. `AIExecutionProfilesModel::handle_cloud_model_event` previously only reacted to per-object events, so execution profiles loaded during initial login sync were invisible to the model: it stayed in `Unsynced` even though the user already had a cloud default.
Add a handler for `CloudModelEvent::InitialLoadCompleted` that calls a new `reconcile_with_cloud_state_after_initial_load`. Reconciliation:
- If the model is `Unsynced` and `CloudModel` contains an execution profile flagged `is_default_profile`, transition to `Synced` adopting the cloud profile's `sync_id` under the existing `ClientProfileId`. Emits `ProfileUpdated`.
- For any non-default profile already in `CloudModel` that isn't tracked in `profile_id_to_sync_id`, register a fresh `ClientProfileId → sync_id` mapping so later edits target the real cloud object. Emits `ProfileCreated` if any non-default profile was newly registered.
This is a pure catch-up on state; it does not modify any cloud object. Combined with change 4, the end-to-end state for an existing user logging in after onboarding is: onboarding's pre-login local writes (if any) land on the Unsynced profile, then initial load arrives, then `InitialLoadCompleted` reconciliation replaces the local Unsynced state with the existing cloud profile's `sync_id`. The local onboarding writes are intentionally discarded at that point in favor of the user's stored values — see change 6 for what happens next.
### 6. Preserve the existing cloud default profile inside `apply_agent_settings`
In `app/src/settings/onboarding.rs`, `apply_agent_settings` runs from `CloudPreferencesSyncer::InitialLoadCompleted` (in `handle_cloud_preferences_syncer_event`) once the user is logged in and the cloud reconciliation from change 5 has already run. If we unconditionally write the onboarding-selected base_model and permissions on top of the now-`Synced` default profile, we clobber values the user previously stored in the cloud.
Add a short-circuit at the top of the `AIExecutionProfilesModel::handle(app).update` closure:
```rust
let default_profile_info = profiles.default_profile(ctx);
let default_profile_id = *default_profile_info.id();

if default_profile_info.sync_id().is_some() {
    // Existing cloud default profile — preserve stored values.
    return;
}
```
Semantics:
- `sync_id().is_some()` means the default profile is backed by a cloud object (either loaded at startup from SQLite cache, or reconciled from the post-login initial load via change 5). That's the marker of "existing user."
- `sync_id().is_none()` keeps its existing meaning: a fresh `Unsynced` local default (brand-new user or new account that has no cloud default yet, possibly with accumulated onboarding writes from change 4). Those continue to flow through the `set_base_model` + permission setters, which will either update the local Unsynced profile (still no personal drive) or promote it to a single cloud object (personal drive now available).
Scope of the short-circuit is strictly the execution-profile block. The preceding `AISettings` updates in `apply_agent_settings` (`default_session_mode`, `should_render_cli_agent_footer`, `show_agent_notifications`) and the sibling `apply_ui_customization_settings` / `is_any_ai_enabled` writes in `apply_onboarding_settings` are **not** affected — onboarding continues to set those for existing users per invariant 14.
## Testing and validation
Each numbered invariant in `specs/GH1066/product.md` maps to at least one test or manual step below.
### Unit tests
Two sibling test files following the `${filename}_tests.rs` + `#[cfg(test)] #[path = ...] mod tests;` convention:
- `app/src/ai/execution_profiles/profiles_tests.rs` covers the execution-profile model changes.
  - `edits_persist_on_unsynced_default_profile_when_logged_out` — builds the model with a logged-out `AuthStateProvider` (via a new `AuthStateProvider::new_logged_out_for_test()` helper in `app/src/auth/auth_state.rs`), calls `set_apply_code_diffs(AlwaysAllow)`, and asserts the default profile now reads `AlwaysAllow`. This would fail under the pre-fix `edit_profile_internal` because the mutated profile was dropped. Guards change 4 and invariant 13.
  - `reconciles_unsynced_default_profile_with_cloud_after_initial_load` — seeds a cloud default profile via `CloudModel::update_objects_from_initial_load` (the no-events path), emits `CloudModelEvent::InitialLoadCompleted`, asserts the model adopted the cloud profile's `sync_id`, and verifies a subsequent edit targets that same `sync_id` (proving there is no duplicate). Guards change 5 and invariants 14/15's "no duplicate default profiles" clause.
- `app/src/settings/onboarding_tests.rs` covers the existing-user preservation behavior end-to-end.
  - `apply_onboarding_settings_preserves_existing_cloud_profile_on_existing_user_login` — seeds an existing user's cloud default profile with distinguishable stored values (`base_model = "claude-existing-cloud-model"`, permissions all `AlwaysAllow`), fires the initial-load reconciliation, calls `apply_onboarding_settings` with a `SelectedSettings::AgentDrivenDevelopment` that picks a different model and `AgentAutonomy::None` (which would map to every permission being `AlwaysAsk`), and asserts every preserved field still reads its cloud-stored value. Guards change 6 and invariant 14.
The earlier pure-function table-driven test over `action_permissions_for_onboarding_autonomy` was removed in favor of these end-to-end tests, which cover the mapping transitively and also catch the plumbing issues.
### Manual validation
- Fresh onboarding, pick **Full**, finish the flow. Open Settings → AI → Execution Profiles and confirm the default profile shows `apply_code_diffs`, `read_files`, `execute_commands`, and `mcp_permissions` all as "Always allow," and `write_to_pty` as "Always allow" (invariant 5).
- Fresh onboarding, pick **Partial**. Confirm `apply_code_diffs = Always ask`, `read_files = Always allow`, `execute_commands = Agent decides`, `mcp_permissions = Agent decides`, `write_to_pty = Always ask` (invariant 6). Also confirm the Partial subtitle on the slide reads the new text (invariant 3).
- Fresh onboarding, pick **None**. Confirm all permissions show "Always ask" (invariant 7).
- From a user in a team workspace that enforces `execute_commands` (or any other supported override), run onboarding and pick Full. Confirm only the enforced field is left untouched and the other fields follow the new Full defaults (invariant 8).
- Visually verify the slide still renders three rows at the same height and the keyboard up/down cycling between Full → Partial → None (and wrap-around) still works, confirming no regression to invariants 1, 9, 12.
- Start an agent session after onboarding with Partial. Issue a request whose command is auto-approve eligible and confirm it runs without a prompt; issue a request whose command is sensitive / outside the allowlist and confirm the agent pauses for approval (invariant 10, Partial row).
- Start an agent session after onboarding with Full. Issue the same kinds of requests and confirm the agent never pauses for read-file, apply-code-diff, or execute-command actions (invariant 10, Full row).
- **Brand-new user, skip login at the end of onboarding.** Finish onboarding picking Full, click "Skip login" on the login slide. Open Settings → AI → Execution Profiles without logging in and confirm the default profile shows the Full autonomy values (not the bare `AIExecutionProfile::default()` values). Guards invariant 13. Log in afterward and confirm the same values are preserved and eventually backed by a single cloud object (no duplicate default profiles).
- **Existing user, log in at the end of onboarding.** Log into an account that already has a default execution profile whose permissions differ from the onboarding-selected autonomy. Finish onboarding picking an autonomy that differs from what's stored (e.g. pick Full when cloud has Partial-ish values). Confirm that after login Settings → AI → Execution Profiles shows the user's previously stored values, not the onboarding values. Guards invariant 14.
- **Brand-new account, sign up at the end of onboarding.** Finish onboarding picking Partial, then sign up for a fresh account. Confirm exactly one default execution profile exists in the account and its values match Partial. Guards invariant 15.
## Risks and mitigations
### Risk: changing defaults overwrites explicit Settings choices (brand-new user re-running onboarding)
`apply_agent_settings` writes permission fields on the default profile whenever onboarding is completed and the profile is still `Unsynced`. A user who completed onboarding, kept their local Unsynced profile, tweaked `apply_code_diffs` in Settings, and then re-ran onboarding would see those explicit changes overwritten.
Mitigation: for any user who has logged in and whose profile has promoted to `Synced`, the preservation guard in change 6 now prevents that overwrite. The remaining case (fully logged-out user re-running onboarding) is an extreme edge case and acceptable given the rest of the mitigation story.
### Risk: Full autonomy now grants `AlwaysAllow` for execute commands
Moving from `AgentDecides` to `AlwaysAllow` on `execute_commands` removes the `AgentDecides`-mediated "ask when uncertain" prompt for sensitive commands. This matches the subtitle, but it is a meaningful increase in Full's default ambient permission.
Mitigation: the command denylist still takes precedence even when `execute_commands = AlwaysAllow`, so commands on the denylist continue to be blocked / gated by the existing enforcement path rather than being auto-run. `AlwaysAllow` is also the existing enum variant already used by `write_to_pty` for Full autonomy, so runtime enforcement of the new default is consistent with what's already shipping and no new codepath is introduced. The slide copy explicitly advertises "without asking," so users who select Full are opting into this behavior for everything outside the denylist.
### Risk: preservation guard could "hide" onboarding from a user whose cloud profile is unexpectedly present
If an existing user with a stale cloud default profile re-runs onboarding expecting their selections to apply, the preservation guard will keep the stored cloud values. This is the desired behavior per invariant 14 but could be surprising.
Mitigation: the user can still edit the default profile explicitly in Settings; those edits go through the Synced branch of `edit_profile_internal` and update the cloud object. Non–execution-profile onboarding settings (session default, CLI agent footer, notifications, UI customization) continue to apply regardless, so the onboarding flow is not a no-op for existing users.
### Risk: `InitialLoadCompleted` reconciliation replaces Unsynced local edits with cloud values
If a logged-out user makes onboarding-driven edits (stored locally per change 4), then logs into an existing account, the reconciliation in change 5 discards the local Unsynced state and adopts the cloud profile. The user ends up with their cloud-stored values, not the onboarding selections they just made.
Mitigation: this is the desired behavior per invariant 14 — existing users expect their stored profile to win over transient onboarding selections. Non-execution-profile AISettings still follow the onboarding selection, so the user's onboarding work is not entirely thrown away.
### Risk: string-level test coupling
Asserting the exact Partial subtitle locks the test to the slide copy. Since the end-to-end tests don't assert subtitles anymore (only behavior), this risk was eliminated as a byproduct of the test restructuring.
## Follow-ups
- Consider whether the agent-slide subtitles should become localizable strings pulled from a single source of truth instead of inline literals; deferred because localization of onboarding is a larger, orthogonal project.
- Consider exposing the onboarding-seeded defaults in Settings as a labeled "onboarding preset" that users can re-apply without re-running onboarding. Out of scope for this change.
- Consider whether `update_objects_from_initial_load` should emit per-object events so that models don't need custom `InitialLoadCompleted` reconciliation. That would be a broader refactor affecting every model that subscribes to `CloudModel` events; revisit if a second model hits the same pitfall.
