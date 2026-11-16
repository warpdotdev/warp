# TECH.md — Rename Oz to Warp Agent in settings and onboarding

Issue: https://github.com/warpdotdev/warp-external/issues/1063
Product spec: `specs/GH1063/product.md`

## Context

This is a rename of the in-app agent from "Oz" to "Warp Agent" across user-facing
strings, the internal enum variants that back those strings, and all call-sites
that referenced the old variant. "Oz" remains reserved for the cloud agent
orchestration platform, so the rename must not touch any cloud surfaces.

Relevant code (prior state):

- `app/src/settings_view/mod.rs` — `SettingsSection::Oz` variant displayed as
  `"Oz"` in the sidebar. `FromStr` mapped `"Oz"` to `SettingsSection::Oz`.
  Default-subpage fallback and `is_ai_subpage` / `ai_subpages()` all referenced
  `SettingsSection::Oz`.
- `app/src/settings_view/ai_page.rs` — `AISubpage::Oz` variant, heading literal
  `"Oz"`, and multiple description strings referencing "Oz" or "Oz agent".
- `crates/onboarding/src/slides/agent_slide.rs` — header title
  `"Customize your Agent, Oz"` and checkbox label `"Disable Oz"`.
- Approximately 15 additional files contained `SettingsSection::Oz` usages
  for navigation actions and settings page dispatch.

Out-of-scope references that must be preserved as "Oz" (verified by grep):

- `app/src/settings_view/mod.rs` — `SettingsSection::OzCloudAPIKeys` display
  `"Oz Cloud API Keys"` and its `FromStr` round-trip.
- `app/src/terminal/view/ambient_agent/harness_selector.rs:62` — `Harness::Oz`
  display name "Oz" in the cloud agent harness menu.
- `app/src/ai/blocklist/agent_view/zero_state_block.rs:388, 404` — "New Oz cloud
  agent conversation" / "New Oz agent conversation". Zero-state copy is not
  covered by issue #1063 and must not be touched in this PR.
- "Oz changelog" toggle labels in `ai_page.rs` (`OtherAIWidget`) are kept as
  "Oz changelog" because they refer to Oz platform release notes, not the
  in-app agent.

## Proposed changes

1. `app/src/settings_view/mod.rs`
   - Rename `SettingsSection::Oz` variant to `SettingsSection::WarpAgent`.
   - In the `Display` impl, the `WarpAgent` arm writes `"Warp Agent"`.
   - In the `FromStr` impl, accept both `"Oz"` (backward-compat legacy name)
     and `"Warp Agent"` as parseable forms that map to
     `SettingsSection::WarpAgent`, per Behavior #8 in `product.md`.
   - Update `is_ai_subpage`, `ai_subpages()`, and the two default-subpage
     fallbacks (`SettingsSection::AI => SettingsSection::WarpAgent`) to use the
     new variant name.
   - Leave `SettingsSection::OzCloudAPIKeys` and its `"Oz Cloud API Keys"`
     display untouched. Do not alter the `"Agents"` umbrella name or subpage
     order.
   - Update the doc-comment on `SettingsSection::AI` to reference `WarpAgent`.

