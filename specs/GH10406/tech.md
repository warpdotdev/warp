# GH10406: Code review find bar lifecycle reset — tech spec
Product spec: `specs/GH10406/product.md`
Related issue: [#10406](https://github.com/warpdotdev/warp/issues/10406)
## Context
The code review find UI is owned by `CodeReviewView` and backed by a code-review-specific `CodeReviewFindModel`. The visible find bar is rendered whenever the model reports that it is open; search results store editor entity IDs, so they are only valid for the editor handles that existed when the search ran.
Relevant code:
- `app/src/code_review/code_review_view.rs:715` — `on_open` reattaches the cached code review view to the pane and reloads diffs for the selected repository.
- `app/src/code_review/code_review_view.rs:764` — `on_close` marks the view closed, unsubscribes from the diff model, removes the LSP footer, and disables metadata refresh, but currently does not reset code review find state.
- `app/src/code_review/code_review_view.rs:2242` — `show_find_bar` opens the model, seeds from selected text when available, runs a search over `editor_handles`, focuses the find bar, and updates search decorations.
- `app/src/code_review/code_review_view.rs:2278` — `close_find_bar` closes the model, clears stored results, sends close telemetry, removes editor find highlights from the currently loaded editors, and notifies the view.
- `app/src/code_review/code_review_view.rs:6842` — `editor_handles` returns only expanded file editors from the current loaded code review state. Search results from a previous loaded state can refer to editor IDs that are no longer present in this iterator.
- `app/src/code_review/code_review_view.rs:6888` — `render` overlays the `find_bar` whenever `find_model.is_find_bar_open()` is true.
- `app/src/code_review/find_model.rs:59` — `CodeReviewFindModel` stores `query_text`, `results`, `selected_match`, `search_handle`, and `is_find_bar_open`.
- `app/src/code_review/find_model.rs:145` — `focus_next_find_match` navigates by looking up a stored result's `editor_id` in the current `editor_handles`. If the result references an old editor, no current editor is found.
- `app/src/code_review/find_model.rs:306` — `run_search` aborts any previous search handle and starts asynchronous searches over the provided editor handles.
- `app/src/workspace/view/right_panel.rs:646` — `close_code_review_view` calls `CodeReviewView::on_close` for a cached repo-specific code review view.
- `app/src/workspace/view/right_panel.rs:664` — `close_active_code_review_view` closes the selected repository's cached view.
- `app/src/workspace/view/right_panel.rs:1573` — `ensure_code_review_view_exists` reopens an existing cached view or creates one for the selected repository.
- `app/src/workspace/view/right_panel.rs:1664` — `RightPanelAction::SelectRepo` closes the active view when switching repositories, updates the selected repo, then ensures the target repo view exists.
The stale-find bug happens because cached `CodeReviewView` instances survive close/reopen and repository-switch cycles. The model can remain open with the old query and result IDs while `on_open` or repo selection creates/reloads a different editor set.
## Proposed changes
### Reset code review find state on lifecycle close
Extend `CodeReviewView::on_close` to reset code review find state before the view is considered detached. This is the shared path for closing the panel and for leaving a repository during repo switch, because `RightPanelView` already calls `close_code_review_view` / `close_active_code_review_view`.
The reset should:
1. Mark the code review find bar closed.
2. Clear query text in the `CodeReviewFindModel`.
3. Clear `results`, `selected_match`, and any in-flight `search_handle`.
4. Clear the visible find input text in the `Find<CodeReviewFindModel>` view.
5. Remove find highlights from currently loaded code review editors.
6. Notify the view so the rendered find overlay disappears before the cached view can be shown again.
### Add an explicit reset API instead of overloading result clearing
`close_find_bar` currently calls `model.clear_results()`, but `clear_results` only clears `results`; it does not clear `query_text`, `selected_match`, or an in-flight search. Add a model-level reset API, for example:
- `CodeReviewFindModel::reset_find_state(&mut self)`:
  - aborts and clears `search_handle`;
  - sets `query_text` to `String::new()`;
  - sets `results` to `None`;
  - sets `selected_match` to `None`;
  - sets `is_find_bar_open` to `false`.
Keep `clear_results` only if other code still needs the narrower behavior; otherwise replace the code-review call sites with the reset API.
### Split user-initiated close from silent lifecycle reset
Keep `close_find_bar` as the user-facing close path used by Escape, the close button, and `Find` events. Update it to use the new reset API and to clear the visible input, so manual close also satisfies PRODUCT.md §19-§20.
To avoid noisy or misleading telemetry, either:
1. make `close_find_bar` send `FindBarToggled { is_open: false }` only when the bar was actually open, and call it from `on_close`; or
2. introduce a private helper such as `reset_find_bar(send_telemetry: bool, ctx)` and call it with `send_telemetry: false` from lifecycle paths.
The second option is preferable because lifecycle cleanup is not the user toggling find off; it is state invalidation caused by panel/repo lifecycle. The behavior is silent from the user's perspective.
### Clear the visible find input
`Find<CodeReviewFindModel>` owns its own `EditorView` for the query input. Resetting only `CodeReviewFindModel::query_text` is not enough because the cached `find_bar` view can keep rendering the old input text when reopened.
Use the existing `Find::set_query_text("", ctx)` API or add a clearer wrapper such as `Find::clear_query_text(ctx)` if setting an empty string through the existing method has unwanted editor-event side effects. If the clear emits a `Find::Event::Update { query: None }`, the model reset should still leave the model closed with no results; avoid triggering a new search during lifecycle reset.
### Clear highlights before editor handles become unreachable
Call the reset while the old loaded state is still available, so the existing highlight-clearing loop can visit the old editors. In `on_close`, reset before mutating state that could deallocate or detach editors. In repository switching, the existing `RightPanelAction::SelectRepo` flow closes the old active view before changing selection, so putting the reset in `CodeReviewView::on_close` is sufficient for the old repository.
### Guard against stale asynchronous searches
The reset API should abort the current `search_handle`. `run_search` already aborts the previous handle before starting a new search; lifecycle reset needs the same protection so a closing panel cannot later apply results into a model that has been closed and cleared.
As an additional defensive check, `handle_run_search_result` may early-return when `is_find_bar_open` is false or `query_text` is empty. This is not a substitute for aborting the handle, but it prevents stale completion callbacks from restoring `results` / `selected_match` after lifecycle reset.
### Repository switch behavior
No new repository-switch action is required if `on_close` owns the reset, because the switch path already funnels through:
1. `RightPanelAction::SelectRepo`;
2. `close_active_code_review_view`;
3. `close_code_review_view`;
4. `CodeReviewView::on_close`;
5. `ensure_code_review_view_exists` for the target repository.
After the reset lands, cached old repository views are clean when hidden, and cached target repository views are clean because they were reset the last time they were closed. If an additional safety net is desired, `ensure_code_review_view_exists` can assert or silently reset find state before reopening an existing cached view, but that should not be necessary once all close paths reset correctly.
## Testing and validation
Map coverage to `product.md` behavior invariants.
### Unit tests
Add unit tests near `app/src/code_review/code_review_view_tests.rs` and/or `app/src/code_review/find_model_tests.rs`.
1. `on_close_resets_find_bar_state`:
   - Build a `CodeReviewView` with a loaded test state.
   - Open find or directly seed model state with `is_find_bar_open = true`, a non-empty query, results, and selected match.
   - Call `view.on_close(ctx)`.
   - Assert `is_find_bar_open == false`, query is empty, results are `None`, selected match is `None`, and no search handle remains. Covers PRODUCT.md §2-§8.
2. `close_find_bar_clears_query_and_selection`:
   - Open find with a query, then trigger the `Find::Event::CloseFindBar` path or call `close_find_bar`.
   - Assert the model and visible query input are cleared, not just hidden. Covers PRODUCT.md §19-§20.
3. `repo_switch_closes_old_find_state`:
   - Exercise `RightPanelAction::SelectRepo` if the right-panel test harness can create two cached code review views.
   - Seed the old selected repo view with open find state, switch to another repo, and assert the old view is reset and the active view renders find closed. Covers PRODUCT.md §10-§17.
If accessing the visible `Find` query text from `code_review_view_tests.rs` is blocked by module privacy, add a small `#[cfg(test)]` getter on the find view or code review view rather than weakening production visibility.
### Integration test
Add or extend a code review integration test only if the existing harness can open multiple repositories and drive the find bar reliably without excessive fixture setup.
Recommended flow:
1. Open a prepared local repo in the code review panel.
2. Use Cmd+F, type a known query, and assert a match count/highlight appears.
3. Close and reopen the code review panel.
4. Assert the find bar is not visible.
5. Reopen find, type the query again, and assert next/previous navigation moves to a current match.
Add a second integration flow for repo switching if the harness already supports multiple repo entries in the panel dropdown; otherwise cover repo switching with unit tests and manual validation.
### Manual validation
1. Close/reopen path: open code review, open find, type a query with matches, close the panel, reopen it, confirm find is closed and highlights are gone, then open find again and verify navigation works.
2. Repository switch path: open code review with at least two repositories in the panel switcher, search in repo A, switch to repo B, confirm find is closed and the old query/highlights are gone, then search in repo B and verify navigation works.
3. In-flight search path: search for a query in a large diff and immediately close or switch repositories; confirm no stale find UI or highlights appear afterward.
### Presubmit
Run the narrow Rust tests first:
- `cargo test -p warp_app code_review::find_model_tests`
- `cargo test -p warp_app code_review::code_review_view_tests`
Then run the repository's normal presubmit or the relevant subset recommended by the `fix-errors` skill if the narrow tests reveal compile or lint issues.
## Risks and mitigations
1. **Clearing the visible input emits an update event.** Mitigate by centralizing reset sequencing so any emitted update leaves the model closed and empty, or by adding a clear method on `Find` that updates the editor without causing a search.
2. **Highlight cleanup needs the old editors.** Mitigate by resetting in `on_close` before any state invalidation or buffer cleanup that would make those handles unreachable.
3. **Spurious telemetry.** Mitigate by distinguishing user-initiated close from silent lifecycle reset, or by only sending close telemetry when the user explicitly closes find.
4. **Cached views reopened from unusual paths.** `on_close` covers the known close and switch paths. If a cached view can become hidden without `on_close`, route that path through `close_code_review_view` or add a defensive reset before reopening existing cached views.
## Parallelization
Parallel sub-agents are not recommended for implementation. The code change is small and tightly coupled to `CodeReviewView`, `CodeReviewFindModel`, and the right-panel lifecycle path, so splitting implementation would increase coordination overhead more than it would reduce wall-clock time.
## Follow-ups
1. Consider whether diff-mode changes or full diff invalidation should also reset find state in a future issue. This spec only requires close/reopen and repository-switch lifecycle resets.
