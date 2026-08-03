# PRODUCT: TUI API-key management menu (CODE-1930)

Linear: [CODE-1930 — TUI: replace BYOK slash commands with a /api-keys inline menu](https://linear.app/warpdotdev/issue/CODE-1930/tui-replace-byok-slash-commands-with-an-api-keys-inline-menu)

## Summary

The Warp TUI provides one `/api-keys` slash command that opens an inline menu for viewing, filtering, setting, replacing, and clearing supported AI-provider credentials. The same menu also exposes the TUI-local Warp credit fallback setting and embeds the existing browser-based X premium/SuperGrok connection flow.

## Figma

- Provider list: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=1633-18526&m=dev
- Standard API-key entry: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=1637-18870&m=dev
- X premium/SuperGrok connection: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=1637-19095&m=dev

The Figma provider-list frame is not authoritative where it leaves `/api-keys` in the input or shows the normal status footer. Opening the menu clears `/api-keys`, and the interaction-specific keybinding footer replaces the normal status footer.

## Goals

- Replace the two provider-key slash commands with one discoverable management surface.
- Let users inspect connection state and manage every TUI-supported provider without running a nested TUI command.
- Preserve the existing public command-line flags and Grok authorization behavior.
- Reuse the existing Warp credit fallback setting across GUI and TUI code while retaining each frontend's independent persisted value.
- Prevent a displayed or preloaded API key from being copied or rendered as plaintext.

## Non-goals

- Sharing provider credentials between the GUI and TUI secure-storage namespaces.
- Synchronizing the Warp credit fallback value between GUI and TUI settings files.
- Changing which providers support pasted keys, changing provider request routing, or validating provider keys with a new network request.
- Changing the Grok OAuth protocol, token refresh behavior, callback port, browser authorization page, or server contract.
- Adding custom endpoint or OpenRouter management to the TUI menu.
- Removing or changing `--set-provider-api-key` and `--clear-provider-api-key`.

## Behavior

### Slash-command and command-line entry points

1. The TUI slash-command menu contains one TUI-only command:
   - Name: `/api-keys`
   - Description: `View and manage API keys`

2. Selecting or submitting `/api-keys` clears the entire input buffer, including `/api-keys`, and opens the API-key menu above an empty focused input.

3. `/add-api-key` and `/clear-provider-api-key` are no longer present in the slash-command menu and are not accepted as TUI slash commands.

4. The public `--set-provider-api-key` and `--clear-provider-api-key` flags remain supported with their existing provider parsing, masked TTY input, piped-input support, secure-storage behavior, success output, and failure exit behavior.

5. Passing Grok to either public command-line flag continues to reject the operation because Grok requires an active browser flow. The error directs the user to manage Grok through `/api-keys`.

6. Opening `/api-keys` does not run a shell command, add shell history, cancel an agent response, interrupt a foreground terminal command, or start a new conversation. Credential changes apply to subsequent eligible requests.

### Main menu

7. The open main menu renders the heading `API keys` followed by these rows in order:
   1. `Anthropic API key`
   2. `Google API key`
   3. `OpenAI API key`
   4. `X premium or SuperGrok subscription`
   5. `Warp credit fallback`

8. Each provider row includes its current state:
   - A stored pasted key or usable Grok access token displays `(Connected)`.
   - No stored credential displays `(Not connected)`.

9. Provider connection state is read from the active TUI credential store. If another TUI process changes a pasted key through a public command-line flag while the menu is open, the row refreshes after the existing cross-process credential notification is received.

10. The highlighted row uses the active theme's inline-menu selection treatment. All parenthetical row states—including `(Connected)`, `(Not connected)`, `(on)`, and `(off)`—use the exact state-suffix styling already used for `(currently on)` and `(currently off)` on the TUI natural-language-detection slash-command row, in both selected and unselected states. No API-key-specific status colors are introduced.

11. Up and Down move the highlighted row. The shared inline-menu mouse behavior may also highlight, activate, and scroll rows.

12. While a provider row is highlighted, the interaction footer replaces the normal TUI status footer and displays:
   - `enter to set api key`
   - `ctrl + x to clear api key` only when the highlighted provider is connected.
   - `esc to close menu`

13. Ctrl-X immediately clears the highlighted provider's stored credential without a confirmation dialog:
   - For OpenAI, Anthropic, and Google, it clears the pasted key.
   - For X premium/SuperGrok, it disconnects the stored OAuth tokens.
   - The menu remains open, the row updates to `(Not connected)`, and focus returns to the main menu input.
   - The cleared provider remains highlighted.
   - Clearing an already-disconnected provider is an idempotent no-op that leaves the menu open.

14. Escape from the main menu closes the menu, clears its filter, and returns focus to the normal empty TUI input.

### Filtering

15. Typing while the main menu is open updates a case-insensitive provider-name filter. The typed text is a menu query and is never submitted as an agent prompt.

16. Filtering applies only to the four provider rows. The `Warp credit fallback` row remains pinned below the filtered provider results.

17. Clearing the input restores all four provider rows.

18. If no provider matches, the menu still shows the pinned Warp credit fallback row; it does not render an empty provider placeholder.

19. Filtering preserves the highlighted provider when it remains visible. If it disappears, the nearest visible selectable row becomes highlighted.

### Warp credit fallback

20. The Warp credit fallback row displays the current TUI-local setting value as `(on)` or `(off)` and the description:
   `in the event of an error, requests may be routed to use Warp credits. Warp will prioritize using your API keys over Warp credits.`

21. The setting uses the same shared setting definition, default, request behavior, and persistence infrastructure as the GUI setting, but GUI and TUI persist independent values in their normal frontend-specific settings files.

22. When the Warp credit fallback row is highlighted, the interaction footer displays only:
   - `enter to toggle warp credit fallback`
   - `esc to close menu`

23. Enter toggles and persists the TUI-local fallback value. The menu remains open and the row updates in place.

24. If persistence fails, the previous value remains authoritative, the menu stays open, and a concise error is shown without exposing settings-file contents.

25. Ctrl-X has no action while the fallback row is highlighted.

### Standard provider key entry

26. Pressing Enter on OpenAI, Anthropic, or Google transitions from the main list to that provider's key-entry state:
   - The filter query is cleared.
   - The provider title is shown above the input.
   - The input border changes to the active theme's lilac/purple credential-entry accent.
   - Input focus remains in the credential field.

27. If the provider is connected, its existing key is loaded into the credential field. If it is disconnected, the field starts empty.

28. Every character in the credential field is rendered as a mask glyph. The plaintext key is never painted into the terminal buffer, status footer, menu rows, error text, logs, or telemetry.

29. The credential field supports normal character entry, deletion, cursor movement, selection, and paste. Selection highlights only mask glyphs.

30. Copy and Cut from the credential field do not write the underlying key to the clipboard. Mouse-based terminal selection can expose only the rendered mask glyphs, not the key.

31. The provider-entry footer replaces the normal status footer and displays:
   - `Connect <provider> API key`
   - `enter to save key`
   - `esc to cancel`

32. Enter with a non-empty field persists that exact key for the selected provider. On success:
   - The entry state closes.
   - The main `/api-keys` menu returns with an empty filter.
   - The provider row displays `(Connected)`.
   - No plaintext key is included in confirmation copy.

33. Enter with an empty field clears the selected provider's key. On success, the main menu returns and the provider row displays `(Not connected)`.

34. A secure-storage failure does not change the in-memory connection state. The user remains in the provider-entry state with their masked draft intact and sees a concise retryable error that does not contain key material.

35. Escape discards edits, does not change the stored key, clears the credential editor, and returns to the main menu with an empty filter.

### X premium/SuperGrok connection

36. Pressing Enter on a disconnected X premium/SuperGrok row starts the existing Grok authorization attempt immediately:
   - The filter query is cleared.
   - The system browser opens the existing authorization URL.
   - The menu remains visible.
   - The Grok row is emphasized and displays `(Connecting...)`.
   - The input border uses the same lilac/purple credential-entry accent.
   - The input accepts an authorization code if the browser displays one.

37. The Grok-entry footer displays:
   - `Connect X premium/SuperGrok`
   - `enter to confirm`
   - `esc to cancel`

38. If the browser callback completes without manual input, the tokens are stored through the existing Grok credential path, the main menu returns automatically, and the Grok row displays `(Connected)`.

39. If the browser displays an authorization code, the user may enter or paste it and press Enter. The code is not masked. While it is exchanged, the row remains in its connecting state and duplicate submissions are ignored.

40. Enter with an empty authorization-code field shows the existing guidance to enter the code and keeps the connection attempt active.

41. A failed manual code exchange returns to code entry with the existing retryable error. A callback failure for which manual entry can no longer succeed shows the existing fatal connection error. Errors are rendered within the inline interaction rather than in the deleted Grok card.

42. The first successful result from either callback or manual exchange wins. Later results from the same or an obsolete attempt are ignored.

43. Escape cancels the active callback listener and manual exchange, clears the code field, and returns to the main menu without changing stored Grok credentials.

44. Selecting an already-connected Grok row preserves the existing behavior: no second authorization starts, and the menu explains that Grok is already connected and can be disconnected with Ctrl-X.

45. Existing Grok eligibility rules remain unchanged. If the build, workspace, or organization does not permit Grok credentials, authorization does not start, the main menu remains open, and the existing policy error is shown.

46. A failure to start the OAuth attempt, including failure to bind the callback listener, leaves the main menu open and shows the existing sanitized error.

### Interaction lifecycle and safety

47. Save, clear, fallback toggle, Grok completion, Grok cancellation, and provider-entry cancellation all return to the main `/api-keys` menu. Only Escape from the main menu closes the overall interaction.

48. Only one API-key interaction state is active at a time. Provider filtering, standard-key editing, and Grok code entry never interpret the same input simultaneously.

49. Losing ownership of the TUI input or closing the containing session cancels an active Grok attempt, clears any credential/code editor, and prevents late asynchronous results from reopening or mutating the closed menu.

50. No user-provided key, Grok authorization code, access token, or refresh token is recorded in slash-command telemetry or error reporting.
