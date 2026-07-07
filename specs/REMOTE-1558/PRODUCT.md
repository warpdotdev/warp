# Local-to-Cloud Handoff: `&` Entrypoint — Product Spec
Linear: [REMOTE-1558](https://linear.app/warpdotdev/issue/REMOTE-1558)
## Summary
Add a fast `&` input entrypoint for local agent users to start a cloud run from their current flow. Typing `&` followed by a prompt lets the user choose a cloud environment from the local agent footer, then pressing Enter either hands off the current local conversation to cloud or starts a fresh cloud run when there is no local conversation history.
## Problem
The existing local-to-cloud handoff flow is discoverable through the footer chip and `/handoff`, but it is not optimized for keyboard-first use. Users who already know they want the next prompt to run in the cloud need a prefix-style flow that feels as lightweight as `!` shell-mode input while preserving handoff context, environment selection, attachments, and error safety.
## Goals
- A user in a local fullscreen agent view can type `& query` to start a cloud run without manually opening a handoff pane and pressing Enter again.
- The `&` flow exposes environment selection before submission by reusing the same visual and interaction patterns as the existing cloud environment selector.
- `/handoff query` starts the same auto-run behavior as `& query`; `/handoff` without a query activates `&` handoff-compose mode, same as the footer chip.
- While `&` mode is active, the input is explicitly locked in AI mode so autodetection and shell-mode transitions cannot steal the prompt.
- Auto-run submissions feel instant: once Warp opens and claims the cloud launch surface, the prompt leaves the editable input and the cloud pane shows a queued/starting state immediately, even if local handoff preparation is still finishing.
- The feature never silently drops the user's prompt or pending file/image attachments when handoff preparation fails.
- Existing local-to-cloud handoff behavior remains available for users who want to review or edit before submitting.
## Non-goals
- Redesigning the environment selector, environment management, or cloud-mode input UI.
- Adding an environment argument syntax to `/handoff`.
- Adding new selected text/block/document context serialization for cloud-mode submit. `&` matches current cloud-mode support by carrying prompt text and pending file/image attachments.
- Changing cloud-to-cloud handoff behavior.
- Bidirectional sync after handoff. Local and cloud conversations still diverge after the handoff point.
- Making pasted `& query` activate handoff mode. The prefix is an explicit typed shortcut.
## Figma
Figma: none provided.
## Behavior
### `&` handoff-compose mode
1. When the user is in a local fullscreen agent view and types `&` as the very first character of an empty input, the input enters handoff-compose mode.
2. `&` only activates handoff-compose mode when it is typed by the user as the first character. It does not activate when:
   - The user pastes text beginning with `&`.
   - The input has leading whitespace before `&`.
   - `&` appears anywhere other than the first character.
   - The user is not in a local fullscreen agent view.
   - The input is already in `!` shell mode.
3. After activation, the visible input behaves like `!` shell-mode input: the literal `&` is removed from the prompt text and rendered as a visible input indicator/chip. The prompt content sent to the cloud does not include the leading `&`. The `&` indicator uses the Agent/AI magenta color, not shell blue.
4. While handoff-compose mode is active, the input is locked in AI mode. Autodetection does not unlock it, and typed `!` is prompt text rather than a shell-mode transition until the user exits cloud handoff mode.
5. While handoff-compose mode is active and the prompt is empty, the input shows explanatory ghost/hint text so the user understands that the prompt will start a cloud run. Once the user has typed any prompt text, this ghost/hint text is hidden.
6. The exit affordance for handoff-compose mode mirrors `!` shell mode: the Agent View message bar shows Enter + "to hand off to cloud" and Backspace + "to dismiss". When the prompt is empty (Backspace can exit immediately), both labels use Agent/AI magenta. When prompt text is present, the Backspace label follows the same disabled/muted treatment as the `!` shell-mode message, while the Enter label stays active-colored.
7. When the prompt is empty, the user can press Backspace to exit cloud handoff mode. Exiting this way removes the `&` indicator and hides the transient environment selector.
8. While handoff-compose mode is active, the agent footer shows a transient environment selector in the existing footer left-side chip area. The selector uses the same visual style, menu behavior, focus behavior, labels, and environment-management affordance as the existing cloud-mode environment selector.
9. The transient environment selector is shown only for `&` handoff-compose mode. It is not shown for ordinary local agent prompts or `/handoff query`.
10. If the user selects an environment from the transient selector, that selection applies to the next `&` submission and persists as the user's last selected cloud environment, matching the existing environment selector behavior.
11. If the user does not explicitly select an environment in the transient selector, submission uses the normal cloud environment defaulting behavior. For a non-empty handoff source, touched-repo overlap may choose a better default after handoff preparation; for an empty source, the run uses the saved/default environment if one exists.
12. An explicit user selection in the transient selector always wins over any touched-repo overlap default discovered later.
13. If no environment exists or no environment is selected, `& query` can still start a cloud run without an environment, matching normal cloud-mode behavior.
14. While in handoff-compose mode, editing the prompt down to empty does not exit the mode; the user remains in handoff-compose mode with the `&` indicator and transient environment selector visible. Exiting requires an explicit affordance: Backspace on an already-empty buffer (item 7), Escape (item 15), or a programmatic buffer clear (e.g. starting a new conversation).
15. Pressing Escape while in handoff-compose mode exits handoff-compose mode, removes the `&` indicator, hides the transient environment selector, and keeps the prompt text in the local input.
16. Normal prompt editing continues to work while in handoff-compose mode. Pending file/image attachments remain visible and editable until the prompt is submitted or the user removes them.
### Submitting `& query`
17. Pressing Enter in handoff-compose mode with a non-empty prompt starts the cloud path. The user is not required to press Enter again in the cloud pane.
18. After Warp opens and claims the destination cloud surface, auto-submit is optimistically queued immediately: the submitted prompt is owned by the pending launch state rather than hydrated into the destination editor, and the cloud pane moves into its queued/starting state before slower handoff preparation finishes.
19. The submitted cloud prompt includes:
   - The prompt text without the leading `&`.
   - Pending file/image attachments.
   - The environment chosen in the transient selector, when the user chose one.
20. If the active local conversation is empty, `& query` starts a normal fresh cloud run:
   - No local conversation fork is created.
   - No local-to-cloud snapshot is prepared.
   - The selected/default environment is used when available.
   - The cloud pane opens through the normal cloud-mode flow.
21. If the active local conversation is non-empty, idle, and has synced cloud conversation identity, `& query` starts a local-to-cloud handoff:
   - The cloud pane opens in the same pane stack as the source pane.
   - The source local agent view exits behind the cloud pane, matching the current handoff navigation behavior.
   - The cloud pane shows the source conversation history as the handoff context.
   - The prompt is optimistically queued as soon as the handoff pane is ready to own it, without appearing as editable destination input; the actual cloud run dispatch occurs automatically once the conversation fork, environment selection, and local snapshot preparation are ready.
22. If the active local conversation is non-empty but currently running or blocked, submission is blocked. Warp shows a toast explaining that handoff can only start from an idle conversation, leaves the user in the source local agent view, and preserves the prompt and pending file/image attachments.
23. If the active local conversation is non-empty and idle but cannot be handed off because it lacks synced cloud conversation identity, submission is blocked. Warp shows a toast, leaves the user in the source local agent view, and preserves the prompt and pending file/image attachments.
24. If handoff preparation fails before a cloud pane exists, such as a failed conversation fork request, Warp stays in the source local agent view, shows a toast, and preserves the prompt and pending file/image attachments.
25. If handoff preparation fails after a cloud pane has opened but before the cloud run is accepted by the server, Warp restores the prompt and pending file/image attachments into the handoff pane, shows a toast, returns the pane to a retryable compose state, and lets the user retry manually from that pane.
26. Warp must not auto-start a non-empty local-to-cloud handoff without the intended local context. If the handoff cannot safely carry the forked conversation and usable prepared snapshot, it falls back to the retryable handoff pane state rather than silently starting a reduced-context cloud run.
27. Once the destination cloud surface claims an auto-submit launch, the source input no longer shows the submitted prompt or pending attachments. If the launch later fails before the server accepts the cloud run, the prompt and attachments are restored in the destination pane rather than the source input.
28. Closing a cloud pane while auto-start preparation is still in progress cancels that pending auto-start from the user's perspective. The source local conversation is unaffected.
### `/handoff`
29. `/handoff` with no query activates `&` handoff-compose mode on the local input, the same as clicking the footer chip or typing `&`. The user sees the `&` indicator, transient environment selector, and message bar hints, and must type a prompt and press Enter to proceed.
30. `/handoff query` starts the same auto-run behavior as `& query`, including immediate optimistic queueing after the destination cloud surface claims the launch.
31. `/handoff query` from an empty local agent conversation starts a normal fresh cloud run.
32. `/handoff query` from a non-empty local conversation attempts local-to-cloud handoff with automatic run start, using touched-repo overlap/default environment selection. It does not show the transient `&` environment selector before starting.
33. `/handoff query` follows the same blocked and failure behavior as `& query`: running/blocked conversations are blocked with a toast; missing synced cloud conversation identity is blocked with a toast; user input and pending file/image attachments are not silently discarded.
34. If `/handoff query` fails before a pane opens, the slash-command text or extracted prompt remains available in the source input so the user can retry or edit it. The exact text representation may follow existing slash-command input conventions, but the user's prompt must not be lost.
### Existing handoff chip
36. Clicking the "Hand off to cloud" footer chip activates `&` handoff-compose mode, matching the same state as typing `&` as the first character. If the user is already in handoff-compose mode, the chip click is a no-op.
37. The chip does not auto-start a cloud run or open a cloud pane. It only enters handoff-compose mode; the user must type a prompt and press Enter to proceed.
38. Because the chip activates `&` mode, the transient environment selector, message bar affordances, and input indicator all appear in the source local footer after clicking.
### Environment behavior
39. Environment selection has three priority levels for auto-started local-to-cloud handoff:
   1. User's explicit transient `&` selection, if present.
   2. Touched-repo overlap default, if a non-empty source conversation identifies one.
   3. Existing cloud environment defaulting behavior.
40. Auto-started fresh cloud runs from empty conversations use only the explicit transient `&` selection or existing cloud environment defaults, because there is no touched local conversation to score for overlap.
41. Environment selector labels, disabled states, keyboard navigation, menu closing behavior, and "New environment" behavior should match the existing selector wherever the selector appears.
### Focus, keyboard, and accessibility
42. Activating `&` mode keeps focus in the prompt editor so the user can continue typing immediately.
43. Opening the transient environment selector follows the existing selector's focus and keyboard behavior. Closing it returns focus to the prompt editor.
44. The `&` indicator and the handoff-compose hint text are part of the input chrome and should be understandable to screen readers at least as well as the existing `!` input indicator and mode hints.
45. The visual `&` indicator and "backspace to exit cloud mode" affordance use Agent/AI magenta so they read as cloud-agent mode, not shell mode.
46. Submitting a successful `& query` moves focus to the opened cloud pane, matching the existing handoff/cloud-mode navigation behavior.
### Invariants
47. The prompt sent to the cloud never includes the leading `&`.
48. `&` handoff-compose mode is always locked AI input while active, and it never changes normal AI-vs-shell classification semantics outside the active prompt.
49. `&` handoff-compose mode and `!` shell mode are mutually exclusive. Activating one must prevent or exit the other; the input must never show both indicators or dispatch both mode behaviors for the same prompt.
50. The user's explicit environment choice is never overwritten by automatic touched-repo overlap.
51. User-entered prompt text and pending file/image attachments are never silently dropped on blocked or failed handoff attempts. If an optimistic auto-submit fails before the server accepts the cloud run, the prompt and attachments are restored into a retryable input.
52. Empty local conversations use fresh cloud-run behavior; non-empty eligible local conversations use local-to-cloud handoff behavior.
53. Local and cloud conversations remain independent after handoff. Continuing in one does not update the other.
