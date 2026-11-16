# PRODUCT.md
## Summary
Surface the execution harness (Warp Agent, Claude Code, Gemini CLI) in the conversation details sidebar as its own labeled field, matching the panel's other metadata fields ("Run ID", "Credits used", "Run time", etc.).
## Figma
None provided. Visual treatment should match the panel's existing metadata fields: a small "Harness" label in the sub-text color above a value row. The value row is a small leading logo tinted with the harness's brand color followed by the harness display name.
## Behavior
1. The conversation details panel renders a "Harness" field in the sidebar whenever the harness for that conversation or task is known. The field is rendered in the same label-over-value style as sibling fields like "Run ID", "Credits used", and "Run time".
2. The label is the literal string "Harness", colored with the panel's sub-text color. The value row sits directly below the label and shows, left-to-right: a small leading harness icon, then the harness's user-visible display name in the theme foreground. Mapping:
    * Warp Agent (the `Oz` harness) → `Icon::Warp`, "Warp Agent", tinted with the theme foreground (same icon + treatment used elsewhere for first-party Warp skills).
    * Claude → `Icon::ClaudeLogo`, "Claude Code", tinted with the Claude brand orange.
    * Gemini → `Icon::GeminiLogo`, "Gemini CLI", tinted with the Gemini brand blue.
3. Third-party harness logos render with their brand color rather than the theme foreground so their visual identity is preserved (e.g. the Claude logo is orange, not white-on-dark).
4. Local (non-ambient) conversations and cloud tasks whose config does not explicitly set a harness render as "Warp Agent", matching the system default.
5. Cloud tasks whose config explicitly sets a harness render that harness, regardless of whether the run succeeded, failed, or is still in progress.
6. If the harness value is not yet known (specifically: the cloud-mode details panel for a shared ambient session, opened before the async task-data fetch resolves), the row is omitted entirely — no placeholder icon, no "Unknown" label, no reserved empty slot. Once the task payload arrives and a harness can be determined, the row appears; sibling rows above and below must not visually jump or reflow in a way that moves other content out from under the user's cursor.
7. The harness row is read-only. It has no click target, no copy button, no tooltip beyond what the icon naturally conveys, and nothing about it changes the currently selected harness for future runs.
8. The row's label text is selectable for copy, consistent with other metadata rows in the panel.
9. The harness row's position is stable across panel modes (conversation vs. task) and across repeated `set_conversation_details` calls for the same conversation; it does not reorder relative to sibling rows between renders.
10. The harness row is rendered identically across every surface that hosts the conversation details panel (management view details pane, transcript viewer side panel, shared-session views).
