# Product Spec: Rich Integration & Notifications for 'agy' (Antigravity) CLI

**Issue:** [warpdotdev/warp#13147](https://github.com/warpdotdev/warp/issues/13147)

## 1. Summary

Enhance native support for the `agy` (Antigravity) CLI agent in Warp. Building upon the basic command detection and branding introduced in [GH11368](https://github.com/warpdotdev/warp/issues/11368), this milestone integrates the structured session listener for OSC 777 notifications and optimizes rich input submission for multiline prompts.

## 2. Problem

While Warp currently recognizes `agy` and displays the Antigravity toolbar and branding:
1. Warp does not process structured notifications emitted during `agy` execution (such as task completion or input requests), leaving the session listener inactive.
2. Submitting multiline prompts or rich context from the Warp footer can encounter race conditions or premature execution when using generic inline PTY writes instead of a delayed-enter strategy.

## 3. Goals

- **Structured Notification Listener:** Wire `CLIAgent::Antigravity` to Warp's `DefaultSessionListener` to process OSC 777 notification sequences (`\x1b]777;notify;{...}`).
- **Rich Input Delivery:** Configure `CLIAgent::Antigravity` to use `RichInputSubmitStrategy::DelayedEnter`, ensuring multiline prompt text and attachments sent from the Warp footer are cleanly buffered and submitted to the agent's PTY.

## 4. Non-Goals

- Distributing or auto-installing external plugins from non-existent third-party repositories. Like Pi, Auggie, and Droid, Warp listens to standard OSC 777 sequences emitted natively or through user hooks without requiring an external package manager.
- Local skill provider scanning for Antigravity-specific private skills.

## 5. Testable Invariants

1. **Session Listener Registration:** Panes running `CLIAgent::Antigravity` are recognized as supported by `is_agent_supported()`, and `create_handler()` yields a valid session handler (`DefaultSessionListener`).
2. **Notification Processing:** Emitting a structured OSC 777 `PluggableNotification` within an `agy` session parses valid agent event payloads (e.g. `task_complete`) and skips raw `session_start` duplicates.
3. **Delayed-Enter PTY Submission:** Submitting rich input text while an Antigravity CLI session is active routes through `RichInputSubmitStrategy::DelayedEnter`, inserting the prompt buffer before dispatching a carriage return.

## 6. Success Criteria

1. Running `agy` in Warp registers the session listener so OSC 777 notification sequences update terminal state and tab status.
2. Multiline inputs submitted from the Warp agent footer deliver reliably without missing lines.

## 7. Validation

- **Unit tests:** Assert `is_agent_supported(&CLIAgent::Antigravity)` is true and `create_handler(&CLIAgent::Antigravity)` returns `Some`.
- **Unit tests:** Assert `rich_input_submit_strategy(CLIAgent::Antigravity)` returns `RichInputSubmitStrategy::DelayedEnter`.
- **Manual verification:** Run `agy` in a local terminal and verify multiline prompt entry and OSC 777 notification handling.