2. `app/src/settings_view/ai_page.rs`
   - Rename `AISubpage::Oz` variant to `AISubpage::WarpAgent`; update
     `AISubpage::from_section` and the `build_page` match arm accordingly.
   - In `GlobalAIWidget::render`, replace `Text::new_inline("Oz", ...)` with
     `Text::new_inline("Warp Agent", ...)`. Keep every other argument, style,
     alignment, and layout constant.
   - In `GlobalAIWidget::search_terms`, keep existing terms (including `"oz"`
     for legacy muscle memory, allowed by Behavior #7) and keep `"warp agent"`
     so the new label is directly searchable.
   - Replace all remaining user-visible description strings that referenced
     "Oz" or "Oz agent" with "the Warp Agent" / "Warp Agent" as appropriate.
     Specifically: command denylist/allowlist descriptions, base model
     description, codebase context description, MCP zero-state and
     allowlist/denylist descriptions, Rules description, Warp Drive context
     description, API keys description, and MCP servers description.
   - Preserve the two "Oz changelog" toggle labels in `OtherAIWidget` and
     `SettingActionPairDescriptions` unchanged — these refer to Oz platform
     release notes, not the in-app agent.

3. `crates/onboarding/src/slides/agent_slide.rs`
   - `render_header`: change paragraph text from `"Customize your Agent, Oz"`
     to `"Customize your Warp Agent"`. Keep font size, weight, layout, and
     surrounding subtitle unchanged.
   - `render_disable_oz_section`: change checkbox label from `"Disable Oz"` to
     `"Disable Warp Agent"`. Keep styling, spacing, `disable_oz_mouse` state
     handle, and the dispatched `AgentSlideAction::ToggleDisableOz` action
     unchanged.
   - Internal identifiers (`disable_oz_mouse`, `disable_oz` field on
     `AgentDevelopmentSettings`, `AgentSlideAction::ToggleDisableOz`,
     `render_disable_oz_section` function name) are kept as-is to avoid
     migration risk for persisted settings and telemetry.

4. Navigation call-sites (approximately 15 files)
   - All `SettingsSection::Oz` references in navigation actions, workspace
     dispatch, settings page helpers, and editable bindings are updated to
     `SettingsSection::WarpAgent`. Affected files include:
     `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`,
     `app/src/ai/blocklist/block.rs`,
     `app/src/ai/blocklist/block/cli.rs`,
     `app/src/ai/blocklist/block/view_impl.rs`,
     `app/src/ai/blocklist/block/view_impl/common.rs`,
     `app/src/ai/blocklist/block/view_impl/output.rs`,
     `app/src/ai/blocklist/prompt/prompt_alert.rs`,
     `app/src/settings_view/billing_and_usage_page.rs`,
     `app/src/terminal/input.rs`,
     `app/src/terminal/input/inline_history/view.rs`,
     `app/src/terminal/input/models/data_source.rs`,
     `app/src/terminal/input/models/view.rs`,
     `app/src/terminal/profile_model_selector.rs`,
     `app/src/terminal/view.rs`,
     `app/src/workspace/mod.rs`,
     `app/src/workspace/view.rs`.

## Testing and validation

Runtime checks:

- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests
  -- -D warnings` must pass (per `WARP.md` PR workflow).
- `cargo nextest run -p warp_app --no-fail-fast` or the relevant subset covering
  `settings_view::mod_test` must pass. The Display test asserts
  `SettingsSection::WarpAgent.to_string() == "Warp Agent"`. The `FromStr` test
  covers both `"Oz"` and `"Warp Agent"` resolving to
  `SettingsSection::WarpAgent`, exercising Behavior #8. All helper tests
  (`is_ai_subpage`, `ai_subpages_list_contains_all_ai_subpage_variants`,
  filter/visibility tests) are updated to reference `SettingsSection::WarpAgent`.
  Existing tests for `OzCloudAPIKeys` are left untouched to guard against
  accidentally renaming the cloud subpage.
- `cargo nextest run -p onboarding` (if a test crate exists for the onboarding
  slide strings; otherwise, this rename is a pure string change and manual
  verification below suffices).

Behavior-to-verification mapping (from `product.md`):

- Behavior #1, #2, #3, #9: manually open the settings UI and confirm the
  sidebar entry reads "Warp Agent", the subpage renders unchanged content, the
  heading above the global toggle reads "Warp Agent", and the "Oz Cloud API
  Keys" entry under "Cloud platform" still reads "Oz Cloud API Keys".
- Behavior #4: toggle the global AI switch and verify it still enables and
  disables AI features as before.
- Behavior #5, #6, #10: launch onboarding (or jump to the agent slide via the
  existing onboarding test fixtures) and confirm the title, subtitle, disable
  checkbox label, autonomy options, and step progress are all correct.
- Behavior #7: search within the settings modal using each of
  `"warp agent"`, `"ai"`, `"agent"`, `"oz"` (should still reach the subpage) and
  `"oz cloud"` (should reach the cloud subpage only).
- Behavior #8: confirm both `"Oz"` and `"Warp Agent"` resolve to
  `SettingsSection::WarpAgent` via the `FromStr` round-trip test.
- Behavior #11: no automated accessibility test exists for these labels; manual
  verification on macOS VoiceOver is sufficient since the visible text is the
  accessible label.
- Behavior #12: toggle the `OpenWarpNewSettingsModes` feature flag and confirm
  the disable row only appears when enabled and always reads "Disable Warp
  Agent" when it does appear.

Manual verification artifacts:

- Screenshots of (a) settings sidebar with the "Agents" umbrella expanded,
  (b) the AI settings page heading, and (c) the onboarding agent slide in both
  feature-flag states.
- After implementation, invoke the `verify-ui-change-in-cloud` skill per the
  repository rule for user-facing client changes.

## Risks and mitigations

- Risk: external deep links or persisted telemetry strings reference `"Oz"` and
  break. Mitigation: `FromStr` accepts both `"Oz"` and `"Warp Agent"` mapping to
  `SettingsSection::WarpAgent`, and the legacy `"oz"` search term is preserved so
  `oz`-based search still lands on the subpage.
- Risk: accidentally renaming cloud Oz surfaces. Mitigation: grep for `"Oz"`
  literals confirms `harness_selector.rs`, `zero_state_block.rs`, and
  `OzCloudAPIKeys` are untouched. The "Oz changelog" toggle labels are
  explicitly preserved.
- Risk: stale comments inside `agent_slide.rs` that still reference "Disable Oz"
  mislead future readers. Mitigation: internal identifiers (`disable_oz_mouse`,
  `AgentSlideAction::ToggleDisableOz`, etc.) intentionally retain the `oz` name;
  comments describing them are acceptable to leave as-is per `WARP.md`.

## Follow-ups

- `SettingsSection::Oz` and `AISubpage::Oz` enum variant renames have been
  completed as part of this implementation.
- Internal identifiers (`disable_oz` setting field,
  `AgentSlideAction::ToggleDisableOz`, `render_disable_oz_section`,
  `disable_oz_mouse`, and related settings/telemetry keys) are intentionally
  kept as-is. They require more care around persisted settings, telemetry event
  names, and potentially GraphQL/analytics schemas.
- The broader zero-state and blocklist strings that still say "Oz agent" (e.g.,
  in `zero_state_block.rs`) should be revisited in a follow-up issue once
  product confirms which of those belong to the in-app agent vs. the cloud agent
  orchestration platform.
