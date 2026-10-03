# Independent Safety Classifier — Technical Notes
Product spec: `specs/agent-decides-safety-classifier/PRODUCT.md`
## Current implementation (how "Agent decides" works today)
The auto-execute decision is made client-side in `app/src/ai/blocklist/permissions.rs`:
- `can_autoexecute_command(...)` (lines ~869-975) is the single decision point.
- Precedence order today: (1) denylist match → `Denied(ExplicitlyDenylisted)`; (2) conversation is in run-to-completion → `Allowed(RunToCompletion)`; (3) branch on the `execute_commands` setting.
- For `ActionPermission::AgentDecides`:
  - If `FeatureFlag::AgentDecidesCommandExecution` is enabled **and** `is_risky == Some(false)` → `Allowed(AgentDecided)`. **This is the self-report we want to replace.**
  - Else if the command contains redirection → `Denied(ContainsRedirection)`.
  - Else if all decomposed commands match the allowlist → `Allowed(ExplicitlyAllowlisted)`.
  - Else if `is_read_only` → `Allowed(AgentDecided)`; otherwise `Denied(AgentDecided)`.
Where `is_risky` comes from: it is the primary model's own `run_shell_command` tool call argument (`risk_category`), threaded through the API conversion (`app/src/ai/agent/api/convert_conversation.rs`) into `AIAgentActionType::RequestCommandOutput { command, is_read_only, is_risky, .. }`, then read in `app/src/ai/blocklist/action_model/execute/shell_command.rs::should_autoexecute` (lines ~106-150) and in `app/src/ai/blocklist/block.rs` (lines ~2714-2749). So the command author (primary agent) also grades the command's risk.
Relevant existing precedent for cheap auxiliary model calls in the client: `SharedBlockTitleGeneration`, `PredictAMQueries`, `PromptSuggestionsViaMAA` — Warp already makes lightweight, non-primary model calls, so a scoped classification call fits existing patterns.
## What Codex actually does (investigation)
The feedback said "codex uses a secondary agent." That's accurate for one specific path, but the important nuance is that Codex does **not** LLM-classify every command. Codex has two layers:
### 1. Deterministic safety, no LLM (the common case)
Codex first tries to clear a command with pure code — no model call:
- `is_known_safe_command` (`codex-rs/.../command_safety/is_safe_command.rs`): a hardcoded allowlist of read-only tools (`cat`, `ls`, `grep`, `head`, `tail`, `wc`, `pwd`, `sed -n {N}p`, `git status|log|diff|show|branch` with read-only flags, `rg`/`find` minus their dangerous flags, etc.). It uses a **tree-sitter bash parse** so that `EXPR1 <safe-op> EXPR2` is safe when both sides are safe and the operator is one of `&&`, `||`, `;`, `|` (rejects redirection/subshells).
- `execpolicy` (`codex-rs/core/src/exec_policy.rs`): a Starlark rule engine that returns `Allow`/`Prompt`/`Forbidden`.
- User/enterprise **rules** (`prefix_rule(pattern=[...], decision="allow"|"deny"|...)`) let admins mark command prefixes as allowed/blocked without approval.
Anything cleared here runs autonomously with zero model calls; the sandbox (Seatbelt/bubblewrap/Windows) is the hard boundary.
### 2. "Auto-review" / Guardian reviewer agent (opt-in, boundary crossings only)
Only when an action would cross the sandbox boundary and would otherwise stop for a human (e.g. `approval_policy = "on-request"` with `approvals_reviewer = "auto_review"`) does Codex escalate to a reviewer. Details (`codex-rs/core/src/guardian/mod.rs`):
- It is a **dedicated Codex review session** (a separate agent) using the `codex-auto-review` model.
- It receives a **compact transcript** (capped token budgets) plus the exact proposed action, and must return strict JSON: `{ risk_level, user_authorization, outcome, rationale }`.
- **Fails closed** on timeout (`GUARDIAN_REVIEW_TIMEOUT = 90s`), execution failure, or malformed output.
- **Circuit breaker:** interrupts the turn after 3 consecutive or 10 total denials per turn.
- It is a *reviewer swap, not a permission grant*: it does not widen the sandbox; it only decides whether an already-approval-requiring action proceeds.
### Takeaways for Warp
- The feedback's instinct is directionally right: safety judgment should be **independent** of the primary agent.
- But "whole separate agent" is heavier than needed for our use case. Codex's reviewer is a full session because it must reason over rich context and can perform read-only checks at the sandbox boundary. Warp's **Agent decides** gate is a narrower yes/no on a single command, so a **single scoped classification call to a cheap model** is a reasonable middle ground — this matches the feedback author's own suggestion.
- The parts of Codex worth borrowing regardless of implementation weight: (a) a **deterministic fast-path first** so most commands never hit an LLM, (b) **fail-closed** semantics, and (c) **structured output with a rationale**.
## Proposed design (hybrid: deterministic fast-path + independent classifier)
Keep `can_autoexecute_command` as the single decision point and, in the `AgentDecides` branch, replace reliance on `is_risky` with an independent verdict, computed in this order:
1. **Denylist** (unchanged, highest precedence) → Denied.
2. **Deterministic fast-path (no LLM):**
   - Redirection / opaque constructs → Denied (as today).
   - Allowlist match → Allowed (as today, no classifier call).
   - Known-safe read-only commands → Allowed. Consider porting a small `is_known_safe_command`-style safelist so the vast majority of read-only commands skip the model entirely.
   - A small hardcoded "known-dangerous" set (e.g. `rm -rf`, `curl|sh`, disk writes to devices) → Denied without a model call.
