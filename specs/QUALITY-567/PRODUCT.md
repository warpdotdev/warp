# PRODUCT — Orchestration Pill Bar

## Summary

When a user is working with an orchestrator agent that has spawned one or more child agents, Warp shows a horizontal "pill bar" above the agent view header listing the orchestrator and each child. Clicking a pill switches the active pane in place to that agent's conversation. When viewing a child agent, the pane title is replaced with a `[Parent] › [Child]` breadcrumb path so the user can navigate back to the orchestrator from the same pane.

## Figma

Figma: https://www.figma.com/design/AsF5uAM6L5tUmc11vm9YSi (nodes `4073-19833`, `4073-17179`)

## Goals

- Make the orchestrator → child relationship discoverable from the pane header without opening a separate panel.
- Let the user move between an orchestrator and its children inside a single pane (no implicit splits, no new tabs).
- Keep the existing single-conversation agent view unchanged when no orchestration is in play.

## Non-goals (V1)

- Hover preview popover on a pill (deferred).
- Pin / unpin a child to keep its pane open as a split (deferred).
- 3-dot menu on a pill (Open in new pane / Open in new tab / Stop agent / Kill agent) — deferred.
- Drag-to-reorder pills.
- Any change to non-orchestration conversations.

## Behavior

1. The pill bar only appears in the **fullscreen agent view** (`AgentView` flag on, `agent_view_controller.is_fullscreen()`), and only when the new `OrchestrationPillBar` flag is enabled.

2. The pill bar is shown only when the **active conversation is the orchestrator** — i.e. the conversation that has child agents underneath it. When the user is viewing a child agent, the pill bar is replaced by breadcrumbs in the title (see (10)–(14)). When there is no orchestration relationship at all, no pill bar is shown and the pane header renders exactly as before.

3. The pill bar is hidden when the orchestrator has zero children. It only appears once at least one child agent has been spawned.

4. Pill ordering is stable: the orchestrator is always the leftmost pill, followed by child pills in the order the orchestrator registered them (i.e. the order in which they were spawned). Sorting by the first exchange's start time is intentionally avoided because a child whose first exchange has not started yet would otherwise sort to the front and pop into a different position once it began streaming, reshuffling the bar. Pills do **not** reshuffle as their statuses update.

5. Each pill is a horizontal stadium-shaped chip containing:
   - A circular avatar (16×16) on the left.
   - A label on the right (truncated with an ellipsis past ~110px).
   - Internal padding: 4px left of the avatar, 10px right of the label, 6px between avatar and label.
   - Pills are 22px tall with a half-stadium corner radius (radius = height/2). Adjacent pills are spaced 6px apart.

6. The orchestrator pill uses the Warp `Oz` glyph on a cyan disc and is labelled with the orchestrator conversation's agent name, falling back to `"Orchestrator"` if no name is set.

7. Each child pill uses:
   - A colored disc whose color is deterministic from the agent's name (hash → 6-color palette of `ansi_fg_blue/magenta/cyan/green/yellow/red`).
   - The first letter of the agent's name (uppercase), in bold, on top of the disc.
   - The agent's name as the label, falling back to `"Agent"` if unset.

Note this is temporary - we'll update this further later.

8. Pill states:
   - **Selected** (the pill matches the active conversation): solid foreground background + inverted text color, label rendered in semibold. Cursor is the default arrow. Clicks are no-ops.
   - **Hover / active click** (any non-selected pill): a slightly brighter neutral background; cursor becomes the pointing hand.
   - **Idle** (non-selected, not hovered): the standard neutral pill background.

9. Clicking a non-selected pill switches the **current pane** to that pill's conversation in place. It does not split the pane or open a new tab. The newly active pill becomes Selected on the next render. After a click on a child pill, the pane header switches from showing the pill bar to showing breadcrumbs (see (10)).

10. While viewing a child agent (the active conversation has a parent), the pane header title area is replaced with a `[Parent] › [Child]` breadcrumb path:
    - Each crumb is a 24px-tall capsule with a 4px corner radius, 6px horizontal padding, and the same avatar treatment as pills (orchestrator uses Oz glyph + cyan disc; child uses deterministic-color disc + initial letter).
    - The separator between crumbs is a `›` chevron icon (16×16) in the standard sub-text color.
    - The parent crumb's label is the parent conversation's title, falling back to its agent name, and finally to `"Orchestrator"`.
    - The trailing (child) crumb is rendered with the brighter "main" text color, no hover, no click.

11. The parent crumb is interactive:
    - Hover: applies a neutral hover background and switches to brighter "main" text color; cursor becomes pointing hand.
    - Click: navigates the current pane back to the orchestrator. The pane header then switches from breadcrumbs back to showing the pill bar (with the orchestrator pill now Selected).

12. Hover state for both pills and the parent crumb persists across renders. Re-renders triggered by status updates, new exchanges, etc. must not zero out hover state mid-interaction.

13. Long agent names truncate with an ellipsis:
    - Pill label: max 110px.
    - Crumb label: max 220px.

14. The pill bar's vertical placement does **not** change the pane header title's vertical centering. The pane title and any header buttons (e.g. `ESC for terminal`) remain visually centered within the standard pane header height; the pill bar appears as a separate row below.

15. When the `OrchestrationPillBar` flag is off, none of the above renders. Existing behavior — including the parent-conversation navigation card from the prior orchestration UI — is preserved exactly.

16. The bar redraws when any of the following change for the orchestrator or its children: conversation status, new exchanges, the active conversation, conversation creation, conversation removal/deletion, or entering/exiting the agent view.

17. When entering or exiting the fullscreen agent view, hover state for all pills resets so a stale hover doesn't persist into the next view.
