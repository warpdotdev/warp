# Refactor AmbientAgentViewModel to be cloud-agent scoped
Linear: APP-4274
## Context
This refactor simplifies the relationship between `AmbientAgentViewModel`, `AgentViewController`, and cloud-mode terminal views.
Before the change, `AmbientAgentViewModel` existed more broadly than its actual responsibility. It represented cloud-agent-specific state, but it could also be present in local/non-cloud agent flows, which made `TerminalView`, `Input`, and `AgentViewController` look like they all owned parallel pieces of agent state. This made it unclear which abstraction was the source of truth for local Agent View state, cloud-agent lifecycle state, root vs nested cloud-mode navigation state, and cloud-only UI like harness selection and ambient progress.
The final model is:
- `AgentViewController` owns local/fullscreen/inline Agent View conversation state.
- `AmbientAgentViewModel` owns cloud-agent conversation/session state only.
- `TerminalView` optionally has an `AmbientAgentViewModel` only when the terminal represents cloud mode.
- Root vs nested cloud mode is derived from `PaneStack`, not stored as parallel navigation state.
Relevant files:
- `app/src/terminal/view.rs:2849` — `TerminalView::is_nested_cloud_mode`
- `app/src/terminal/view.rs:2970` — `TerminalView::new(..., is_cloud_mode: bool, ...)`
- `app/src/terminal/input.rs:1660` — `Input.ambient_agent_view_state`
- `app/src/terminal/input.rs:1669` — private `AmbientAgentViewState`
- `app/src/terminal/input.rs:2176` — construction of `AmbientAgentViewState`
- `app/src/terminal/view/pane_impl.rs:939` — `TerminalView::is_ambient_agent_session`
- `app/src/terminal/view/ambient_agent/model.rs:77` — `AmbientAgentViewModel`
- `app/src/terminal/view_test.rs:338` — root/nested cloud-mode keymap regression test
## Goals
Make `AmbientAgentViewModel` cloud-scoped, avoid constructing dummy ambient models for local/non-cloud Agent View flows, keep cloud-agent state attached to the terminal/view entry for the cloud conversation, remove denormalized cloud navigation state from `TerminalView`, make the schema encode invariants around cloud-only UI state, and preserve cloud setup, composing, spawning, shared-session viewer, progress, and cancellation flows.
## Non-goals
This does not redesign `AgentViewController`, change local Agent View behavior except by removing ambient-model coupling, change cloud-agent server APIs or task spawning semantics, or change `PaneStack` ownership semantics.
## Proposed changes
### `AmbientAgentViewModel` is optional on `TerminalView`
`TerminalView::new` takes `is_cloud_mode: bool` and constructs an ambient model only when that flag is true. Cloud-mode terminals get `Some(ModelHandle<AmbientAgentViewModel>)`; normal/local terminals get `None`.
This makes the presence of `AmbientAgentViewModel` meaningful: if it exists, the terminal is capable of cloud-agent UI and lifecycle behavior.
### `AgentViewController` no longer carries ambient model state
`AgentViewController` remains responsible for Agent View entry/exit/conversation display state. Cloud-agent-specific progress, setup, harness, task, environment, and cancellation state lives in `AmbientAgentViewModel`.
This removes the previous parallel-state ambiguity where local Agent View and ambient-agent state appeared coupled even when local flows did not need ambient state.
### `AmbientAgentViewModel` represents only cloud-agent lifecycle state
`AmbientAgentViewModel` starts in a cloud-relevant state (`Composing`) and tracks cloud-only concepts:
- setup/composing/waiting/running/failure/cancelled status
- selected cloud environment
- selected harness
- spawned task ID
- cloud-agent request
- progress timing
- setup command state
Because non-cloud terminals do not construct this model, the model no longer needs a “not ambient agent” status variant or parent-terminal bookkeeping.
### `Input` groups ambient-only UI state
`Input` now stores `ambient_agent_view_state: Option<AmbientAgentViewState>` instead of independent optional ambient-model and harness-selector fields.
`AmbientAgentViewState` contains:
- `view_model: ModelHandle<AmbientAgentViewModel>`
- `harness_selector: ViewHandle<HarnessSelector>`
The grouped state is constructed only when `ambient_agent_view_model` exists. This encodes the invariant directly: no ambient model means no harness selector, and if a harness selector exists then an ambient model exists.
Accessors on `Input` preserve existing call-site ergonomics:
- `ambient_agent_view_model()`
- `harness_selector()`
### Cloud navigation is derived from `PaneStack`
The refactor removes `CloudAgentNavigation` and `TerminalView.cloud_agent_navigation`.
`TerminalView::is_nested_cloud_mode` now checks:
1. The terminal is an ambient/cloud session.
2. The terminal has an owning `PaneStack`.
3. The terminal’s view appears in the stack.
4. The terminal is not the first entry.
This makes `PaneStack` the source of truth:
- first stack entry = root cloud-mode pane
- later stack entries = nested cloud-mode panes
- no stack = not nested
This avoids stale cached navigation state and handles root/nested state consistently after push/pop. The important subtlety is that stack depth alone is insufficient: a root pane in a stack with depth greater than one is still root. The implementation checks the terminal’s actual position in the stack instead.
### Root cloud-mode keymap context uses derived state
The root cloud-mode key is set only when the terminal is an ambient/cloud session and is not nested according to `PaneStack`.
This preserves the intended behavior for bindings like setting input mode to Agent Mode: root cloud-mode panes should not accidentally enter local Agent View.
### Pane chrome and cloud-mode entry use derived nested state
The pane header and cloud-agent entry paths now consult `is_nested_cloud_mode` instead of cached navigation state.
Important behavior:
- nested cloud-mode panes show parent/back navigation where appropriate
- starting cloud mode from a nested cloud-mode pane can pop back to the parent and start a sibling run
- root cloud-mode panes remain distinguishable from nested panes even when the stack depth is greater than one
## End-to-end flow
### Local/non-cloud Agent View
1. `TerminalView::new(..., is_cloud_mode: false, ...)`
2. `ambient_agent_view_model = None`
3. `Input.ambient_agent_view_state = None`
4. `AgentViewController` handles Agent View state
5. Cloud-only UI/state is absent
### Cloud-mode terminal
1. `TerminalView::new(..., is_cloud_mode: true, ...)`
2. `AmbientAgentViewModel` is constructed for that terminal view
3. `Input` creates `AmbientAgentViewState`
4. Cloud setup/composing/progress/cancellation use the ambient model
5. Root vs nested navigation is derived from `PaneStack`
### Nested cloud-mode pane
1. A cloud-mode terminal is pushed onto a `PaneStack`
2. `PaneStack::push` calls `TerminalView::set_pane_stack`
3. `is_nested_cloud_mode` finds the terminal’s index in `PaneStack::entries`
4. Index `> 0` means nested
5. Keymap/chrome/cloud-entry behavior follows from that derived state
## Review comment resolutions
### Group `AmbientAgentViewModel` and `HarnessSelector`
Resolved by introducing private `AmbientAgentViewState` in `Input`.
This prevents impossible/ambiguous optional-state combinations and makes the schema match the real invariant.
### Remove denormalized cloud navigation
Resolved by removing `CloudAgentNavigation` and `TerminalView.cloud_agent_navigation`.
Root/nested state now comes from the owning `PaneStack`.
## Testing and validation
Focused validation passed:
- `cargo test -p warp root_cloud_mode_pane_sets_root_cloud_mode_context_key --features local_tty,local_fs`
- `cargo test -p warp set_input_mode_agent_does_not_enter_local_agent_from_root_cloud_mode_pane --features local_tty,local_fs`
- `cargo check -p warp --features local_tty,local_fs`
The key regression test is `root_cloud_mode_pane_sets_root_cloud_mode_context_key` in `app/src/terminal/view_test.rs:338`.
It verifies:
1. A standalone cloud-mode terminal gets `ROOT_CLOUD_MODE_PANE_KEY`.
2. After creating a `PaneStack`, the root terminal still gets the key.
3. A pushed nested cloud-mode terminal does not get the key.
The test keeps the `PaneStack` handle alive through assertions so `TerminalView`’s weak stack handle can upgrade during keymap evaluation.
## Risks and mitigations
### Optional ambient model requires many call sites to handle `None`
Call sites use optional accessors or only pass the model into cloud-capable components when present. This makes non-cloud behavior explicit rather than hidden behind dummy state.
### Deriving nested state from `PaneStack` could misclassify panes
The helper checks stack membership and entry index, not only stack depth. This keeps root panes root even when they have nested children.
### Cloud shared-session viewers still need ambient state
Shared-session viewer construction passes `is_cloud_mode` through to `TerminalView::new`, so cloud-mode shared session viewers still construct `AmbientAgentViewModel`.
## Follow-ups
- Cloud UI verification requires pushing the branch so a cloud agent can build/test the changed client state.
- If more cloud-only UI state is added to `Input`, it should live under `AmbientAgentViewState` rather than as independent optional fields.
- If more root/nested cloud behavior appears, it should continue to derive from `PaneStack` rather than reintroducing cached navigation state.
