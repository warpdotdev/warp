# APP-4080: Use Latest User Prompt as Conversation Title in Tab Names

## Summary

Add a user setting named `Use latest user prompt as conversation title in tab names` that controls which conversation text Warp shows in vertical tabs for agent conversations.

The setting is disabled by default. In the default disabled state, vertical tabs use conversation titles for both Oz agent conversations and plugin-backed third-party CLI agent sessions. When enabled, vertical tabs use the latest user prompt when one is available, consistently across the same agent conversation types.

The desired outcome is that Warp's default tab-name behavior uses stable, title-like conversation labels, while users who prefer prompt-based labels can explicitly opt into using the latest prompt.

## Problem

Vertical tabs are a primary navigation surface for users who keep multiple terminal and agent sessions open. Agent rows need a short piece of conversation text that helps users recognize which conversation is which.

Today, the behavior is inconsistent:

- Oz agent conversations use the conversation title when available.
- Plugin-backed third-party CLI agent sessions use the latest user prompt ahead of title-like session metadata.

That makes the same vertical-tabs surface feel unpredictable. Users cannot rely on "agent conversation text" meaning the same thing across Oz and third-party agents, and users who prefer prompt-based labels do not have a single opt-in setting that applies to both.

## Goals

- Add one user-facing setting named `Use latest user prompt as conversation title in tab names`.
- Default the setting to disabled, so tab names use conversation titles unless the user opts into prompt-based labels.
- Apply the setting consistently to Oz agent conversations and plugin-backed third-party CLI agent sessions.
- Provide a prompt-based option for users who prefer the latest user prompt as the vertical-tab label.
- Keep non-agent terminal panes and non-plugin-backed CLI agent detections on their current terminal fallback behavior.
- Make vertical-tabs row text, vertical-tabs search, and vertical-tabs detail-sidecar text agree on the same selected conversation text.

## Non-goals

- Changing how conversation titles are generated.
- Adding manual per-conversation renaming.
- Changing pane header titles, horizontal tab titles, the command palette conversation list, or the conversation list panel.
- Changing plain terminal pane title precedence.
- Adding support for third-party CLI sessions that do not expose plugin-backed session metadata.
- Redesigning the vertical tabs settings UI beyond adding this setting in the appropriate AI settings area.

## Figma / design references

Figma: none provided

Use the existing settings UI patterns for a simple toggle.

## User experience

### Setting location and default

Warp exposes a setting in the AI settings area, near settings that affect agent input or agent conversation behavior.

The setting should communicate that it opts into prompt-based agent tab names. Suggested copy:

- Setting label: `Use latest user prompt as conversation title in tab names`
- Description: `Show the latest user prompt instead of the generated conversation title for Oz and third-party agent sessions in vertical tabs.`

The setting is disabled by default for all users.

### Eligible conversations

The setting applies only to agent conversations shown as terminal rows in the vertical tabs panel:

- Oz agent conversations, including local agent conversations and cloud agent conversations shown in Warp.
- Plugin-backed third-party CLI agent sessions, such as supported Claude Code, Codex, or similar sessions where Warp receives structured session metadata from the agent plugin.

The setting does not apply to:

- Plain terminal panes.
- CLI agent sessions detected only from a running command when no plugin-backed session metadata is available.
- Non-terminal panes such as code panes, notebooks, workflows, settings, files, or Warp Drive objects.

### Disabled behavior: use conversation title

When the setting is disabled, vertical tabs should show the best available conversation title for eligible agent rows. This is the default behavior.

For Oz agent conversations:

- Show the generated conversation title when it exists and is non-empty.
- If the generated title is not yet available, show the best available title fallback for the conversation, such as the initial user prompt or an explicit default title for empty conversations.
- Empty new conversations continue to use existing placeholder copy such as `New agent conversation` or `New cloud agent`.

For plugin-backed third-party CLI agent sessions:

