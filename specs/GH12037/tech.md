# Async-find query-refinement fast path — Tech Spec
Product spec: `specs/GH12037/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/12037
Inspected commit: `6c4125ce193e06680af27d9a4eb6c4474b41ac95`

## Context
Async terminal find already has a refinement hook, but the hook still performs a full restart. `AsyncFindController::start_find` detects strict-prefix literal refinements before constructing a fresh config, then calls `filter_results_for_refinement` and returns ([`app/src/terminal/find/model/async_find.rs:626 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L626-L647)). The current `filter_results_for_refinement` immediately rebuilds full `collect_block_info`, clears all `BlockFindResults`, enqueues every block through `FindWorkQueue::enqueue_full_scan`, and respawns the same background task as a normal search ([`app/src/terminal/find/model/async_find.rs:1036 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L1036-L1131)).

The relevant existing data is already in memory. `BlockFindResults` stores terminal matches by `(BlockIndex, GridType)`, AI/rich-content matches by `EntityId`, and total-index maps used for ordering ([`app/src/terminal/find/model/async_find.rs:243 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L243-L254)). `BlockFindResults::total_match_count` already gives the zero-candidate check ([`app/src/terminal/find/model/async_find.rs:257 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L257-L261)). `process_message` appends full-block results, auto-focuses the first known match, merges dirty ranges, and clamps focus ([`app/src/terminal/find/model/async_find.rs:718 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L718-L803)). Focus ordering is recomputed from the total-index maps in `update_cached_focused_match` ([`app/src/terminal/find/model/async_find.rs:506 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find.rs#L506-L617)).

The async work queue supports full terminal-block scans and AI-block scan requests through `enqueue_full_scan` ([`app/src/terminal/find/model/async_find/work_queue.rs:70 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find/work_queue.rs#L70-L96)). The background task already processes `FullBlock`, `DirtyRange`, and `AIBlock` work items and emits `Done` when the queue drains ([`app/src/terminal/find/model/async_find/background_task.rs:56 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/async_find/background_task.rs#L56-L128)). This means the safe first implementation can keep the background scanner unchanged and change only the work set constructed by `filter_results_for_refinement`.

The legacy sync block-list path is separate. It stores `BlockListMatch` snapshots in `BlockListFindRun` and has its own `rerun_on_block` splicing logic ([`app/src/terminal/find/model/block_list.rs:281 @ 6c4125c`](https://github.com/warpdotdev/warp/blob/6c4125ce193e06680af27d9a4eb6c4474b41ac95/app/src/terminal/find/model/block_list.rs#L281-L580)). This spec intentionally scopes the implementation to async find, matching the existing refinement hook and the pre-stable async-find rollout.

## Proposed changes

### 1. Replace the full refinement restart with a candidate-block restart
Keep the existing refinement predicate in `start_find`: regex disabled, same case-sensitivity, and `new_query` is a strict string prefix extension of the old query. Inside `filter_results_for_refinement`, build the new `AsyncFindConfig` first and return through `clear_results` for empty/whitespace queries exactly as today.

Only take the candidate fast path when the previous broader run has reached `AsyncFindStatus::Complete`. If the broader run is still `Scanning` or has been cancelled back to `Idle`, fall back to the current full refined async search path so the refined query cannot miss blocks that the broader query had not scanned yet.

Before clearing `block_results`, snapshot the candidate set from the previous completed results:

- Terminal candidates: every `BlockIndex` that has a non-empty match vec in `block_results.terminal_matches` for either `GridType::Output` or `GridType::PromptAndCommand`.
- Rich-content candidates: every `EntityId` that has a non-empty match vec in `block_results.ai_matches`.
- Previous focus candidate: the currently cached focused terminal/AI match, if any, so focus can be restored when the same occurrence survives the refinement.

Deduplicate terminal block indices. Preserve enough ordering metadata to enqueue newest-first work and repopulate total-index maps. Prefer reusing `terminal_total_indices` / `ai_total_indices` for candidates already known to the controller, but validate that each terminal block still exists under the current `BlockList` before enqueueing it.

Then run the same cancellation/generation setup as a normal restart, but enqueue only candidate blocks/views instead of all blocks:

1. `cancel_current_find()` to stop the old generation and close the previous queue.
2. Set `current_config`, `block_sort_direction`, `current_find_options`, `status = Scanning`, and clear old match storage.
3. Build `Vec<BlockInfo>` from candidates rather than `collect_block_info(model.block_list(), &config)`.
4. Respect `config.blocks_to_include` by intersecting candidate terminal blocks with the selected block set. Rich-content candidates should only be included when the active find scope allows rich content in the same way a normal scoped find would.
5. Repopulate `terminal_total_indices` / `ai_total_indices` from the candidate `BlockInfo` list.
6. Create result/throttle streams, enqueue the candidate `BlockInfo` list with `enqueue_full_scan`, and spawn the background task.

If the previous completed total match count is zero or the candidate list becomes empty after validation/scope intersection, do not spawn a background task. Instead, update `current_config`, `block_sort_direction`, and `current_find_options`, clear results, set focus/cache to `None`, set `status = Complete`, and emit a throttled or direct `FindEvent::RanFind` through the existing caller path. Because `TerminalFindModel::run_find` emits `RanFind` after `start_find`, `filter_results_for_refinement` does not need to add a new event unless tests reveal no repaint occurs in direct controller usage.

### 2. Preserve result ordering and focus semantics
A candidate-block restart still uses the existing background scanner, `process_message`, and `update_cached_focused_match`, so final ordering should match a fresh full scan as long as the candidate `BlockInfo` list carries correct total indices. Add a small helper to build ordered candidate `BlockInfo` from previous result maps:

- `fn collect_refinement_candidate_block_info(&self, block_list: &BlockList, config: &AsyncFindConfig) -> Vec<BlockInfo>`
- or an equivalent private method on `AsyncFindController` if borrowing `self` and the terminal-model lock is simpler.

The helper should sort the returned `BlockInfo` newest-first, matching `collect_block_info`, before enqueueing. Sorting by `TotalIndex` descending works for both terminal and rich-content candidates and keeps background scan order consistent with normal async find.

For focus preservation, prefer a narrow implementation:

1. Snapshot the old focused match identity before clearing results.
2. After each result batch, existing auto-focus/clamp behavior remains valid.
3. After scan completion, if the old focused span survived, set `focused_match_index` to its new index and update the cache.

If preserving focus by span requires invasive plumbing, keep the current async behavior of focusing the first available match and document the product open question. The minimum requirement is no stale/out-of-range focus and parity with a fresh scan once focus is moved.

### 3. Keep row-level candidate scanning out of the first patch unless profiling requires it
The issue suggests scanning only rows that previously contained matches. That is theoretically valid for strict literal refinements, but the current queue and message semantics are optimized for full blocks and dirty ranges. Reusing `DirtyRange` work items for refinement would require careful handling of:

- converting previous `AbsoluteMatch` rows back to current relative row ranges while accounting for scrollback truncation;
- expanding candidate ranges enough to catch matches that extend beyond the old match span, including wrapped terminal lines and multi-row matches;
- merging many small ranges per block/grid without producing duplicate or out-of-order matches;
- preserving final parity with a fresh full scan for long queries and wide-character rows.

Therefore the first implementation should be block-granular. It removes the worst full-buffer rescan by skipping every block with zero prior matches while keeping the existing scanner as the correctness oracle inside candidate blocks. Add row-level refinement as a follow-up only if benchmarks show matched blocks are still too expensive.

A later row-level design should introduce explicit refinement work-item semantics instead of overloading dirty-range invalidation, for example `FindWorkItem::CandidateRange { block_index, grid_type, row_range, num_lines_truncated }`, with tests proving fresh-scan parity for wrapped lines, wide characters, multi-row matches, overlapping candidate ranges, and truncation.

### 4. Test-only hooks for queue composition
Current tests can observe controller state and block results but not the exact initial work queue after a refinement. Add minimal `#[cfg(test)]` helpers rather than production logging:

- expose candidate collection as a pure helper that tests can call directly;
- or expose `FindWorkQueue::len` plus a way to inspect pending work-item block indices under `#[cfg(test)]`.

Prefer testing the candidate helper directly and then using an end-to-end controller test to assert refined results, because inspecting the queue after spawning the background task can be timing-sensitive.

### 5. No production-code changes outside async find unless needed by tests
Expected production files:

- `app/src/terminal/find/model/async_find.rs` — refinement candidate helper and `filter_results_for_refinement` rewrite.
- `app/src/terminal/find/model/async_find_tests.rs` — unit and async controller coverage.

Avoid changing UI code, feature flags, telemetry, sync `block_list.rs`, or `background_task.rs` for the block-level implementation. If a helper is added to `work_queue.rs` solely for test observability, keep it behind `#[cfg(test)]`.

## Testing and validation

### Unit tests
Add tests to `app/src/terminal/find/model/async_find_tests.rs`:

1. `test_refinement_candidates_include_only_blocks_with_prior_terminal_matches`
   - Seed `BlockFindResults` with matches in two terminal blocks and no matches in a third existing block.
   - Assert candidate collection returns only the two matched block indices.

2. `test_refinement_candidates_deduplicate_prompt_and_output_matches`
   - Seed matches for both `PromptAndCommand` and `Output` in the same block.
   - Assert the candidate block appears once.

3. `test_refinement_zero_previous_matches_completes_without_queue`
   - Start or seed a broader query with zero matches, refine it, and assert `status == Complete`, `match_count() == 0`, and no work queue/task handle is active.

4. `test_refinement_respects_blocks_to_include_in_results`
   - Seed matches in blocks 1 and 2, refine with `blocks_to_include_in_results = Some(vec![2])`, and assert only block 2 is a candidate.

5. `test_non_refinement_still_collects_all_blocks`
   - Exercise deletion, unrelated replacement, regex enabled, and case-sensitivity changed. Assert the normal `start_find` path is used by checking final parity and candidate helper is not invoked, or by checking all blocks with matches can still be found.

6. `test_refinement_candidate_order_is_newest_first`
   - Seed total indices out of insertion order and assert candidate `BlockInfo` is sorted by descending `TotalIndex`.

7. `test_refinement_skips_removed_or_truncated_candidate_blocks`
   - Seed results for a block that no longer exists in the `BlockList` and assert it is not enqueued.

### End-to-end parity tests
Add an async test that builds a mock terminal model with many blocks, runs a broad query, then refines it:

1. Run a normal async find for `foo` to completion.
2. Run refinement to `foobar` through `start_find`.
3. Independently run a fresh async find or sync `run_find_on_block_list` for `foobar` on the same terminal state.
4. Assert same final match count, same terminal ranges, and same ordering for both `MostRecentLast` and `MostRecentFirst` sort directions.

Add a sparse-match variant with many no-match blocks and a test-only candidate helper assertion that only the previously matched blocks are included.

### Focus tests
Add controller tests for the product focus invariants:

- If the focused match's span still matches the refined query, focus remains on that span or, if span preservation is deferred, focus remains valid and deterministic.
- If the focused match disappears but other matches remain, focus clamps to a valid match.
- If all matches disappear, `focused_match_index()` returns `None`.

### Race/generation tests
Extend existing cancellation/generation coverage with rapid refinement:

1. Start a broad query that has pending work.
2. Refine it before the old queue drains.
3. Deliver or simulate a stale result message from the old generation.
4. Assert the active refined query's count/highlights are not updated by stale results.

If direct stale-message simulation is hard, rely on the existing generation-guard tests and add a regression test that rapid `foo` → `foob` → `foobar` leaves the controller's `current_config.query == "foobar"`, status eventually complete, and final matches equal a fresh scan for `foobar`.

### Manual validation
1. Create a session with thousands of blocks where only a few contain `foo` and fewer contain `foobar`.
2. Search `foo`, wait for completion, then type `bar` one character at a time.
3. Confirm the find bar remains responsive, final counts are correct, and CPU usage is lower than a fresh full-buffer scan on each keystroke.
4. Repeat with zero matches for the broad query and confirm refinements complete immediately.
5. Repeat with regex enabled and with case-sensitivity toggles to confirm they still use the normal full-search path.

Run before merging:

- `cargo test -p warp --lib terminal::find::model::async_find::tests`
- `cargo nextest run -p warp --lib terminal::find::model::async_find::tests` if the package/test target supports nextest in the local checkout.
- `cargo fmt`
- Repository presubmit or the narrower Rust lint command used by the async-find owners if available.

## Parallelization
Parallel sub-agents are not recommended for the first implementation. The production change should stay tightly scoped to `AsyncFindController::filter_results_for_refinement` and nearby tests; splitting implementation across agents would increase merge risk without saving much wall-clock time.

A useful parallel split for later benchmarking would be:

- Agent A: implement and validate the block-level fast path in a local worktree such as `../warp-refinement-block-fastpath` on branch `oz-agent/refinement-block-fastpath`.
- Agent B: independently prototype row-level candidate scanning in `../warp-refinement-row-prototype` on branch `oz-agent/refinement-row-prototype`, producing benchmark results and parity failures rather than merge-ready code.
- Lead: merge only Agent A for the first PR, then use Agent B's results to decide whether row-level scanning should become a follow-up issue.

## Risks and mitigations

### Risk: false negatives when the previous broad scan was incomplete
If refinement starts while the broad query is still scanning, candidate collection would only know about blocks already scanned. That could omit matches that a completed broad scan would have found in older blocks.

Mitigation: gate the candidate fast path on `status == Complete`. Incomplete broader runs fall back to the normal full async search for the refined query. Add tests that rapid `foo` → `foob` refinement during a scan still produces fresh-scan parity.

### Risk: focus identity preservation becomes too invasive
Preserving exact focus by span across a restart may require mapping old `AbsoluteMatch`/AI match IDs into the new ordered match list after results stream in.

Mitigation: first ensure focus is always valid and final ordering is correct. Add exact focus preservation only if the implementation can do it without weakening generation safety.

### Risk: block-level scanning is not enough for very large matched blocks
A block with huge output and one broad-query match still gets fully rescanned for every refinement.

Mitigation: block-level scanning is safe and removes no-match blocks, which is the largest waste described in the issue. Keep row-level candidate scanning as an explicit follow-up with correctness tests around wrapped lines and absolute-row conversion.

### Risk: candidate helper accidentally includes empty result entries
`terminal_matches` can contain empty vectors after dirty-range updates. Including empty vectors would enqueue unnecessary work.

Mitigation: candidate collection must filter `Vec::is_empty()` for terminal and AI maps. Add a unit test that empty entries are ignored.

### Risk: scoped find regresses
`blocks_to_include_in_results` is part of `AsyncFindConfig`, and normal `collect_block_info` currently handles selected-block scope.

Mitigation: candidate collection must intersect prior matched terminal blocks with the selected block list and keep selected-block tests. Do not reuse unscoped candidate maps blindly.

## Follow-ups
- Benchmark block-level refinement on synthetic large scrollback and real dogfood sessions before enabling async find broadly.
- Revisit whether incomplete broad scans can safely accelerate refinements without sacrificing fresh-scan parity.
- Prototype explicit row-level candidate work items if matched blocks remain a measurable CPU bottleneck.
- Consider a similar refinement optimization for the legacy sync path only if the sync path remains relevant after async find is stable.
