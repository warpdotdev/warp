# Handoff Environment Selection: PWD-Based Overlap at Activation — Tech Spec
Product spec: `specs/APP-4427/PRODUCT.md`
Linear: [APP-4427](https://linear.app/warpdotdev/issue/APP-4427)

## Context
Environment auto-selection for `&` handoff has two layers:
- **Layer 1** (activation time): `start_pwd_environment_overlap` in `input.rs:3769` spawns an async task that resolves the pwd's git repo and calls `pick_handoff_overlap_env` to select the best environment. `EnvironmentSelector::ensure_default_selection` (`environment_selector.rs:372`) also runs synchronously to pick a saved/MRU fallback.
- **Layer 2** (post-dispatch): After pressing Enter, an async task in `workspace/view.rs:13422-13488` re-resolves the pwd repo and overwrites the environment on the new cloud pane model.

Layer 2 causes a visible environment flicker after the handoff pane opens. The compose state already has the correct environment by the time Enter is pressed — layer 2 is redundant.

Key files:
- `app/src/terminal/input/handoff_compose.rs` — `HandoffComposeState` model
- `app/src/terminal/input.rs:3739` — `activate_cloud_handoff_compose`
- `app/src/terminal/input.rs:3769` — `start_pwd_environment_overlap` (layer 1)
- `app/src/terminal/input.rs:3973` — `maybe_launch_cloud_handoff_request` (Enter dispatch)
- `app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs:372` — `ensure_default_selection`
- `app/src/workspace/view.rs:13319` — `complete_local_to_cloud_handoff_open`
- `app/src/workspace/view.rs:13450-13464` — post-dispatch overlap overwrite (to remove)
- `app/src/workspace/action.rs:493` — `OpenLocalToCloudHandoffPane` action

## Proposed changes
### 1. Rename action field and pass compose state's selection
Rename `explicit_environment_id` → `environment_id` on `OpenLocalToCloudHandoffPane` (`workspace/action.rs:498`). In `maybe_launch_cloud_handoff_request` (`input.rs:3991`), pass `selected_environment_id().cloned()` from the compose state instead of `explicit_environment_id()`. This carries the pwd-overlap result (or MRU fallback) to the new cloud pane.

### 2. Remove async env overwrite in `complete_local_to_cloud_handoff_open`
Remove the `resolve_repo_for_path` call inside the async task (`workspace/view.rs:13435-13442`), the `pwd_repo` plumbing through the tuple, and the env overwrite block in the completion handler (`view.rs:13450-13464`). The `source_pwd` capture (`view.rs:13420`) is also removed. The workspace derivation and snapshot upload continue unchanged.

### 3. Clean up `PendingHandoff.explicit_environment_id`
Remove the `explicit_environment_id` field from `PendingHandoff`, the `pending_handoff_has_explicit_environment()` method on `AmbientAgentViewModel` (`model.rs:549`), and the guard in `EnvironmentSelectorTarget::ensure_default_environment_id` (`environment_selector.rs:84-87`). The guard is no longer needed: the env is set on the model before the `EnvironmentSelector` is created, so `ensure_default_selection` already short-circuits at line 374 (`if current_selection.is_some() { return; }`).

### 4. Update all dispatch and handler sites
Rename `explicit_environment_id` → `environment_id` at all sites that construct or destructure the action, and update intermediate function signatures (`start_local_to_cloud_handoff`, `complete_local_to_cloud_handoff_open`, `start_fresh_cloud_launch`, `restore_source_handoff_draft`, `restore_cloud_handoff_draft`).

## Testing and validation
### Manual
- In a terminal whose pwd is inside a repo that matches exactly one environment: type `&`, observe the matching environment in the chip. Press Enter — the cloud pane opens with the same environment, no shift.
- Same setup but explicitly pick a different environment from the dropdown, press Enter — the explicit choice is preserved.
- In a directory outside any git repo: type `&`, observe fallback to saved/MRU default. Press Enter — same env, no shift.
- `/handoff query` from a repo-matching directory: cloud pane opens with its own default selection (compose state isn't active in this path).

## Parallelization
This is a small, tightly coupled change across a few files in a single call chain. A single agent should implement it sequentially.
