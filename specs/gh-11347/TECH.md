# GH-11347 — Clarify when SSH warpification takes effect

Issue: [#11347 — SSH warpification toggle does not apply to current session](https://github.com/warpdotdev/warp/issues/11347)

## Context

The Settings action currently toggles and persists `WarpifySettings.enable_ssh_warpification`, sends telemetry, and enables or disables the extension-install dropdown. It does not notify a terminal or reconnect SSH ([`warpify_page.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/settings_view/warpify_page.rs#L378-L410)). The toggle row has no description today ([`warpify_page.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/settings_view/warpify_page.rs#L631-L657)).

That behavior reflects a real lifecycle boundary rather than only a missing event dispatch. When a local PTY is created, Warp reads the setting into `PtyOptions.enable_ssh_wrapper` ([`terminal_manager.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/local_tty/terminal_manager.rs#L785-L806)). The Unix launcher then writes that captured value to `WARP_USE_SSH_WRAPPER` in the new shell environment ([`unix.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/local_tty/unix.rs#L360-L365)); the equivalent value is also set when constructing sandboxed terminal environments ([`unix.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/local_tty/unix.rs#L883-L890)). Updating the persisted setting cannot mutate the environment or wrapper setup of a shell process that is already running.

The remote-server path has a second hard boundary. `ModelEventDispatcher` enters it only when an `InitShell` payload identifies an SSH-wrapper session ([`model_events.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/model_events.rs#L71-L89)). `RemoteServerController` then requires the wrapper-provided socket and ControlMaster ownership before it can check, install, or connect the extension ([`remote_server_controller.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/writeable_pty/remote_server_controller.rs#L193-L242)). An unwrapped SSH connection already in progress does not provide a safe input to that state machine.

Retrofitting the current connection would therefore require substantially broader work: live shell-environment reconfiguration, a recoverable way to establish or adopt a ControlMaster for an existing SSH process, session bootstrap migration, and cancellation semantics for in-flight setup. This spec instead implements the issue's accepted UX outcome: clearly state that the setting applies to newly created terminal sessions and leave running connections untouched (PRODUCT invariants 1–9).

## Proposed changes

### Settings copy

In [`app/src/settings_view/warpify_page.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/settings_view/warpify_page.rs#L600-L657), define a local constant for the user-facing description and pass it as the existing description argument of `render_body_item` for **Warpify SSH Sessions**:

> Changes apply to new terminal sessions. Open a new tab or pane, then reconnect to SSH.

Use the existing Settings description renderer and styling; do not add a custom text element, action, toast, modal, or terminal event. The copy remains present regardless of the toggle value so enable and disable have the same lifecycle contract (PRODUCT invariants 1, 8, and 9).

Do not change the setting definition, storage key, synchronization behavior, telemetry event, or toggle handler. `EnableSshWarpification` remains the canonical persisted boolean ([`settings.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/warpify/settings.rs#L49-L58)), and dependent controls retain their current state handling ([`warpify_page.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/settings_view/warpify_page.rs#L659-L723)) (PRODUCT invariants 2–7).

### End-to-end state flow

```text
User toggles Warpify SSH Sessions
  -> persist enable_ssh_warpification and send existing telemetry
  -> update existing dependent Settings controls
  -> leave all running local terminals and SSH sessions unchanged
  -> user opens a new terminal
  -> terminal creation snapshots the new value into WARP_USE_SSH_WRAPPER
  -> user starts SSH
     -> disabled: normal SSH path
     -> enabled and eligible: existing wrapper -> SshInitShell -> remote-server flow
```

The last enabled branch keeps the existing controller state machine: wrapper session detection, binary/preinstall checks, existing-binary connect or auto-update, install-mode choice, and safe fallback ([`remote_server_controller.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/terminal/writeable_pty/remote_server_controller.rs#L245-L352)). No transition is added for a setting change during `AwaitingCheck`, `AwaitingUserChoice`, `AwaitingInstall`, or `AwaitingConnect`; those states belong to the already-started connection and finish under the policy captured when that terminal was created (PRODUCT invariants 2, 4, and 6).

## Testing and validation

### Automated checks

- Add a focused Settings rendering assertion if the existing test harness can inspect `WarpifyPageView`: assert that the **Warpify SSH Sessions** row renders the exact supporting text while enabled and while disabled (PRODUCT invariants 1 and 8).
- If that view is not exposed to a lightweight render test, keep this copy-only change covered by the visual checks below rather than introducing a new broad GUI harness solely for one static description.
- Run `./script/format` and the repository-prescribed `cargo clippy` command before publishing the implementation PR.
- Run the narrow Settings/view tests that own `WarpifyPageView`. The existing remote-server integration builder already covers the new-terminal enabled path by setting the install mode before terminal creation and then entering SSH ([`remote_server.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/integration/src/test/remote_server.rs#L31-L62)); this copy-only change does not require duplicating that expensive connection test.

### Visual evidence mapping

Capture GUI evidence from the Warpify Settings page at the default window size and at the narrowest supported Settings width:

| Evidence | Expected result | Product invariants |
| --- | --- | --- |
| Toggle enabled | Description is visible under **Warpify SSH Sessions**, wraps without clipping, and dependent controls remain enabled | 1, 7, 8 |
| Toggle disabled | The same description remains visible and readable; dependent controls retain their existing disabled treatment | 1, 7, 8 |
| Accessibility inspection | Description is exposed as readable static text and does not create an extra focus target | 8 |

No terminal screenshot is required to prove that a running connection remains unchanged: the implementation deliberately adds no terminal mutation. If maintainers request behavioral evidence, record a short manual sequence showing an existing SSH connection survives both toggle directions and the new copy tells the user to open a new terminal (PRODUCT invariants 2, 4, 6, and 9).

## Risks

- **Copy precision:** saying only “new SSH sessions” would be misleading because the wrapper policy is captured by the containing local terminal. The proposed copy explicitly says “new terminal sessions.”
- **Expectation gap:** users may prefer immediate warpification. The description resolves the reported ambiguity but does not deliver live migration; that remains a separate, substantially larger product and architecture project.
- **Localization and wrapping:** the added sentence increases row height. Visual verification at narrow width and enlarged text must ensure the switch stays aligned and the description is not clipped.
- **Architecture drift:** the SSH remote-server implementation is evolving. The UX contract depends only on the stable process boundary—an existing shell environment and SSH process cannot be retroactively replaced by persisting a setting—not on a particular controller state name.

## Parallelization

Implementation should be completed as one workstream. The code change and its visual verification both touch the same Settings row, so parallel implementation would add coordination overhead without reducing risk. Repository-wide format and clippy checks can run concurrently after the final edit; GUI screenshots should run after formatting so they represent the exact submitted state.
