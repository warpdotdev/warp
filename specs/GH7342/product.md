# GH7342: Product Spec — Customizable Spinner Verbs
## 1. Summary
Let users customize the generic `Warping...` in-progress text shown while Warp Agent or Oz is working. Users can keep the default, choose a built-in themed pack, or provide a custom comma-separated list of short phrases. Warp chooses one phrase per warping session, meaning one LLM response or active response stream, and appends the standard ellipsis styling at render time.
## 2. Problem
The generic `Warping...` message is currently fixed. Users who personalize Warp's UI and workflow cannot customize this visible loading state, and natural-language requests such as "change my spinner verbs" need a single canonical setting to modify.
## 3. Goals
- Preserve the current `Warping...` behavior by default.
- Provide a Settings UI for choosing the default, a built-in pack, or a custom list.
- Provide an equivalent user-editable settings-file contract.
- Treat "spinner verbs", "warping verbs", and "flavor verbs" as the same Warp Agent/Oz preference.
- Keep tool-specific and status-specific progress text intact.
- Keep the selected phrase stable while one LLM response streams, so the shimmer animation does not reset every render.
- Normalize custom values so settings-file edits and synced values behave safely.
## 4. Non-goals
- No hidden or "suppress the spinner text" mode.
- No customization of tool-specific messages such as `Searching codebase...`, `Grepping...`, `Finding files...`, `Executing command...`, `Waiting for command to exit...`, MCP tool/resource messages, document-generation messages, summarization messages, or passive-code-diff messages.
- No customization of fallback-model labels such as `Warping with {model}.` or `Warping with another model.`
- No customization of app-launch/startup splash screens or application boot loading messages, even if another surface uses `Warping...`.
- No per-workspace, per-session, per-conversation, CLI flag, markdown, rich-text, animation, icon, color, font, or placement customization.
- No user-created named packs in the first implementation.
## 5. Behavior
1. Default behavior is unchanged. With no custom verbs configured, the generic loading state displays `Warping...`.
2. The persisted settings are `agents.warp_agent.spinner_verbs`, an explicit source mode, and `agents.warp_agent.custom_spinner_verbs`, a custom list used only when the source mode is `custom`. Both sync like comparable public Warp Agent preferences.
3. Settings exposes a `Spinner verbs` control in the Warp Agent/Oz settings area.
4. The Settings UI offers these modes:
   - `Default` — stores `spinner_verbs = "default"` and displays `Warping...`.
   - `Medieval` — stores `spinner_verbs = "medieval"`.
   - `Conspiracy` — stores `spinner_verbs = "conspiracy"`.
   - `Cooking` — stores `spinner_verbs = "cooking"`.
   - `Warpy` — stores `spinner_verbs = "warpy"`.
   - `Custom` — shows an editor for a comma-separated custom list.
5. Built-in packs are curated, read-only source-code lists. Applying a pack stores the pack identifier in `agents.warp_agent.spinner_verbs`; Warp resolves the current pack phrases from source at render time.
6. Custom values are entered as comma-separated phrases, for example `Cooking, chopping, slicing`.
7. The UI saves custom editor content on blur or Enter, not on every keystroke. While the user has unsaved edits, the UI keeps showing the Custom editor and does not overwrite the in-progress text with external setting changes. Once the user saves or selects another mode, later settings changes can resync the editor. Selecting Default or a pack preserves the saved custom list so users can return to Custom without retyping it.
8. Each generic warping session chooses one phrase from the normalized list at random. A "warping session" is one LLM response or active response stream, not a full terminal session or conversation. The phrase stays stable for that response. When alternatives exist, the next response avoids immediately repeating the previous raw phrase.
9. A warping session is keyed by the active response stream when available, falling back to the exchange key. One backend response that appends multiple exchanges should keep the same displayed phrase. A single conversation can include multiple warping sessions and may show different spinner verbs across separate responses.
10. Rendering appends `...` to a selected phrase unless it already ends in `.`, `!`, `?`, or `…`.
11. Custom values are normalized before persistence and again at the renderer boundary so direct settings-file edits and synced values cannot bypass validation:
    - Trim leading and trailing whitespace.
    - Drop empty entries.
    - Strip trailing `.` and `…` characters before display formatting.
    - Drop entries that become empty after trimming/stripping, including dots-only entries.
    - Truncate each phrase to `MAX_WARPING_VERB_CHARS` characters before the render-time ellipsis.
    - Cap the stored/displayed list at `MAX_CUSTOM_WARPING_VERBS` entries.
12. If `spinner_verbs = "custom"` and normalization produces an empty list, Warp falls back to `Warping...`.
13. The custom list applies only to the generic in-progress spinner for Warp Agent and Oz. More specific progress labels continue to take precedence.
14. Fallback-model messages continue to render as model-specific `Warping with ...` labels rather than using custom spinner verbs.
15. Settings-file changes apply without restarting Warp through the existing settings hot-reload path and normal UI re-rendering.
16. Custom phrase contents are UI preference data only. They are not sent to the AI model as prompt context. Telemetry must not include custom phrase text.
17. Long displayed text uses the same clipping behavior as the existing loading text and must not wrap or overlap adjacent controls.
## 6. Success Criteria
1. With default settings, the generic loading state still displays `Warping...`.
2. Selecting each built-in pack in Settings updates the generic loading state to use one phrase from that pack.
3. Entering a comma-separated custom list in Settings updates the generic loading state to use one normalized phrase from that list.
4. Directly editing `agents.warp_agent.spinner_verbs` and `agents.warp_agent.custom_spinner_verbs` in the settings file updates the generic loading state after hot reload.
5. Raw settings or synced values that contain blanks, dots-only entries, trailing ellipses, lowercase starts, over-long entries, or more than the max number of entries are normalized safely at the settings boundary and before display.
6. One LLM response keeps a stable displayed phrase while it streams.
7. Subsequent responses avoid immediate repeats when the list contains alternatives.
8. Tool-specific labels and fallback-model labels continue to override custom spinner verbs.
9. Custom phrase text is not included in prompt context or telemetry payloads.
10. Long custom text remains single-line and clipped.
## 7. Validation
- Manual: start an agent request with default settings and confirm the generic label remains `Warping...`.
- Manual: select each built-in pack and confirm generic loading uses one of that pack's phrases.
- Manual: enter custom phrases with spaces, punctuation, emoji, and lowercase starts; confirm the displayed text is normalized and receives a single ellipsis.
- Manual: edit `agents.warp_agent.spinner_verbs` and `agents.warp_agent.custom_spinner_verbs` while Warp is running and confirm hot reload updates the next generic loading state without restart.
- Manual: make unsaved edits in the Custom editor while changing the setting externally; confirm the in-progress editor text is not overwritten until saved or the mode changes.
- Manual: trigger codebase search, grep, file glob, command execution, command waiting, MCP calls, summarization, passive diff generation, and fallback-model routing; confirm their specific labels still override custom verbs.
- Manual: try an over-long custom phrase and confirm the row clips without wrapping or overlapping buttons.
- Unit: cover normalization, list capping, raw renderer-boundary normalization, blank fallback, per-response stability, and no-immediate-repeat selection.
## 8. Open Questions
- Should Product/Design adjust the names or contents of the built-in packs before this feature leaves dogfood?