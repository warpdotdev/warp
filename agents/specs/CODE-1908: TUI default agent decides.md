*Spec: TUI default execution profile uses Agent Decides for shell commands*

== PRODUCT ==
*Summary:* When Warp starts the headless TUI without an explicit
`agents.execution_profiles` collection, its materialized default agent profile
must use `AgentDecides` for `execute_commands` (running shell commands). The
same request must not change GUI defaults or overwrite profiles/settings that a
user has already explicitly configured.

*Key design choices:* Keep the shared permission default conservative
(`AlwaysAsk`), and apply the less restrictive value only at the TUI-only
missing-collection seed. Preserve the existing command denylist and every other
profile field; an explicit collection is authoritative and must bypass the
seed.

*Behavior* (numbered, testable invariants):
1. A fresh TUI launch with no explicitly set `agents.execution_profiles`
   collection creates the default profile and sets
   `execute_commands == ActionPermission::AgentDecides`.
2. A GUI launch with no user profile remains on the existing default path and
   materializes `execute_commands == ActionPermission::AlwaysAsk`.
3. A TUI or GUI launch with an explicitly set profile collection returns the
   stored `execute_commands` value unchanged, including an explicit
   `AlwaysAsk` or `AlwaysAllow` value.
4. The TUI-only override changes no other permission or profile data. In
   particular, the command denylist remains intact and continues to take
   precedence over `AgentDecides`; allowlists, file permissions, PTY/MCP
   permissions, model selections, and profile identity remain as stored or
   produced by the existing seed.
5. The change applies only to the interactive TUI launch mode. CLI and remote
   launch modes retain their dedicated behavior, and the shared
   `AIExecutionProfile::default` remains `AlwaysAsk`.

== TECH ==
*Context:* At commit `1d9be246cb413217d83c9b3cf2cc26aedf3a0a75`, launch-mode
selection in `app/src/ai/execution_profiles/profiles.rs:61-116` routes TUI
through the settings-backed profile source while GUI `App`/`Test` launches
are feature-flagged and legacy/CLI/remote modes use other paths. In
`app/src/ai/execution_profiles/profiles.rs:181-209`, `AIExecutionProfilesModel::new`
checks for a TUI launch with no explicitly set execution-profile collection and
inserts the default profile returned by
`create_default_from_legacy_settings`. That helper at
`app/src/ai/execution_profiles/mod.rs:95-115` copies the legacy command
denylist/allowlist and directory allowlist, then uses struct update syntax.
The shared `AIExecutionProfile::default` at
`crates/cloud_object_models/src/ai_execution_profile.rs:414-449` currently sets
`execute_commands` to `AlwaysAsk`, so changing it would broaden the autonomy
change to GUI and other callers.

The focused reproduction was attempted during triage with
`cargo test -p warp --lib ai::execution_profiles::profiles_tests::ignores_shared_default_profile_created_from_cloud -- --nocapture`;
the runner could not fetch the pinned `core-foundation` dependency because Cargo
was denied access to `/usr/local/cargo/git/db/...` (`Permission denied (os error
13)`). Static path inspection confirms the reported behavior and the
TUI-vs-GUI boundary; the regression criteria below must be run once the
dependency-cache permissions are available.

*Design alternatives*:
- Change `AIExecutionProfile::default.execute_commands` to `AgentDecides`.
  This is the smallest textual diff, but it changes every consumer of the
  shared default, including GUI fallback/legacy profiles and any future
  non-TUI caller. Reject because this permission default is security-sensitive
  and violates the GUI invariant.
- Change `create_default_from_legacy_settings` globally. This keeps the
  profile construction centralized, but that helper is used by GUI legacy
  fallback and migration as well as TUI, so it has the same cross-surface
  blast radius. Reject.
- Add a launch-mode-specific TUI seed override in the existing TUI branch
  (or a helper called only by that branch), after obtaining the existing legacy
  seed. Select this option: it preserves the shared default and GUI/migration
  paths, keeps the existing denylist/allowlist copying in one place, and makes
  the security boundary visible at the point where TUI is selected.

