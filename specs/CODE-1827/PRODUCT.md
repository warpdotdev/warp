# CODE-1827: TUI Local-to-Cloud Handoff

## Summary

Warp’s headless TUI lets users fork a local Oz conversation into an Oz cloud run with `/handoff [prompt]`. Handoff preserves the local transcript, gathers the required cloud configuration in a blocking card, transfers applicable local context, and leaves the user with explicit choices to open the cloud run, continue locally, or start a new conversation.

## Problem

TUI users can run local agents and launch cloud agents, but they cannot move an existing local conversation and workspace state into a cloud run. Replacing the current TUI conversation with a cloud view would also hide the local transcript and make the forked nature of handoff unclear.

## Goals

- Provide a keyboard-first local-to-cloud handoff flow for Oz conversations.
- Preserve the local conversation and make local continuation explicit.
- Match the established GUI handoff semantics for conversation forking, continuation prompts, environment suggestion, snapshots, and guardrails.
- Keep cloud environment creation and cloud-run inspection outside the TUI.

## Non-goals

- Adding an `&` prefix entrypoint to the TUI.
- Adding a TUI environment-creation form.
- Turning the local TUI session into a cloud session or embedding a cloud-run viewer.
- Supporting a selectable harness, execution location, worker host, or API key.
- Moving an entire orchestration tree into cloud.
- Adding in-card retry after a fatal handoff failure.
- Adding cloud-to-local or cloud-to-cloud handoff behavior.

## Figma

Figma: none provided. The existing TUI orchestration card is the structural and copy-style reference. The GUI `&` handoff affordance is the accent-color reference.

## Behavior

### Availability and command composition

1. `/handoff` is available only in a local Oz conversation when the existing local-to-cloud handoff settings, account policy, privacy policy, AI availability, and native-platform requirements permit handoff.

2. `/handoff` is not available from a cloud-agent conversation.

3. Selecting `/handoff` in the slash-command menu inserts `/handoff ` into the input instead of executing it immediately.

4. While `/handoff ` is in the input with no argument, ghost text communicates that adding a prompt is optional.

5. The user may submit either:
   - `/handoff`, to hand off with no explicit prompt.
   - `/handoff <prompt>`, to provide the cloud run’s initial prompt.

6. Leading and trailing whitespace is trimmed from a non-empty prompt argument. The remaining prompt text is preserved for the cloud run, apart from the same query-mode parsing that applies to normal agent prompts.

7. If the selected local conversation has no content:
   - `/handoff` with no prompt is rejected with a transient bottom-row message explaining that there is nothing to hand off.
   - `/handoff <prompt>` enters the same configuration flow and creates a fresh cloud run rather than forking a conversation.

### Eligibility and cancellation

8. Before cancelling local generation, Warp checks whether a long-running command is active. If one is active:
   - Handoff is rejected.
   - The command and agent conversation remain untouched.
   - The prompt argument and pending images remain in local input.
   - A transient bottom-row message tells the user to cancel the command or wait for it to finish.

9. Before cancelling an orchestrator conversation, Warp checks every loaded child conversation. An in-progress or blocked child prevents handoff.

10. When a child prevents handoff:
   - The parent and children remain untouched.
   - The prompt argument and pending images remain in local input.
   - A transient bottom-row message explains that active child work must finish or be cancelled first.

11. Finished, failed, and cancelled child conversations do not prevent parent handoff.

12. After the command and orchestration guardrails pass, submitting `/handoff` immediately cancels an in-progress or blocked source response before showing the handoff card.

13. The source’s active state is captured before cancellation. Later prompt synthesis uses that captured state; cancellation must not cause the cloud run to forget that it interrupted active work.

14. Eager cancellation also occurs when configuration is subsequently blocked by no available environment or an incompatible model. The agent must never continue streaming behind the blocking handoff card.

15. Eager cancellation does not itself fork the conversation, upload a snapshot, or create a cloud run.

### Blocking card and focus

16. After preparation succeeds, a handoff card replaces the local input area. The existing transcript remains visible and unchanged.

17. The handoff card belongs to the source TUI session. If another session becomes focused through existing session navigation, the handoff state does not move to that session; returning to the source session restores its card and state.

18. While the handoff card is active:
   - Normal input, image attachments, inline menus, footer content, and response-summary/warping rows are hidden.
   - Keyboard focus belongs to the handoff card or its active selector.
   - The hidden local input’s cursor and editing state are not mutated.

19. The card follows the TUI orchestration card’s visual hierarchy:
   - A tinted title row.
   - A tinted body containing configuration or status.
   - A separate key-hint row.
   - Inline attention and error presentation consistent with orchestration.

20. The card uses the active theme’s normal ANSI magenta—the same themed color source as the GUI `&` handoff indicator. Color is not the only indicator of card state or action availability.

### Environment and model configuration

21. The configuration card displays exactly two editable values:
   - Environment.
   - Model.

22. Execution is always remote and the harness is always Oz. Location, harness, worker host, and API-key choices are not shown.

23. The initial environment is the user’s saved/recent environment when one remains available.

24. While the card is open, Warp asynchronously inspects the current working directory’s Git repository. If an available environment contains that repository, Warp may replace the default suggestion with the most-recent matching environment.

25. A repository-based suggestion never overwrites a selection the user explicitly made.

26. Environment discovery does not delay showing the card or prevent the user from interacting with it.

