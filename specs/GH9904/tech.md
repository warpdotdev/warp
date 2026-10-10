# GH9904: Tech Spec — Rename auto-generated plan titles

Product spec: `specs/GH9904/product.md`

## Problem

AI plan titles are stored on `AIDocument` and rendered in the `AIDocumentView` pane header, but there is no user-facing rename flow. Agent streaming updates can also overwrite the title through `apply_streamed_agent_update`, so a user-provided title needs both a UI path and a model-level lock that prevents later agent title updates from replacing it.

The implementation spans the plan pane header, AI document model, local SQLite restoration, Warp Drive notebook sync, conversation artifacts, and search/listing surfaces that read plan titles.

## Relevant code

- `app/src/ai/ai_document_view.rs:138` — `AIDocumentAction` currently has plan header actions but no rename actions.
- `app/src/ai/ai_document_view.rs:166` — `AIDocumentView` owns document ID/version, header button state, and pane configuration.
- `app/src/ai/ai_document_view.rs:521` — `refresh` makes earlier versions and streaming-created documents read-only, then updates the pane header title.
- `app/src/ai/ai_document_view.rs:603` — `render_plan_header` renders the centered static title via `render_pane_header_title_text`.
- `app/src/ai/ai_document_view.rs:1018` — action handling for plan-specific header actions.
- `app/src/ai/ai_document_view.rs:1272` — `render_header_content` supplies the custom plan header.
- `app/src/ai/document/ai_document_model.rs:103` — `AIDocument` stores `title`, `version`, editor model, sync ID, and user edit status.
- `app/src/ai/document/ai_document_model.rs:466` — `apply_streamed_agent_update` currently overwrites `doc.title`.
- `app/src/ai/document/ai_document_model.rs:714` — `apply_persisted_content` restores persisted content and optional title from SQLite.
- `app/src/ai/document/ai_document_model.rs:759` — `update_title` can update a title but does not normalize input, lock user titles, enqueue persistence, or sync Warp Drive.
- `app/src/ai/document/ai_document_model.rs:1005` — `persist_content_to_sqlite` emits title through `ModelEvent::SaveAIDocumentContent`.
- `app/src/ai/document/ai_document_model.rs:1024` — `maybe_update_cloud_notebook_data` updates notebook data but preserves notebook title.
- `app/src/ai/document/ai_document_model.rs:1145` — `create_notebook_in_plan_folder` creates `CloudNotebookModel` with the document title.
- `app/src/server/cloud_objects/update_manager.rs:2001` — `update_notebook_title` updates an existing notebook title.
- `app/src/notebooks/mod.rs:47` — `CloudNotebookModel` stores title, data, `ai_document_id`, and conversation ID; `display_name` returns title.
- `app/src/pane_group/pane/ai_document_pane.rs:38` — pane snapshots include `title` from `AIDocumentModel`.
- `app/src/app_state.rs:222` — `AIDocumentPaneSnapshot::Local` stores document ID, version, content, and title.
- `crates/persistence/src/schema.rs:24` — `ai_document_panes` has nullable `title` and content columns.
- `crates/persistence/src/model.rs:913` — persistence models for `ai_document_panes`.
- `app/src/persistence/sqlite.rs:1266` — `save_ai_document_content` updates content, version, and title by document ID.
- `app/src/persistence/sqlite.rs:2575` — app-state restoration reads `title` back into `AIDocumentPaneSnapshot`.
- `app/src/ai/blocklist/action_model/execute/create_documents.rs:61` — create-document finalization applies streamed title/content and adds `Artifact::Plan` with the generated title.
- `app/src/ai/artifacts/mod.rs:31` — `Artifact::Plan` stores an optional title for conversation artifact rendering and persistence.
- `app/src/ai/agent/conversation.rs:1166` — `update_plan_notebook_uid` mutates plan artifacts when sync completes; similar plumbing is needed for title updates.
- `app/src/terminal/input/plans/search_item.rs:33` — inline plan menu renders `AIDocument.title`.
- `app/src/search/ai_context_menu/notebooks/data_source.rs:50` — AI context menu filters plans by `CloudNotebookModel.ai_document_id` and searches display names.
- `app/src/search/command_search/notebooks/notebooks_data_source.rs:37` — command search snapshots `CloudNotebookModel.title`.
- `app/src/drive/items/notebook.rs:35` — Warp Drive list and preview render notebook titles.
- `app/src/view_components/clickable_text_input.rs:20` and `app/src/view_components/submittable_text_input.rs:29` — existing reusable inline text input patterns.
- `app/src/code/file_tree/view/editing.rs:147` and `app/src/workspace/view/vertical_tabs.rs:3418` — existing inline rename/editor patterns for file tree and vertical tabs.

## Current state

### Title ownership

`AIDocument.title` is the live title for a plan. `AIDocumentView::refresh` copies it into `PaneConfiguration`, and `render_plan_header` reads it from `AIDocumentModel` for the custom header.

### Persistence

The pane snapshot path already stores `title: Option<String>` for `AIDocumentPaneSnapshot::Local`. `AIDocumentPane::snapshot` gets the title from the current `AIDocument`. SQLite stores that title in `ai_document_panes.title`; `save_ai_document_content` updates it whenever document content saves.

