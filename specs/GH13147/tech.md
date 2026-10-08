# Technical Spec: Rich Integration & Notifications for 'agy' (Antigravity) CLI

See `specs/GH13147/product.md` for the product spec.

**Issue:** [warpdotdev/warp#13147](https://github.com/warpdotdev/warp/issues/13147)

## 1. Context

In [GH11368](specs/GH11368/product.md), `CLIAgent::Antigravity` was added for command detection (`agy`) and UI branding. At the time, session listeners and rich notifications were deferred.

Currently:
- In `app/src/terminal/cli_agent_sessions/listener/mod.rs` (lines 39–53, 76–85), `CLIAgent::Antigravity` is omitted from `is_agent_supported()` and matches `None` in `create_handler()`.
- In `app/src/terminal/view/use_agent_footer/mod.rs` (lines 135–144), `CLIAgent::Antigravity` uses `RichInputSubmitStrategy::Inline`, which immediately streams text with an inline newline instead of using a delayed carriage return.

## 2. Proposed Changes

### 2a. Enable Session Listener (`app/src/terminal/cli_agent_sessions/listener/mod.rs`)

1. Add `CLIAgent::Antigravity` to `is_agent_supported()`:
   ```rust
   pub fn is_agent_supported(agent: &CLIAgent) -> bool {
       matches!(
           agent,
           CLIAgent::Claude
               | CLIAgent::OpenCode
               | CLIAgent::Codex
               | CLIAgent::Gemini
               | CLIAgent::Auggie
               | CLIAgent::Droid
               | CLIAgent::Pi
               | CLIAgent::OhMyPi
               | CLIAgent::Grok
               | CLIAgent::WarpTui
               | CLIAgent::Antigravity
       )
   }
   ```

2. Add `CLIAgent::Antigravity` to the `DefaultSessionListener` branch in `create_handler()`:
   ```rust
   CLIAgent::Claude
   | CLIAgent::OpenCode
   | CLIAgent::Gemini
   | CLIAgent::Auggie
   | CLIAgent::Droid
   | CLIAgent::Pi
   | CLIAgent::OhMyPi
   | CLIAgent::WarpTui
   | CLIAgent::Antigravity => Some(Box::new(DefaultSessionListener)),
   ```

3. Remove `CLIAgent::Antigravity` from the `None` fallback pattern in `create_handler()`.

### 2b. Configure Delayed-Enter Submit Strategy (`app/src/terminal/view/use_agent_footer/mod.rs`)

Move `CLIAgent::Antigravity` from `RichInputSubmitStrategy::Inline` to `RichInputSubmitStrategy::DelayedEnter`:

```rust
fn rich_input_submit_strategy(agent: CLIAgent) -> RichInputSubmitStrategy {
    match agent {
        CLIAgent::Codex => RichInputSubmitStrategy::BracketedPaste,
        CLIAgent::OhMyPi => RichInputSubmitStrategy::BracketedPaste,
        CLIAgent::Copilot => RichInputSubmitStrategy::BracketedPasteDelayedEnter,
        CLIAgent::Claude
        | CLIAgent::OpenCode
        | CLIAgent::Gemini
        | CLIAgent::Auggie
        | CLIAgent::Grok
        | CLIAgent::CursorCli
        | CLIAgent::Antigravity => RichInputSubmitStrategy::DelayedEnter,
        CLIAgent::Hermes => RichInputSubmitStrategy::BracketedPaste,
        CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Pi
        | CLIAgent::Kiro
        | CLIAgent::Goose
        | CLIAgent::Vibe
        | CLIAgent::WarpTui
        | CLIAgent::Unknown => RichInputSubmitStrategy::Inline,
    }
}
```

## 3. Data Flow & Notification Handling

1. An `agy` process running in the terminal emits an OSC 777 escape sequence formatted as:
   `\x1b]777;notify;{"event":"task_complete","body":"Task finished"}\x1b\`
2. The terminal PTY parser extracts the `PluggableNotification` and invokes `CLIAgentSessionHandler::try_parse`.
3. `DefaultSessionListener` filters out initial `SessionStart` duplicates and forwards valid agent event payloads (`task_started`, `task_complete`, `prompt_required`) to `CLIAgentSessionsModel`.
4. `CLIAgentSessionsModel` publishes UI updates, adjusting the tab indicator and active status.

## 4. Testing and Validation

1. **Invariant 1 Verification:**
   - In `app/src/terminal/cli_agent_sessions/listener/mod_tests.rs`, assert `is_agent_supported(&CLIAgent::Antigravity)` returns true.
   - Assert `create_handler(&CLIAgent::Antigravity)` returns `Some`.
2. **Invariant 2 Verification:**
   - Send mock OSC 777 payloads to a session with `CLIAgent::Antigravity` and verify `task_complete` is dispatched.
3. **Invariant 3 Verification:**
   - In `app/src/terminal/view/use_agent_footer/` tests, assert `rich_input_submit_strategy(CLIAgent::Antigravity)` equals `RichInputSubmitStrategy::DelayedEnter`.