27. `E` opens the environment selector. Within the selector:
   - Arrow keys navigate.
   - Enter applies the selected environment and returns to the card.
   - Escape returns to the card without applying the currently highlighted change.

28. `M` opens the model selector. It provides the same searchable model-selection behavior as the existing TUI model selector.

29. The model initially matches the local conversation’s model when that model can run in Oz cloud.

30. If the local model cannot run in Oz cloud:
   - The card identifies the model as incompatible.
   - Handoff confirmation remains unavailable.
   - The user must explicitly choose a compatible model.
   - Warp never silently substitutes `auto` or another model.

31. Enter confirms handoff only when the selected environment still exists and the selected model remains compatible.

### No-environment state

32. If the user has no cloud environments, the card explains that an environment is required and does not offer an in-TUI creation form.

33. In the no-environment state:
   - Enter opens `https://docs.warp.dev/agent-platform/cloud-agents/environments`.
   - `R` refreshes cloud environments.
   - Escape or Ctrl-C cancels handoff.

34. The card automatically observes environment changes. When an environment becomes available, the same card transitions to normal configuration without losing the prompt, images, or captured source state.

35. Manual refresh and automatic environment updates must not create a separate handoff flow or duplicate the card.

### Prompt and image ownership

36. Pending TUI images move with the handoff prompt when the card opens:
   - They disappear from the normal local attachment bar.
   - They remain associated with the pending handoff.
   - They are included in the cloud run if handoff succeeds.

37. If pre-confirmation handoff is cancelled or handoff later fails fatally, the prompt argument and images return to local input.

38. After successful handoff, the prompt and images are considered consumed. Choosing **Continue locally** reopens a clean local input.

### Pre-confirmation cancellation

39. Before confirmation, Escape or Ctrl-C cancels the handoff card.

40. Cancelling before confirmation:
   - Creates no server conversation fork.
   - Uploads no snapshot.
   - Creates no cloud run.
   - Reopens local input.
   - Restores only the optional prompt argument, without restoring `/handoff`.
   - Restores pending images.

41. Cancelling pre-confirmation does not automatically resume a response that was eagerly cancelled.

### Commitment and progress

42. Enter on a valid configuration commits the handoff. Confirmation is the point of no return.

43. After confirmation:
   - Configuration and cancellation actions disappear.
   - The card displays handoff progress.
   - Ctrl-C is consumed without cancelling handoff or exiting the TUI.
   - The local input remains blocked until a cloud run is created or handoff fails.

44. The committed operation revalidates environment, model, and handoff availability before external work begins. If revalidation fails, the card returns to editable configuration without creating a fork or cloud run.

45. For a non-empty source conversation, handoff creates an independent cloud fork. The original local conversation and transcript remain available and do not synchronize with subsequent cloud changes.

46. For an empty source conversation with an explicit prompt, handoff creates a fresh cloud run without a conversation fork.

47. A permitted orchestrator handoff forks only the selected conversation. The local orchestration tree remains available locally, and the cloud run receives the context necessary to understand that it came from an orchestration handoff.

48. Snapshot collection for a permitted orchestrator includes workspace changes represented by completed child conversations as well as the selected parent conversation.

49. Handoff waits for local workspace snapshot preparation to settle before creating the cloud run.

50. Snapshot derivation or upload failure is non-fatal:
   - The failure is not displayed in the card.
   - Handoff continues without the missing snapshot.
   - Conversation history is still handed off.

51. An explicit user prompt is sent unchanged even when local generation was interrupted or a snapshot exists.

52. With no explicit user prompt, the cloud run receives:
   - Interrupted source with snapshot content: `Continue. Apply the workspace changes from my previous session.`
   - Interrupted source without snapshot content: `Continue`.
   - Idle source with snapshot content: `Apply the workspace changes from my previous session.`
   - Idle source without snapshot content: no initial prompt.

### Successful handoff

53. Handoff is considered created when Warp receives a cloud run identifier.

54. The card then becomes a static completed card. It does not continue tracking queued, setup, running, completed, cancelled, or failed cloud-task state.

55. For a forked conversation, the completed card provides:
   - Enter: open the cloud run.
   - `C`: continue locally.
   - `N`: start a new conversation.

56. For a fresh cloud launch with no source conversation, the completed card omits **Continue locally** and provides only:
   - Enter: open the cloud run.
   - `N`: start a new conversation.

57. Opening the cloud run launches its Oz URL in the system browser.

58. Opening the cloud run does not dismiss the card, replace the TUI transcript, switch to a cloud-run view, or unblock local input.

59. **Continue locally** only removes the card and reopens the existing local input. It does not insert or submit `Continue`.

60. **Start new conversation** removes the card and performs the TUI’s existing new-conversation behavior.

### Failures and state changes

61. Fatal handoff failure after confirmation:
   - Removes the blocking card.
   - Reopens local input.
   - Restores the optional prompt argument and pending images.
   - Shows a transient bottom-row error.
   - Does not offer in-card retry.

62. A user may submit `/handoff` again after a fatal failure.

63. If handoff becomes unavailable while the card is still editable—for example, because account policy or privacy settings change—the card closes, local input is restored, and no external handoff work begins.

64. Once confirmation has committed the operation, later focus changes or card re-renders must not start a second fork, snapshot upload, or cloud run.

65. Delayed environment suggestions, refresh results, or other asynchronous pre-confirmation updates must be ignored after the card is cancelled, replaced, or committed.

66. The TUI must never show two handoff cards for the same session at once.
