# HOA Onboarding Flow for Existing Users

Linear: APP-3809

## Summary

A guided onboarding flow that introduces existing Warp users to House of Agents (HOA) features: vertical tabs, agent inbox, and default tab config creation. The flow is shown once, behind a feature flag (`HOAOnboardingFlow`), and is only shown to users who did not go through the new-user onboarding (i.e. existing users who update to the HOA release).

## Problem

Existing Warp users will receive a major update with HOA features (vertical tabs, agent inbox/notifications, native code review, CLI agent integrations) but have no guided introduction to these changes. Without a targeted onboarding flow, users may not discover or understand the new capabilities, leading to lower adoption of key features.

## Goals

- Introduce existing users to HOA features through a 4-step guided flow.
- Let users configure vertical vs. horizontal tabs with a live toggle.
- Guide users to create their default tab config (session type, directory, worktree).
- Show the flow exactly once per user, gated behind `HOAOnboardingFlow` feature flag.
- Reuse the existing session config modal rendering code for the tab config step.

## Non-goals

- Redesigning the new-user onboarding flow.
- Onboarding for features not shown in the Figma (e.g. MCP servers, voice input).
- Animation of the welcome banner hero art (static image is acceptable for v1).
- Allowing users to navigate backward through steps.

## Figma

https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7495-108638&m=dev

## User Experience

### Triggering conditions

- The `HOAOnboardingFlow` feature flag is enabled (dogfood initially).
- The user has NOT gone through the new-user onboarding for this version.
  (Use a `private_user_preferences` key, similar to `HasCompletedOnboarding`, to track whether this flow has been shown.)
- The flow is shown at most once. Completing any step or dismissing marks it as done.
- New users who complete the standard onboarding flow should have this flag pre-set so they never see the HOA onboarding.

### Flow overview

The flow has 4 sequential steps. The user progresses forward only (no back navigation). Each step must be completed or the flow dismissed before normal interaction resumes.

---

### Step 1: Welcome Banner

**Presentation**: A centered modal dialog overlaying the terminal window with a semi-transparent background scrim.

**Content**:
- Hero image area at top (Claude, OpenAI, OpenCode logos — static image).
- X (close) button in the top-right corner of the hero area.
- "New" badge (magenta pill).
- Title: "Introducing first-class support for Claude Code, Codex, and OpenCode"
- 4 feature bullet points, each with an icon:
  1. **Vertical tabs** (layout-left icon): "Rich tab-titles and customizable content, so you can keep an eye on your agents how you want."
  2. **Agent inbox and notifications** (inbox-01 icon): "Get notified when agents need approval or feedback, so nothing blocks progress."
  3. **Native code review** (message-check-square icon): "Review and refine code with your CLI agent without leaving Warp."
  4. **Warp's input bar** (text-input icon): "Move faster with a smarter input. Attach images, use voice, and trigger actions - all from one place."
- Primary CTA button: "See what's new" (light text on dark background, full width within the card).

**Behavior**:
- Clicking "See what's new" advances to Step 2.
- Clicking the X button dismisses the entire flow and marks it complete. The user never sees it again.
- The scrim blocks interaction with the terminal behind it.

---

### Step 2: Vertical Tabs Callout

**Presentation**: A tooltip/popover (480px wide) anchored to the vertical tab sidebar area, with a pointer/arrow indicating the target. The terminal remains visible behind it, but workspace interactions are blocked until the user advances or dismisses the flow. The vertical tabs sidebar should already be open/visible at this point.

**Content**:
- Title: "Introducing vertical tabs - the new default"
- Description: "Vertical tabs display all panes within each terminal window and have rich headers and context for all CLI agents."
- Checkbox (unchecked by default): "Switch back to horizontal tabs"
- Progress indicator: 3 dots, dot 1 active (filled blue).
- "Next" button (primary, right-aligned in footer).

**Behavior**:
- The checkbox is a **live toggle**: checking it immediately switches the tab layout to horizontal tabs. Unchecking switches back to vertical. The underlying tab layout setting is persisted when toggled.
- The popover arrow/pointer updates position based on the current tab layout:
  - Vertical tabs: pointer points toward the left sidebar.
  - Horizontal tabs: pointer points toward the top tab bar.
- Clicking "Next" advances to Step 3.

---

### Step 3: Agent Inbox Callout

**Presentation**: A tooltip/popover (480px wide) anchored to the inbox icon in the title bar (top-right area), with a pointer/arrow indicating the inbox icon.

**Content**:
- Title: "Meet your new agent inbox"
- Description: "Your inbox is your central place to manage agent notifications and access produced artifacts like plans and PRs."
- Progress indicator: 3 dots, dot 2 active.
- "Next" button (primary, right-aligned in footer).

