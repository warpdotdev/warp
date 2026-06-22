# REMOTE-1591: Tech spec — Environment creation modal for handoff

## Context

When a user enters `&` handoff-compose mode but has zero cloud environments, there is currently no way to create one without leaving the flow. See `PRODUCT.md` for detailed user-facing behavior.

### Relevant code

**Handoff compose flow (input layer)**
- `app/src/terminal/input.rs` — `maybe_launch_cloud_handoff_request()` (line ~3810) is the Enter handler for `&` compose mode. Currently returns `true` (consumed) when the prompt is empty, or collects attachments and dispatches `WorkspaceAction::OpenLocalToCloudHandoffPane`.
- `app/src/terminal/input/handoff_compose.rs` — `HandoffComposeState` model tracking `&` mode activation and selected environment.

**Environment selector chip**
- `app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs` — `refresh_button()` (line 437) sets the chip label. Falls back to `"New environment"` when no env is selected.

**Environment creation form**
- `app/src/settings_view/update_environment_form.rs` — `UpdateEnvironmentForm` view. Already supports modal-style use via `show_header` (line 332), `should_handle_escape_from_editor` (line 336), and `auth_source` (line 342). Emits `UpdateEnvironmentFormEvent::Created { environment, share_with_team }` on submit.

**Prior art: embedding the form in non-settings contexts**
- `app/src/terminal/view/ambient_agent/first_time_setup.rs` — `FirstTimeCloudAgentSetupView` wraps the form with `show_header=false`, `should_handle_escape_from_editor=true`, handles `Created` by calling `UpdateManager::create_ambient_agent_environment()`. This is the pattern to follow for environment creation logic.

**Prior art: modal overlay rendering**
- `app/src/settings_view/agent_assisted_environment_modal.rs` — `AgentAssistedEnvironmentModal` renders using `Dialog::new().with_close_button().with_child().build()` wrapped in `Dismiss::new().prevent_interaction_with_other_elements()`, inside a `Container` with `ColorU::new(0, 0, 0, 179)` background. Uses `show()`/`hide()` visibility toggle and emits `Cancelled`/`Confirmed` events. This is the rendering pattern to follow.
- `app/src/ui_components/dialog.rs` — `Dialog` component used by modal overlays. Provides title, close button, child content, and bottom row.

**Workspace handoff dispatch**
- `app/src/workspace/view.rs` — `start_local_to_cloud_handoff()` (line 12972) and `start_fresh_cloud_launch()` (line 12938) handle `WorkspaceAction::OpenLocalToCloudHandoffPane`. The workspace is also where top-level overlays like `remove_tab_config_confirmation_dialog` are owned and rendered.

## Proposed changes

### 1. New view: `HandoffEnvironmentCreationModal`

New file: `app/src/settings_view/handoff_environment_creation_modal.rs`

A thin modal wrapper around `UpdateEnvironmentForm`, following the `AgentAssistedEnvironmentModal` pattern for rendering and `FirstTimeCloudAgentSetupView` for form configuration and environment creation logic.

**View state:**
- `visible: bool`
- `environment_form: ViewHandle<UpdateEnvironmentForm>`
- `close_button_mouse_state: MouseStateHandle`
- `scroll_state: ClippedScrollStateHandle` (the form is tall — needs scrolling within the modal)

**Public API:**
- `show(&mut self, ctx)` — sets `visible = true`, resets form to `Create` mode, focuses name field
- `hide(&mut self, ctx)` — sets `visible = false`

**Events:**
```rust
enum HandoffEnvironmentCreationModalEvent {
    Created { env_id: SyncId },
    Cancelled,
    CreationFailed { error_message: String },
}
```

The `Created` event carries a `SyncId::ServerId` — guaranteed to be a server-recognized ID. The modal uses `create_ambient_agent_environment_online` (an inline server call with built-in retries) so the `ServerId` is available before the event is emitted, eliminating any `ClientId` → `ServerId` sync race. On failure, `CreationFailed` is emitted with the error message so the workspace can show a toast.

**Form configuration** (matching `FirstTimeCloudAgentSetupView`):
- `show_header = false` → submit button renders at bottom-right of form body
- `should_handle_escape_from_editor = true` → Escape in any editor emits `Cancelled`
- `auth_source = AuthSource::CloudSetup` → GitHub auth redirects back in-place

