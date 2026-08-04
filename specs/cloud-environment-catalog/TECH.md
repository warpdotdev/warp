# Shared Cloud Environment Catalog
## Context
Cloud environments are persisted in `CloudModel`, but frontend consumers currently project that persistence state independently.

- [`app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs (175-421) @ 8be97b2cb`](https://github.com/warpdotdev/warp/blob/8be97b2cb7967e3ee89f8ac1977be87f1f4d137c/app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs#L175-L421) subscribes directly to `CloudModel`, repeatedly reads `CloudAmbientAgentEnvironment`, sorts it, resolves defaults, and persists selection from the GUI view.
- [`app/src/ai/orchestration/providers.rs (142-193) @ 8be97b2cb`](https://github.com/warpdotdev/warp/blob/8be97b2cb7967e3ee89f8ac1977be87f1f4d137c/app/src/ai/orchestration/providers.rs#L142-L193) independently reads the same cloud objects for orchestration defaults and persistence.
- [`app/src/lib.rs (2037-2055) @ 8be97b2cb`](https://github.com/warpdotdev/warp/blob/8be97b2cb7967e3ee89f8ac1977be87f1f4d137c/app/src/lib.rs#L2037-L2055) registers `CloudModel`, which is the source of truth for cloud-object persistence.

The GUI and headless TUI use different view frameworks, so they cannot share selector views. They can share a single model that projects environment identity, display data, ordering, updates, defaults, persistence, and refresh behavior from `CloudModel`.

This is an internal state-ownership refactor. Environment selection behavior and cloud-object persistence semantics remain unchanged.
## Proposed changes
### Shared catalog
Add `CloudEnvironmentCatalog` under `app/src/ai/cloud_environments/` and register it as a singleton immediately after `CloudModel`.

The catalog:

- Subscribes once to `CloudModel`.
- Stores frontend-safe `CloudEnvironment` summaries containing only `SyncId` and display name.
- Orders summaries by most-recent task use, then case-insensitive display name, matching the existing GUI selector.
- Separately retains the orchestration GUI's existing most-recent, then case-sensitive-name fallback ID so sharing the catalog does not change either GUI consumer's default selection.
- Emits `CloudEnvironmentCatalogEvent` only when the projected catalog changes.
- Defers `ObjectCreated` refresh by one app task because `CloudModel` emits that event before inserting the object.
- Refreshes when the separately fetched environment last-task timestamps are merged into `CloudModel`.
- Resolves the saved environment only while it remains in the catalog, otherwise falling back to the first recency-ordered environment.
- Persists only IDs that remain present in the catalog.
- Exposes the existing out-of-band `UpdateManager` refresh operation for frontends that offer explicit refresh.

`CloudModel` remains the persistence source of truth. The catalog owns no network or database state and does not expose cloud-object model internals.
### GUI migration
Update `EnvironmentSelector` to retain and subscribe to `ModelHandle<CloudEnvironmentCatalog>`.

Menu rows, selected-row highlighting, default selection, labels, live updates, and selection persistence read through the catalog. The view continues to own menu visibility, focus, telemetry, and target-specific selection mutation.

Move the full-object recency sorting helper into the cloud-environment module so repository-overlap code can retain its pure full-object scoring without depending on a GUI view module.
### Shared provider migration
Update orchestration environment default and persistence providers to use `CloudEnvironmentCatalog`. This keeps validity and recency state shared while preserving the orchestration GUI's existing case-sensitive name tie-break.
### TUI consumption
Export the catalog summary, event, and model through `app/src/tui_export.rs` on the TUI handoff branch.

`TuiHandoffModel` will subscribe to the singleton catalog rather than creating a TUI-specific projection. It will continue to own pending-handoff selection state and will use catalog methods for:

- Current environment rows and valid IDs.
- Default selection.
- Manual refresh.
- Live transition from the no-environment state.

Repository-overlap suggestion remains asynchronous and applies through `PendingHandoff::set_environment_id(..., false)`, preserving explicit-selection precedence.
## Testing and validation
- Add catalog unit coverage proving an `ObjectCreated` event refreshes only after `CloudModel` inserts the new object.
- Add catalog unit coverage for last-task timestamp reordering and both existing GUI name tie-break policies.
- Run focused `CloudEnvironmentCatalog`, environment selector, orchestration provider, and handoff pipeline tests.
- Run the complete `warp_tui` test suite after the TUI branch consumes the catalog.
- Run `./script/format`.
- Run the applicable `warp` and `warp_tui` clippy commands with warnings denied.
- Manually verify the GUI environment selector still:
  - Restores a valid saved environment.
  - Falls back to the most-recent environment.
  - Updates after environment creation or deletion.
  - Persists explicit selection.
- Manually verify the TUI handoff card transitions from no environments to configuration and preserves explicit selection during repository suggestion.
## Risks and mitigations
- **Registration ordering:** register the catalog after `CloudModel` in production and shared test initializers.
- **Premature create events:** defer projection refresh for `ObjectCreated`.
- **Ordering drift:** centralize recency ordering in the cloud-environment module and test consumer-visible order.
- **Cross-frontend coupling:** expose summaries and catalog operations only; keep GUI/TUI interaction state in their respective view or handoff models.
## Parallelization
Do not parallelize this migration. Catalog shape, registration, GUI migration, shared provider migration, and upstack TUI consumption touch one tightly coupled API boundary and must land sequentially across the Graphite stack.