3. **Independent safety classifier (the new part):** for commands not resolved above, call a **cheap/fast model** whose *only* job is to classify the command as safe/unsafe to auto-run, given minimal context (the command string, cwd, shell, and optionally the current user request — but **not** framed as the task-completing agent). It returns structured output `{ verdict: safe|unsafe|uncertain, rationale }`.
   - `safe` → `Allowed(AgentDecided)` (or a new `Allowed(SafetyClassifierApproved)` reason for telemetry).
   - `unsafe` / `uncertain` → `Denied` (surface to user).
   - **timeout/error/malformed → Denied (fail closed).**
### Where the classifier call lives
Two options; recommend evaluating both:
- **Server-side (preferred for consistency):** the multi-agent backend runs the classification as part of producing the `run_shell_command` tool call, attaching an *independent* `safety_verdict` field (distinct from the primary agent's `risk_category`) to the action. Pros: single place, shared prompt, easy evals, cheaper model routing; the client just reads `safety_verdict` instead of `is_risky`. This is the cleanest way to guarantee independence from the primary agent's context.
- **Client-side:** the client issues an auxiliary model call in `should_autoexecute` before executing, mirroring existing auxiliary-call patterns (`SharedBlockTitleGeneration`, `PredictAMQueries`). Pros: no protocol change; contained blast radius. Cons: adds a synchronous round-trip on the client and a second code path.
Because `can_autoexecute_command` is currently synchronous and pure, a client-side call requires making the auto-execute check async (or precomputing the verdict when the action arrives, then having `can_autoexecute_command` read the cached verdict). The precompute-then-read approach keeps `can_autoexecute_command` synchronous and testable.
### Feature flagging & rollout
- New `FeatureFlag::AgentDecidesSafetyClassifier` (compile-time feature + runtime check per `app/src/features.rs` conventions), independent of `AgentDecidesCommandExecution` so we can A/B: off = current `is_risky` self-report; on = independent classifier.
- Add telemetry mirroring `AutoexecutedAgentModeRequestedCommand { reason }` with the new reason(s) and (optionally) the classifier verdict + latency so we can measure precision/coverage and cost.
### Key files to touch
- `app/src/ai/blocklist/permissions.rs` — decision logic + new `CommandExecutionPermissionAllowedReason::SafetyClassifierApproved` / denied reason.
- `app/src/ai/blocklist/action_model/execute/shell_command.rs` — obtain/cache the independent verdict before `should_autoexecute`.
- `app/src/features.rs` — new feature flag.
- `app/src/ai/blocklist/permissions_tests.rs` — tests (see PRODUCT.md §7).
- If server-side: the multi-agent backend + the action proto/`convert_conversation.rs` to carry `safety_verdict`.
### Open questions
- Server-side vs. client-side call site (recommendation above: server-side for guaranteed independence + shared evals).
- Exact cheap model to use and its latency/cost budget.
- How much context to give the classifier without re-introducing the same injection surface (recommend: minimal, command-centric context; do not pass the full agent scratchpad).
- Whether to surface the rationale in the UI when a command is held back (follow-up, flagged).
