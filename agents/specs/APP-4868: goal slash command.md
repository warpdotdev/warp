# Spec: `/goal` bundled slash command (standing completion-condition + per-turn verifier loop)

Linear: [APP-4868](https://linear.app/warpdotdev/issue/APP-4868/add-a-goal-bundled-slash-command-standing-completion-condition-per) · Target repo: `warpdotdev/warp` (the Warp **client**, not `warp-server`) · Estimate: **L** · Pinned at `warpdotdev/warp` HEAD `f1547fef`

Originating thread: https://warpdev.slack.com/archives/C0BCE7AELJ2/p1784212702783289

---

== PRODUCT ==

*Summary:* Add a `/goal <condition>` bundled slash command to the Warp GUI client that installs a **standing, measurable completion condition** on the active **local-agent** conversation. After every worker turn, an **independent, tool-less judge model** grades the transcript against the condition and returns `yes` / `no` / `impossible` plus a reason. On `no`, the reason is fed back as next-turn guidance and the worker auto-continues; on `yes` the goal auto-clears; on `impossible` (or when the N-turn spend cap is reached) the goal clears and the user is notified with the reason. An active-goal indicator shows the condition and turn progress and offers a manual clear. **v1 is local-agent-only and client-only** (no `warp-server` changes); cloud-agent support is explicitly out of scope.

*Key design choices:*
1. **v1 scope = local-agent-only / client-only.** `/goal` is gated to local Agent Mode (availability excludes cloud agents, like `/compact`). Cloud-agent turns execute server-side, so verifying them needs a `warp-server` turn-boundary/verifier hook — a cross-repo follow-up, not v1.
2. **`/goal <condition>` is an immediate local action, not a prompt.** It sets an `ActiveGoal` on the current conversation and shows the indicator. It is *not* submitted-as-prompt and *not* a `SlashCommandRequest` — the condition is standing state, not a worker prompt. `/goal` with no argument clears any active goal.
3. **The judge runs client-side as a separate, tool-less MAA request**, mirroring the existing out-of-band read-only passive-suggestion request pattern (`build_passive_suggestions_request_params`). It grades only what the worker surfaced in the transcript (no tools). Default judge model is a small/fast model, configurable via AI settings; falls back to the conversation's active model if unset/unresolvable.
4. **"No → continue" reuses the existing `AIAgentInput::UserQuery` pipeline** (no new server-side input variant), so v1 stays client-only. The judge's reason becomes a continuation user query. The loop **auto-advances turns but does not bypass auto-approve / command-approval gates** — the next turn runs under whatever fast-forward setting the conversation has.

*Behavior* (numbered, testable invariants from the user's view):
1. `/goal <condition>` appears as a bundled slash command in the GUI Agent View when AI is enabled and the `GoalSlashCommand` flag is on, and is hidden from cloud-agent conversations and from the TUI.
2. Running `/goal <condition>` sets (or replaces) the active goal on the current conversation and displays a persistent active-goal indicator showing the condition and turn progress (e.g. `Goal: <condition> · turn N/cap`), with a clear (`×`) control. No worker prompt is submitted by `/goal` itself.
3. Running `/goal` with no argument (or clicking the indicator's clear control) clears any active goal and hides the indicator.
4. After each successfully-completed worker turn while a goal is active, the judge evaluates the transcript against the condition and returns exactly one of `yes` / `no` / `impossible` plus a reason.
5. On `yes`: the goal auto-clears, the indicator hides, and a brief success notice is shown; the loop stops.
6. On `no`: the turn counter increments; if under the cap, the judge's reason is injected as a continuation prompt and the worker auto-continues (a new turn begins); the indicator's turn progress updates.
7. On `impossible`: the goal clears, the indicator hides, and the user is notified with the judge's reason; the loop stops.
8. When the turn counter reaches the N-turn cap (default 20, configurable): the goal clears, the indicator hides, and the user is notified that the spend cap was reached (not as success); the loop stops.
9. A user-initiated stop/cancel of the in-flight turn halts auto-continue for that boundary (respects user intent); the goal stays set and the loop resumes after the next user-driven turn completes.
10. The judge-driven auto-continue does not bypass auto-approve: commands the worker proposes on a continuation turn are subject to the conversation's existing fast-forward/approval gates.
11. The active goal is scoped to its conversation and is in-memory only in v1 — it does not persist across app restarts. Switching conversations does not carry the goal to another conversation.
12. Setting/clearing a goal, judge verdicts, and cap/impossible outcomes emit telemetry events.

== TECH ==

*Context (how the area works today, commit-pinned at `f1547fef`):*
- Bundled slash commands are thin `StaticCommand` metadata in `warpdotdev/warp/app/src/search/slash_command_menu/static_commands/commands.rs`; `all_commands()` (`commands.rs:625`) returns the vec, gated by `FeatureFlag::*.is_enabled()`. `/compact` is the closest analog with real downstream behavior (`commands.rs:393`).
- Execution branches in `Input::execute_slash_command` (`warpdotdev/warp/app/src/terminal/input/slash_commands/mod.rs:485`): either an immediate local action, or submit-as-prompt. Only `/compact`, `/plan`, `/orchestrate` are submitted-as-prompt (`slash_command_is_submitted_as_prompt`, `mod.rs:1553`); `/plan` and `/orchestrate` become a `UserQuery` with `UserQueryMode::Plan|Orchestrate` via `extract_user_query_mode` (`warpdotdev/warp/app/src/ai/agent/mod.rs:2617`). Every other command is an immediate action.
- `SlashCommandRequest` (`warpdotdev/warp/app/src/ai/blocklist/controller/slash_command.rs:25`) is the enum for slash commands handled through the AI query flow (`/compact` → `Summarize` → `AIAgentInput::SummarizeConversation`); sent via `BlocklistAIController::send_queued_slash_command_request` (`warpdotdev/warp/app/src/ai/blocklist/controller.rs:1423`).
- `AIAgentInput` (`warpdotdev/warp/app/src/ai/agent/mod.rs:2676`); the normal prompt path is `AIAgentInput::UserQuery { query, context, static_query_type, referenced_attachments, user_query_mode, running_command, intended_agent }` (`mod.rs:2678`).
- GUI Agent Mode turn completion: `ResponseStream::on_response_stream_complete` (`warpdotdev/warp/app/src/ai/blocklist/controller/response_stream.rs:397` / `:420` / `:344`) emits `ResponseStreamEvent::AfterStreamFinished` (`response_stream.rs:375`); the controller handles it in `handle_response_stream_event` (`warpdotdev/warp/app/src/ai/blocklist/controller.rs:2817`). `PassiveSuggestionTrigger::AgentResponseCompleted` (`controller.rs:2185`) already fires on response completion, and `build_passive_suggestions_request_params` (`controller.rs:2150`) is the existing out-of-band, read-only, separate-model request pattern to mirror for the judge.
- The CLI/ambient driver's only post-turn code today is conversation persistence, `SavePoint::PostTurn` (`warpdotdev/warp/app/src/ai/agent_sdk/driver.rs:3487`) — not verification. No `GoalLoop` / `ActiveGoal` / verifier mechanism exists anywhere in the client (confirmed by triage).
- `FeatureFlag` enum lives in `warpdotdev/warp/crates/warp_features/src/lib.rs:8`; rollout lists are `DOGFOOD_FLAGS` (`lib.rs:939`), `PREVIEW_FLAGS` (`lib.rs:998`), `RELEASE_FLAGS` (`lib.rs:1003`). The repo has an `add-feature-flag` skill (`.agents/skills/add-feature-flag/SKILL.md`) to follow. `/compact` is gated by `SummarizationConversationCommand` (`lib.rs:471`); `/queue` by `QueueSlashCommand` (`lib.rs:713`).
- TUI supports a subset via `TuiSlashCommand` (`mod.rs:132`); unsupported commands return `None` from `from_static_command` and stay hidden from the TUI.
- Validation gate: `warpdotdev/warp/script/presubmit` (fmt + clippy `-D warnings` + nextest). UI guidelines skill: `.agents/skills/gui-ui-guidelines/SKILL.md`.

*Design alternatives* (per decision point with more than one reasonable approach):

- **v1 scope: local-only vs. include cloud agents** — *local-only (chosen).* Cloud-agent turns execute server-side; verifying them needs a `warp-server` turn-boundary/verifier hook (cross-repo, larger blast radius, server API/contract design). Local-only keeps v1 to one repo and one execution model. *Alternative rejected:* ship both and require a server hook — higher risk and blocks the client feature on a server change. Cloud support is a tracked follow-up.

- **How `/goal` executes: immediate local action vs. `SlashCommandRequest` vs. submit-as-prompt** — *immediate local action (chosen).* The goal is standing conversation state, not a worker prompt; setting it should not consume a turn or appear as a user query. *Alternative A rejected:* route through `SlashCommandRequest::SetGoal` like `/compact`'s `Summarize` — but `SetGoal` is a state mutation, not an `AIAgentInput` the worker acts on, so forcing it through the AI-query flow is the wrong abstraction. *Alternative B rejected:* submit-as-prompt (like `/plan`) — would send the condition as a worker prompt, conflating "set a standing condition" with "issue a task."

- **"No → continue" mechanism: reuse `UserQuery` vs. a dedicated hidden input** — *reuse `AIAgentInput::UserQuery` (chosen).* It reuses the entire existing turn pipeline with no server-side changes, keeping v1 client-only. The judge's reason becomes the continuation `query`. *Alternative rejected:* a new `AIAgentInput::GoalFollowUp { guidance }` rendered as a hidden/system turn — cleaner transcript semantics, but a new `AIAgentInput` variant requires `warp-server` API/convert support (`warpdotdev/warp/app/src/ai/agent/api/convert_to.rs`), crossing repos and breaking local-only scope. Tracked as a future enhancement; for v1 the continuation is displayed as a (badged) user turn so the user can see what guidance was injected.

- **Where the judge runs: client-side separate MAA request vs. server-side verifier** — *client-side (chosen).* Mirrors `build_passive_suggestions_request_params` (`controller.rs:2150`): an out-of-band, read-only, tool-less request with a small/fast model, driven entirely by the client. The server need not know it is a "judge" — it is just a grading prompt over the transcript + condition. *Alternative rejected:* a server-side verifier hook — only needed for cloud agents, out of v1 scope.

- **Judge model default: small/fast configurable vs. active model** — *small/fast configurable with active-model fallback (chosen).* Judging every turn with the (possibly large) active model is costly; a small/fast default keeps the loop cheap. Add a `goal_judge_model: Option<ModelId>` AI setting defaulting to the fastest model available in the user's plan; fall back to the conversation's active model when unset or unresolvable. *Alternative rejected:* always use the active model — simplest but expensive and against the "small/fast judge" intent.

- **Spend cap unit: turns vs. tokens/cost** — *turns (chosen).* A turn cap (default 20, configurable via `goal_max_turns`) is simple to reason about, deterministic in tests, and maps directly to "after every turn." *Alternative rejected:* a token/cost cap — more economic but non-deterministic and harder to test; can be added later.

- **Goal lifecycle: in-memory vs. persisted across restarts** — *in-memory, conversation-scoped (chosen for v1).* Avoids persistence-schema changes (`warpdotdev/warp/app/src/ai/blocklist/persistence.rs`) and keeps v1 scoped. *Alternative rejected:* persist `ActiveGoal` with the conversation — useful but adds serialization/migration work; tracked as a v1 limitation / follow-up.

- **TUI support: GUI-only vs. GUI+TUI** — *GUI-only for v1 (chosen).* The TUI would need a `TuiSlashCommand::Goal` variant + handler + indicator rendering. `/goal` simply is not added to `TuiSlashCommand` so it stays hidden from the TUI. *Alternative rejected:* ship TUI too — extra surface; tracked as a follow-up.

*Proposed changes:*
1. **Feature flag** — `warpdotdev/warp/crates/warp_features/src/lib.rs`: add `GoalSlashCommand` to the `FeatureFlag` enum (near `SummarizationConversationCommand`, `lib.rs:471`); add it to `DOGFOOD_FLAGS` (`lib.rs:939`) for initial rollout. Follow the repo's `add-feature-flag` skill.
2. **Command registration** — `warpdotdev/warp/app/src/search/slash_command_menu/static_commands/commands.rs`: add `pub static GOAL: LazyLock<StaticCommand>` (mirror `COMPACT`, `commands.rs:393`) with `name: "/goal"`, `availability: Availability::AGENT_VIEW | Availability::AI_ENABLED | Availability::NOT_CLOUD_AGENT`, `auto_enter_ai_mode: true`, `argument: Some(Argument::optional().with_hint_text("<completion condition>"))`. Register it in `all_commands()` (`commands.rs:625`) gated by `FeatureFlag::GoalSlashCommand.is_enabled()`. Do **not** add to `slash_command_is_submitted_as_prompt` (`mod.rs:1553`) or `TuiSlashCommand` (`mod.rs:132`).
3. **`/goal` execution arm** — `warpdotdev/warp/app/src/terminal/input/slash_commands/mod.rs::execute_slash_command` (`mod.rs:485`): add a `commands::GOAL.name` match arm. If the argument is present and non-empty → `self.ai_controller.update(ctx, |c, ctx| c.set_active_goal(condition, ctx))`; if absent/empty → `c.clear_active_goal(ctx)`. Return `true` (handled). This sets/clears state only; it does not submit a prompt.
4. **Active goal state** — add an `ActiveGoal { condition: String, turn_count: usize, max_turns: usize }` (plus an "active" flag) held per-conversation on the blocklist history model (`warpdotdev/warp/app/src/ai/blocklist/history_model.rs`), in-memory and **not** serialized in v1. Add `BlocklistAIController::set_active_goal` / `clear_active_goal` / `active_goal_for(conversation_id)` and history-model accessors `set_goal` / `clear_goal` / `increment_turn` / `active_goal`.
5. **Post-turn judge hook** — in the GUI Agent Mode response-completion path (`on_response_stream_complete` → `AfterStreamFinished`, `response_stream.rs:375`/`:397`; controller `handle_response_stream_event`, `controller.rs:2817`): when a worker turn completes successfully (`StreamFinished` with `Done`, not cancelled/errored) and the conversation has an active goal, spawn the judge request — an out-of-band, read-only, tool-less MAA request mirroring `build_passive_suggestions_request_params` (`controller.rs:2150`), carrying the conversation transcript + the condition + a grading prompt that returns structured `yes` / `no` / `impossible` + `reason`. On judge response:
   - `yes` → `clear_active_goal`; show success notice; stop.
   - `no` → `increment_turn`; if `turn_count >= max_turns` → clear goal + cap-reached notice + stop; else submit a continuation `AIAgentInput::UserQuery` (judge's reason as `query`, `user_query_mode: Normal`) via the controller's normal request path (`send_request_input`) — this starts the next turn, completing the loop.
   - `impossible` → `clear_active_goal`; show notice with reason; stop.
   A user-initiated cancel of the in-flight turn skips auto-continue for that boundary (the goal stays set; the loop resumes after the next user-driven turn). The judge request itself must not create a visible blocklist exchange (like passive suggestions).
6. **Judge model + caps config** — `warpdotdev/warp/app/src/settings/` (AISettings): add `goal_judge_model: Option<ModelId>` (default `None` → resolve fastest available, else active model) and `goal_max_turns: usize` (default `20`), following existing AISettings patterns.
7. **Active-goal indicator UI** — Agent View: a persistent indicator (condition + `turn N/cap` + clear `×`) placed per `.agents/skills/gui-ui-guidelines/SKILL.md` (e.g. input footer/status area). Binds to the conversation's `ActiveGoal`. Hidden when no goal is active. Updated on each turn-counter change and cleared on `yes`/`impossible`/cap/manual-clear.
8. **Telemetry** — add events for goal set / cleared / judge verdict (`yes`/`no`/`impossible`) / cap-reached, following `.agents/skills/add-telemetry/SKILL.md`.

*Open questions resolved:*
- *Evaluator/judge model + where it runs* → small/fast configurable model, client-side tool-less MAA request mirroring the passive-suggestion out-of-band pattern; active-model fallback. (Design alternatives above.)
- *How the condition is checked each turn* → a post-turn hook on GUI Agent Mode response completion (`AfterStreamFinished`); judge is tool-less and grades only the transcript.
- *Spend caps + `impossible`* → N-turn cap (default 20, configurable); `impossible` clears with reason; both stop the loop and notify.
- *Active-goal indicator UI + set/clear* → persistent Agent View indicator with turn progress + clear control; `/goal <condition>` sets/replaces, `/goal` (no arg) or `×` clears.
- *Composition with Agent Mode / auto-approve* → auto-continue reuses `UserQuery` and does **not** bypass auto-approve; a user stop skips auto-continue for that boundary.
- *Scope (local vs. cloud)* → v1 local-agent-only / client-only; cloud agents out of scope (needs `warp-server` hook). (Brief's important scope decision.)
- *Lifecycle / persistence* → in-memory, conversation-scoped, not persisted across restarts in v1.
- *Naming/positioning vs. `/plan` and `/orchestrate`* → `/goal` installs a standing condition + verifier loop; `/plan` and `/orchestrate` are one-shot prompt submissions. No conflict.
- *TUI* → GUI-only for v1.

*Validation & verification criteria* (must ALL pass before merge):
1. `/goal` appears in the GUI Agent View slash menu when `FeatureFlag::GoalSlashCommand` is enabled and AI is on, and is hidden from cloud-agent conversations and the TUI — verified by `commands_tests.rs` assertions on `all_commands()` inclusion/exclusion and `TuiSlashCommand::from_static_command` returning `None` for `/goal`; checked by `cargo nextest run -p warp --lib slash_command` (or the repo's command-registry test target).
2. `slash_command_is_submitted_as_prompt` returns `false` for `/goal` (it is an immediate action, not a prompt) — new unit test in `warpdotdev/warp/app/src/terminal/input/slash_commands/mod_tests.rs` asserting `/goal` is not in the submitted-as-prompt set.
3. `ActiveGoal` state lifecycle is correct: `set_goal` stores condition + resets `turn_count` to 0 + applies `max_turns`; `increment_turn` advances the counter; `clear_goal` removes it; `active_goal` reads current state — new unit tests next to the history model (`warpdotdev/warp/app/src/ai/blocklist/history_model_tests.rs`) named `goal_state_lifecycle_*`.
4. The judge loop drives the conversation end-to-end with a **mocked judge**: set goal → worker turn completes → judge returns `no` → a continuation `UserQuery` is submitted and `turn_count` increments → worker turn completes → judge returns `yes` → goal is cleared and no further continuation is sent. The judge is a separate model call, so the test injects deterministic judge responses (mock the judge MAA response the way passive-suggestion tests do, or via the repo's LLM-mock/test-double pattern). New test `goal_loop_no_then_yes_clears` in `warpdotdev/warp/app/src/ai/blocklist/controller_tests.rs`.
5. `impossible` escape hatch: judge returns `impossible` → goal clears, a notice is produced, and no continuation is submitted — new test `goal_loop_impossible_clears`.
6. N-turn cap: judge returns `no` on the cap-th turn → goal clears with a cap-reached notice and no further continuation — new test `goal_loop_turn_cap_stops`.
7. User-cancel interaction: a user-initiated cancel of the in-flight turn skips auto-continue for that boundary (goal stays set) — new test `goal_loop_user_cancel_skips_continue`.
8. Auto-approve composition: the continuation `UserQuery` runs under the conversation's existing approval gates (the goal loop does not force-approve commands) — asserted in the loop test by confirming continuation-proposed commands still route through the normal approval path.
9. `/goal` with no argument clears an active goal; with an argument sets/replaces it — new test in `mod_tests.rs` / `commands_tests.rs` exercising the `execute_slash_command` `GOAL` arm for both cases.
10. Judge request is tool-less and read-only and does not create a visible blocklist exchange — asserted in the loop test (no new exchange block appears for the judge call).
11. `./script/presubmit` passes (fmt, `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`, nextest) from `warpdotdev/warp`.
12. **User-facing visual proof (computer use):** in a running Warp build with `GoalSlashCommand` enabled, screenshots show (a) the active-goal indicator appearing after `/goal <condition>` with correct turn progress, (b) the indicator updating as turns advance on `no`, and (c) the indicator clearing on `yes`/`impossible`/cap/manual-clear. Per `factory-verification` this is a client UI change, so visual proof is mandatory and must be attached to the task record and PR body.

*Out of scope for v1 (tracked follow-ups):*
- Cloud-agent support (requires a `warp-server` turn-boundary/verifier hook).
- Persisting the active goal across app restarts.
- TUI support (`TuiSlashCommand::Goal` + indicator rendering).
- A dedicated hidden `AIAgentInput::GoalFollowUp` variant for cleaner continuation transcript semantics (needs server support).
- Token/cost-based spend caps (in addition to the turn cap).