- Show the agent session's title-like metadata when it exists and is non-empty.
- Title-like metadata should represent a stable conversation/session title or summary, not merely the most recent user prompt.
- If no title-like metadata is available yet, fall back to the latest user prompt when available.
- If neither title-like metadata nor a user prompt is available, keep the existing terminal fallback behavior.

When a title becomes available or changes while the row is visible, the vertical tab text updates in place without requiring the user to switch tabs, reopen the panel, or restart the session.

### Enabled behavior: use latest user prompt

When the setting is enabled, vertical tabs should show the latest available user prompt for eligible agent rows.

For Oz agent conversations:

- Show the latest user-authored prompt in the active conversation when one exists.
- If there is no user prompt yet, fall back to the same placeholder or terminal fallback that the default title behavior would use for an empty conversation.

For plugin-backed third-party CLI agent sessions:

- Show the latest user prompt provided by the plugin session metadata when it exists.
- If no user prompt is available, fall back to title-like metadata if available.
- If neither prompt nor title-like metadata is available, keep the existing terminal fallback behavior.

Changing the setting updates existing vertical-tabs rows on the next settings refresh or normal UI update; users should not need to create new conversations for the setting to take effect.

### Relationship to vertical tabs display settings

The setting controls the agent "conversation text" value used by vertical tabs. It does not force that value to be the primary row text in every vertical-tabs configuration.

Specifically:

- When the vertical tabs primary info setting is configured to show command/conversation text, eligible agent rows use this setting to choose between title and latest user prompt.
- When the compact subtitle setting is configured to show command/conversation text, eligible agent row subtitles use this setting to choose between title and latest user prompt.
- The vertical-tabs detail sidecar uses this setting for the command/conversation text field of eligible agent pane sections.
- When vertical tabs are configured to prioritize working directory or branch instead of command/conversation text, the primary row may continue to show that non-conversation metadata.

The setting must not change working-directory, branch, PR, diff-stat, status, or kind-badge rendering.

### Manual tab names and title overrides

If the user has manually renamed a tab or pane in a way that creates an explicit vertical-tabs title override, that user-provided override remains higher priority than this setting.

In that case:

- The visible overridden title remains unchanged.
- The setting may still affect secondary conversation text in places that do not use the override, such as the detail sidecar's conversation text field.
- Toggling the setting must not clear or rewrite the manual name.

### Status and metadata behavior

Agent status indicators remain independent from the conversation text setting.

- Oz rows keep existing Oz status behavior.
- Plugin-backed third-party CLI agent rows keep existing agent-specific status behavior.
- Plain terminal rows keep existing terminal status and title fallback behavior.

The setting only chooses the text used to identify the conversation; it does not alter whether a row is considered an Oz row, a third-party agent row, or a terminal row.

### Empty, missing, and stale metadata

Fallback behavior should avoid blank vertical-tab labels.

- Empty strings and whitespace-only titles or prompts are treated as missing.
- If the preferred text type is missing, Warp falls back to the other text type when available.
- If both title-like metadata and prompt metadata are missing, Warp uses the existing terminal fallback chain.
- A stale previous prompt must not replace a newer conversation title when the setting is disabled and the newer title is available.
- A stale title must not replace a newer user prompt when the setting is enabled and the newer prompt is available.

### Search behavior

Vertical-tabs search should include the same conversation text that is visibly rendered for eligible agent rows.

For discoverability, search may also include the non-rendered counterpart when available. For example, when the setting is disabled and the title is visible, searching for the latest user prompt may still match the row. However, the visible row text after filtering should continue to follow the setting.

### Scope across app sessions and devices

The setting is a user preference and should persist across app restarts.

If Warp normally syncs comparable AI display preferences across devices, this setting should follow that same sync behavior. If comparable AI display preferences are local-only, this setting should follow that local-only behavior instead.

## Success criteria

