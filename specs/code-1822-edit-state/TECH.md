# TECH: Frontend-neutral orchestration edit state

## Context

The GUI's multi-agent (`run_agents`) approval surfaces — the confirmation card and the
plan-card config block — share a run-wide edit state (model, harness, execution mode,
auth-secret selection) plus the transition/validation logic that keeps that state
consistent as the user edits it. Before this change, all of it lived inline in the GUI
controls module, entangled with `warpui` rendering types:

- [`app/src/ai/blocklist/inline_action/orchestration_controls.rs @ f4bdf422`](https://github.com/warpdotdev/warp/blob/f4bdf4227/app/src/ai/blocklist/inline_action/orchestration_controls.rs)
  — the config-fields struct (then named `OrchestrationEditState`, renamed here to
  `OrchestrationConfigState`), `AuthSecretSelection`, catalog lookups
  (`get_base_model_choices`, `is_model_in_filtered_choices`, `first_filtered_model_id`),
  persistence helpers (`persist_*`), default resolution (`resolve_*`), validation
  predicates (`should_show_auth_secret_picker`, `accept_disabled_reason_with_auth`,
  `harness_is_selectable`), and the picker populate/sync functions, all in one GUI file.
- [`app/src/ai/blocklist/inline_action/run_agents_card_view.rs @ f4bdf422`](https://github.com/warpdotdev/warp/blob/f4bdf4227/app/src/ai/blocklist/inline_action/run_agents_card_view.rs)
  — confirmation-card view; owns a per-view `saved_model_per_harness` map and calls the
  controls helpers with closure-based fallbacks.
- [`app/src/ai/document/orchestration_config_block.rs @ f4bdf422`](https://github.com/warpdotdev/warp/blob/f4bdf4227/app/src/ai/document/orchestration_config_block.rs)
  — plan-card config block; duplicates the same state-handling patterns.
- [`app/src/ai/blocklist/action_model/execute/run_agents.rs @ f4bdf422`](https://github.com/warpdotdev/warp/blob/f4bdf4227/app/src/ai/blocklist/action_model/execute/run_agents.rs)
  — the `RunAgentsExecutor`; imports auth-secret execution helpers
  (`can_execute_with_auth_secret`, `populate_default_auth_secret_for_execution`) from the
  GUI controls module.

A TUI orchestration card is planned. It must apply the exact same state math (harness
switching with per-harness model memory, Local/Cloud toggling, model revalidation against
live catalogs, auth-secret gating) without depending on GUI element types. That requires
splitting the frontend-neutral logic out of `orchestration_controls.rs`.

## Proposed changes

Create `app/src/ai/orchestration/`, a frontend-neutral domain module registered as
`pub(crate) mod orchestration;` in `app/src/ai/mod.rs`. Nothing in the module may depend
on `warpui::elements` or other GUI rendering types; it only reads/writes app singletons
through `AppContext`.

- `config_state.rs` — `OrchestrationConfigState` (model_id, harness_type, execution_mode,
  auth_secret_selection) and `AuthSecretSelection` (`Unset` / `Inherit` / `Named` /
  `CreatingNew`), plus the pure conversions and mutations that were previously inline:
  `from_run_agents_fields`, `from_orchestration_config`, `to_orchestration_config`,
  `resolve_from_config`, `override_from_approved_config`,
  `toggle_execution_mode_to_remote`, `sanitize_for_local_execution`,
  `accept_disabled_reason`, `auth_secret_name`.
- `edit_state.rs` — edit-application logic with catalog access injected as
  callbacks so cores are unit-testable without app singletons:
  `OrchestrationConfigState::apply_execution_mode_change`, `apply_auth_secret_change`,
  `revalidate_after_catalog_change`, and `OrchestrationEditState` — the config state plus
  the per-harness `saved_model_per_harness` memory (previously a per-view field on each
  card), with `apply_harness_change` handling save/restore/fallback of model selection
  and auth re-resolution.
- `validation.rs` — shared predicates both frontends must gate through identically:
  `harness_is_selectable` (excludes Gemini,
  product-disabled and not-installed local harnesses), `should_show_auth_secret_picker`,
  `auth_secret_selection_required`, `accept_disabled_reason_with_auth`,
  `empty_env_recommendation_message`.
- `providers.rs` — `AppContext`-backed catalog lookups, default resolution, and
  persistence: `get_base_model_choices`, `is_model_in_filtered_choices`,
  `first_filtered_model_id`, `harness_save_key`, `resolve_default_environment_id`,
  `resolve_default_host_slug`, `resolve_recent_host_slug`,
  `resolve_auth_secret_selection_for_harness`, `persist_environment_selection`,
  `persist_host_selection`, `persist_auth_secret_selection`,
  `can_execute_with_auth_secret`, `populate_default_auth_secret_for_execution`, and the
  `ORCHESTRATION_WARP_WORKER_HOST` / `ORCHESTRATION_ENV_NONE_LABEL` constants.

GUI adaptation:

- `orchestration_controls.rs` keeps everything rendering-related: picker chrome/styling,
  `OrchestrationControlAction`, `OrchestrationPickerHandles`, the catalog-to-`MenuItem`
  populate bodies (`populate_model_picker_for_harness`, `populate_harness_picker`,
  `create_environment_picker`, `populate_environment_picker`,
  `populate_auth_secret_picker_for_harness`, `populate_host_picker`,
  `sync_picker_selections`), and render helpers. Its `apply_harness_change`,
  `apply_execution_mode_change`, and `repopulate_all_pickers` become thin wrappers that
  delegate the state math to `OrchestrationEditState` / `OrchestrationConfigState` and
  then repopulate the affected pickers. A `pub use` shim re-exports the moved domain
  names so existing GUI import paths keep compiling. The `DEFAULT_MODEL_LABEL` and
  `AUTH_SECRET_INHERIT_LABEL` display strings stay defined in this GUI file.
- `run_agents_card_view.rs` and `orchestration_config_block.rs` hold an
  `OrchestrationEditState` instead of a bare state plus ad-hoc model-memory map, and
  call the thin wrappers with an `Option<String>` fallback base-model id instead of a
  closure.
- `execute/run_agents.rs` imports the auth-secret execution helpers from
  `crate::ai::orchestration` instead of the GUI controls module.
- `app/src/tui_export.rs` exports the neutral surface for the upcoming TUI card:
  `OrchestrationConfigState`, `OrchestrationEditState`, `AuthSecretSelection`, the
  validation predicates, and the `persist_*` / `resolve_*` providers and
  `ORCHESTRATION_*` constants.

## Testing and validation

- Unit tests colocated with the domain module per repo convention
  (`config_state_tests.rs`, `edit_state_tests.rs`, `validation_tests.rs`): config
  round-trips and overrides, execution-mode toggling (OpenCode→Oz reset, environment
  pre-fill), harness-change model memory save/restore with injected catalog callbacks,
  catalog-change revalidation (vanished model reset, deleted secret drop, `Unset`
  re-seed), and the auth-secret Accept gate.
- Pre-existing GUI tests (`run_agents_card_view_tests.rs`, `run_agents_tests.rs`,
  `orchestration_config_block` coverage) must pass unchanged apart from import paths;
  the GUI tests that only exercised moved state math now live in the domain test files.
- Commands: `cargo check -p warp` and
  `cargo nextest run -p warp -E 'test(orchestration) + test(run_agents) + test(run_agents_card_view) + test(orchestration_config_block)'`,
  plus `./script/format`.
- Manual: the confirmation card and plan-card config block behave identically to before —
  harness switching preserves per-harness model choices, Local/Cloud toggling pre-fills
  the default environment and resets invalid models, and Accept stays gated on
  auth-secret selection for non-Oz cloud harnesses.

## Parallelization

Not beneficial: this is a single-PR extraction where the domain module, GUI adaptation,
and export surface are tightly coupled and must land atomically.

## Follow-ups

Later PRs build on this module: a plain-data option-snapshot layer (shared row/badge
snapshots consumed by both frontends' pickers), the TUI host/environment selector, and
the TUI orchestration card itself.
