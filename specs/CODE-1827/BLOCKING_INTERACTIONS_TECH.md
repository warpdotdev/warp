# CODE-1827 PR 1: Centralized TUI Blocking Interactions

## Context

This behavior-preserving base PR centralizes the TUI logic that determines which interaction blocks normal input, where that interaction renders, and which child owns focus. References are pinned to warp commit `c4cc7be9477897c75c34bdc75c1a324c25b12f27`.

- [`crates/warp_tui/src/agent_block.rs (71-91)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/agent_block.rs#L71-L91) defines `TuiBlockingChild` inside transcript-block code.
- [`crates/warp_tui/src/agent_block.rs (797-838)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/agent_block.rs#L797-L838) resolves ask-question, permission, and orchestration children from the front blocked action.
- [`crates/warp_tui/src/transcript_view.rs (610-616)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/transcript_view.rs#L610-L616) scans transcript blocks for that child.
- [`crates/warp_tui/src/terminal_session_view.rs (603-642)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/terminal_session_view.rs#L603-L642) performs variant-specific focus transfer.
- [`crates/warp_tui/src/terminal_session_view.rs (1606-1641)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/terminal_session_view.rs#L1606-L1641) detects blocker changes.
- [`crates/warp_tui/src/terminal_session_view.rs (3193-3335)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/crates/warp_tui/src/terminal_session_view.rs#L3193-L3335) separately suppresses input, attachments, menus, footer, and response status.

The refactor must preserve every existing ask-question, permission, and orchestration interaction. It introduces no handoff command, card, or user-visible behavior.

## Proposed changes

Add `crates/warp_tui/src/blocking_interaction.rs` with one `TuiBlockingInteractionModel` per `TuiTerminalSessionView`.

The model owns only cross-cutting interaction projection:

- Registered action-backed interactive views keyed by `AIAgentActionId`.
- One optional session-owned interaction slot reserved for future non-action interactions.
- Active-interaction resolution from the authoritative front blocked action or session slot.
- Deterministic precedence, stable identity, focus transfer, render placement, and change notification.

Represent active interactions with an exhaustive enum for ask-question, permission, and orchestration. A placement value distinguishes:

- Transcript-owned interactions, which remain rendered by their agent block while the normal input stack is suppressed.
- Input-area interactions, which replace the normal input stack. PR 1 provides the mechanism but no production input-area interaction.

The model must not own AI action status/results or feature-specific card state. `BlocklistAIActionModel` remains authoritative for action-backed blocking.

Wire the migration:

1. Create the model after `BlocklistAIActionModel` and before transcript child views.
2. Pass its handle through `TuiTranscriptView` into `TuiAIBlock`.
3. Register ask-question, permission, and orchestration handles when their existing views are created.
4. Remove registrations with their owning action/view and prune terminal actions.
5. Subscribe to `BlocklistAIActionModel`, resolve the front blocked action, and notify only when active identity changes.
6. Remove transcript-wide active-blocker scanning.
7. Derive session focus and input suppression from one active snapshot.
8. Keep action completion, rendering, and keybindings in their existing views.

Keep visibility minimal: types are `pub(crate)` only when cross-module use requires it; otherwise prefer `pub(super)` or private helpers.

## Testing and validation

Add focused unit/render tests:

- Front blocked action selects its registered interaction.
- Non-blocked, finished, stale, and unregistered actions produce no active interaction.
- Session-owned precedence is deterministic.
- Removed registrations cannot remain active.
- Unrelated action updates do not churn focus.
- Transcript placement suppresses input without moving the transcript card.
- Input-area placement renders in the input slot.
- Input text, cursor, selection, attachments, and inline-menu state survive suppression.
- Focus transfers directly between consecutive blockers and returns after the last blocker.

Retain existing ask-question, permission, option-selector, and orchestration-card dispatch/render tests.

Run:

- `./script/format`
- Focused `cargo nextest run -p warp_tui` tests.
- The applicable `warp_tui` clippy command from `./script/presubmit`.

## Risks and mitigations

- Keep the action queue authoritative so the new model cannot drift from action status.
- Key registrations by `AIAgentActionId` and prune on both action and view removal.
- Compare active identities before notifying to avoid focus churn.
- Keep this PR strictly behavior-preserving and free of handoff-specific code.

## Parallelization

Do not parallelize implementation. Focus, registration, transcript ownership, and session rendering change together and should be reviewed as one coherent refactor.
