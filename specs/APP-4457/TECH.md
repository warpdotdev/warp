# APP-4457 — Shared-session scrollback restore optional active block

Linear: [APP-4457](https://linear.app/warpdotdev/issue/APP-4457/fix-shared-session-scrollback-restore-crash-when-active-block-is)
Sentry: [7475296168](https://warpdotdev.sentry.io/issues/7475296168/?alert_rule_id=15217488&alert_type=issue&notification_uuid=93ccd349-0267-4363-90d3-925a1c8ca43f&project=5701177&referrer=slack)

## Context

Shared-session scrollback restoration crashed because the viewer restore path inferred that the final serialized block was always the unfinished active prompt block. That contract is too strong: the sharer serializes only blocks that pass shared-session visibility rules, so the active block can be absent when it is hidden or otherwise not scrollback-eligible.

The producer side lives in `app/src/terminal/shared_session/mod.rs`. `SharedSessionScrollbackType` starts the serialized range at the selected block or active block (`mod.rs:178`), and `to_scrollback` filters each block through `Block::is_scrollback_block_for_shared_session` before serializing it (`mod.rs:199`). That filter is implemented in `app/src/terminal/model/block.rs:1506`; it excludes hidden/restored blocks and therefore does not guarantee the active block will be emitted.

The consumer side lives in `app/src/terminal/model/blocks.rs`. `BlockList::load_shared_session_scrollback` handles initial viewer restore (`blocks.rs:711`), and `BlockList::append_followup_shared_session_scrollback` handles cloud/follow-up session append (`blocks.rs:742`). Both paths need the same scrollback shape so initial joins and follow-up joins do not diverge.

The affected Sentry issue points at a debug assertion in the old restore shape: after `split_last()`, the final item was asserted to be unfinished. When the final serialized item was a completed block, that assertion failed instead of restoring completed history and leaving the viewer with a valid active block.

## Proposed changes

Treat a shared-session scrollback snapshot as completed historical blocks plus an optional active prompt block. The active block is present only when the final serialized block is unfinished (`completed_ts.is_none()`). All other serialized blocks are historical blocks and must be completed before they are restored.

Add a small private parser near the restore code:

- `SharedSessionScrollbackBlocks` in `app/src/terminal/model/blocks.rs:548`
- `completed_blocks: &[SerializedBlock]`
- `active_block: Option<&SerializedBlock>`

The parser calls `split_last()` only to classify the final item. If the final item has `completed_ts.is_none()`, it becomes the optional active block and the prefix becomes completed history. Otherwise the full slice is completed history and `active_block` is `None`.

Update `load_shared_session_scrollback` to:

1. Finish any pre-existing unfinished local active block before restore.
2. Restore each parsed completed block only if it has both `start_ts` and `completed_ts`.
3. Restore the optional active block when present.
4. Otherwise call `ensure_active_block_after_shared_session_scrollback` to create a fresh post-bootstrap active block if the current active block is finished.

Update `append_followup_shared_session_scrollback` to use the same parsed shape while preserving its existing duplicate-block behavior:

1. Skip completed blocks whose IDs already exist in the viewer model.
2. Finish the current active block before restoring new completed blocks or a new active block.
3. Skip duplicate active blocks by ID.
4. If the follow-up snapshot has no active block, keep the current unfinished active block when one exists; otherwise create a fresh hidden active block.

Update producer/consumer comments so they describe the actual contract: active prompt state is included only when the active block is scrollback-eligible, not unconditionally.

## Testing and validation

Add regression coverage for both restore paths:

- `app/src/terminal/shared_session/mod_tests.rs:304` — initial restore with a completed final serialized block restores all completed blocks and creates a fresh hidden active block.
- `app/src/terminal/shared_session/viewer/event_loop_tests.rs:284` — follow-up append with a completed final serialized block preserves duplicate skipping, appends new completed history, and leaves a fresh hidden active block.

Keep existing active-block-present tests unchanged so normal live-sharing behavior remains covered:

- `test_loading_scrollback`
- `test_loading_scrollback_in_alt_screen`
- `test_append_followup_scrollback_skips_duplicates`

Validation commands:

- `cargo nextest run -p warp --lib test_loading_scrollback`
- `cargo nextest run -p warp --lib test_append_followup_scrollback`
- `git --no-pager diff --check -- app/src/terminal/model/blocks.rs app/src/terminal/model/block.rs app/src/terminal/shared_session/mod.rs app/src/terminal/shared_session/mod_tests.rs app/src/terminal/shared_session/viewer/event_loop_tests.rs specs/APP-4457/TECH.md`

Do not use `cargo fmt --all` or file-specific `cargo fmt`. If formatting is required before review, use the repo-standard `cargo fmt`.

## Parallelization

Sub-agents are not useful for this change. The implementation is small and tightly coupled across one restore parser, two restore paths, nearby contract comments, and focused tests. Parallel edits would introduce coordination overhead and risk conflicting changes in the same files. Validation is also fast enough to run sequentially.

## Risks and mitigations

The main risk is accidentally changing the active-block-present path used by normal live sharing. Keep that path covered by the existing `test_loading_scrollback` and follow-up duplicate tests, and make the new parser classify an active block solely by the persisted completion state.

Follow-up append has a second risk: completed history from the original snapshot may be replayed during follow-up attach. Preserve ID-based duplicate skipping for completed and active blocks so the follow-up path appends only newly observed blocks.
