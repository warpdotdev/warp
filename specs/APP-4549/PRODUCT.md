# APP-4549: Disable the Feedback Bundled Skill
Linear: [APP-4549](https://linear.app/warpdotdev/issue/APP-4549/add-setting-to-disable-the-feedback-skill)
Figma: none provided
## Summary
Add a user-visible setting that controls whether Warp’s built-in `feedback` bundled skill is available to Oz in the app. The setting should default to enabled so existing behavior is unchanged, while giving users a clear opt-out when they do not want Oz to use Warp’s in-app feedback filing workflow.
## Problem
The bundled `feedback` skill is useful for turning rough Warp product feedback into filed GitHub issues, but it is not appropriate for every user or workspace. Users need a way to disable that built-in skill without disabling all bundled skills, all skills, or Oz entirely.
## Goals
- Let users disable only Warp’s built-in `feedback` bundled skill.
- Preserve current behavior by default.
- Make the setting discoverable from the existing Agent settings surface.
- Ensure disabling the skill prevents Oz from discovering or using that built-in skill in future app interactions.
## Non-goals
- Removing the `feedback` skill files from the shipped app bundle.
- Disabling user-created home or project skills that happen to be named `feedback`.
- Disabling other bundled skills.
- Changing the feedback skill’s instructions, issue filing behavior, or target repository.
- Adding organization-level policy controls for bundled skills.
## Behavior
1. Warp exposes a setting for the built-in `feedback` bundled skill in the existing Agent settings UI.
2. The setting defaults to enabled for all users. Existing users should see no behavior change until they turn it off.
3. When enabled, the built-in `feedback` bundled skill is available exactly as it is today:
   - It can appear in skill selection surfaces that include bundled skills.
   - It can be advertised to Oz as an available bundled skill.
   - Oz can read and invoke the bundled skill when the normal skill-triggering conditions apply.
4. When disabled, the built-in `feedback` bundled skill is unavailable in the app:
   - It does not appear in skill selection surfaces that include bundled skills.
   - It is not advertised to Oz in the available-skills context.
   - If a stale or explicit reference attempts to read the bundled `feedback` skill, Warp treats it as unavailable and does not expose the skill content.
5. Toggling the setting affects subsequent skill discovery and subsequent Oz requests. A user should not need to restart Warp for future requests to stop including the built-in `feedback` skill.
6. The setting controls only Warp’s built-in bundled skill with the bundled ID `feedback`.
7. User-created skills are unaffected:
   - A home skill named `feedback` remains available according to normal home-skill rules.
   - A project skill named `feedback` remains available according to normal project-skill rules.
   - A user-created skill with feedback-related instructions remains readable and invokable if it is otherwise in scope.
8. Other bundled skills are unaffected. Disabling the feedback skill must not hide or disable bundled skills such as settings, PR comments, MCP, Figma, or any future bundled skills.
9. The setting is independent of global AI enablement:
   - If global AI is disabled, the setting may render disabled like nearby Agent settings.
   - The stored value is still preserved while global AI is disabled.
   - Re-enabling global AI restores the feedback skill according to the stored setting value.
10. The setting should be represented in user-editable settings using a stable, descriptive key so users can configure it outside the UI when settings-file support is enabled.
11. The setting should follow existing settings sync behavior for user-level Agent preferences, so a user’s choice can carry across devices when settings sync is enabled.
12. The UI label and description should make the scope clear: the toggle controls Warp’s built-in feedback skill, not all feedback mechanisms and not all skills.
13. Search within settings should find the toggle with terms like “feedback,” “skill,” “bundled skill,” and “agent.”
14. Turning the setting off should not delete any app resource, modify any user skill files, or change git state.
15. If the setting cannot be read, Warp should fall back to the default enabled behavior rather than unexpectedly removing the skill.
