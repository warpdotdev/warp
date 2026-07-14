# PRODUCT: TUI Orchestration Permission and Configuration
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)

## Summary
The Warp TUI gains an interactive permission card for an active `RunAgents` request. A user can review the proposed agents and run-wide configuration, accept or reject the request, or edit its configuration using the keyboard or mouse without leaving the TUI.

## Figma
- Initial acceptance card: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=766-18419&m=dev
- Location page: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=777-13269&m=dev
- Harness page: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=783-13443&m=dev
- Model page: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=783-13585&m=dev

## Goals
- Give the TUI keyboard- and mouse-driven controls for reviewing, editing, accepting, and rejecting an active orchestration request.
- Provide parity with the GUI's existing cloud orchestration choices and validation while preserving the TUI-specific interaction design.
- Establish reusable TUI patterns for option selection and for temporarily replacing the main input with a blocking interaction.

## Non-goals
- Creating API keys or environments from this flow. The TUI chooses among existing resources.
- The GUI's “continue without orchestration” action. The TUI offers Accept, Edit, and Reject.
- Changing how the agent decides to request orchestration. This spec begins when a `RunAgents` request is ready for user confirmation.
- Implementing a separate TUI orchestration executor. A valid acceptance uses the existing shared execution path; upstream work that makes orchestration requests reachable end to end in the TUI may land in a stacked change.

## Behavior
### Active interaction and input visibility
1. When a `RunAgents` request reaches the front of the confirmation queue and awaits a decision, its permission card becomes the active interaction.
2. While the permission card or one of its configuration pages is active, the main input row, input cursor, and normal input footer are hidden. The permission surface provides the relevant action hints in their place.
3. Hiding the input preserves its draft text, cursor position, selection, editor mode, and scroll state without modification.
4. Pending requests behind the active front-of-queue blocker do not independently affect input visibility or focus.
5. Accepting or rejecting the request ends the blocking interaction immediately, even while an accepted request continues into a spawning state. The main input and footer reappear with their preserved state, and prior focus is restored.
6. If the next queued action is also a blocking interaction, focus and input visibility transition directly to that interaction without briefly exposing an editable input.
7. A restored, completed, cancelled, rejected, spawning, succeeded, partially succeeded, or failed card is non-interactive and does not hide the input.
8. A request can be accepted or rejected only once.

### Acceptance card
9. The initial card shows:
   - “Can I start additional agents for this task?”
   - The agent-provided summary of why orchestration is requested.
   - Every proposed agent's name.
   - The current run-wide location, harness, and model.
   - For Cloud runs, the current API-key choice when applicable, host, and environment.
10. Returning from configuration updates the displayed run-wide values. The user always reviews the final values on the acceptance card before launching.
11. Every proposed agent has a deterministic color-and-glyph identity that remains stable for the life of the request, including across re-renders and configuration edits.
12. Agent identities use theme-derived ANSI colors rather than fixed RGB values. The palette provides at least 32 distinct color-and-glyph combinations, covering the current maximum agents in one request.
13. If a future request exceeds the number of unique combinations, the palette cycles deterministically. No agent is omitted and rendering does not fail.
14. The card uses the orchestration treatment from the designs: one 10%-magenta-tinted body/header surface, a yellow square attention glyph, primary text for content, muted separators and metadata, and bold magenta emphasis for selected configuration options.
15. Text and agent identities wrap and reflow at narrow terminal widths. If the complete card cannot fit vertically, it remains navigable without clipping required configuration or actions.
16. On the acceptance card:
   - Enter accepts the current configuration.
   - Ctrl+E opens configuration.
   - Ctrl+C rejects the request.
17. The footer renders these bindings using the active theme and the exact actions available in the current state.

### Configuration flow
18. Configuration is a sequence of single-field pages. The tinted card keeps the permission title visible, then shows `Edit agent configuration` with right-aligned `← n of m →` navigation, one bold question, a selectable option list, and contextual keybinding hints below the tinted surface.
19. Cloud uses this order:
   1. Location
   2. Harness
   3. API key, only when the selected harness supports managed credentials
   4. Host
   5. Environment
   6. Model
20. The page count is dynamic. Adding or removing the conditional API-key page immediately updates the current position and total.
21. Selecting Local immediately forces the Warp harness and removes the Harness, API key, Host, and Environment pages. The flow becomes Location (1 of 2) followed by Model (2 of 2).
22. Selecting Cloud restores the applicable Cloud page sequence and valid selections from the current edit session when possible.
23. Each page initially highlights the request's current value, including values inherited from an approved plan configuration. If the current value is unavailable, the page highlights the appropriate valid default and clearly reflects the replacement.
24. A confirmed selection is saved immediately to the edit session.
   - Tab and Right move to the next applicable page without confirming the current highlight.
   - Left moves to the previous applicable page without confirming the current highlight.
   - Navigation clamps at the first and final pages; the unavailable boundary arrow is muted.
