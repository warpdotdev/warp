# APP-4549: Tech Spec — Feedback Bundled Skill Setting
Linear: [APP-4549](https://linear.app/warpdotdev/issue/APP-4549/add-setting-to-disable-the-feedback-skill)
## Context
`PRODUCT.md` defines the user-visible behavior: add a default-on setting that disables only Warp’s built-in `feedback` bundled skill.
The relevant code already has a central bundled-skill activation path:
- `app/src/ai/skills/skill_manager.rs (20-47)` — `BundledSkillActivation` models whether a bundled skill is active.
- `app/src/ai/skills/skill_manager.rs (361-389)` — bundled skills are loaded from app resources and assigned an activation condition.
- `app/src/ai/skills/skill_manager.rs (591-600)` — `activation_for_bundled_skill` currently makes most bundled skills, including `feedback`, always active.
- `app/src/ai/skills/skill_manager.rs (185-201)` — `get_skills_for_working_directory` appends bundled skills whose activation condition is enabled.
- `app/src/ai/skills/skill_utils.rs (94-121)` — `list_skills_if_changed` sends available skill descriptors to the agent when the list changes.
- `app/src/ai/blocklist/action_model/execute/read_skill.rs (36-63)` — `read_skill` resolves `SkillReference::BundledSkillId` through `SkillManager::skill_by_reference`.
- `app/src/settings/ai.rs (715-1464)` — `AISettings` defines user-level Agent settings, including public TOML-backed settings under `agents.warp_agent.*`.
- `app/src/settings_view/ai_page.rs (6024-6223)` — the Agent settings “Other” widget renders related Agent toggles using shared helpers.
- `app/src/bin/generate_settings_schema.rs (146-199)` and `script/prepare_bundled_resources (132-159)` — public settings with `toml_path` are included in the generated settings schema bundled with app resources.
Bundled skills are copied into the application bundle from `resources/bundled` by `script/prepare_bundled_resources:48`. This change should not try to remove the feedback skill from the bundle at build time; it should make the skill inactive at runtime.
## Proposed changes
1. Add a public AI setting in `app/src/settings/ai.rs`.
   - Suggested field: `feedback_bundled_skill_enabled`.
   - Suggested generated setting type: `FeedbackBundledSkillEnabled`.
   - Type: `bool`.
   - Default: `true`.
   - Supported platforms: `SupportedPlatforms::ALL`.
   - Sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`.
   - Private: `false`.
   - Suggested TOML path: `agents.warp_agent.other.feedback_bundled_skill_enabled`.
   - Description: “Whether Warp’s built-in feedback skill is available to the Warp Agent.”
2. Extend bundled skill activation in `app/src/ai/skills/skill_manager.rs`.
   - Add a `BundledSkillActivation` variant for settings-backed feedback activation, for example `FeedbackSkillSetting`.
   - Update `BundledSkillActivation::is_enabled` to consult `AISettings::as_ref(ctx).feedback_bundled_skill_enabled` for that variant.
   - Update `activation_for_bundled_skill` so `skill_id == "feedback"` uses that variant.
   - Keep `modify-settings` on `RequiresFile` and all unrelated bundled skills on their existing activation behavior.
3. Add an activation-aware lookup for bundled skill reads.
   - Preserve raw lookup behavior where the UI needs historical metadata for already-rendered outputs.
   - Add a method such as `skill_by_reference_if_active(&self, reference: &SkillReference, ctx: &AppContext) -> Option<&ParsedSkill>`, or update `ReadSkillExecutor` to pattern-match bundled references and call `active_bundled_skill(id, ctx)`.
   - Use the activation-aware path in `ReadSkillExecutor` so `read_skill` cannot expose `@warp-skill:feedback` content when the setting is disabled.
   - Path-based user skills should continue to use `skills_by_path` and should not be affected by the feedback bundled-skill setting.
4. Add the UI toggle in `app/src/settings_view/ai_page.rs`.
   - Import the generated `FeedbackBundledSkillEnabled` setting type.
   - Add a `SwitchStateHandle` to `OtherAIWidget`.
   - Add `AISettingsPageAction::ToggleFeedbackBundledSkill`.
   - Handle the action by toggling `AISettings.feedback_bundled_skill_enabled` and notifying the view.
   - Render the toggle in the existing Agent “Other” section using `render_ai_setting_toggle`.
   - Suggested label: “Enable built-in feedback skill”.
   - Suggested description: “Let Oz use Warp’s built-in skill for turning Warp product feedback into GitHub issues.”
   - Update `OtherAIWidget::search_terms` to include feedback, skill, and bundled skill.
5. Schema and resources.
   - No manual schema file changes should be necessary. The setting should appear in generated schema output through the existing settings inventory path.
   - Normal resource preparation should continue to copy the `feedback` skill file into the bundle.
6. Keep the implementation narrow.
   - Do not alter `resources/bundled/skills/feedback/SKILL.md`.
   - Do not add broad bundled-skill allow/deny lists.
   - Do not change skill deduplication, provider precedence, or user-created skill scoping.
## Testing and validation
Map tests to the product behavior in `PRODUCT.md`:
1. Product behavior 2 and 3: add a skill manager test showing `feedback` is active by default and included in `get_skills_for_working_directory` when bundled skills are enabled.
2. Product behavior 4: add a skill manager test that sets `feedback_bundled_skill_enabled` to `false` and verifies `feedback` is excluded from returned bundled skill descriptors.
3. Product behavior 8: in the same test or a companion test, verify another bundled skill remains included when feedback is disabled.
4. Product behavior 7: verify path-based skills named `feedback` are still returned according to existing home/project skill scope rules when the bundled feedback skill is disabled.
5. Product behavior 4: add or update `read_skill_tests` so `ReadSkillExecutor` returns an error for `SkillReference::BundledSkillId("feedback")` when the setting is disabled.
6. Product behavior 3 and 8: add a read-skill test showing an enabled bundled skill can still be read.
7. Product behavior 9, 12, and 13: update `ai_page_tests` if the existing settings page tests assert widget search/filtering or rendered action coverage for the “Other” widget.
8. Run `cargo fmt`.
9. Run targeted tests:
   - `cargo test -p warp skill_manager_tests`
   - `cargo test -p warp read_skill_tests`
   - `cargo test -p warp ai_page_tests`
   Adjust exact package/test filters if local test names differ.
10. If this is prepared for PR review, follow repo guidance and run the required formatting and clippy checks before opening or updating a PR.
## Risks and mitigations
- Stale skill context could still reference `@warp-skill:feedback`. Mitigation: guard direct `read_skill` execution with the same activation state used by skill listing.
- The setting could accidentally disable user-authored skills named `feedback`. Mitigation: check only the bundled skill ID, not parsed skill name or path-based references.
- Other bundled skills could regress if activation is generalized too broadly. Mitigation: add tests that feedback is disabled while another bundled skill remains active.
- Settings UI copy could imply all feedback mechanisms are disabled. Mitigation: label and description should explicitly say “built-in feedback skill.”
## Parallelization
Do not parallelize the implementation. The setting definition, activation logic, direct read enforcement, and UI toggle are tightly coupled and touch overlapping files, so a single implementer should make the code changes in one checkout on branch `safia/app-4549-add-setting-to-disable-the-feedback-skill`.
Validation can be parallelized after implementation if desired:
- Agent A: local execution in `/Users/captainsafia/code/warp`, same branch, owns `skill_manager_tests` and `read_skill_tests`.
- Agent B: local execution in a separate worktree, for example `/Users/captainsafia/code/warp-app-4549-ui-tests` on branch `safia/app-4549-ui-validation`, owns `ai_page_tests` and settings UI review.
If using the optional validation split, Agent B should not modify source files unless asked; it should report failures and suggested fixes back to the main branch owner. The final PR should land as a single branch/PR from `safia/app-4549-add-setting-to-disable-the-feedback-skill`.