1. A user can find a setting in the AI settings area named `Use latest user prompt as conversation title in tab names`.
2. The setting is disabled by default.
3. With the setting disabled, an Oz agent conversation row in vertical tabs shows the generated conversation title when one exists.
4. With the setting disabled, a plugin-backed third-party CLI agent session row in vertical tabs shows title-like session metadata when one exists, instead of preferring the latest user prompt.
5. With the setting enabled, an Oz agent conversation row in vertical tabs shows the latest user-authored prompt when one exists.
6. With the setting enabled, a plugin-backed third-party CLI agent session row in vertical tabs shows the latest user prompt when one exists.
7. Oz and plugin-backed third-party CLI agent sessions follow the same setting semantics: disabled prefers title, enabled prefers latest user prompt.
8. Empty or whitespace-only titles and prompts are ignored, and rows fall back to non-empty text rather than rendering blank labels.
9. Empty Oz conversations continue to show existing placeholder copy such as `New agent conversation` or `New cloud agent`.
10. Third-party CLI agent sessions without plugin-backed metadata keep the existing terminal fallback behavior.
11. Plain terminal rows keep the existing terminal title / last command / new session fallback behavior.
12. Vertical-tabs primary row text, compact command/conversation subtitles, and detail-sidecar command/conversation text all use the same setting-driven conversation text for eligible agent rows.
13. Vertical-tabs search matches the visible setting-driven conversation text for eligible agent rows.
14. Toggling the setting updates existing eligible rows without creating new conversations.
15. Manual tab or pane title overrides remain higher priority than the setting and are not cleared by toggling it.
16. Agent status pills, kind badges, PR badges, diff-stat badges, branch text, and working-directory text are unchanged by the setting.
17. The setting persists across app restarts.

## Validation

- **Default setting state**: Install or reset settings, open the AI settings area, and verify `Use latest user prompt as conversation title in tab names` is disabled by default.
- **Oz title default**: Create an Oz conversation that receives a generated title. With vertical tabs enabled and the setting disabled, verify the row shows the generated title rather than the latest prompt text.
- **Oz prompt enabled**: Enable the setting, send a follow-up prompt in the same Oz conversation, and verify the vertical-tabs conversation text updates to the latest user prompt.
- **Oz empty conversation**: Start a new empty local Oz conversation and a new empty cloud agent conversation. Verify placeholder copy remains non-empty and appropriate.
- **Third-party title default**: Start a plugin-backed third-party CLI agent session with title-like session metadata and a user prompt. With the setting disabled, verify the vertical-tab row shows the title-like metadata rather than the prompt.
- **Third-party prompt enabled**: Enable the setting in the same plugin-backed session and verify the row shows the latest user prompt.
- **Missing metadata fallback**: Test a plugin-backed third-party session before title-like metadata arrives. Verify the row falls back to prompt text when available and never renders blank text.
- **No-plugin fallback**: Start a CLI agent session detected only from the running command without plugin metadata. Verify the row keeps the existing terminal fallback behavior regardless of the setting.
- **Compact subtitle**: Configure compact vertical tabs so additional metadata shows command/conversation text. Toggle the setting and verify the subtitle changes consistently for Oz and plugin-backed third-party agent rows.
- **Detail sidecar**: Hover an eligible Oz row and an eligible third-party agent row. Verify the sidecar's command/conversation text follows the setting.
- **Search**: Search vertical tabs for the visible title/prompt text and verify the correct eligible agent row matches.
- **Manual override**: Rename a tab or pane, toggle the setting, and verify the manual title remains visible and unchanged.
- **Regression / plain terminal**: Open a plain terminal pane and verify its vertical-tab label precedence remains unchanged.
- **Persistence**: Toggle the setting, restart Warp, and verify the setting value and vertical-tabs behavior are preserved.

## Open questions

- What exact storage/sync policy should this setting use relative to existing AI display settings?
- What exact title-like field should third-party CLI agent plugins expose or map into if they currently only provide prompt and summary metadata?