There is no persisted field that says whether the title was authored by the user. Without that flag, a restored custom title cannot reliably be protected from future agent title updates.

### Warp Drive and search

Plans synced to Warp Drive are represented as notebooks with `CloudNotebookModel.ai_document_id = Some(...)`. Search and Warp Drive surfaces generally read `CloudNotebookModel.title`, while live inline plan menus read `AIDocument.title`.

`UpdateManager::update_notebook_title` already exists and should be used for synced plans. `create_notebook_in_plan_folder` already uses the current document title when first syncing a plan.

### Agent overwrites

`apply_streamed_agent_update` always sets `doc.title = new_title`. CreateDocuments finalization also updates the conversation artifact title from the agent-supplied document title. A model-level guard is required so UI code cannot be the only thing preserving user titles.

## Proposed changes

### 1. Add title lock state to AI documents

Add a field to `AIDocument`:

- `user_title_locked: bool`

The field is `false` for newly generated plans and restored plans without persisted lock data. It becomes `true` only after a successful non-empty user rename.

Add a small normalization helper in `AIDocumentModel`:

- trim leading/trailing whitespace
- reject empty strings as no-op
- keep the normalized title otherwise

Prefer a method with explicit intent over broadening `update_title`:

- `rename_document_title_by_user(id, title, ctx) -> bool`
- `apply_agent_title_update(id, title, ctx)` or guarded logic inside existing agent update methods

`rename_document_title_by_user` should:

1. normalize title
2. no-op for empty or unchanged title
3. set `doc.title`
4. set `doc.user_title_locked = true`
5. enqueue local title persistence
6. update any synced Warp Drive notebook title
7. update the conversation plan artifact title
8. emit `AIDocumentModelEvent::DocumentUpdated { source: User }`
9. emit any narrower title/save-status event if the final implementation adds one

`apply_streamed_agent_update` should update content as it does today, but should only update `doc.title` when `user_title_locked` is `false`. If the title is locked, the document body still updates and `DocumentUpdated` still emits so views refresh.

`create_new_version_and_apply_diffs` and `restore_document_edit` do not currently accept a new title, so they should not need title-lock changes unless implementation discovers another agent title update path.

### 2. Persist title lock locally

Add `user_title_locked` to local app-state restoration for AI document panes:

- `AIDocumentPaneSnapshot::Local { user_title_locked: bool, ... }`
- `crates/persistence/migrations/<timestamp>_add_ai_document_user_title_locked/up.sql`
- `crates/persistence/migrations/<timestamp>_add_ai_document_user_title_locked/down.sql`
- `crates/persistence/src/schema.rs`
- `crates/persistence/src/model.rs::AIDocumentPane`
- `crates/persistence/src/model.rs::NewAIDocumentPane`
- `app/src/persistence/sqlite.rs` save/read paths

Use a nullable or defaulted Boolean migration so existing rows restore as `false`.

Update `AIDocumentPane::snapshot` to include the current document's lock state. Update `AIDocumentModel::apply_persisted_content` to accept and apply persisted lock state when restoring an existing document or creating a free-floating document from SQLite.

Also update `ModelEvent::SaveAIDocumentContent` and `save_ai_document_content` so title-only changes can persist the current title and lock state even when the markdown body did not change.

### 3. Add title-only save path

Current save throttling is content-oriented. A rename must persist and sync even when document content is unchanged.

Implementation options:

- broaden the dirty flag naming from content to document data and reuse the existing throttled save channel, or
- add a direct `persist_title_to_sqlite` / `enqueue_save` call from the rename method.

Prefer reusing the existing throttled save channel if it remains clear, but ensure a title-only rename cannot be dropped because `content_dirty_flags` is false. If the dirty map remains content-specific, add a separate title dirty flag or rename it to document dirty data.

### 4. Sync Warp Drive title

When the renamed document has `sync_id`:

- if it is a `ServerId` or queued `ClientId`, call `UpdateManager::update_notebook_title(Arc<String>, sync_id, ctx)`
- rely on existing pending-change and retry behavior
- leave data sync unchanged

When the document has not been synced yet, no immediate Warp Drive call is needed. `create_notebook_in_plan_folder` should pick up `doc.title` when sync starts.

If a Plans folder creation is pending and the document is in `pending_document_queue`, update that queued `PendingDocument.title` when renaming so the eventual notebook creation uses the latest title.

### 5. Update conversation artifact title

Add a method to `AIConversation` similar to `update_plan_notebook_uid`:

- `update_plan_title(document_uid, title, terminal_view_id, ctx)`

It should:

1. find the matching `Artifact::Plan` by `document_uid`
2. update `title`
3. persist conversation state through `write_updated_conversation_state`
4. emit `UpdatedConversationArtifacts` when a terminal view ID is available

Call this from `AIDocumentModel::rename_document_title_by_user` by resolving the document's `conversation_id` and associated terminal view through `BlocklistAIHistoryModel`.

