# Disable Suggested Rules Setting

## Summary

Add a user-facing toggle to disable the AI's suggested-rules chips that appear after agent responses. When disabled, the AI will not present inline rule-suggestion chips to the user.

Figma: none provided

## Behavior

1. A new setting, **Suggested Rules**, appears in the **Knowledge** section of AI settings (Settings → Oz → Knowledge), directly below the existing **Rules** toggle. The setting is only present when the `SuggestedRules` feature flag is enabled.

2. The **Suggested Rules** toggle is on by default (`true`). The description reads: "Let AI suggest rules to save based on your interactions."

3. When **Suggested Rules** is on, the agent may show inline rule-suggestion chips at the bottom of an agent response block after the response completes, as it does today. No change to existing behavior.

4. When **Suggested Rules** is turned off, no rule-suggestion chips appear at the bottom of any agent response block — including responses that would otherwise have generated suggestions. Suggestions that have already been rendered (from a past response in the same session) are not retroactively hidden; only future responses are affected.

5. The toggle is independently controllable from the parent **Rules** toggle (which gates whether saved rules are included in agent requests). Turning **Rules** off does not automatically turn **Suggested Rules** off, and vice versa.

6. The toggle is synced to the cloud across devices via the standard global sync mechanism (same behavior as other Active AI settings such as Prompt Suggestions).

7. The setting is scoped to the `agents.warp_agent.active_ai.rule_suggestions_enabled` TOML key. Users who set this key in their settings file have their preference respected on next launch.

8. The **Suggested Rules** toggle is disabled (greyed out, not interactive) when the top-level global AI toggle (**Oz**) is off, matching the visual and interaction behavior of all other AI sub-settings.

9. The setting is not visible in the settings UI when the `SuggestedRules` feature flag is disabled. The setting value is still persisted if previously set, so enabling the flag later restores the user's preference.

10. Agent mode workflow suggestion chips (the fallback shown when there are no rule suggestions) are unaffected by this setting — they are controlled by the `SuggestedAgentModeWorkflows` feature flag.

11. A **"Don't show again"** button appears in the suggestions footer of the AI block, to the left of the existing **Dismiss** button, whenever one or more rule-suggestion chips are visible. Clicking it permanently disables the **Suggested Rules** setting (same effect as toggling it off in Settings) and immediately removes the rule-suggestion chips from the current block.
