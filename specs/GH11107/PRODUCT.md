# GH11107: Reduce first-time agent onboarding callouts
## Summary
Reduce the first-time Agent Modality onboarding tutorial from four callouts to two callouts, and remove the FTUE tab/session configuration modal from the handoff into the tutorial. The shorter flow should teach the same essential concepts with less interruption: terminal input can route commands or natural language, and agent conversations now live in their own scoped agent experience.
## Problem
The current Agent Modality first-time tutorial shows four sequential callouts before the user can finish onboarding. That amount of instructional UI is too heavy for a first-run experience, especially because some concepts can be combined without losing clarity.
## Goals
- Show at most two callouts for the Agent Driven Development onboarding tutorial.
- Preserve the natural language detection opt-in/override explanation.
- Preserve the transition from terminal input into the scoped agent experience.
- Preserve project initialization behavior for users who selected a project.
- Preserve the ability to finish without submitting anything for users who did not select a project.
- Start the post-slide tutorial directly without requiring first-time users to configure a tab, session type, worktree, or startup config.
- Reuse the existing callout visual style, button conventions, keyboard shortcuts, progress dots, and placement patterns.
## Non-goals
- Redesigning the callout component.
- Changing the broader onboarding slides before the callout tutorial starts.
- Changing the natural language detection setting outside the first-run tutorial.
- Changing how the agent experience itself works after the tutorial completes.
- Removing or changing the manual session configuration modal outside FTUE.
- Changing reusable tab config, worktree, or new-session menu behavior outside the FTUE handoff.
- Introducing a new visual treatment, animation, or Figma-driven layout for the callouts.
## Figma
Figma: none provided. This change uses the existing onboarding callout component and consolidates existing callout content.
## Behavior
1. When a first-time user enters the Agent Driven Development tutorial with Agent Modality enabled, Warp shows exactly two sequential callouts.
1. First-time Agent Driven Development users do not see the tab config/session config modal between completing onboarding slides and starting the callout tutorial.
1. First-time Agent Driven Development users are not required to choose a session type, project setup, worktree setup, or startup tab config before the callout tutorial starts.
1. If the user selected a project in the onboarding slides, Warp should continue using that selection for the tutorial/project initialization path without re-asking through the tab config modal.
1. Manual session configuration remains available from its existing non-FTUE entrypoints.
2. The two-callout sequence is:
   - Callout 1: terminal input with natural language support.
   - Callout 2: Warp's agent experience.
3. Callout 1 teaches that the terminal input can be used for terminal commands and can also support natural language requests for the agent.
4. Callout 1 includes the natural language detection explanation:
   - Natural language detection is off by default when the user's setting is initially off.
   - If enabled, Warp can autodetect plain-English agent requests typed into terminal input.
   - The user can override auto-detection with the configured input-mode toggle keybinding.
5. If natural language detection was initially off, Callout 1 includes a checkbox labeled `Enable Natural Language Detection`.
6. The natural language detection checkbox reflects the current setting value while the callout is visible.
7. Toggling the natural language detection checkbox updates the setting immediately.
8. If natural language detection was already enabled before the tutorial started, Callout 1 does not need to show the enable checkbox. It should instead use shorter copy focused on the override keybinding.
9. Callout 1 has a primary `Next` action for Agent Driven Development users.
10. Callout 1 shows the first active progress dot in a two-dot sequence for Agent Driven Development users.
11. While Callout 1 is visible, the tutorial remains anchored to the terminal input / terminal context. The user should not be moved into the scoped agent experience before advancing past Callout 1.
12. Advancing from Callout 1 enters the scoped agent experience and shows Callout 2.
13. Callout 2 teaches that agent conversations are their own scoped view outside the terminal, and that the user can press `ESC` to return to the terminal.
14. Callout 2 shows the second active progress dot in a two-dot sequence.
15. For users who selected a project before the tutorial:
   - Callout 2 offers an initialization action.
   - The primary action is `Initialize`.
   - A secondary action lets the user skip initialization.
   - Choosing `Initialize` submits the initialization flow just as the current final onboarding callout does.
   - Choosing skip finishes the tutorial without submitting initialization.
   - Pressing `ESC` while this callout is focused still exits the scoped agent experience and returns to terminal context.
16. For users who did not select a project before the tutorial:
   - Callout 2 offers a primary `Finish` action.
   - Callout 2 offers a secondary `Back to terminal` action with the `ESC` keybinding.
   - Choosing `Finish` ends the tutorial without submitting a prompt.
   - Choosing `Back to terminal` exits the scoped agent experience, clears the tutorial prompt, and returns to terminal context.
17. The existing placeholder prompt behavior should remain coherent:
   - Callout 1 should populate terminal-context sample input.
   - Callout 2 should populate agent-context sample input, or `/init` for the project initialization path.
   - Finishing, skipping, or returning to terminal clears tutorial-provided input.
18. Terminal-intention onboarding remains terminal-focused. It should not enter the scoped agent experience or show the agent-experience callout unless the user chose the Agent Driven Development path.
19. Keyboard shortcuts continue to work while each callout is focused:
   - `Enter` advances or activates the primary action.
   - `Backspace`/delete activates the skip action only when a skip action is visible.
   - `Escape` returns to terminal while the final agent-experience callout is visible.
20. Existing telemetry concepts remain meaningful:
   - A display event is recorded for each visible callout.
   - A next event is recorded when the user advances.
   - A completion event is recorded with the user's completion type.
   - Removed callouts should not be reported as displayed.
21. The shortened flow should not regress non-Agent-Modality onboarding. Universal Input onboarding should continue to use its existing flow.
22. If onboarding is triggered while Warp is running in a mode that cannot show onboarding, no callouts are shown, matching existing behavior.
23. If a tutorial is already active, starting the tutorial again should continue to be ignored rather than creating duplicate callouts.
24. The callout UI uses the existing component styling, theme colors, progress dots, button styling, and layout behavior. This change is a content and flow reduction, not a visual redesign.
