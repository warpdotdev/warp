# Disable Suggested Rules Setting — Tech Spec

See `PRODUCT.md` for user-visible behavior.

## Context

Rule suggestions are inline chip views shown at the bottom of an AI block after the agent response completes. The feature is gated by `FeatureFlag::SuggestedRules`.

**Relevant files:**

- `app/src/settings/ai.rs` — `AISettings` settings group; all Active AI toggles (`prompt_suggestions_enabled_internal`, `code_suggestions_enabled_internal`, etc.) follow the same `define_settings_group!` + getter pattern.
- `app/src/ai/blocklist/block.rs (2137–2189)` — `handle_complete_output` creates `SuggestionChipView` instances for each suggested rule. The check `if FeatureFlag::SuggestedRules.is_enabled()` guards the entire block.
- `app/src/settings_view/ai_page.rs` — `AIFactWidget` renders the Knowledge section. Contains the `ToggleRules` and `ToggleWarpDriveContext` toggle rows.

The pattern for an opt-out Active AI setting already exists verbatim for Prompt Suggestions (`prompt_suggestions_enabled_internal` / `is_prompt_suggestions_enabled`) and Code Suggestions.

## Proposed Changes

### 1. New setting (`app/src/settings/ai.rs`)

Add `rule_suggestions_enabled_internal: RuleSuggestionsEnabled` to the `define_settings_group!(AISettings, …)` macro:

```
rule_suggestions_enabled_internal: RuleSuggestionsEnabled {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.warp_agent.active_ai.rule_suggestions_enabled",
    description: "Controls whether the agent suggests rules to save after responses.",
    feature_flag: FeatureFlag::SuggestedRules,
}
```

The `feature_flag` field causes the setting to be excluded from the user-facing JSON schema when `SuggestedRules` is not enabled for the current build channel.

Add the getter to `impl AISettings`:

```rust
pub fn is_rule_suggestions_enabled(&self, app: &warpui::AppContext) -> bool {
    self.is_active_ai_enabled(app) && *self.rule_suggestions_enabled_internal
}
```

This follows the same pattern as `is_prompt_suggestions_enabled` and `is_code_suggestions_enabled`.

### 2. Enforcement in `handle_complete_output` (`app/src/ai/blocklist/block.rs`)

Change the guard from:

```rust
if FeatureFlag::SuggestedRules.is_enabled() {
```

to:

```rust
if FeatureFlag::SuggestedRules.is_enabled()
    && AISettings::as_ref(ctx).is_rule_suggestions_enabled(ctx)
{
```

This is the single chokepoint where `SuggestionChipView::new_rule_chip` instances are created. No other code path renders rule-suggestion chips.

### 3. UI toggle in `AIFactWidget` (`app/src/settings_view/ai_page.rs`)

- Add `rule_suggestions_toggle: SwitchStateHandle` to `AIFactWidget`.
- Add `RuleSuggestionsEnabled` to the `use crate::settings::{…}` import block.
- Add `ToggleRuleSuggestions` variant to `AISettingsPageAction`.
- Add a handler for `ToggleRuleSuggestions` in `handle_action` that calls `settings.rule_suggestions_enabled_internal.toggle_and_save_value(ctx)`.
- Add a `render_rule_suggestions_toggle` method to `AIFactWidget` following the same structure as `render_warp_drive_context_toggle`.
- In `AIFactWidget::render`, call `render_rule_suggestions_toggle` conditionally:
  ```rust
  if FeatureFlag::SuggestedRules.is_enabled() {
      column.add_child(self.render_rule_suggestions_toggle(view, ai_settings, app));
  }
  ```
  Insert this between the rules toggle + "Manage rules" button and the Warp Drive context toggle.

The toggle renders with `is_any_ai_enabled` as the `is_toggleable` argument (not `is_active_ai_enabled`), consistent with other Knowledge-section toggles like `ToggleRules`.

### 4. "Don't show again" button (`app/src/ai/blocklist/block.rs`, `app/src/ai/blocklist/block/view_impl/output.rs`, `app/src/ai/blocklist/block/view_impl.rs`)

A second dismiss button is added to the suggestions footer that permanently disables the setting in addition to clearing the chips.

**`block.rs`:**
- Add `disable_rule_suggestions_button: ViewHandle<ActionButton>` field to `AIBlock`.
- Create the button in `AIBlock::new` with label `"Don't show again"` and theme `SuggestionDismissButtonTheme`, dispatching `AIBlockAction::DisableRuleSuggestions` on click.
- Add `AIBlockAction::DisableRuleSuggestions` variant. The handler:
  1. Calls `conversation.dismiss_current_suggestions()` on the history model (for persistence of the dismissal).
  2. Sets `settings.rule_suggestions_enabled_internal` to `false` via `set_value`.
  3. Calls `self.suggested_rules.clear()` to immediately remove the chip views from the block, ensuring the footer disappears even if `conversation.existing_suggestions` has not yet been populated by `mark_request_completed`.

**`output.rs` (`Props` and `render_suggested_rules_and_prompts_footer`):**
- Add `disable_rule_suggestions_button: &'a ViewHandle<ActionButton>` to `Props`.
- In `render_suggested_rules_and_prompts_footer`, build a `right_buttons` row that conditionally prepends `disable_rule_suggestions_button` (with `margin_right: 4.0`) before `dismiss_suggestion_button` when `has_suggested_rules` is true.

**`view_impl.rs`:**
- Pass `disable_rule_suggestions_button: &self.disable_rule_suggestions_button` when constructing `output::Props`.

## Testing and Validation

- **Behavior 1–2 (toggle placement and default):** Open Settings → Knowledge. Confirm the toggle appears under the Rules toggle and is on by default.
- **Behavior 3 (on = chips appear):** With the toggle on, run an agent interaction that returns rule suggestions. Verify chips appear as before.
- **Behavior 4 (off = chips suppressed):** Turn the toggle off. Run a new agent interaction. Verify no rule-suggestion chips appear. Existing chips from prior responses in the session remain visible.
- **Behavior 5 (independence):** Turn Rules off; verify Suggested Rules toggle state is unchanged and vice versa.
- **Behavior 8 (greyed out when global AI off):** Turn global AI off. Confirm the Suggested Rules toggle is visually disabled and non-interactive.
- **Behavior 9 (hidden without feature flag):** In a build without `SuggestedRules` enabled, confirm the toggle is absent from the settings UI.