25. Confirming a selection on a non-final page advances to the next applicable page.
26. Confirming the final page returns to the acceptance card without launching. A second Enter on the acceptance card is required to launch the edited request.
27. Esc returns to the acceptance card and retains selections confirmed on completed pages. The current page's highlighted but unconfirmed option is discarded.
28. Ctrl+C rejects the entire request from any configuration page.

### General option selection
29. Every configuration page uses the same option-selection behavior and presentation.
30. Up and Down move the highlight through options without confirming a value. Four rows are visible at once; navigation scrolls when the highlight moves beyond that viewport and shows `↑` / `↓` overflow markers.
31. Enter confirms the highlighted option.
32. Number keys 1–9 confirm the corresponding visible option, when present, and advance immediately. The shortcuts are viewport-relative so they remain useful in long, scrolled lists.
33. Options beyond the four-row viewport remain reachable with scrolling, Up and Down, and Enter.
34. Clicking an enabled option confirms it and advances exactly like Enter.
35. Mouse-wheel and trackpad input scroll lists that exceed the available height.
36. Rows render with `(n)` number prefixes. The selected row is bold magenta without a separate marker or background. Disabled rows can be highlighted for context but cannot be confirmed; they show a concise reason when available.
37. Empty lists show a non-selectable empty state rather than leaving a blank surface.

### Field-specific choices
38. Location offers Cloud and Local.
39. Harness shows the same live availability, display labels, ordering, and disabled reasons as the GUI's orchestration controls, subject to the TUI's Local behavior in (21).
40. Model shows the same live, harness-specific catalog, labels, ordering, and default behavior as the GUI:
   - Warp Cloud excludes unsupported custom models.
   - Warp Local includes models supported by local Warp agents.
   - Non-Warp Cloud harnesses include their harness default and server-provided models.
41. API key appears only for Cloud harnesses that support managed credentials. It offers:
   - “Skip (advanced)” to inherit credentials from the selected worker environment.
   - Existing named managed secrets valid for the selected harness.
   - No resource-creation option.
42. Host appears only for Cloud. It offers the Warp-hosted option, the workspace default when configured, known connected self-hosted workers, the user's recent custom host when available, and a custom-host text-entry option.
43. A custom host is trimmed and validated before it is confirmed. Invalid or empty custom input remains editable and shows a concise error. Once confirmed, the user-entered value replaces the generic `Custom host…` option text and pre-fills the editor if selected again.
44. Environment appears only for Cloud. It offers “Empty environment” plus existing environments, using the same labels and default-selection behavior as the GUI. It does not offer environment creation.
45. Switching Location or Harness revalidates all dependent fields:
   - Local forces Warp and removes Cloud-only values from the launch configuration.
   - A Harness change restores that harness's prior model selection when still valid, otherwise it selects the appropriate default.
   - A Harness change re-resolves the API-key choice for the new harness.
   - Values that remain applicable are preserved.
46. The incoming per-call computer-use value is preserved through editing but is not presented as a configuration page because the GUI does not expose it as an editable orchestration choice.

### Loading, refresh, and failures
47. Pages backed by data that has not loaded show a non-selectable Loading row.
48. A load failure shows an inline error and a Retry action reachable by keyboard and mouse.
49. A prior selection remains visible while its catalog refreshes or retries. A transient failure never silently clears it.
50. Live catalog changes refresh the relevant list. If the selected item disappears, the page explains that it is unavailable and selects a valid default only when required to proceed.
51. Secret values are never displayed. The UI shows managed-secret names only.

### Validation, acceptance, and rejection
52. The acceptance card validates the edited configuration before launch using the same rules as the GUI and shared execution path.
53. Invalid or incomplete configurations cannot launch. Enter leaves the card active and shows a visible reason directing the user to the field that needs attention.
54. Examples of blocked acceptance include an unavailable local configuration, an unsupported Cloud harness, and a required API-key choice that is still unset.
55. Accepting a valid request sends the edited request through the shared orchestration execution path and transitions the card to the existing spawning/result presentation.
56. Rejecting resolves the request as rejected and transitions the card to a non-interactive terminal presentation.
57. Spawning, mixed success, success, failure, cancellation, and denial use the existing TUI tool-status semantics and restore the input as described in (5).
