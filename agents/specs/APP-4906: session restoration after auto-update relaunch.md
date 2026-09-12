# Spec: Preserve split-pane session state across profile migration and auto-update relaunch

Linear: [APP-4906](https://linear.app/warpdotdev/issue/APP-4906/session-restoration-missing-for-right-pane-after-auto-update-relaunch)
Originating thread: https://warpdotdev.slack.com/archives/C0BDQDW8V5E/p1784640055603899
Estimate: M (3) — inferred from the two persistence/restore paths; tracker fields were unavailable in this run because `LINEAR_API_KEY` was not set.
Commit-pinned references below are anchored at `warpdotdev/warp@c353a2a4121521058bb73e8417c3a1b357c2e860`.

## PRODUCT

**Summary:** After an auto-update relaunch, Warp must restore the complete
workspace the user had before quitting, including recently created or split
terminal panes, their persisted terminal blocks, and the active execution
profile when that profile is represented by the new file-backed collection.
Restoration must remain safe across the rollout window in which old clients
persist legacy cloud-profile IDs and new clients persist stable
`ExecutionProfileId` values.

**Key design choices:**
1. **Use a dual-format, forward-compatible profile reference.** Keep the
   existing legacy `active_profile_id` SQLite column and its `SyncId` payload for
   rollback and old clients, and add a nullable stable execution-profile ID
   column for settings-backed snapshots. New clients read both forms; old
   clients ignore the new nullable column and continue restoring panes with
   their legacy behavior.
2. **Make stale profile references non-fatal.** A missing, legacy-only, or
   deleted profile reference falls back to the settings collection's required
   `default` profile and logs the condition. It must never abort restoration of
   the terminal pane, tab, or surrounding split tree.
3. **Make shutdown durability an explicit writer barrier.** The SQLite writer
   drains all events queued before termination, checkpoints the WAL on its
   write connection, verifies the checkpoint result, and only then joins and
   permits the auto-update relaunch sequence to continue. Connection-drop
   autocheckpointing remains a fallback, not the correctness contract.

**Behavior** (numbered, testable invariants from the user's view):
1. **Complete split restoration.** Starting from a workspace with a right split
   pane, recently created blocks in both panes, and a saved snapshot, an
   auto-update relaunch restores the same tab/window tree and both terminal
   panes. The right pane is not silently dropped or replaced by a one-block
   terminal because a profile lookup or WAL checkpoint failed.
2. **Settings-backed active profile survives restart.** When
   `FileBackedExecutionProfiles` is enabled, a terminal whose active profile is
   a stable `ExecutionProfileId` persists that reference and restores the same
   profile after restart. The `default` profile remains the fallback when the
   saved ID is absent or no longer exists.
3. **Legacy profile snapshots remain readable.** A snapshot written by a
   legacy-cloud client with only a serialized `SyncId` remains readable after
   the settings backend is enabled. If the ID can be mapped to the deterministic
   migrated legacy key and that key still exists, it is selected; otherwise the
   pane uses `default` and the rest of the pane is still restored. No legacy
   snapshot may cause the containing pane or tab to be discarded.
4. **Migration is one-way and rollback-safe.** During the compatibility window,
   GUI migration retains an explicitly present settings collection; otherwise
   it imports only owned, server-backed legacy profiles using `default` for the
   default profile and deterministic `legacy-<hex(server-id)>` keys for custom
   profiles; if there are no eligible legacy profiles it synthesizes `default`
   from legacy scalar settings. Migration never creates, updates, or deletes
   legacy cloud profile objects. A flag-off GUI client continues to use those
   legacy objects, and re-enabling the flag reuses the preserved collection.
5. **Invalid or stale profile data fails closed.** A malformed settings
   collection retains the existing settings subsystem's default/last-known-good
   behavior. A stale active-profile reference selects `default`, emits a
   diagnostic, and does not change the persisted collection or interrupt pane
   restoration.
6. **Shutdown persists all queued state.** Before either a normal quit or an
   auto-update relaunch, every snapshot/block event queued before the shutdown
   barrier is applied to SQLite. The writer executes an explicit WAL checkpoint
   after those writes, observes `busy = 0` and zero remaining WAL frames, and
   joins before the relaunch child is spawned.
7. **Old clients remain operational.** An older client opening a database that
   contains the new nullable stable-profile column can still read/write its
   known columns and restore the pane tree. It may not restore a settings-only
   active profile, but it must not fail database initialization or lose the
   workspace snapshot.

**Non-goals:**
- Removing legacy execution-profile cloud objects or the
  `FileBackedExecutionProfiles` rollout flag.
- Introducing bidirectional synchronization between the legacy profile-object
  store and the settings collection.
- Changing the terminal block schema or replacing the existing transactional
  app-state snapshot write.
- Changing the update download/apply state machine beyond making its shutdown
  persistence prerequisite explicit.

## TECH

**Context — current data flow and confirmed defect:**
- `app/src/pane_group/mod.rs:1711-1736 @ c353a2a4` restores
  `TerminalPaneSnapshot.active_profile_id` by calling
  `AIExecutionProfilesModel::get_profile_id_by_sync_id`. This logs a failure and
  leaves the pane on its default profile when the lookup returns `None`.
- `app/src/ai/execution_profiles/profiles.rs:61-100 @ c353a2a4` selects the
  settings collection when `FileBackedExecutionProfiles` is enabled for GUI
  launches and always for TUI launches. The settings source has no legacy
  cloud-ID map.
- `app/src/ai/execution_profiles/profiles.rs:807-888 @ c353a2a4` already
  resolves a settings profile by `ExecutionProfileId`, but
  `get_profile_id_by_sync_id` intentionally returns `None` for the settings
  source. This is the first root cause: the restore caller asks the wrong
  identity namespace.
- `app/src/ai/execution_profiles/profiles.rs:453-540 @ c353a2a4` implements
  one-time migration precedence, deterministic legacy keys, and the
  server-ID readiness guard. The implementation must preserve those semantics,
  not add a second migration path.
- `app/src/ai/execution_profiles/config.rs:22-184 @ c353a2a4` defines stable
  profile keys, the reserved `default` key, deterministic legacy IDs, and
  all-or-nothing collection validation.
- `app/src/pane_group/pane/terminal_pane.rs:473-568 @ c353a2a4` currently
  snapshots only `active_profile().sync_id()`. Settings-backed profiles have
  no sync ID, so a settings-only active profile cannot survive a new snapshot.
- `app/src/app_state.rs:192-214 @ c353a2a4` defines
  `TerminalPaneSnapshot.active_profile_id` as `Option<SyncId>`, and
  `app/src/persistence/sqlite.rs:2186-2226 @ c353a2a4` decodes that JSON
  without a settings-ID alternative.
- `crates/persistence/migrations/2025-08-27-150949_add_active_profile_to_terminal_panes/up.sql`
  adds the existing nullable `active_profile_id` column. The Diesel model and
  schema are at `crates/persistence/src/model.rs:408-432` and
  `crates/persistence/src/schema.rs:408-424 @ c353a2a4`.
- `app/src/persistence/sqlite.rs:224-236 @ c353a2a4` enables WAL with
  `wal_autocheckpoint=500` and relies on SQLite to autocheckpoint when the
  connection closes. The triage reproduction observed 105 WAL frames still
  pending during the auto-update restart, with the newest blocks missing from
  the right pane. This is the second root cause: shutdown correctness depends
  on an implicit connection-close side effect.
- `app/src/persistence/sqlite.rs:539-608 @ c353a2a4` batches events and returns
  immediately on `ModelEvent::Terminate`; `app/src/persistence/mod.rs:204-248
  @ c353a2a4` sends that event and joins the writer.
- `app/src/lib.rs:2521-2563 @ c353a2a4` terminates the persistence writer before
  tearing down PTYs and calling `autoupdate::spawn_child_if_necessary`, so this
  is the correct shutdown boundary for a synchronous checkpoint.
- `app/src/workspace/global_actions.rs:131-168 @ c353a2a4` enqueues complete
  app snapshots through the same writer channel. `save_app_state` is already a
  transaction (`app/src/persistence/sqlite.rs:876-904 @ c353a2a4`), so this
  change must make the transaction durable rather than rewrite its contents.

### Design alternatives

**Active-profile persistence and compatibility:**
- **Dual columns with dual-read (chosen).** Add a nullable
  `active_profile_execution_id TEXT` column and a corresponding optional
  `ExecutionProfileId` field to `TerminalPaneSnapshot`. Keep
  `active_profile_id` unchanged for legacy `SyncId` data. Settings-backed
  snapshots write the stable field; legacy-backed snapshots write the legacy
  field. New readers prefer the stable field for the settings source and the
  legacy field for the legacy source, with deterministic legacy-ID conversion
  and default fallback for old/stale rows. Pros: old clients retain a
  parseable schema and old rows remain readable; new profiles can actually
  survive restart. Cons: one nullable migration and a short compatibility
  period with two fields.
- **Map a legacy `SyncId` directly to a settings key (rejected as the sole
  design).** Derive `legacy-<hex(server-id)>` during restore and leave the
  snapshot shape unchanged. Pros: smallest diff. Cons: newly created
  settings-only profiles have no `SyncId` to persist, and `ClientId` snapshots
  cannot be mapped deterministically; custom active profiles would still reset
  on the next split/restart.
- **Ignore all profile IDs when the settings backend is enabled (rejected as
  the sole design).** This would avoid the `None` lookup and safely use
  `default`. It is safe for old rows but regresses the user's selected custom
  profile and does not satisfy active-profile restoration.

**Migration contract:**
- **Presence-based, one-way import (chosen).** Preserve an explicit collection;
  otherwise import owned, fully server-backed legacy objects with deterministic
  keys; otherwise synthesize a default from legacy scalar settings. Keep
  legacy objects and the flag for rollback. Pros: matches the existing
  `migrate_settings_profiles` implementation and allows old/new clients to
  coexist without destructive writes.
- **Live bidirectional bridge (rejected).** Continuously mirror settings and
  cloud objects. It would create conflict loops, make rollback nondeterministic,
  and violate the existing one-way migration contract.

**WAL durability:**
- **Checkpoint in the writer termination barrier (chosen).** Add a
  checkpoint-and-terminate control event or equivalent termination handling
  inside the SQLite writer. FIFO ordering drains writes first; the writer runs
  `PRAGMA wal_checkpoint(TRUNCATE)`, validates the result, then returns and is
  joined. Pros: one write connection owns the checkpoint, no cross-thread
  race, and both normal quit and auto-update use the same guarantee.
- **Rely on `SqliteConnection` drop/autocheckpoint (rejected).** This is the
  current behavior and is contradicted by the triage observation of pending WAL
  frames during relaunch.
- **Open a second connection from the app callback (rejected).** It can race the
  writer or hold a reader lock, and it would weaken the event-drain ordering.

### Proposed changes

1. **Persist a stable profile reference without breaking old rows.**
   - Add a nullable `active_profile_execution_id TEXT` column in a new
     `crates/persistence/migrations/` migration, update
     `crates/persistence/src/schema.rs` and `crates/persistence/src/model.rs`,
     and include the field in `NewTerminalPane`/`TerminalPane` reads and writes.
   - Extend `TerminalPaneSnapshot` with an optional `ExecutionProfileId`
     reference while retaining `active_profile_id: Option<SyncId>`.
   - In `TerminalPane::snapshot`, write the stable reference when the active
     model uses the settings backend; keep writing the legacy field when the
     legacy cloud-object backend is authoritative. The settings collection's
     required `default` ID must be treated like any other valid stable ID.
   - In SQLite restore, decode both nullable columns. Preserve the old JSON
     decoder for `active_profile_id`; malformed profile-reference JSON is
     treated as absent rather than making `read_node` fail.
   - In `PaneGroup` restore, use the stable ID with
     `get_profile_by_id` when the settings backend is active. In legacy mode,
     retain `get_profile_id_by_sync_id`. For a legacy `SyncId` encountered by
     the settings backend, derive the deterministic legacy key only when it is
     a server ID and the key exists; otherwise select `default`. Every failed
     lookup logs a diagnostic and continues creating the terminal pane.
   - When a settings collection reload removes an active stable ID, retain the
     existing `active_profiles_per_session` pruning behavior and let the pane
     resolve to `default`.
   - Do not mutate/delete legacy cloud objects during this change. Keep the
     existing migration event subscriptions and pending-server-ID retry.

2. **Make the SQLite shutdown boundary durable.**
   - Add a writer-owned checkpoint helper in `app/src/persistence/sqlite.rs`
     that runs `PRAGMA wal_checkpoint(TRUNCATE)` on `current_conn`, handles a
     busy/error result through the existing error-reporting path, and exposes
     the observed busy/log/checkpointed counts to tests or equivalent
     diagnostics.
   - Change the termination control path so the writer processes every normal
     event queued before the barrier, checkpoints only after those events have
     completed, and returns only after the checkpoint attempt. The barrier must
     not be deduplicated away or treated as an ordinary model event.
   - Keep `PersistenceWriter::terminate` synchronous: send the barrier, join
     the writer, and do not call `autoupdate::spawn_child_if_necessary` until
     that join returns. Producers must stop enqueueing persistence work before
     this barrier, preserving the existing ordering in `on_will_terminate`.
   - If the checkpoint reports a busy/error result, report the failure with
     database context and preserve the already-committed WAL data; do not claim
     a successful checkpoint in logs or tests. The process may continue its
     existing termination path, but the failure must be observable for follow-up
     remediation.
   - Keep logout pause/remove/reconstruct semantics unchanged; the explicit
     checkpoint is for the normal termination barrier and must not delete or
     reopen the database.

3. **Add regression coverage next to the affected paths.**
   - Extend execution-profile tests for settings-backed stable-ID lookup,
     deterministic legacy-ID fallback, stale-ID default fallback, migration
     precedence, and rollback/flag-off behavior.
   - Add a pane restoration test that creates a terminal snapshot with a
     settings `ExecutionProfileId`, restores it through `PaneGroup`, and asserts
     the selected profile and pane tree are present. Add a legacy snapshot case
     proving a stale `SyncId` does not drop the pane.
   - Add SQLite writer tests using a temporary database that enqueue a block and
     app snapshot, send the termination barrier, join the writer, reopen the
     database, and assert both records are present and the WAL checkpoint result
     has zero remaining frames. Include the barrier-ordering case where multiple
     events are queued before termination.
   - Add migration coverage for opening an old database with a null new column
     and for the schema migration itself; verify the old `active_profile_id`
     payload is not rewritten or deleted.

**Open questions resolved:**
- *What identity should settings-backed snapshots persist?* A stable
  `ExecutionProfileId` in a new nullable column. The current `SyncId` field is
  retained exclusively for legacy/old-client compatibility.
- *What should an old `SyncId` do after migration?* If it is a server ID and
  its deterministic `legacy-<hex(server-id)>` key exists, select that profile;
  otherwise select `default`, log, and continue restoring the pane. Client IDs
  have no deterministic imported key and therefore use `default`.
- *Should migration mutate legacy cloud objects?* No. The existing one-way,
  presence-based migration and rollback contract remains authoritative.
- *Where must WAL checkpointing happen?* On the SQLite writer connection after
  all pre-termination events and before its joined handle returns; the app
  callback remains responsible for joining before update relaunch.
- *What if checkpointing is busy or errors?* Report the failure with context
  and do not report success; preserve the WAL and existing committed data rather
  than deleting/reconstructing the database. A future retry/diagnostic can act
  on the observable failure.
- *Does the UI need a new setting or feature flag?* No. This is a correctness
  fix in the existing rollout path; the existing profile feature flag controls
  source selection and is not replaced.

**Risks / blast radius:**
- **Schema compatibility:** the new nullable column must be additive and
  explicitly selected by new Diesel code. Older binaries must continue using
  their known columns; the migration must never rewrite `active_profile_id`.
- **Stale profile references:** settings files or cloud updates can delete a
  profile between snapshot and restore. The default fallback and pane-first
  restoration order prevent a stale profile from dropping a tab.
- **Migration divergence:** old and new clients can edit different persistence
  representations during rollback. This is accepted for the existing rollout
  window; no destructive reconciliation is added.
- **Shutdown latency:** a full/truncate checkpoint can add bounded latency to
  quit/relaunch. It is preferable to silently losing recent blocks; log the
  duration alongside the existing writer shutdown timing.
- **Writer ordering/races:** a barrier sent while another producer is still
  enqueueing could leave work after the checkpoint. Preserve the existing
  shutdown ordering (notebooks first, writer termination before PTY teardown)
  and add a test that verifies all pre-barrier events are durable.
- **User-visible verification:** this is a GUI-visible session restoration
  change. Code tests and presubmit are necessary but not sufficient; the
  implementation must capture computer-use visual proof after a relaunch.

## Validation & verification criteria (must ALL pass before merge)

1. **Original repro — split pane and recent blocks survive relaunch.** On a
   client with `FileBackedExecutionProfiles` enabled, create a terminal,
   execute commands that produce multiple persisted blocks, split a right pane,
   execute at least one command in the right pane, and trigger the real
   auto-update apply/relaunch flow. After the new process starts, both panes,
   their split orientation, and all blocks that existed before termination are
   present. The right pane must not show only its first block. Checked by the
   Warp GUI integration/relaunch harness or an equivalent real-client run, with
   before/after evidence attached.
2. **Active settings profile regression (fails before / passes after).** A new
   test constructs a settings-backed `TerminalPaneSnapshot` with a stable
   `ExecutionProfileId`, restores the pane, and asserts
   `AIExecutionProfilesModel::active_profile` returns that ID. The same test
   asserts that a missing/stale ID falls back to `default` while the terminal
   pane remains in the restored tree. Checked by the focused `warp` unit test
   covering `PaneGroup`/execution profiles.
3. **Legacy snapshot compatibility regression.** A test loads a snapshot
   containing only the pre-change `active_profile_id` JSON (`SyncId`) against
   the settings backend, verifies deterministic server-ID mapping when the
   migrated key exists, and verifies default fallback (without an error result
   or dropped pane) when it does not. Checked by the focused persistence/pane
   restoration tests.
4. **Migration precedence and rollback contract.** Tests cover: explicit
   settings collection wins; owned server-backed legacy profiles import with
   `default` plus deterministic custom IDs; pending client-only profiles defer
   migration and retry after server ID assignment; no legacy object is mutated,
   created, or deleted; flag-off uses legacy objects; re-enabling uses the
   preserved settings collection. Checked by
   `cargo nextest run -p warp --lib execution_profiles`.
5. **Additive SQLite migration.** A migration test opens a pre-change database,
   applies the new migration, and verifies the new stable-profile column is
   nullable while the existing `active_profile_id` value is byte-for-byte
   unchanged. A schema/model test verifies an older-client-style query remains
   valid against the migrated database. Checked by the persistence migration
   and SQLite tests.
6. **Writer drains events before checkpoint.** A new temporary-database test
   queues multiple `SaveBlock`/`Snapshot` events followed by the termination
   barrier, joins the writer, reopens the database, and finds every queued
   record. The test must fail if the barrier returns before earlier events are
   applied. Checked by the focused SQLite writer test.
7. **Explicit checkpoint result is clean.** The writer termination test
   observes `PRAGMA wal_checkpoint(TRUNCATE)` (or the equivalent helper result)
   after the barrier and asserts `busy = 0` and zero remaining WAL frames. It
   also exercises the error/busy reporting path without treating it as a
   successful checkpoint. Checked by the focused SQLite test suite.
8. **Auto-update ordering.** A test or instrumentation assertion proves
   `PersistenceWriter::terminate` has joined after checkpoint completion before
   `autoupdate::spawn_child_if_necessary` is invoked. Normal quit and
   auto-update relaunch use the same ordering. Checked by the shutdown callback
   test/harness and code review of `app/src/lib.rs`.
9. **No collateral damage to persistence.** Existing SQLite tests for app-state
   transactions, block restoration, logout pause/remove/reconstruct, event
   deduplication, and normal writer shutdown remain green. Checked by
   `cargo nextest run -p warp --lib persistence` (or the repository-equivalent
   persistence test target).
10. **Presubmit.** `./script/presubmit` passes from the repository root,
    including `./script/format`, workspace clippy with `-D warnings`, builds,
    and the configured test suites.
11. **User-facing visual proof.** Use computer use against the built Warp GUI
    to capture (a) the split workspace before termination with recent blocks in
    both panes and (b) the restored workspace after the auto-update/relaunch
    path. The after image must visibly show the right pane still present with
    its recent block history and no restoration error replacing the pane.
    Attach the screenshot/video proof to the task record and PR; media is not
    committed to the branch. If the environment cannot perform a real update,
    exercise the same `on_will_terminate` + relaunch sequence with the
    integration harness and state the limitation explicitly rather than
    claiming visual verification.

