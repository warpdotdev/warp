# Inline Create-API-Key Flow on Orchestration Cards

Linear: [QUALITY-702](https://linear.app/warpdotdev/issue/QUALITY-702)

## 1. Summary

When an orchestration card asks the user to start additional child agents under a non-Oz harness (Claude Code, Codex, etc.) and the user has no managed API key for that harness yet, the card now lets the user create one without leaving the conversation. A workspace-level modal hosts the same create-key form used by cloud-mode FTUX, scoped to the card's current harness. The card's API-key picker also gains a permanent "+ New API key…" entry users can click any time. Until the user makes an explicit choice — either picking a managed key or clicking "Inherit key from environment" — the Accept button is disabled with a tooltip explaining why.

## 2. Problem

Orchestration cards have always exposed an "API key" picker for non-Oz harnesses, but the picker assumed at least one managed key already existed for the active harness. When it didn't, the dropdown was effectively empty and there was no in-card path to create one. Users had to drop out of the conversation, find the cloud-mode FTUX, create a key, then come back and re-trigger the card. Worse, Accept would silently dispatch with whatever was inherited from the worker environment, which usually wasn't what the user wanted and often failed downstream. This made the very first orchestration attempt under a new harness a dead end for many users.

## 3. Goals

- Let users create a managed API key directly from any orchestration card whose active harness needs one.
- Give users a permanent, discoverable affordance to add a new key even after they already have some.
- Block Accept until the user has either picked a managed key or explicitly chosen to inherit, with a clear, in-context reason.
- Auto-prompt for key creation exactly once per harness/execution-mode combination so the first-run experience is opinionated without becoming naggy.
- Match the cloud-mode create-key UX exactly so users see one consistent form regardless of where they invoke it.

## 4. Non-goals

- No changes to how managed keys are stored, encrypted, or transmitted to harness processes.
- No new key types or harness integrations beyond what cloud mode already supports.
- No changes to the cloud-mode (single-agent) FTUX user experience.
- No changes to Oz, which has no concept of per-harness API keys.
- No per-conversation or per-plan key overrides — keys remain user-scoped and persist via the same `last_selected_auth_secret` setting used by cloud mode.

## 5. User experience

### Picker contents

The auth-secret picker on both the `RunAgents` confirmation card and the plan card's orchestration config block now contains, in this order:

1. **Inherit key from environment** — always present. Selecting it records an explicit "inherit" choice; child agents will pick up credentials from the worker's shell environment.
2. **Each managed key** the user has for the active harness, in the order the server returns them.
3. **+ New API key…** — present for any harness that supports at least one managed-secret type. Selecting it opens the workspace create-key modal scoped to that harness.

While the harness's key list is still loading, the picker shows a single disabled "Loading…" entry alongside Inherit. If the fetch fails it shows "Unable to load secrets" instead.

### Picker trigger label

The label on the closed picker reflects the user's current selection:

- A managed key by name when one has been picked.
- "Inherit key from environment" when the user explicitly chose to inherit.
- "+ New API key…" when the user has made no choice yet and the harness supports managed secrets. (Falls back to the inherit label for harnesses with no managed-secret types.)

The label always renders in the dropdown's default text color — no greyed-out placeholder treatment.

### Auto-open of the create-key modal

The first time a card renders for a non-Oz harness whose managed-key list is loaded and empty, the workspace pops the create-key modal automatically. This happens at most once per card per harness/execution-mode combination. Cancelling or skipping the modal leaves the picker on "+ New API key…" and the Accept gate firing; switching harness or toggling Local/Cloud resets the one-shot so the new harness gets its own fresh prompt.

The auto-open is suppressed for cards that are not in an interactive confirmation state: cards that are denied, already auto-launching, currently spawning, restored from history, or whose action is already finished or running async. The auto-open also waits for the secrets list to actually resolve to "loaded and empty" — it does not fire while the list is in flight, has not been fetched, or failed.

### Accept gate

The Accept button is disabled when the user has not yet made an auth-secret choice (the picker shows "+ New API key…"). The button's tooltip explains why, e.g. "Pick an API key or choose to inherit from the environment before accepting." Picking either a managed key or explicitly choosing Inherit immediately re-enables Accept.

### Create-key modal

The modal is workspace-owned and blocks the rest of the UI while open. Internally it hosts the same `AuthSecretFtuxView` component used by cloud-mode FTUX, parameterized with the card's current harness. The modal lets the user:

- Choose a key type (when the harness has more than one).
- Enter the key value and a display name.
- Submit, cancel, or skip (skip is hidden in this modal mode — the picker's existing "Inherit key from environment" entry plays that role).

When submission succeeds, the modal closes, the new key is persisted as the active selection for that harness via the same `last_selected_auth_secret` setting cloud mode uses, and the originating card automatically adopts the new key as its selection. The Accept gate immediately clears.

On cancel, the modal closes and the card's state is left untouched (picker stays on "+ New API key…" so the user can try again). On submission failure, the modal stays open with an inline error so the user can correct and retry.

### Harness switching

When the user changes the harness on a card, the one-shot guard resets and the new harness's selection state is re-read from persisted settings. If the new harness also has no managed keys, the modal will auto-open again — once — for that harness.

## 6. Success criteria

- A card for a non-Oz harness with zero managed keys auto-opens the create-key modal exactly once.
- Cancelling the modal does not re-pop it on the next render or notify cycle for the same card.
- The picker always shows "+ New API key…" as an actionable entry for harnesses with at least one managed-secret type.
- Selecting "+ New API key…" opens the modal regardless of whether managed keys already exist.
- Accept is disabled with an explanatory tooltip whenever the picker shows "+ New API key…".
- Successful key creation auto-selects the new key on the originating card and re-enables Accept.
- Cancelling the modal leaves the card's selection unchanged and the Accept gate still firing.
- Switching harness or toggling Local/Cloud on a card resets the one-shot auto-open guard for the new state.
- Cloud-mode (single-agent) FTUX behavior is unchanged end to end.
- Restored cards, denied cards, spawning cards, auto-launched cards, and terminal-state cards never auto-open the modal.

## 7. Validation

### Automated

- `cargo check -p warp`
- `cargo fmt`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`

### Manual

- Clear all managed Claude Code keys, then ask the agent to orchestrate with Cloud + Claude Code. Confirm the modal auto-opens once. Cancel it; confirm it does not re-pop. Click "+ New API key…" in the picker; confirm the modal re-opens.
- Create a key in the modal; confirm the picker auto-selects it and Accept enables.
- Cancel the modal; confirm the picker stays on "+ New API key…" and Accept stays disabled with a hover tooltip.
- Explicitly select "Inherit key from environment"; confirm Accept enables.
- Switch the card's harness from Claude Code to Codex (with no Codex keys present); confirm the modal auto-opens once for Codex.
- Toggle the card from Cloud to Local and back; confirm the auto-open re-arms for the new mode.
- Open a plan card with an approved orchestration config that uses a non-Oz harness with no keys; confirm the same auto-open + picker behavior on the plan card's inline config block.
- Restore a conversation containing a previously-displayed orchestration card; confirm no modal pops.
- Run the cloud-mode FTUX flow end to end; confirm it is unchanged.