**Behavior**:
- Clicking "Next" advances to Step 4.

---

### Step 4: Default Tab Config

**Presentation**: A popover/panel (480px wide) anchored to the vertical tabs panel in vertical-tabs mode, or to the new-tab button in horizontal-tabs mode.

**Content**:
- Title: "Create your default tab config"
- Description: "A tab config defines what opens when you create a new tab. Select a repo, choose a session type (terminal, Warp agent, or third-party agents like Claude or Codex), and optionally attach a worktree. This setup is used for every new tab."
- **Session type** selector: pill/chip buttons matching the existing session config modal (reuse the same rendering code and logic, including dynamic filtering of session types based on whether Oz/AI is enabled).
- **Select directory** button: opens the native file picker. Displays the selected path in user-friendly form (e.g. `~/warp-internal`). Defaults to the user's home directory.
- **Enable worktree support** checkbox with description: "Work on multiple branches at once. Worktrees give each tab its own copy of the repo, so you don't need to switch branches or stash changes." Disabled when the selected directory is not a git repo, with a tooltip explaining why.
- Progress indicator: 3 dots, dot 3 active.
- "Finish" button (primary, right-aligned in footer).

**Behavior**:
- Session type, directory, and worktree selections follow the same logic as the existing `SessionConfigModal`.
- Clicking "Finish" saves the tab config as the user's default and closes the flow. The flow is marked complete.
- Clicking "Finish" is the only way to leave this step. The tab config is saved and the flow is marked complete.

---

### Shared rendering

The session type picker, directory selector, and worktree checkbox in Step 4 should be extracted from the existing `SessionConfigModal` into shared rendering functions so the same code is used in both the onboarding flow and the standalone modal.

### Feature flag

- Flag name: `HOAOnboardingFlow`
- Added to `DOGFOOD_FLAGS` initially.
- Gates the entire flow: when disabled, existing users see nothing.

### Persistence

- A `private_user_preferences` key (e.g. `HasCompletedHOAOnboarding`) tracks whether the flow has been shown.
- The key is set to `true` when:
  - The user clicks "Finish" on Step 4.
  - The user clicks X on the welcome banner.
- The key is also set to `true` at the end of new-user onboarding, so new users never see this flow.

## Success Criteria

1. When `HOAOnboardingFlow` is enabled and the user has not seen the flow, the welcome banner appears on app launch.
2. The welcome banner displays all 4 feature descriptions with correct icons and text matching the Figma.
3. Clicking "See what's new" transitions to the vertical tabs callout (Step 2) with vertical tabs visible.
4. The "Switch back to horizontal tabs" checkbox live-toggles the tab layout setting. The UI immediately reflects the change (sidebar ↔ top tabs). The popover re-anchors its pointer.
5. Clicking "Next" on Step 2 transitions to the agent inbox callout (Step 3), anchored to the inbox icon.
6. Clicking "Next" on Step 3 transitions to the tab config step (Step 4).
7. The session type picker in Step 4 displays the same options as the existing `SessionConfigModal` (filtered by AI availability).
8. The directory picker opens a native file dialog and the worktree checkbox disables for non-git directories, matching existing behavior.
9. Clicking "Finish" saves the default tab config and closes the flow.
10. After completing or dismissing the flow, it never appears again for that user.
11. Users who complete the new-user onboarding never see this flow.
12. The flow is forward-only: no back button, no clickable progress dots.
13. Progress dots accurately reflect the current step (1/3, 2/3, 3/3) across Steps 2–4.

## Validation

- **Unit tests**: Verify the persistence logic (flag set on completion, flag set on dismiss, flag prevents re-show, flag set after new-user onboarding).
- **Manual / computer-use verification**:
  - Walk through all 4 steps and confirm layout, text, and icons match the Figma.
  - Verify the vertical tabs checkbox live-toggles the layout.
  - Verify the tab config step reuses session config modal behavior (session types, directory picker, worktree).
  - Verify dismissing via X at any point marks the flow complete.
  - Verify a fresh user who completes new-user onboarding does not see this flow.
- **Integration tests**: If feasible, test the full step progression and that the flow doesn't appear after completion.

## Resolved Design Decisions

1. **Hero art**: Static raster image asset for v1.
2. **Popover anchoring**: The existing FTU callout pattern (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:1758-1840`) uses stacked `CalloutTriangleBorderDown` / `CalloutTriangleFillDown` icons for the arrow and `OffsetPositioning::offset_from_save_position_element` for anchoring to named UI elements. Extract this into a shared callout helper and reuse it for Steps 2–3.
3. **Dismiss controls**: Only Step 1 has an X/close button. Steps 2–3 use "Next" and Step 4 uses "Finish".
