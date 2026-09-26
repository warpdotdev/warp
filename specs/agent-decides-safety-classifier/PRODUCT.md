# Independent Safety Classifier for "Agent decides" Command Execution
## 1. Summary
When the **Execute commands** permission is set to **Agent decides**, Warp should determine whether a command is safe to auto-execute using an **independent, dedicated safety check** — not the primary agent's self-assessment. Today the same agent that proposes a command also declares whether that command is "risky", and the client trusts that self-report to decide auto-execution. This spec proposes replacing that self-report with an independent lightweight LLM classification call (a "safety classifier"), gated behind a deterministic fast-path for the common cases.
## 2. Problem
Under **Agent decides**, the client currently auto-executes a command when the primary agent labels it as not risky (`is_risky == Some(false)`) behind the `AgentDecidesCommandExecution` feature flag. See `app/src/ai/blocklist/permissions.rs` (`can_autoexecute_command`, lines ~916-953) and `app/src/ai/blocklist/action_model/execute/shell_command.rs` (`should_autoexecute`, lines ~106-150). The `is_risky` value is supplied by the primary model as the `risk_category` argument of its own `run_shell_command` tool call.
This has a structural conflict of interest:
- **Self-grading.** The entity generating the command is the same entity grading its safety. An over-eager or mistaken agent that mislabels a destructive command as "not risky" gets it auto-executed with no independent check.
- **Prompt-injection surface.** If the primary agent is steered by injected instructions (e.g. from file contents or command output), the same injection can coerce it to under-report risk, defeating the guardrail.
- **Inconsistent calibration.** Risk labeling quality varies by model and by how much the primary agent is "focused" on the task vs. on safety. Safety judgment is a side concern competing for attention with task completion.
The user feedback references Codex, which uses a *separate* reviewer for boundary-crossing actions. The open question raised was whether Warp needs a whole separate agent or whether a single cheap LLM call suffices, and what Codex actually does. (See Technical Notes / TECH.md for the Codex findings.)
## 3. Goals
- Under **Agent decides**, base the auto-execute decision on an **independent** safety signal rather than the primary agent's self-reported risk.
- Keep the common case fast and cheap: obviously-safe and obviously-unsafe commands should not require an extra model round-trip.
- **Fail closed:** any uncertainty, timeout, or error in the classifier results in *not* auto-executing (the command is surfaced to the user), never in auto-executing.
- Make the decision explainable (a short rationale we can log/telemeter and, where useful, show the user).
- Ship behind a feature flag so we can A/B against the current self-report behavior.
## 4. Non-goals
- Changing behavior for **Always allow**, **Always ask**, or **Run to completion** modes. This spec only affects the **Agent decides** path.
- Changing the user-facing allowlist/denylist semantics. Denylist still takes precedence; allowlist matches still auto-approve without invoking the classifier.
- Replacing the OS/sandbox boundary. This is an auto-execute gating decision, not a sandbox.
- Building a full multi-turn "reviewer agent" with its own tools. This spec proposes a single scoped classification call, not an agent loop.
## 5. User experience
Behavior is largely invisible when it works well:
- A command the classifier deems safe auto-executes exactly as it does today under **Agent decides**.
- A command the classifier deems unsafe (or that it can't confidently clear) is **surfaced for the user to run/approve**, exactly like a command that's denied today.
- No new required user configuration. The setting still reads **Agent decides**.
Latency: because obviously-safe/unsafe commands take the deterministic fast-path, most commands are unaffected. Commands that require the classifier incur one lightweight model call (target: a cheap/fast model, comparable to existing auxiliary calls such as title generation and query prediction). If the classifier is slow or unavailable, we fail closed to "ask the user" rather than blocking the agent indefinitely.
Optionally (follow-up, flagged): when a command is held back due to a classifier "unsafe" verdict, show the one-line rationale in the requested-command UI so the user understands why it wasn't auto-run.
## 6. Success criteria
- For a labeled evaluation set of safe and unsafe commands, the independent classifier has **strictly higher precision on "safe" (fewer unsafe commands auto-executed)** than the current primary-agent self-report, with auto-execute *coverage* on genuinely-safe commands no worse than a small, agreed regression bound.
- Zero auto-executions on classifier timeout/error (verified by tests).
- Added end-to-end latency on the classifier path is within an agreed budget (e.g. p50 under a few hundred ms using the cheap model); fast-path commands add ~0ms.
## 7. Validation
- Unit tests in `app/src/ai/blocklist/permissions_tests.rs` covering: denylist precedence unchanged; allowlist match short-circuits classifier; fast-path safe/unsafe classification; classifier "unsafe" → Denied; classifier error/timeout → Denied (fail closed); flag off → current `is_risky` behavior preserved.
- An offline evaluation comparing classifier verdicts vs. primary-agent self-report on a curated command corpus.
- Manual QA (see checklist).
## 8. Manual QA checklist
- With the flag on and **Agent decides**: read-only commands (`ls`, `git status`) auto-run; a clearly destructive command (`rm -rf`, `curl ... | sh`) is held for the user.
- Force a classifier failure (e.g. offline) and confirm commands are held for the user, not auto-run.
- Confirm denylisted commands are still blocked and allowlisted commands still auto-run without a classifier call.
- Flip the flag off and confirm behavior matches today's `is_risky`-based path.
---
# Technical Notes
See `specs/agent-decides-safety-classifier/TECH.md` for the current implementation trace, the Codex investigation (deterministic safelist + opt-in "Auto-review"/Guardian reviewer agent), and the proposed hybrid design.
