# PRODUCT: TUI tool-call permission requests (CODE-1809)

Linear: [CODE-1809](https://linear.app/warpdotdev/issue/CODE-1809/implement-permissions-requests-ui-for-tool-calls)

## Summary

The TUI respects the user's active AI execution profile instead of force-enabling Fast Forward for every conversation. When that profile requires approval for a tool call, the TUI shows an inline permission request that lets the user approve it, reject it, or replace it with written guidance for the agent.

## Figma

- Permission request and Yes/No/Other options: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-22641&m=dev
- Editable shell-command request: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-22740&m=dev

## Non-goals

- Adding a Fast Forward toggle, keybinding, footer control, or settings surface to the TUI.
- Changing permission policy, execution-profile defaults, denylist behavior, or organization-level restrictions.
- Editing file diffs or arguments for tools other than shell commands.
- Replacing the specialized ask-question or orchestration configuration flows.
- Revalidating file contents between preview generation and acceptance beyond the behavior shared with the GUI.

## Behavior

### Permission policy and blocking

1. New TUI conversations begin in the normal permission mode defined by the user's active AI execution profile. The TUI does not force Fast Forward on when it creates or selects a new conversation.

2. Actions that the active profile permits automatically continue to execute without prompting. Existing hard denials and restrictions continue to take precedence exactly as they do in the GUI.

3. When an action requires confirmation, the action remains blocked and does not produce its external side effect until the user approves it.

4. The blocked action renders inline at its normal position in the transcript. The request contains:
   - A human-readable question describing the permission being requested.
   - A body describing the proposed action.
   - A numbered option list: `(1) yes`, `(2) no`, and `(3) Other`.
   - A full-width card with a visually distinct header and body.
   - A keybinding-hint footer below and outside the tinted card body.

5. `yes` is selected by default when the request first becomes interactive. The active option uses the established TUI option-selector treatment shown in Figma.

6. The user can move through the options with the existing option-selector interactions, including arrow keys, number keys, Enter, mouse selection, and scrolling when necessary. Permission interactions use the same focus and selection conventions as existing TUI question and orchestration flows.

7. Only the currently actionable blocked request consumes permission-response input. Completed, cancelled, queued, or historical tool calls do not intercept the permission keybindings.

8. Losing terminal focus, scrolling the request off screen, resizing the terminal, or switching transcript focus does not approve or reject the request. The pending choice and any in-progress edit remain available when the request is visible again.

### Yes, No, and Other

9. Choosing `yes` approves the pending action once. The action then transitions through its normal running and terminal success or failure states.

10. Choosing `no` rejects the pending action using the GUI's existing rejection semantics and copy. Rejecting one action does not change the execution profile or enable a permanent denial.

11. Choosing `Other` opens an inline editable field for replacement guidance. The field accepts the same ordinary text-editing interactions as other TUI free-text inputs.

12. Submitting non-empty Other text sends it to the agent as a new user prompt in the same conversation. Like an ordinary follow-up submitted during an active turn, this supersedes the blocked action and any remaining queued actions without executing them. It does not first invoke the `no` or Esc rejection path.

13. Other text does not mutate the pending action's arguments or approve any replacement action implicitly. Any new action proposed by the agent is evaluated independently against the active execution profile.

14. Whitespace-only Other text is not submitted. Cancelling the Other editor returns to the option list without resolving the pending action or losing the entered text during that interaction.

15. Esc from the option list rejects the pending action consistently with the displayed `Esc to cancel` affordance and the GUI's rejection behavior.

### Generic tool requests

16. Tool types without a specialized body share one permission-request presentation. The body displays the action type and the user-relevant arguments needed to understand what will happen, using structured labels rather than debug formatting.

17. The shared presentation covers every action type that can require confirmation and does not otherwise have a specialized TUI interaction, including file reads, artifact uploads, codebase search, grep, file glob, MCP tool calls, MCP resource reads, computer-use requests, writes to long-running shell commands, shell-control transfer, and new-conversation suggestions.

18. Long or multiline arguments wrap within the available terminal width. The request remains usable at narrow widths and does not require horizontal scrolling.

19. Sensitive-value treatment matches the equivalent GUI permission request. The TUI does not introduce a second, inconsistent permission payload or display policy.

### Shell-command requests

20. A shell-command permission request shows the proposed command between the permission question and the Yes/No/Other options, matching the Figma structure.

21. `yes` is initially selected. Pressing Up from the selected `yes` option enters command-editing mode. The displayed edit/save keybinding also enters and exits command-editing mode.

22. In command-editing mode:
    - The cursor appears in the command field.
    - Normal text editing and multiline behavior apply.
    - The option list remains visible but has no active selection.
    - The command is not executed merely by editing or leaving the editor.

23. Pressing Enter or Down while editing saves the current command text and returns focus to the option list with `yes` selected. Approving afterward executes the edited command as the same pending action.

24. Returning to the option list preserves the edited command. The user can re-enter editing before resolving the request.

25. Selecting `no` or submitting Other guidance rejects the command without executing either its original or edited text.

26. An empty or whitespace-only edited command cannot be approved for execution. The permission request remains unresolved until the user supplies an executable command, chooses no, or submits Other guidance.

### File-edit requests

27. A file-edit permission request displays the complete proposed diff before any file is created, modified, renamed, or deleted.

28. File diffs are read-only. The user can inspect and collapse them but cannot edit the proposed content.

29. A single-file request renders the existing per-file summary and diff section above the Yes/No/Other options. It begins collapsed.

30. A multi-file request renders the existing outer aggregate group above the options. The outer group and each nested per-file section begin collapsed and remain independently expandable.

31. Every file-edit view begins collapsed, regardless of whether the action requires permission or autoexecutes. This same initial state applies when a historical file-edit view is reconstructed.

32. Expanding the outer group reveals the per-file summaries. Expanding a file reveals its proposed diff using the existing TUI diff presentation, including filenames, operation verbs, line counts, context elision, line numbers, and theme colors.

33. The user may approve a file-edit request while every diff remains collapsed. Expanding a diff is optional and does not unlock or otherwise change the approval options.

34. Accepting the request applies the complete proposed set of file edits through the normal shared execution path. A multi-file request is approved or rejected as one action; individual files cannot be approved independently.

35. Rejecting the request or submitting Other guidance leaves every affected file unchanged.

36. Collapse state is preserved when the request resolves. Sections the user expanded remain expanded; sections left collapsed remain collapsed in the completed transcript entry.

37. If a proposed diff cannot be computed, the request shows the normal failure or fallback summary rather than an empty or misleading preview. It must never imply that unavailable changes were reviewed.

### Existing specialized requests and lifecycle

38. Ask-question retains its existing multi-question and custom-answer interaction. It is not replaced by the generic permission request.

39. Run-agents retains its existing orchestration configuration and confirmation flow. It is not reduced to a generic Yes/No/Other request.

40. Once a permission request resolves, its interactive options no longer accept input. Its transcript entry shows the normal result state for the action: running, succeeded, failed, or rejected.

41. If the pending action is cancelled or resolved by another event before the user responds, the permission controls become inactive and cannot execute or reject the stale action.

42. Consecutive permission requests are handled one at a time in action order. Resolving one request allows the next eligible action to proceed or present its own request.

43. Permission requests use semantic colors and styles from the active theme. No behavior, selection state, or status is communicated by color alone.
