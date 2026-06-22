# Local-to-Cloud Handoff: UI Polish — Product Spec
Linear: [REMOTE-1519](https://linear.app/warpdotdev/issue/REMOTE-1519/make-ui-better-for-local-cloud-handoff)
## Summary
Polish the local-to-cloud handoff (REMOTE-1486) so that the cloud-mode pane that opens next to the local pane already shows the source conversation, and looks identical to a regular fresh cloud-mode run while the cloud agent is starting up. Today the user clicks the chip and is dropped into a blank pane that only fills in once the cloud agent's first turn streams in.
## Problem
Two related rough edges in the V0 handoff flow:
1. The new cloud-mode pane is empty between chip click and the cloud agent's first response. The user has lost their context — they have to remember what they handed off, or look at the local pane next to it.
2. The cloud-mode setup-v2 affordances (the "Running setup commands…" collapsible row that wraps the environment startup PTY output, the cloud-mode loading screen / queued-prompt indicator) work for fresh cloud-mode runs but render incorrectly during handoff. The handoff pane shows raw startup output instead of the polished setup-v2 surface.
## Goals
- The handoff pane is hydrated with the source conversation's AI exchanges immediately on chip click. The user sees the same conversation history they were just looking at, in the new pane, before they finish typing the follow-up.
- The cloud agent's shared-session replay (which rebroadcasts every exchange in the forked conversation) does not double-render content already on screen. Only genuinely new exchanges from the cloud agent appear after replay.
- The handoff pane uses the cloud-mode setup-v2 affordances during the loading phase, the same way a fresh cloud-mode run does: queued-prompt indicator, "Setting up environment" loading screen, "Running setup commands…" collapsible block wrapping the startup PTY output.
## Non-goals
- Bidirectional sync after handoff. The forked conversation diverges at chip-click; later edits in the local pane do not propagate to the cloud, and vice versa. Same posture as REMOTE-1486 V0.
- Restoring shell command blocks from the local pane into the new cloud pane. Only the conversation's AI exchanges are hydrated; terminal output that lived on the local terminal (e.g. unrelated commands run between agent turns) stays on the local pane.
- Cloud→cloud setup-v2 fixes. The cloud-cloud follow-up path (REMOTE-1290) may have similar gaps but is out of scope here; we'll only address local→cloud.
- A local "this conversation was handed off to <link>" breadcrumb on the source pane.
## Behavior
### Fork timing and hydration on chip click
1. Clicking the "Hand off to cloud" chip (or invoking `/oz-cloud-handoff`) immediately mints a server-side fork of the source conversation. The new conversation token is returned synchronously to the client.
2. The new cloud-mode pane opens next to the local pane and is pre-populated with the source conversation's AI exchanges, rendered with live (non-restored) appearance — visually indistinguishable from staying in the local pane.
3. The forked conversation appears in the user's history under their account, owned by them.
4. Subsequent edits in the local pane after chip click do **not** appear in the handoff pane. The cloud agent will work against the conversation as it was at chip-click time. Users who want a more recent snapshot must close the handoff pane and click the chip again.
### Eligibility and fallback
5. Per-conversation eligibility requires an active, non-empty conversation with a synced server token. When the active conversation isn't eligible, the chip surfaces an error toast in the local window and **does not open** any pane. The local conversation is unaffected and the user can retry once the source has synced.
6. If the server fork call fails for any reason (network, auth, source not synced to GCS), the new pane is **not** opened. The failure surfaces as the same error toast in the local window. The local conversation is unaffected and the user can retry by clicking the chip again.
### Cloud session replay and dedup
7. When the cloud agent's shared session connects to the handoff pane, the agent's conversation replay rebroadcasts every exchange in the forked conversation. Because we already pre-populated the same exchanges, the replay events are suppressed at the response-stream level, identical to how cloud→cloud follow-up sessions handle stale replay (REMOTE-1290).
8. After the replay completes, genuinely new exchanges (the cloud agent's first response to the user's follow-up prompt) are appended normally. The user sees a smooth transition from "frozen pre-handoff state" to "cloud agent answering my follow-up prompt".
### Setup-v2 affordances during loading
9. After the user submits, the handoff pane shows the same cloud-mode setup-v2 affordances a fresh cloud-mode run shows:
    - The submitted prompt as a queued user-query indicator (REMOTE-1454 visual treatment, no Send-now / dismiss buttons).
    - The "Setting up environment" loading screen during the pre-session phase.
    - The "Running setup commands…" collapsible row that wraps environment startup PTY output once the shared session connects.
10. When the cloud agent's first turn arrives, the queued-prompt indicator and the setup-v2 affordances tear down on the same transitions a fresh cloud-mode run uses (`AppendedExchange` for Oz, `HarnessCommandStarted` for non-Oz).
### Edge cases
11. If the user closes the handoff pane between chip click and submit, the server-side fork is orphaned (visible in the user's conversation history but never run against). V0 does not clean these up.
12. If the user clicks the chip twice on the same source conversation, two independent forks are minted — same as today's REMOTE-1486 chip behavior; nothing changes here.
13. The local pane is unaffected throughout: its conversation is not duplicated, archived, or annotated. The user can keep typing in the local pane.
## Success criteria
- Clicking the chip on a long conversation produces a fully populated handoff pane within ~300ms (network-dependent on the fork RPC), without flicker.
- The user never sees duplicate exchange blocks during the cloud agent's session connect / replay phase.
- The handoff pane's loading-phase UI is byte-for-byte identical to a fresh cloud-mode run's (modulo the pre-populated exchanges above the queued-prompt indicator).
