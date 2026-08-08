# Live updates for inline history search — Tech Spec

GitHub issue: [warpdotdev/warp#11474](https://github.com/warpdotdev/warp/issues/11474)

Product spec: [`specs/GH11474/product.md`](product.md)

Researched commit: [`a7a8f1ec792d9eed1702cac2dc21b8fdcf589979`](https://github.com/warpdotdev/warp/commit/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979)

## Context

Inline history is backed by an `InputBufferModel`, a `SearchMixer`, and an `InlineMenuView`. The regular history view starts a query when the suggestions mode changes to inline history, but its buffer subscription only reruns the query when `pending_initial_buffer_sync` is armed. Every later edit returns without searching, which is why the reported `sdf` → `gcloud` edit leaves the original no-results state visible.

The Cloud Mode V2 prompt-history wrapper independently implements the same one-shot gate, so changing only the regular view would leave the two user-facing history surfaces inconsistent.

Relevant code at the researched commit:

- [`app/src/terminal/input/inline_history/view.rs:158-166`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/inline_history/view.rs#L158-L166) stores the regular view's mixer, buffer model, tab-selection state, and `pending_initial_buffer_sync` flag.
- [`app/src/terminal/input/inline_history/view.rs:308-331`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/inline_history/view.rs#L308-L331) queries on menu open but ignores buffer changes after the one pending initial sync.
- [`app/src/terminal/input/inline_history/view.rs:388-420`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/inline_history/view.rs#L388-L420) turns selected rows into preview events and reruns the current query when tabs change.
- [`app/src/terminal/input/inline_history/view.rs:495-536`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/inline_history/view.rs#L495-L536) reads the opening query from the buffer and intentionally reads tab-switch queries from the mixer so a preview cannot replace the typed query.
- [`app/src/terminal/input/cloud_mode_v2_history_menu.rs:41-47`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/cloud_mode_v2_history_menu.rs#L41-L47) and [`:87-143`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/cloud_mode_v2_history_menu.rs#L87-L143) contain the parallel state, the one-shot buffer subscription, and a single undifferentiated dismissal path for both Escape and row-click dismissal.
- [`app/src/terminal/input/cloud_mode_v2_history_menu.rs:173-209`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/cloud_mode_v2_history_menu.rs#L173-L209) arms the initial sync and runs prompt-history queries from the current buffer.
- [`app/src/terminal/input.rs:5514-5581`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input.rs#L5514-L5581) applies selected command, prompt, and conversation previews through ephemeral editor-buffer replacements.
- [`app/src/terminal/input.rs:5583-5591`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input.rs#L5583-L5591) closes the menu through `close_and_restore_buffer`.
- [`app/src/terminal/input/suggestions_mode_model.rs:8-82`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/suggestions_mode_model.rs#L8-L82) snapshots the buffer only when the menu opens and returns that original snapshot when the menu closes.
- [`app/src/terminal/input/buffer_model.rs:12-59`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/input/buffer_model.rs#L12-L59) mirrors editor content and cursor position, but its content-change event carries neither cursor position nor whether the change came from the user or a history preview.
- [`crates/warp_search_core/src/mixer.rs:201-236`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/crates/warp_search_core/src/mixer.rs#L201-L236) starts each query with a new generation, and [`:334-374`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/crates/warp_search_core/src/mixer.rs#L334-L374) aborts prior async work and discards callbacks from stale generations. The history source is synchronous today, but the mixer already supplies the latest-query guarantee required by Product Behavior 11.

The core constraint is that selected rows intentionally write previews into the same editor buffer that supplies search text. Rerunning on every undifferentiated buffer event would therefore turn a preview into a new query and can create a query → selection → preview → query feedback loop. The implementation needs explicit preview provenance, not a text-equality heuristic.

## Proposed changes

1. **Add deterministic update provenance to `InputBufferModel`.**
   - Extend `InputBufferUpdateEvent` with a small source enum that distinguishes ordinary editor updates from `InlineHistoryPreview` updates, plus the cursor point captured from the same editor snapshot as `new_content`. Consumers must not combine event text with a later cursor read or infer provenance from text equality after the event arrives.
   - Add a monotonically identified preview-mutation reservation to `InputBufferModel`. Before changing the editor for a selected row, `Input` reserves a token together with the pre-mutation buffer value. While that reservation is active, every content event caused by the mutation is stamped `InlineHistoryPreview`; the reservation remains active so a multi-step workflow insertion cannot classify its second update as user input.
   - After the editor mutation, `Input` reads the actual final editor value and finalizes the same token. If the buffer model has already observed that final value, finalization clears the reservation. Otherwise the model retains the token with that expected final value and consumes it when the matching deferred buffer update arrives. Editor events are queued, and `InputBufferModel` reads the editor's current value when each event is delivered; deferred events from a multi-step workflow mutation therefore observe the finalized buffer rather than replaying intermediate strings. A nonmatching deferred update cancels the stale reservation and is classified as an ordinary editor update, so a newer user edit cannot be swallowed.
   - Comparing the model's mirrored value with the observed final editor value also handles no-op previews: when the selected preview already equals the buffer and no content event is emitted, finalization clears the token immediately. The next real user edit therefore cannot inherit stale preview provenance. Token identity prevents finalization for an older preview from clearing a newer reservation.

2. **Mark all temporary history previews at their source.**
   - In `Input::handle_inline_history_menu_event`, reserve/finalize a preview token around the `SelectCommand`, `SelectAIPrompt`, and `SelectConversation` editor mutations before calling `set_buffer_text_ignoring_undo` or the workflow insertion path.
   - Finalize with the editor's observed post-mutation buffer rather than a value predicted from the selected row. This makes the contract apply to linked-workflow previews whose insertion path can add or transform content, as well as command/prompt replacements and the empty conversation preview.
   - Do not mark `AcceptCommand`, `AcceptAIPrompt`, or conversation navigation as previews; those paths close/accept the menu and keep their existing semantics.

3. **Advance dismissal state only for non-preview updates.**
   - Change `InputSuggestionsModeModel::new` to receive its `ModelContext`, update the constructor call in `Input`, and subscribe to `InputBufferUpdateEvent` there. While inline history is open, every event not marked `InlineHistoryPreview` refreshes `buffer_to_restore` from the event's atomic `new_content`/cursor tuple; preview events leave the saved user-authored state untouched.
   - Keep this behavior specific to `InputSuggestionsMode::InlineHistoryMenu`; other inline menus retain their current snapshot-on-open behavior.

4. **Make regular inline history react to user-authored buffer updates.**
   - Replace the `pending_initial_buffer_sync` gate in `InlineHistoryMenuView` with a buffer subscription that returns only when inline history is closed or the event source is `InlineHistoryPreview`.
   - For every other content update, call `open_with_current_buffer`. That updates the mixer's query before any new selection emits its preview, so `current_query_text` remains the latest user-authored query for tab switches.
   - Remove the one-shot field and arming method. The special `/conversations` clear-and-deferred-open flow remains covered because a mode-open query reads the latest buffer and any later buffer synchronization now triggers the same live subscription rather than requiring a separate one-shot path.

5. **Apply the same contract to Cloud Mode V2.**
   - Replace the Cloud Mode V2 wrapper's `pending_initial_buffer_sync` gate with the same mode-open/preview guard and live buffer rerun.
   - Remove its one-shot field and `arm_initial_buffer_sync` call from the `/conversations` path. Keep its `PromptHistory`-only filter and compact layout unchanged.
   - Split non-accepting dismissal from row-click acceptance at the inline-menu event boundary. Escape and Down-past-the-final-result must emit the restoring close path, while a prompt row click must commit its selected prompt and close without restoring `buffer_to_restore`. Do not route both through one undifferentiated `Dismissed` branch.
   - Both history views consume the source carried by the same `InputBufferUpdateEvent`; they must not maintain separate preview-detection heuristics.

6. **Preserve existing result and dismissal mechanics.**
   - Continue to use `SearchMixer::run_query`; do not add a second debounce or generation counter. Its existing abort/generation logic is the source of truth for Product Behavior 11.
   - Continue to let `InlineMenuView` reconcile selection and emit the first result preview. The provenance guard prevents that preview from recursively rerunning the query.
   - Continue to route non-accepting dismissal through `close_and_restore_buffer`. Because the suggestions model now advances the saved snapshot only for user-authored changes, that path restores either the opening state or latest user edit as required by Product Behavior 8–9. Accepting paths close without restoration.

## Testing and validation

1. Add focused model tests for the new suggestions-mode contract:
   - Opening inline history snapshots text and cursor; a normal buffer edit advances the saved dismissal state (Product Behavior 2, 8–9).
   - An event marked with preview provenance does not advance that state (Product Behavior 4–6).
   - Closing after a user edit followed by one or more previews restores the user edit, while closing without a user edit restores the opening state.
   - Other inline-menu modes keep their existing snapshot behavior (Product Behavior 14).

2. Add focused `InputBufferModel` provenance tests before view-level regressions:
   - Preview events are classified correctly whether the observer callback runs before or after token finalization.
   - A same-text preview cancels its token without emitting a content update; the following user edit is classified as ordinary.
   - A nonmatching update cancels a pending expected-value token and remains ordinary, and an older token cannot consume or clear a newer reservation.
   - Finalizing from the editor's observed content correctly classifies a transformed, multi-step linked-workflow preview and an empty conversation preview, including queued delivery after finalization.
   - The emitted text and cursor come from one editor snapshot, so a synchronous result preview cannot pair user-authored text with the preview cursor.

3. Add regression coverage for both live-query views with deterministic history fixtures:
   - `sdf` opens to no results, editing to a known prefix produces results, and Escape restores the edited prefix rather than `sdf` (Product Behavior 1–3, 9).
   - A matching prefix edited to an unknown value reaches no results; clearing the input returns unfiltered results (Product Behavior 2–3).
   - Selecting multiple rows does not change the mixer's query or cause another query run; editing after a preview does (Product Behavior 4–7).
   - The regular view and Cloud Mode V2 prompt-history wrapper pass the same edit/preview/dismissal assertions (Product Behavior 13).
   - Cloud Mode V2 Escape restores the latest user edit, while clicking a prompt row commits the selected prompt instead of restoring the query (Product Behavior 9–10).
   - The `/conversations` clear-and-deferred-open flow still searches the cleared/latest buffer after the one-shot arming path is removed.

4. Exercise the mixer's existing stale-generation tests and add a view-level rapid-edit test that dispatches multiple buffer updates before prior work settles, then asserts the final visible state uses only the newest query (Product Behavior 11). Do not duplicate `SearchMixer` generation logic in the history tests.

5. Run the repository-required checks before publishing implementation changes:
   - `./script/format`
   - The `cargo clippy` command used by `./script/presubmit`
   - The focused Rust test targets containing the suggestions-mode and inline-history regressions
   - The relevant GUI integration test when the history flow is exercisable in `crates/integration`

6. Manually verify with `./script/run` and capture a short screen recording showing both no-results → results and results → no-results transitions, selection preview without query drift, and Escape preserving the latest user edit. Repeat the core prompt-history flow in Cloud Mode V2. Confirm keyboard focus, cursor placement, category switching, and result acceptance remain unchanged (Product Behavior 1–14).

## Risks and mitigations

- **Preview feedback loop:** an unmarked preview would rerun the query and potentially preview again. Keep token reservation at the three centralized `Select*` handlers and assert query-run counts in tests.
- **Leaked or stale provenance:** an unchanged preview or delayed event could otherwise misclassify a later edit. Cancel same-text and nonmatching reservations, consume matching tokens once, and cover both event-delivery orders in model tests.
- **Dismissal regression in other menus:** update the saved snapshot only for inline history mode and retain snapshot-on-open behavior everywhere else.
- **Surface drift:** share the preview-classification contract and mirror regression tests across the regular and Cloud Mode V2 views.

## Parallelization

Parallel implementation is not recommended. Preview provenance, dismissal snapshots, both buffer subscriptions, and their regression tests form one tightly coupled state transition; splitting them across agents would create overlapping edits in `input.rs` and `suggestions_mode_model.rs` and make intermediate branches behaviorally incomplete.

Use one local implementation worktree at `/tmp/warp-gh11474-inline-history` on branch `contributor/gh11474-inline-history`, and land the model, both views, tests, and manual evidence in one PR. The work can be ordered sequentially as: add provenance/snapshot state, mark preview producers, update both consumers, add tests, then run validation.
