*Spec: TUI default execution profile uses AgentDecides for shell commands*

== PRODUCT ==
*Summary:* When Warp starts the headless TUI with no explicitly configured
`agents.execution_profiles` collection, its materialized default agent profile
must use `ActionPermission::AgentDecides` for `execute_commands`. The GUI must
retain its existing `ActionPermission::AlwaysAsk` default, and any profile
collection the user explicitly configured must remain unchanged.

*Key design choices:* Keep the shared `AIExecutionProfile::default` conservative
(`AlwaysAsk`) and define `AIExecutionProfile::default_profile_for_tui` as the
same default with `execute_commands == AgentDecides`. Reuse that default while
building the existing legacy-settings seed so command lists and every other
field retain their current behavior.
Treat an explicitly set collection as authoritative, and preserve the existing
denylist precedence over the less restrictive profile setting.

*Behavior* (numbered, testable invariants):
1. A fresh `LaunchMode::Tui` with no explicitly set
   `AISettings::execution_profiles` collection materializes the reserved
   default profile with `execute_commands == AgentDecides`.
2. A fresh GUI/default path (`LaunchMode::App`/`Test` using the existing
   non-TUI default path) continues to materialize
   `execute_commands == AlwaysAsk`; the shared `AIExecutionProfile::default`
   remains `AlwaysAsk`.
3. If `AISettings::execution_profiles` is explicitly set before model
   construction, both TUI and GUI paths preserve the stored collection
   exactly, including an explicitly stored `AlwaysAsk` or `AlwaysAllow`
   `execute_commands` value and any non-default profiles.
4. The TUI seed changes no field other than `execute_commands`: profile identity
   and name, `apply_code_diffs`, `read_files`, `write_to_pty`,
   `mcp_permissions`, `ask_user_question`, `run_agents`, model selections,
   context limits, autosync/web-search flags, command allowlist, directory
   allowlist, and command denylist retain the values produced by the existing
   seed.
5. The TUI-only behavior does not alter CLI, remote-server, feature-gated GUI,
   cloud migration, or other callers of the shared default constructor.
6. With the resulting TUI default, a command matching the profile or
   organization command denylist still requires approval even when
   `execute_commands` is `AgentDecides`; denylist precedence remains intact.

== TECH ==
*Context:* At commit
`a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`, launch-mode selection in
`app/src/ai/execution_profiles/profiles.rs:64-101 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`
selects the settings-backed source for `LaunchMode::Tui`, while GUI
`LaunchMode::App`/`Test` and CLI/remote modes retain their existing sources.
`AIExecutionProfilesModel::new` at
`app/src/ai/execution_profiles/profiles.rs:183-299 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`
seeds a TUI collection only when `execution_profiles.is_value_explicitly_set()`
is false, inserting the profile returned by
`create_default_from_legacy_settings`.

`create_default_from_legacy_settings` at
`app/src/ai/execution_profiles/mod.rs:95-116 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`
copies the legacy command denylist, user-added command allowlist, and directory
allowlist, then uses struct-update syntax with `AIExecutionProfile::default`.
The shared default at
`crates/cloud_object_models/src/ai_execution_profile.rs:394-421 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`
sets `execute_commands` to `AlwaysAsk`, which is correct for the GUI and other
shared callers but currently leaks into the fresh TUI seed. Effective command
permission lookup reads the selected profile at
`app/src/ai/blocklist/permissions.rs:334-349 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`;
`can_autoexecute_command` checks the merged denylist before applying the
profile-level execution setting at
`app/src/ai/blocklist/permissions.rs:899-976 @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`.
Existing model coverage is in
`app/src/ai/execution_profiles/profiles_tests.rs @ a41e5846bf2bcfa4e56dc0f203799d1bbc6d1eeb`.

The reported defect was reproduced at the code path during triage and the
focused test was re-attempted for this spec. The current environment cannot
create Cargo's pinned git dependency directory
`/usr/local/cargo/git/db/core-foundation-rs-efc549ee4b022354` (`Permission
denied (os error 13)`), so the regression command must be rerun once dependency
cache permissions are available.

*Design alternatives*:
- Change `AIExecutionProfile::default.execute_commands` to `AgentDecides`.
  This is a small diff, but it changes every consumer of the shared default,
  including GUI fallback/migration and any future non-TUI caller. Reject:
  the permission default is security-sensitive and violates the GUI invariant.
- Change `create_default_from_legacy_settings` globally. This keeps profile
  assembly centralized, but the helper is shared by TUI, GUI legacy fallback,
  and migration, so it has the same cross-surface blast radius. Reject.
- Add `AIExecutionProfile::default_profile_for_tui` and use it as the base for
  the existing missing-collection seed. Select this option: it names the
  surface-specific default, reuses the existing list and field initialization,
  preserves the shared default and GUI/migration paths, and is directly
  testable without changing persistence semantics.