When `CreateDocumentsExecutor` finalizes a document after a streaming preview, use the model's effective title for the artifact if the document already exists and is title-locked. This prevents the final action result from replacing the artifact title with the agent-provided title.

### 6. Add inline rename UI to `AIDocumentView`

Add view state:

- `is_renaming_title: bool`
- `title_rename_editor: ViewHandle<EditorView>` or a small dedicated view if existing text-input wrappers cannot select all text on focus
- title hover/focus mouse state for the pencil affordance

Add actions:

- `BeginRenameTitle`
- `CommitRenameTitle`
- `CancelRenameTitle`

Wire editor events:

- `Enter` -> commit
- `Blurred` -> commit
- `Escape` -> cancel
- optional `Edited` -> notify for sizing/error state

When beginning rename:

1. verify the title is currently renamable
2. prefill editor with the current title
3. select the full buffer
4. focus the editor
5. set `is_renaming_title = true`

When committing:

1. verify the plan is still renamable
2. read the editor buffer
3. call `AIDocumentModel::rename_document_title_by_user`
4. clear rename state and editor focus/buffer
5. refresh header buttons/title and notify

When canceling:

1. discard buffer
2. clear rename state
3. restore normal title rendering

If selecting the whole buffer requires EditorView support not exposed by the current wrappers, add the narrow API to `EditorView` rather than duplicating editor internals.

### 7. Render header title affordance

Replace the center element in `render_plan_header` with a helper:

- `render_plan_title(title, can_rename, is_renaming, app) -> Box<dyn Element>`

Static state:

- render the title with existing clipping behavior
- on hover/focus, show a pencil icon or equivalent rename affordance when `can_rename`
- dispatch `BeginRenameTitle` according to the final product decision for single-click, double-click, or pencil click

Rename state:

- render the text input in the centered title area
- keep left and right header columns stable
- do not show the pencil icon

`can_rename` should be false when:

- the viewed version is not the current document version
- `AIDocumentModel::is_document_creation_streaming(document_id)` is true
- the associated conversation is in a read-only shared-session viewer state
- any final product rule disables renaming while the conversation is actively streaming

Use the same source of truth for disabling as the content editor where possible so title and body editability do not diverge unintentionally.

### 8. Keep pane configuration in sync

`AIDocumentView::refresh` already pushes document title into `PaneConfiguration`. A user rename emits `DocumentUpdated`, so existing subscribers should refresh the header. Verify that rename commits also update the pane configuration even if the current view does not change versions.

If there are multiple open views of the same plan, all should update because they subscribe to `AIDocumentModelEvent::DocumentUpdated`.

### 9. Testing and validation

Add or update tests in `app/src/ai/document/ai_document_model_tests.rs`:

- user rename trims whitespace and locks title
- empty user rename is ignored
- unchanged user rename does not emit or persist duplicate updates
- agent streamed title updates before lock
- agent streamed title does not overwrite after lock while content still updates
- title-only rename enqueues SQLite persistence
- persisted title lock restores for existing and free-floating documents
- pending Warp Drive document queue title is updated on rename

Add or update persistence tests in `app/src/persistence/sqlite_tests.rs`:

- `ai_document_panes.user_title_locked` defaults false for older rows
- save/read round trip includes title and lock state

Add Warp Drive/update-manager-facing tests where existing harnesses allow:

- rename after sync calls notebook-title update and preserves data
- rename before sync creates notebook with custom title

Add UI tests where practical:

- begin rename shows prefilled title input
- `Enter` commits
- blur commits
- `Escape` cancels
- empty submission preserves old title
- older version and streaming plan hide or disable affordance

Manual validation:

- generate a plan, rename it, and verify header updates
- restart app and verify restored title
- sync plan and verify Warp Drive title/search results
- continue an agent flow and verify agent title updates do not overwrite the custom title

## Risks and mitigations

- **Header drag or click conflicts:** Single-click title editing may conflict with header drag expectations. Mitigate by using a pencil click and/or double-click if design chooses the safer interaction.
- **Dropped title-only persistence:** Existing save path is content-driven. Mitigate with explicit title dirty handling and tests where content is unchanged.
- **Partial title propagation:** Some surfaces use `AIDocument.title`, others use `CloudNotebookModel.title`, and conversation artifacts store their own title. Mitigate by centralizing user rename logic in `AIDocumentModel` and updating all title mirrors from that method.
- **Agent finalization overwrite:** Streaming finalization can reapply agent titles and artifacts. Mitigate by checking `user_title_locked` in `apply_streamed_agent_update` and using effective model title when updating artifacts.
- **SQLite migration compatibility:** Existing installations lack title-lock data. Mitigate with default `false` and tests for old rows.
- **Warp Drive conflict behavior:** Notebook title updates may conflict with remote edits. Mitigate by using existing `UpdateManager::update_notebook_title` revision and pending-change logic rather than adding a separate sync path.

## Follow-ups

- Add a reset-to-generated-title action if product wants a way to clear the user lock.
- Add design mocks for the exact hover/focus affordance and click target.
- Consider exposing rename from Warp Drive context menus or plan search results if users expect rename outside the header.
