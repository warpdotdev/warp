# Ask-User-Question Autonomy Speedbump

Linear: [QUALITY-512](https://linear.app/warpdotdev/issue/QUALITY-512/add-ask-user-question-permission-speedbump)

## 1. Summary
When Agent Mode first uses the Ask Question tool on a local client, show a compact inline footer on the Ask-User-Question card that lets the user adjust the active execution profile's Ask Question permission. The footer uses a dropdown with the existing three permission values and links to the AI Autonomy settings page.
## 2. Problem
Users can control how often Agent Mode pauses to ask them questions, but that control is buried in settings. Ask Question is also most noticeable at the moment the agent pauses, so the relevant permission should be surfaced in context the first time the user encounters it. Existing autonomy speedbumps already teach file-read and command execution permissions in context; Ask Question should follow the same pattern.
## 3. Goals
- Surface Ask Question autonomy controls at the moment the tool first appears.
- Let the user update the active execution profile without leaving the conversation.
- Link to the AI Autonomy settings page for users who want the full settings UI.
- Show the nudge only once per local client install/profile state so it does not become noisy.
- Support both normal Ask Question cards and first-use auto-approve skipped Ask Question cases.
- Match the compact visual rhythm of existing autonomy speedbump footers.
## 4. Non-goals
- No new Ask Question permission values.
- No changes to the AI settings page layout.
- No changes to the active Ask-User-Question answer/skip flow.
- No per-conversation or per-repository overrides.
- No global reset UI for speedbumps.
## 5. User experience
### Trigger
The speedbump is seeded when all of the following are true:
- `FeatureFlag::AskUserQuestion` is enabled.
- Agent Mode autonomy is allowed for the workspace.
- The local one-shot setting `should_show_agent_mode_ask_user_question_speedbump` is `true`.
- The completed agent output contains an Ask-User-Question action.

The trigger intentionally includes auto-approve conversations. If Ask Question is skipped because auto-approve is active, the first skipped Ask Question card can still show the footer so the user can discover and adjust the setting.
### One-shot semantics
The one-shot flag is local-only and is not synced through Warp Drive. The flag is consumed once the footer is successfully attached to an Ask-User-Question view. If the agent output is processed before the view exists, the flag remains `true` and is consumed later when the matching view is created and the footer can actually be installed.

The flag is consumed even if the user does not interact with the footer. This keeps the behavior to a single first-use display: if the user notices it and changes the setting, great; if they ignore it, the nudge is still considered displayed and will not reappear on future cards.
### Card placement and layout
- The footer renders as the bottom strip of the Ask-User-Question card.
- The footer is compact, with reduced vertical padding and a smaller dropdown so it feels similar to existing read-file/checkmark speedbumps.
- When the footer is present, the main Ask-User-Question card content does not keep rounded bottom corners; the footer owns the bottom radius so the combined card reads as one attached surface.
- The footer appears for both collapsed and expanded completed cards.
### Footer content
- Left side: short explanatory text and a dropdown.
- Right side: `Manage AI Autonomy permissions` link.
- The settings link opens the AI settings page scoped to the Autonomy section.
### Dropdown behavior
The dropdown options match the settings page order:
- `Never ask` → `AskUserQuestionPermission::Never`
- `Ask unless auto-approve` → `AskUserQuestionPermission::AskExceptInAutoApprove`
- `Always ask` → `AskUserQuestionPermission::AlwaysAsk`

The selected value reflects the active execution profile's current `ask_user_question` permission. Selecting an option immediately updates the active profile, emits telemetry, hides the footer on the current card, and leaves the local one-shot flag consumed.
### Overlay behavior
The dropdown menu renders above surrounding block content. Its options are clickable even when the card is embedded in terminal rich content, and clicking the dropdown underlay dismisses the menu without terminal text selection intercepting the interaction.
## 6. Success criteria
- First normal Ask Question invocation on a local client shows the footer on the resulting card.
- First auto-approve skipped Ask Question invocation also shows the footer on the resulting card.
- The local one-shot flag is not consumed if no matching view exists yet.
- The local one-shot flag is consumed once the footer is attached to the matching view.
- Ignoring the footer does not cause it to reappear on future Ask Question cards.
- Selecting any dropdown option updates the active profile immediately and hides the footer on the current card.
- The dropdown selection stays in sync with external active-profile changes while the footer exists.
- The settings link opens AI settings at the Autonomy section.
- Dropdown options can be clicked and dismissed reliably above terminal/block content.
- Collapsed and expanded cards with the footer render as one attached card with no double-rounded seam.
## 7. Validation
Automated validation:
- `cargo check -p warp`
- `cargo check --all-targets -p warp`
- `cargo nextest run --no-fail-fast -p warp ask_user_question`
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
- `git --no-pager diff --check`
## 8. Manual QA checklist
- Reset `ShouldShowAgentModeAskUserQuestionSpeedbump` locally to `true`.
- Trigger Ask Question in a normal conversation and confirm the compact footer appears on the card.
- Trigger Ask Question in auto-approve mode and confirm the skipped card can show the same footer on first display.
- Re-trigger Ask Question after ignoring the footer and confirm the footer does not reappear once the flag has been consumed.
- Select each dropdown option and confirm the active profile's setting changes in AI settings.
- Confirm selecting a dropdown option hides the footer.
- Open the dropdown near other rich content and confirm its menu appears above other content, options are clickable, Escape/outside click dismisses it, and terminal selection does not intercept clicks.
- Confirm collapsed and expanded cards have flattened attached corners with the footer present.
