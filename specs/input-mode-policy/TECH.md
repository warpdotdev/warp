# InputModePolicy: view-agnostic `BlocklistAIInputModel`

References are pinned to commit `51145bb70dc2e461d1152880e8f173dce28ac165`.

## Context

The TUI frontend (`crates/warp_tui`) is growing input-mode behavior — first `!` shell mode ([`specs/CODE-1805`](../CODE-1805/TECH.md)), later natural-language autodetection — and should reuse the GUI's input-mode state machine, `BlocklistAIInputModel` ([`app/src/ai/blocklist/input_model.rs @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs)), rather than duplicating it.

The blocklist stack is otherwise already view-agnostic. `BlocklistAIController`, `BlocklistAIContextModel`, and `BlocklistAIInputModel` take a `ConversationSelectionHandle = ModelHandle<Box<dyn ConversationSelection>>` ([`app/src/ai/blocklist/conversation_selection.rs:17`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/conversation_selection.rs#L17)) so each frontend supplies its own selection semantics: the GUI's `AgentViewConversationSelection` ([`app/src/ai/blocklist/agent_view/conversation_selection.rs:125`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/agent_view/conversation_selection.rs#L125)) is backed by `AgentViewController`; the TUI's `TuiConversationSelection` (`crates/warp_tui/src/conversation_selection.rs:232`) is local state. The TUI already constructs the full model stack this way (`crates/warp_tui/src/terminal_session_view.rs (80-121)`).

`BlocklistAIInputModel` is the one model that breaks the pattern: it consults GUI-only state directly instead of asking an injected abstraction. Concretely, it makes the same GUI-shaped decision — "is this surface a fullscreen agent view or a top-level terminal, and which autodetection setting applies?" — in five places:

1. The AI-lock gate in `set_input_config_internal` ([input_model.rs (510-517)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L510-L517)): when `FeatureFlag::AgentView` is enabled, `{AI, locked}` configs are silently rejected unless a conversation is active or CLI-agent rich input is open.
2. The initial config in `::new` ([input_model.rs (389-399)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L389-L399)).
3. The `AISettings`-changed subscription's three branches ([input_model.rs (266-317)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L266-L317)).
4. The `ConversationSelection` Activated/Deactivated decision table ([input_model.rs (319-387)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L319-L387)), including `AgentViewEntryOrigin::ClearBuffer` and attachment-forced-AI handling.
5. `is_autodetection_enabled_for_current_context` ([input_model.rs (597-628)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L597-L628)) and `InputConfig::unlocked_if_autodetection_enabled` ([input_model.rs (152-165)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L152-L165)).

For a TUI consumer this coupling is not just untidy — it is incorrect: the gate in (1) silently no-ops the TUI's desired default (`{AI, locked}` with no active conversation), and the reactive writes in (3)/(4) can flip a TUI surface into `{Shell, locked}` (e.g. on conversation deactivation with NLD disabled), which the TUI would misread as user-requested shell mode.

## Proposed changes

Extract the five decision points into a view-supplied trait, following the `ConversationSelection` injection pattern. This is a **pure refactor**: GUI behavior is unchanged, and the model's public API (mutators/readers) is untouched.

### New trait (`app/src/ai/blocklist/input_mode_policy.rs`)

```rust path=null start=null
/// View-supplied policy for input-mode decisions the model cannot make
/// view-agnostically (lock gating, autodetection context, reactive
/// config transitions).
pub trait InputModePolicy: 'static {
    /// The config the surface starts with.
    fn initial_config(&self, app: &AppContext) -> InputConfig;
    /// Whether the input may currently be locked to AI.
    fn allows_locked_ai_input(&self, app: &AppContext) -> bool;
    /// Whether NL autodetection is enabled for the surface's current context.
    fn is_autodetection_enabled(&self, app: &AppContext) -> bool;
    /// Config to apply in response to a conversation-selection event; None leaves it unchanged.
    fn config_on_conversation_selection_changed(
        &self, event: &ConversationSelectionEvent, current: InputConfig, app: &AppContext,
    ) -> Option<PolicyConfigUpdate>;
    /// Config to apply when AI settings change; None leaves it unchanged.
    fn config_on_ai_settings_changed(
        &self, event: &AISettingsChangedEvent, current: InputConfig,
        is_autodetection_enabled_for_current_context: bool, app: &AppContext,
    ) -> Option<PolicyConfigUpdate>;
}

pub type InputModePolicyHandle = Rc<dyn InputModePolicy>;
```

The reactive hooks receive the raw events, so view-specific payloads (fullscreen vs. inline, entry origins, exit-before-new-entrance) stay a concern of the implementing view — the trait signature carries no GUI vocabulary beyond the shared event types. `PolicyConfigUpdate` bundles exactly what the previously-inlined GUI decision code passed to the model's internal setter: the config, its recorded decision source, and (for one agent-view entry path) a brief autodetection suppression. The settings hook receives the model's guarded autodetection context as a `bool`. Computing it takes the terminal-model lock, so the model computes it only for `AIAutoDetectionEnabled` events — the one event whose handling can need it — and passes `false` for all others.

`BlocklistAIInputModel::new` gains an `InputModePolicyHandle` parameter; the five decision points above become calls through it. The reactive *subscriptions* stay in the model — the ~35 GUI mutator call sites rely on the model self-healing across CLI-agent-input close and agent-view enter/exit — only their *decisions* move. The `CLIAgentSessionsModel` restore subscription ([input_model.rs (234-264)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L234-L264)) is pure restore-what-was-saved mechanism and stays as-is.

### GUI implementation (`app/src/ai/blocklist/agent_view/gui_input_mode_policy.rs`)

`GuiInputModePolicy` holds the `ConversationSelectionHandle`, the `BlocklistAIContextModel` handle (for `has_locking_attachment`), and the surface id (for `CLIAgentSessionsModel::is_input_open`). Each trait method transplants the corresponding branch verbatim, including the `FeatureFlag::AgentView` checks — the flag becomes a GUI-policy detail instead of a model detail. Constructed next to the model in `Input::new` ([`app/src/terminal/view.rs:3472`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/view.rs#L3472)).

### TUI implementation (`crates/warp_tui/src/input_mode_policy.rs`)

`TuiInputModePolicy`: `initial_config` = `{AI, locked}` (the TUI input is agent-first), `allows_locked_ai_input` = `true`, `is_autodetection_enabled` = `false`, and both reactive hooks return `None` — conversation and settings changes never rewrite TUI input mode. When TUI autodetection ships, this one file changes (`is_autodetection_enabled` flips to a real setting and `initial_config` unlocks).

### Exports

Re-export `InputModePolicy`, `InputModePolicyHandle`, `InputConfig`, `InputType`, and `InputTypeAutoDetectionSource` in `app/src/tui_export.rs` so `warp_tui` can implement the trait and drive the model.

### Deliberately out of scope

- The `entered_agent_mode_num_times` settings bump on AI-type transitions ([input_model.rs (532-539)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L532-L539)) stays shared mechanism; the semantic ("entered agent mode") holds on both surfaces.
- `set_input_config_for_classic_mode`'s `InputSettings` check ([input_model.rs (458-478)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L458-L478)) stays; the TUI never calls it.
- `InputConfig::unlocked_if_autodetection_enabled` remains as a GUI-side helper for its callers in `app/src/terminal/input.rs`; the model itself stops calling it in favor of the policy.

## Testing and validation

- This is a no-behavior-change refactor for the GUI: `app/src/terminal/input_tests.rs` (~43 references) exercises every input-mode flow (⌘I toggle, `!` prefix, `&` handoff, ctrl-c reset, history selection, workflow insertion, attachment locking) through the real `Input` view and must pass with **no assertion changes**. Same for `app/src/terminal/view_tests.rs` and `queued_prompts_tests.rs`.
- New `input_mode_policy` unit tests in `warp_tui` assert the TUI policy's determinism: initial `{AI, locked}` sticks (the old gate would have rejected it), and reactive hooks leave the config untouched across conversation activate/deactivate.
- `./script/presubmit` before submitting.

## Follow-ups

- `specs/CODE-1805` (stacked on this PR) consumes the policy for TUI `!` shell mode.
- When TUI autodetection is built, extend `TuiInputModePolicy` rather than the model.