**Environment creation** (inside the modal's `handle_environment_form_event`):
- On `UpdateEnvironmentFormEvent::Created { environment, share_with_team }`:
  1. Resolve owner via `cloud_environments::owner_for_new_environment()` / `owner_for_new_personal_environment()`
  2. Generate `ClientId::default()`
  3. Call `UpdateManager::create_ambient_agent_environment_online()` — returns `Future<Result<ServerId>>`
  4. Hide the modal immediately
  5. `ctx.spawn` the future:
     - On `Ok(server_id)`: emit `Created { env_id: SyncId::ServerId(server_id) }`
     - On `Err(err)`: log the error and emit `CreationFailed { error_message }`

**Rendering** (following `AgentAssistedEnvironmentModal`):
- When `visible == false`, render `Empty`
- When visible: `Dialog::new("Create environment", None, dialog_styles(appearance)).with_close_button(...).with_child(scrollable_form).with_width(DIALOG_WIDTH).build()` → `Dismiss::new().prevent_interaction_with_other_elements().on_dismiss(cancel)` → `Container` with dark overlay background + window corner radius
- The form content has no fixed max height — the dialog sizes to its content, with a `ClippedScrollable` safety net for very small windows

### 2. Input: intercept Enter when no environments exist

In `input.rs`, `maybe_launch_cloud_handoff_request()`:

After the existing empty-prompt early return (which already handles the no-op case), check whether environments are empty. If so, emit `Event::OpenHandoffEnvironmentCreationModal` and return early instead of collecting attachments. A new `OpenHandoffEnvironmentCreationModal` variant is added to `Input::Event`.

The prompt and attachments stay in the input buffer — the user will see them unchanged when the modal closes.

### 3. Workspace: own the modal and wire up handoff auto-submit

In `workspace/view.rs`:

**New field** on `Workspace`:
```rust
handoff_environment_creation_modal: Option<ViewHandle<HandoffEnvironmentCreationModal>>,
```

The modal is created on-demand (not pre-constructed at workspace init) to avoid unnecessary overhead. It is stored as `Option<ViewHandle>` and rendered as a `ChildView` in the workspace's `render()` method when `Some`, following the same pattern as `lightbox_view`.

**Subscribe to modal events** when the modal is created in `show_handoff_environment_creation_modal`:
- `HandoffEnvironmentCreationModalEvent::Created { env_id }` →
  1. Set `handoff_environment_creation_modal = None`
  2. Get the active terminal view's input and read the prompt + attachments from its `&` compose state
  3. Use the input's `collect_cloud_launch_attachments()` and `editor.buffer_text()` to build a `PendingCloudLaunch`
  4. Clear the input buffer and exit `&` compose mode
  5. Dispatch `WorkspaceAction::OpenLocalToCloudHandoffPane { launch, explicit_environment_id: Some(env_id) }`
- `HandoffEnvironmentCreationModalEvent::Cancelled` →
  1. Set `handoff_environment_creation_modal = None`
  2. Re-focus the active terminal input (the `&` compose state and prompt are already preserved)
- `HandoffEnvironmentCreationModalEvent::CreationFailed { error_message }` →
  1. Set `handoff_environment_creation_modal = None`
  2. Show an error toast: "Failed to create environment: \<error_message\>"
  3. Re-focus the active terminal input

**New workspace action** `ShowHandoffEnvironmentCreationModal` — creates the modal on-demand, subscribes to its events, stores the handle, and calls `ctx.notify()` to trigger a re-render. The terminal view subscribes to `Input::Event::OpenHandoffEnvironmentCreationModal` and dispatches this action.

**Render** — add the modal overlay to the workspace's render output when `handoff_environment_creation_modal.is_some()`, using the same overlay stacking pattern as `lightbox_view`.

### 4. Ghost text fallback

In `input.rs`, `set_zero_state_hint_text()` (line ~6120), the handoff compose branch falls back to `CLOUD_HANDOFF_HINT_TEXT` when no environment is found. Update this constant from `"Start a cloud run"` to `"Handoff to cloud"` to match behavior 2 in `PRODUCT.md`.

## Testing and validation

**Unit tests** (in `handoff_compose_tests.rs` and `input_test.rs`):
- Test that `maybe_launch_cloud_handoff_request` emits `OpenHandoffEnvironmentCreationModal` when environments list is empty and prompt is non-empty (behavior 5)
- Test that empty prompt + no environments returns `true` without emitting (behavior 6)
- Test that with environments present, Enter submits normally (behavior 16)

**Unit tests** (new file `handoff_environment_creation_modal_tests.rs`):
- Test `show()` resets form to Create mode and sets `visible = true`
- Test that form `Created` event triggers `UpdateManager::create_ambient_agent_environment` and emits `Created { env_id }`
- Test that form `Cancelled` event emits `Cancelled`

**Manual validation:**
- Enter `&` with no environments → ghost text shows "Handoff to cloud"
- Type a prompt and press Enter → modal opens with environment creation form, prompt preserved in input behind modal
- Fill out form, submit → environment created, handoff auto-submits with new environment
- Escape from modal → returns to `&` compose with prompt intact
- Create env via Settings while in `&` mode → chip updates reactively (behavior 17)
