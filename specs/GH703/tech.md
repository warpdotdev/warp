# TECH.md — Linear deeplink must not silently auto-submit prompts into the agent

**GitHub Issue:** [warpdotdev/warp-external#703](https://github.com/warpdotdev/warp-external/issues/703)
**Product Spec:** `specs/GH703/product.md`

## Context

`warp://linear/work?prompt=<PROMPT>` is a trusted `warp://` URI. The handler decodes the `prompt` query parameter and feeds it into a new agent conversation. A shortcut in `try_enter_agent_view` today promotes *any* origin with an `initial_prompt` to auto-submit when the focused terminal was already in fullscreen agent view, which causes the Linear-supplied prompt to be sent to the LLM without any user interaction. This is contrary to the intent encoded in `AgentViewEntryOrigin::should_autotrigger_request`, which deliberately excludes `LinearDeepLink`.

The relevant code paths today:

- `app/src/uri/mod.rs:429-443` — `UriHost::Linear` match arm parses `LinearAction::WorkOnIssue`, builds `LinearIssueWork`, and dispatches into the new-or-existing window action.
- `app/src/linear.rs:30-38` — `LinearIssueWork::from_url` reads `prompt` from the URL with no validation beyond "non-empty".
- `app/src/root_view.rs:1107-1119,2875-2891` — `open_linear_issue_work_in_{new,existing}_window` forwards `LinearIssueWork` to `workspace.open_linear_issue_work(...)`.
- `app/src/workspace/view.rs:15888-15937` — `open_linear_issue_work` opens a new tab and calls `terminal_view.enter_agent_view_for_new_conversation(prompt, AgentViewEntryOrigin::LinearDeepLink, ctx)`.
- `app/src/terminal/view/agent_view.rs:48-83` — `enter_agent_view_for_new_conversation` delegates to `try_enter_agent_view`.
- `app/src/terminal/view/agent_view.rs:160-290` — `try_enter_agent_view` captures `was_in_agent_view_already` and runs `if origin.should_autotrigger_request() || was_in_agent_view_already { send_user_query_in_conversation(...) }`. This is where the auto-submit occurs for `LinearDeepLink`.
- `app/src/ai/blocklist/agent_view/controller.rs:172-205` — `AgentViewEntryOrigin::LinearDeepLink` is defined and `should_autotrigger_request` intentionally returns `false` for it.
- `app/src/uri/uri_test.rs:293-347` — existing unit tests covering the Linear URI parsing surface.

The ephemeral "enter again to send" path already exists in the `else` arm of `try_enter_agent_view` (`app/src/terminal/view/agent_view.rs:240-266`): it populates the agent input buffer via `self.input.update(...).replace_buffer_content(&initial_prompt, ctx)` and shows an ephemeral message tagged `ENTER_AGAIN_TO_SEND_MESSAGE_ID`. That is the desired user experience for Linear-originated prompts.

Product invariants 1–13 in `specs/GH703/product.md` define the required behavior.

## Proposed changes

### 1. Remove the implicit `was_in_agent_view_already` shortcut (committed decision)
`try_enter_agent_view`'s auto-submit condition is the root cause. We make the auto-submit decision a function of the origin alone — the `was_in_agent_view_already` shortcut is deleted outright. This was evaluated against two options during spec review; we commit to **option (a)**:
- **(a) chosen:** Only `origin.should_autotrigger_request()` governs auto-submit. The prior shortcut existed to preserve "typed a new prompt in agent view and pressed Cmd+Enter" ergonomics, but that flow already enters via `AgentViewEntryOrigin::Input { was_prompt_autodetected: true }` (in the allowlist) or `AgentViewEntryOrigin::Keybinding`/`SlashCommand` paths. Any origin that legitimately needs to auto-submit when the user is already in agent view must be added to `should_autotrigger_request` explicitly.
- (b) rejected: gating the shortcut on a new `is_trusted_user_originated` predicate keeps the footgun. If a future `AgentViewEntryOrigin` gets added and forgets to opt out of the predicate, the URI-injection bug reopens. Option (a) removes the whole class of problem.
The concrete change replaces:
```rust path=null start=null
if origin.should_autotrigger_request() || was_in_agent_view_already {
    // ...send to LLM
}
```
with a single check whose result depends only on the origin and the caller's explicit `AutoSubmitPolicy` (see step 3).
**Audit of other origins (in-scope, committed deliverable).** Every `AgentViewEntryOrigin` variant was enumerated against `should_autotrigger_request`'s allowlist. The allowlist intentionally contains only: `Input { was_prompt_autodetected: true }`, `SlashCommand { trigger: !is_keybinding() }`, `Cli`, `AcceptedPromptSuggestion`. Every other variant — including `CodexModal`, `CloudAgent`, `ChildAgent`, `ProjectEntry`, `OnboardingCallout`, `LinearDeepLink`, `AgentViewBlock`, `AgentRequestedNewConversation`, `SharedSessionSelection`, `RestoreExistingConversation`, `InlineCodeReview`, `ConversationSelector`, `AgentModeHomepage`, `AIDocument`, `AutoFollowUp`, `AcceptedUnitTestSuggestion`, `AcceptedPassiveCodeDiff`, `ImageAdded`, `SlashInit`, `CreateEnvironment`, `Keybinding`, `CodeReviewContext`, `InlineHistoryMenu`, `InlineConversationMenu`, `ConversationListView`, `DefaultSessionMode`, `LongRunningCommand`, `Onboarding`, `ClearBuffer`, `PromptChip`, plus the soon-to-be-removed `ContinueConversationButton`/`ViewPassiveCodeDiffDetails`/`ResumeConversationButton` — now receives the explicit "populate input buffer + show `ENTER_AGAIN_TO_SEND_MESSAGE_ID`" behavior for `initial_prompt`. None of them carry URI- or network-sourced prompt strings except `LinearDeepLink` today, but the shortcut removal future-proofs the decision.
### 2. Keep the "populate buffer + enter again to send" path as the default for Linear
With the shortcut removed, `LinearDeepLink` naturally falls into the existing `else` branch of `try_enter_agent_view` (input buffer replacement + ephemeral `ENTER_AGAIN_TO_SEND_MESSAGE_ID` message). No new UI surface is needed; this is already the behavior for non-auto-triggering origins that arrive with an `initial_prompt` when the user is not in fullscreen agent view.
### 3. Defense-in-depth: opt `open_linear_issue_work` out of auto-submit at the call site (committed shape)
We commit to the **"explicit parameter on `try_enter_agent_view` + thin wrapper method at the public API"** shape:
1. Add a private `AutoSubmitPolicy` enum in `app/src/terminal/view/agent_view.rs` with two variants: `FromOrigin` (today's behavior after step 1) and `NeverAutoSubmit` (forced draft).
2. Thread `auto_submit: AutoSubmitPolicy` as a new parameter on `try_enter_agent_view`. Inside `try_enter_agent_view`, the auto-submit decision becomes `match auto_submit { FromOrigin => origin.should_autotrigger_request(), NeverAutoSubmit => false }`.
3. Expose two public entrypoints from `TerminalView`:
   - `enter_agent_view_for_new_conversation(prompt, origin, ctx)` — unchanged signature for existing callers; uses `AutoSubmitPolicy::FromOrigin`.
   - `enter_agent_view_for_new_conversation_with_prompt_draft(prompt, origin, ctx)` — new wrapper for call sites that handle prompts from URI-sourced, IPC, or otherwise untrusted input; uses `AutoSubmitPolicy::NeverAutoSubmit`.
4. `open_linear_issue_work` in `app/src/workspace/view.rs` calls `enter_agent_view_for_new_conversation_with_prompt_draft` instead of `enter_agent_view_for_new_conversation`.
The parameter-plus-wrapper shape was chosen over a pure wrapper because the wrapper alone cannot prevent a future internal refactor from re-introducing an implicit fullscreen promotion inside `try_enter_agent_view`; the explicit policy parameter makes the decision visible at every call site of `try_enter_agent_view` and forces a typed match.
Any future origin that should behave the same way can call the draft entrypoint at its call site without needing to re-audit `try_enter_agent_view`.
This is defense-in-depth: step 1 is sufficient on its own for correctness; step 3 ensures that even if `should_autotrigger_request` ever grows a buggy allowlist entry, Linear deeplinks remain safe.
### 4. Leave `LinearAction`, `LinearIssueWork::from_url`, and the URI validation layer unchanged
No parsing, validation, or sanitization of the `prompt` query parameter is added. The mitigation depends on user gesture, not on string filtering — content-based filters are trivially bypassable for prompt injection. `LinearIssueWork::from_url` continues to decode `prompt` verbatim (still filtering empty strings, matching product invariant 6).
### 5. Telemetry and logging redaction
No telemetry schema changes are required. `did_auto_trigger_request` in `TelemetryEvent::AgentViewEntered` naturally reports `false` for Linear deeplinks after the fix; no extra code is needed.
To honor product invariant 12 (logging / telemetry redaction), the implementation commits to the following, and reviewers should verify:
- `TelemetryEvent::AgentViewEntered` and `TelemetryEvent::LinearIssueLinkOpened` carry only origin classifiers and fixed schema fields. The `initial_prompt` value is never attached to a telemetry payload.
- The `log::error!` call in `enter_agent_view_for_new_conversation_with_policy` interpolates only `origin` (a compile-time enum discriminant) and `e` (a structured `EnterAgentViewError` whose `Display` does not include the prompt body). The `initial_prompt: Option<String>` is explicitly not passed to any formatting macro in this file.
- `self.show_error_toast(e.to_string(), ctx)` relies on `EnterAgentViewError: Display`; the error types enumerated in `EnterAgentViewError` do not carry user-prompt strings. New error variants added in the future must not include the prompt body.
- Conversation title fallback remains `"Linear Issue"` (product invariant 9) so the prompt never becomes a title.

## End-to-end flow (after the fix)

1. OS/dispatch primitive hands `warp://linear/work?prompt=<ATTACKER>` to Warp.
2. `validate_custom_uri` → `UriHost::Linear` → `LinearAction::WorkOnIssue` → `LinearIssueWork::from_url` → `open_linear_issue_work_in_{new,existing}_window`.
3. `workspace.open_linear_issue_work` opens a new tab and invokes `enter_agent_view_for_new_conversation_with_prompt_draft(prompt, AgentViewEntryOrigin::LinearDeepLink, ctx)` (new entrypoint from step 3).
4. `try_enter_agent_view` enters agent view for the new conversation. Because `should_autotrigger_request()` returns `false` for `LinearDeepLink` and the `was_in_agent_view_already` shortcut has been removed (step 1) — or, belt-and-suspenders, because the caller passed `NeverAutoSubmit` (step 3) — the code takes the "replace buffer + show ephemeral message" branch.
5. The user sees the prompt in the input buffer and the "press enter again to send to agent" message. No LLM request has been made.
6. The user either sends (explicit Enter inside the confirmation window) or edits/clears the prompt.

## Testing and validation
Tests map back to the numbered product invariants in `specs/GH703/product.md`.
1. **Unit tests in `app/src/terminal/view_test.rs` (invariants 1, 2, 3, 4, 6).** Three tests are implemented:
   - `linear_deeplink_populates_input_as_draft_when_not_in_agent_view` — enters via `enter_agent_view_for_new_conversation_with_prompt_draft` when agent view is inactive; asserts the input buffer contains the attacker prompt and the ephemeral message id is `ENTER_AGAIN_TO_SEND_MESSAGE_ID`.
   - `linear_deeplink_does_not_auto_submit_when_already_in_agent_view` — first enters fullscreen agent view via `Input { was_prompt_autodetected: false }`, then dispatches the Linear deeplink. Asserts a new conversation id is allocated, the prompt lands in the input buffer instead of being auto-submitted, and the ephemeral affordance is shown.
   - `linear_deeplink_via_default_entrypoint_does_not_auto_submit_in_fullscreen` — verifies the defense-in-depth layer: even if a caller forgets to use the draft entrypoint and goes through `enter_agent_view_for_new_conversation` directly, `LinearDeepLink` still does not auto-submit because the `was_in_agent_view_already` shortcut is gone.
2. **Regression guard for other origins (invariant 10).** The existing `clear_buffer_action_in_fullscreen_agent_view_starts_new_conversation` test and the broader agent-view test suite cover `AgentViewEntryOrigin::Input { was_prompt_autodetected: true }` and keyboard-driven flows. The allowlist in `should_autotrigger_request` is unchanged, so origins that auto-submit today (`Cli`, `Input { was_prompt_autodetected: true }`, `SlashCommand { trigger: !is_keybinding() }`, `AcceptedPromptSuggestion`) continue to do so.
3. **Unit test for URL parsing (already present, keep).** `app/src/uri/uri_test.rs` covers `validate_custom_uri_linear`, `test_linear_action_parse_*`, and `test_linear_issue_work_*`. No changes required; they continue to exercise the decoding path to ensure we don't regress the URI schema.
4. **Manual verification (invariants 1–5, 7, 11).**
   - On macOS, Linux, and Windows, open `warp://linear/work?prompt=<attacker+payload>` while the focused terminal is already in fullscreen agent view. Confirm the prompt shows up in the input, the ephemeral "enter again to send" message is visible, and no LLM request is made until the user presses Enter.
   - Repeat with the focused terminal not in agent view.
   - Repeat with the app closed (cold start) to cover the dispatch path used by `open_linear_issue_work_in_new_window`.
   - Repeat with Warp backgrounded at dispatch time (invariant 11): foreground Warp afterward and confirm the prompt stays as a draft and the affordance is still shown.
   - Confirm that opening the URL does not cause any of `read_files`, `StartAgent`, `SendMessageToAgent`, `FetchConversation`, or `UseComputer` tool calls to be issued.
5. **Telemetry and redaction spot check (invariants 8, 12).** Inspect the emitted `AgentViewEntered` event and confirm `did_auto_trigger_request = false`. Confirm `LinearIssueLinkOpened` still fires once per dispatch. Grep `log::` output from the agent-view entry path to confirm the verbatim prompt never appears in a log line.
6. **`./script/presubmit`** passes (fmt, clippy, tests).

## Risks and mitigations

1. **Removing `was_in_agent_view_already` could regress other flows.** Some origins may rely on this shortcut today (e.g. entering a second conversation from agent view with a typed prompt). Mitigation: the agent-view unit tests in `app/src/terminal/view_test.rs` continue to exercise `Input { was_prompt_autodetected: false }`, `Input { was_prompt_autodetected: true }`, and keyboard-driven flows. Because any origin that legitimately needs to auto-submit is already in `should_autotrigger_request`'s allowlist, removing the fullscreen promotion does not change their behavior. If a regression does surface, the correct fix is to add the specific origin to the allowlist — not to re-introduce a generic fullscreen shortcut.
2. **Other URI-originated origins could have the same bug.** `CodexModal`, `ProjectEntry`, `OnboardingCallout`, and future URI-sourced origins were audited in step 1 above and are not in the auto-submit allowlist; the shortcut removal already protects them. New URI- or IPC-sourced call sites should additionally use `enter_agent_view_for_new_conversation_with_prompt_draft` for defense-in-depth.

3. **User confusion from unexpected input content.** A user who did not intend to open a Linear deeplink will still see the URI-supplied prompt in their agent input. This is a strict improvement over silent submission: they can inspect, edit, or clear it. No mitigation needed; the ephemeral "enter again to send" message already signals that the prompt is pending.

4. **Attacker-supplied very long prompts.** Existing input buffer handling already supports arbitrarily long content. No change needed.

## Follow-ups
- The audit of other `AgentViewEntryOrigin` variants (previously a follow-up) is now promoted into step 1 above and is a committed deliverable of this change. No follow-up issue is required; the grep-based audit is covered by the shortcut removal.
- Coordinate with the owners of #655 (Windows named pipe) and #666 (Linux D-Bus) to gate `warp://` dispatch on a platform-trusted source. This spec is orthogonal defense; with that work landed the prompt-injection blast radius shrinks further. Tracked separately in those issues — this spec does not block on them.
- Consider adding a dedicated UI indicator (banner or toast) identifying a prompt as "from a Linear deeplink" so the user knows its provenance at a glance. Not required for the fix but would improve trust; can be added iteratively once the safety invariant is in place.