*Proposed changes:* Add `AIExecutionProfile::default_profile_for_tui`, built
with struct-update syntax from `AIExecutionProfile::default` and changing only
`execute_commands` to `ActionPermission::AgentDecides`. Build the TUI
legacy-settings seed from that profile and insert it under
`ExecutionProfileId::default_profile()`. Keep the
`!is_value_explicitly_set()` guard unchanged. Do not modify
`AIExecutionProfile::default`, GUI `App`/`Test` construction, CLI profile
construction, cloud migration, or denylist evaluation. Add focused model tests
beside the existing `profiles_tests.rs` coverage, using its singleton setup and
launch-mode fixtures.

*Open questions resolved:* The ticket explicitly scopes the behavior to the
TUI, requires GUI `AlwaysAsk`, and requires explicit user profiles to remain
untouched. Triage and the independent investigation confirmed that the missing
TUI collection is the only incorrect seed and that the shared default is used
by GUI and other callers. The target is the client repository
`warpdotdev/warp`; no server change is required. This changes headless
permission/model initialization rather than rendered layout or interaction, so
verification is code-level (regression tests plus presubmit), with no
computer-use screenshot requirement.

*Risks / blast radius:* `AgentDecides` can execute commands without prompting
when its heuristic is confident. Accidentally changing the shared default,
GUI fallback, an explicit collection, or denylist evaluation would weaken
controls outside the request. Mitigate with paired TUI/GUI default tests,
explicit-collection preservation tests, field-by-field seed comparisons,
denylist-precedence coverage, adjacent migration/isolation tests, and the full
presubmit gate.

*Validation & verification criteria* (must ALL pass before merge):
1. Add and run a regression test named
   `tui_missing_collection_seeds_agent_decides_for_execute_commands` (or an
   equivalently explicit name) in
   `app/src/ai/execution_profiles/profiles_tests.rs`. Start with a fresh
   settings graph whose `execution_profiles` value is not explicitly set,
   construct `AIExecutionProfilesModel` with `LaunchMode::Tui`, and assert that
   the persisted reserved default profile and the model's `default_profile()`
   both report `execute_commands == ActionPermission::AgentDecides`. The test
   must fail on the current base commit (`AlwaysAsk`) and pass after the fix.
2. In that TUI regression test (or a paired focused test), configure non-empty
   legacy command allowlist/denylist and directory allowlist values, then assert
   the seeded profile retains those values. Also compare every unrelated
   permission/profile field with the existing legacy seed, proving that only
   `execute_commands` is overridden.
3. Add and run
   `tui_explicit_collection_preserves_execute_commands` (or equivalent) that
   pre-populates an explicitly set TUI collection with a default profile whose
   command permission is `AlwaysAsk` plus at least one custom profile and
   non-default field. Construct the TUI model and assert the collection remains
   equal to the pre-populated value; the TUI seed must not run.
4. Add and run
   `gui_default_execute_commands_remains_always_ask` (or equivalent) using the
   existing GUI `App`/`Test` default path with no cloud default and no explicit
   collection. Assert the effective default is `AlwaysAsk` and
   `AIExecutionProfile::default().execute_commands` is still `AlwaysAsk`.
5. Add or extend an explicit-profile GUI guard (or parameterize the explicit
   collection test) so a GUI collection with stored `AgentDecides`,
   `AlwaysAsk`, and/or `AlwaysAllow` values is returned unchanged; this proves
   the TUI override cannot rewrite GUI user data.
6. Exercise command permission evaluation with the resulting TUI default:
   a command matching the profile's denylist must return a denied result with
   `CommandExecutionPermissionDeniedReason::ExplicitlyDenylisted`, and an
   organization denylist entry must also remain effective. Use the existing
   `BlocklistAIPermissions::can_autoexecute_command` test seam or the narrowest
   deterministic equivalent.
7. Re-run the original focused profile test/reproduction after restoring the
   Cargo dependency-cache permissions:
   `cargo test --manifest-path /workspace/warp/Cargo.toml -p warp --lib ai::execution_profiles::profiles_tests::tui_missing_collection_seeds_agent_decides_for_execute_commands -- --nocapture`.
   Record the before/after result: the base TUI seed is `AlwaysAsk`, while the
   fixed TUI seed is `AgentDecides`, and the GUI/explicit-profile guards pass.
8. Run the adjacent execution-profile regression set in
   `app/src/ai/execution_profiles/profiles_tests.rs`, including migration,
   explicit-collection, shared-profile ownership/isolation, and settings
   persistence tests, to demonstrate no profile-selection or migration
   regressions.
9. Run `./script/presubmit` from the client repository root and require its
   formatting, lint, test, and build checks to pass. This is a headless model
   initialization change; no UI/computer-use step is required.