*Proposed changes:* In the `AIExecutionProfilesModel::new` TUI missing-
collection branch, materialize the current legacy seed, set only its
`execute_commands` field to `ActionPermission::AgentDecides`, and insert that
profile into the default-profile key. Do not enter this branch when
`is_value_explicitly_set()` is true. Do not modify
`AIExecutionProfile::default`, the GUI `App`/`Test` branches, CLI profile
construction, or command denylist evaluation. Add focused model tests beside
the existing `profiles_tests.rs` coverage; use the existing singleton setup
and `LaunchMode` fixtures rather than introducing a second profile
initialization path.

*Open questions resolved:* The ticket explicitly scopes the change to the TUI,
requires GUI `AlwaysAsk`, and requires existing explicit profiles to remain
unchanged. Triage confirmed the missing-collection seed and the shared default
call chain. The target is the client repository `warpdotdev/warp`, despite the
generic foreman configuration naming `warp-server`; no server change is
required. This is a headless settings/model initialization change, so code-level
tests and presubmit are the verification surface; no rendered UI or
computer-use proof is required.

*Risks / blast radius:* `AgentDecides` permits the agent to execute commands
without a prompt when it is confident, so accidentally changing the shared
default, a GUI fallback, an explicit profile, or the denylist would weaken
security controls outside the request. Mitigate with paired TUI/GUI default
tests, an explicit-profile preservation test, assertions that denylist and
other fields are unchanged, and review of the launch-mode branch.

*Validation & verification criteria* (must ALL pass before merge):
1. Add and run a regression test named
   `tui_missing_collection_seeds_agent_decides_for_execute_commands` (or an
   equivalently explicit name) in
   `app/src/ai/execution_profiles/profiles_tests.rs`. Start with no explicitly
   set `AISettings::execution_profiles`, construct
   `AIExecutionProfilesModel` with `LaunchMode::Tui`, and assert that the
   persisted default profile has `execute_commands ==
   ActionPermission::AgentDecides`. The test must fail against the current
   implementation (which produces `AlwaysAsk`) and pass with the fix.
2. In the same TUI regression test, assert the seeded profile still has the
   legacy denylist and allowlist values from `AISettings` and the existing
   default values for unrelated permission/profile fields. This proves the
   override is limited to `execute_commands`.
3. Add and run a test named
   `tui_explicit_collection_preserves_execute_commands` (or equivalent) that
   pre-populates an explicitly set TUI collection with a default profile whose
   `execute_commands` is `AlwaysAsk` and another profile/value that exercises
   persistence. Construct the TUI model and assert the stored values and
   profile collection are byte-for-byte/equality preserved; the seed override
   must not run.
4. Add and run a GUI guard test named
   `gui_default_execute_commands_remains_always_ask` (or equivalent) that
   exercises the no-explicit-profile GUI/default path and asserts
   `execute_commands == ActionPermission::AlwaysAsk`. Also assert
   `AIExecutionProfile::default().execute_commands` remains `AlwaysAsk` so
   future shared-default edits cannot silently widen this TUI-only change.
5. Exercise the denylist precedence path with the resulting TUI default:
   a command matching `command_denylist` must still require approval even
   though the profile's `execute_commands` is `AgentDecides`. Use the existing
   command-permission/blocklist test seam or add the narrowest deterministic
   assertion available; do not weaken or empty the denylist as part of this fix.
6. Re-run the original focused command-level reproduction after restoring the
   Cargo dependency-cache permissions:
   `cargo test -p warp --lib ai::execution_profiles::profiles_tests::tui_missing_collection_seeds_agent_decides_for_execute_commands -- --nocapture`.
   Record the before/after result: the pre-fix seed is `AlwaysAsk`, and the
   post-fix TUI seed is `AgentDecides`, with the GUI and explicit-profile guard
   tests passing.
7. Run the adjacent execution-profile regression set, including the existing
   migration, explicit-collection, and shared-profile isolation tests in
   `app/src/ai/execution_profiles/profiles_tests.rs`, to demonstrate no
   migration, ownership, or profile-selection regressions.
8. Run `./script/presubmit` from the repository root and require formatting,
   clippy, tests, and build checks to pass. No UI/computer-use step is required
   because this change only alters a headless model default and does not change
   rendered TUI layout or interaction widgets.
