# Desktop execution-host selection: technical design
## Context
Implement the numbered behaviors in [PRODUCT.md](PRODUCT.md). This is a proposed fix,
not an implementation or an incident-confirmed reproduction.
Source evidence at client revision `c10fc07d74faa400dc527ed747b30fcb44928772`:
- [`host_selector.rs (159-254)`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/view/ambient_agent/host_selector.rs#L159-L254)
  restores `CloudAgentSettings.last_selected_host` into `selected`. Default updates
  preserve a saved choice, but construction does not emit `HostSelected`.
- [`input.rs (2529-2611)`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/input.rs#L2529-L2611)
  initializes the launch model from `effective_default_host`, not the restored selection.
  Its workspace-event handler also assigns that default after updating the selector.
  Explicit-selection events instead copy the selector's selected host.
- [`model.rs (1125-1134)`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/view/ambient_agent/model.rs#L1125-L1134)
  copies `worker_host` into the spawn snapshot.
Consequently, a restored self-hosted label can submit `None` or a different default.
Reselecting the host synchronizes the state, matching the reported workaround.
At server revision `a4522bfa2781ba067f77b4dc2e00181f44d270dd`,
[`add_task.go (408-424)`](https://github.com/warpdotdev/warp-server/blob/a4522bfa2781ba067f77b4dc2e00181f44d270dd/logic/ai/ambient_agents/add_task.go#L408-L424)
can fill an absent host from workspace defaults after execution-principal defaults.
An unresolved empty host becomes Warp-hosted in
[`worker_routing.go (61-100)`](https://github.com/warpdotdev/warp-server/blob/a4522bfa2781ba067f77b4dc2e00181f44d270dd/logic/ai/ambient_agents/sources/worker_routing.go#L61-L100).
An ordinary explicit self-hosted slug is not this empty-host case.
## Proposed changes
### Authoritative pane-local selection
Keep selection resolution in `HostSelector`; do not add a new shared global model.
Represent an optional explicit choice separately from the existing configured default.
Restore the explicit choice once at construction and update it on `SelectHost`.
Derive both the displayed host and an optional request-host accessor from those values:
- Explicit choice: use `Host::worker_host_value()`, including the existing Warp sentinel.
- No explicit choice, configured default: use that default's request value.
- Neither: display the existing Warp fallback and return `None` for the request.
Avoid independent mutable copies of the resolved selection inside the selector. Its
default badge still reflects the configured default even when an explicit choice wins.
`set_default_host` and `clear_default_host` must preserve the pane's explicit choice,
not infer its provenance by rereading another window's shared saved preference. The
shared setting remains the restoration source for new composers; no migration or
cross-window live preference subscription is introduced.
### Synchronize the launch model
In `Input::build_host_selector`, apply the effective default, then read the selector's
resolved request host and assign it to `AmbientAgentViewModel.worker_host`. Do this
unconditionally, including `None`, so retained model state cannot leak into a new pane.
After `HostSelected`, use the same accessor rather than mapping the fallback display
host directly. In the existing workspace-event handler, apply or clear the effective
default, then copy the selector's resolved request host into the model. Retain the
current window-event filter and connected-worker refresh.
Perform these reads and assignments synchronously in the existing UI-context update
path; do not defer synchronization until menu close or a later render. Eager and lazy
attachment must use the same builder. The snapshot builder and server need no policy
changes.
### Alternatives not chosen
Emitting `HostSelected` only during construction is insufficient: the input subscribes
after construction. Initializing from `selected().worker_host_value()` without retaining
selection provenance would send explicit Warp for the unspecified fallback, suppressing
server defaults. Changing backend empty-host routing would alter intentional API behavior
instead of repairing the client mismatch.
## Testing and validation
Add focused regressions at the existing app test layer. Exercise selector updates and
composer wiring, then assert the resulting spawn snapshot or serialized request host;
do not stop at a getter, private field, or label assertion.
Use the existing fresh/lazy composer attachment fixture in
[`input_tests.rs:1587`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/input_tests.rs#L1587)
and the two-window/team fixture at
[`input_tests.rs:1680`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/input_tests.rs#L1680).
The nearby host-selector coverage at
[`input_tests.rs:3453`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/input_tests.rs#L3453)
provides another existing view-level entry point. Seed `CloudAgentSettings` through the
normal settings fixture; no exported test-only production API is needed. Snapshot
assertions can follow the existing `spawn_config_*` patterns in
[`model_tests.rs:85`](https://github.com/warpdotdev/warp/blob/c10fc07d74faa400dc527ed747b30fcb44928772/app/src/terminal/view/ambient_agent/model_tests.rs#L85).
- Behaviors 1-3 and 10: restore a saved self-hosted choice with no default, submit without
  clicking the menu, and assert that host in the snapshot. Repeat with a conflicting
  default and exercise both eager and lazy attachment.
- Behaviors 4-5: distinguish no preference/no default (omitted host) from an explicit
  Warp choice (hosted sentinel). Verify no preference with a configured default submits
  the default. Inspect serialization to ensure omission and explicit Warp differ.
- Behaviors 6-8: refresh/add/change/remove the current default, and switch the window's
  team. Check explicit choices are retained while inherited choices update. Exercise a
  sibling window's saved-choice/team change without retargeting the first composer.
- Behaviors 9 and 11: update connected-worker availability and explicitly reselect a
  host. Check request selection remains stable, and retain existing menu/focus tests.
- Behavior 12: confirm follow-ups use the existing run's message-only path; do not add
  relocation behavior or a redundant server routing test for unchanged logic.
For implementation, run the smallest applicable app test filter, targeted Clippy, and
the applicable formatter in the order defined by `AGENTS.md`. Broad platform coverage
belongs to normal CI; a full local presubmit is not required by this spec.
Use `cargo nextest run -p warp --lib --features local_tty,local_fs` with a filter for
the new host-selection regressions and the existing attachment/host-selector tests.
Run `cargo clippy -p warp --all-targets --tests --features local_tty,local_fs -- -D warnings`,
then the applicable repository formatter. These are future implementation checks,
not results claimed by this spec-only change.
Customer confirmation still needs the desktop OS/channel/build, affected and working
run IDs, and whether each was a fresh launch. Compare the sanitized request host,
persisted snapshot, resolved worker kind/slug, and execution location before and after
reselection. No UI reproduction or customer-run log inspection has been performed.
Computer-use verification is a separate human choice before implementation; do not
treat this spec request as authorization to operate a running interface.
## Parallelization
Use one implementation workstream after approval. Selector provenance, input wiring,
and regression fixtures are tightly coupled; parallel authors would add coordination
cost without independent ownership. An independent review can follow implementation.
## Risks
Accidentally treating the fallback Warp label as an explicit choice would change server
default inheritance. Rereading the global preference during refresh would confuse
another window's saved choice with this pane's active choice. The wire-level and
multi-window regressions above guard both boundaries.
Existing global preference scope, permission checks, and disconnected-worker handling
remain unchanged. Spec approval does not establish that the reported customer's build
contains this defect; implementation and incident confirmation must remain distinct.
