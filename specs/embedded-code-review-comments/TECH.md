# Embedded Code Review Comments Technical Spec
## Context
This spec implements the behavior described in [`PRODUCT.md`](PRODUCT.md). The legacy floating-overlay path (`CommentEditor`, `PendingComment`) is unchanged and remains active when `FeatureFlag::EmbeddedCodeReviewComments` is off.

**High-level architecture**: each inline comment has one `InlineCommentView` keyed by `CommentId`. When a comment is created or the batch refreshes, `InlineCommentsController` reconciles the view map, reads each view's current height from its inner render state, and pushes a `CommentBlock` to `CodeEditorModel::set_inline_comment_blocks`. The model stores these as `BlockItem::EmbeddedComment` entries in a sumtree alongside regular text blocks and diff removed-line blocks; the sumtree's aggregate layout pushes surrounding lines down by each block's reserved height.

Key files:
- `app/src/code/editor/inline_comment_view.rs` — stable per-comment view; owns the body editor across all mode transitions
- `app/src/code/editor/inline_comments.rs` — `InlineCommentsController`; owns the view map and syncs blocks
- `app/src/code/editor/embedded_comment.rs` — `LaidOutInlineSavedComment` / `RenderableHostedComment` (new inline path) and `EmbeddedCommentSpace` / `LaidOutEmbeddedCommentSpace` (legacy markdown-embed path, see Decision 5)
- `crates/editor/src/render/model/mod.rs` — `BlockItem::EmbeddedComment`, `apply_comment_blocks`, `reset_comment_blocks`
- `app/src/code/editor/model.rs` — `CodeEditorModel::set_inline_comment_blocks`

## Design Decision 1: How the editor space grows with content
The body editor inside each `InlineCommentView` has its own inner `RenderState` whose `height()` reflects the content laid-out height. The outer editor's reserved block height is kept in sync by observing this inner render state and re-pushing an updated `CommentBlock` whenever it changes.

**Data flow**

1. `InlineCommentsController::wire_view` subscribes to the inner render state of each newly created view:
   ```rust path=app/src/code/editor/inline_comments.rs start=209
   let inner_render_state = view.as_ref(ctx).inner_render_state(ctx);
   ctx.observe(&inner_render_state, |me, _, ctx| {
       me.sync_inline_comment_blocks(ctx);
   });
   ```
2. On every layout pass of the body editor (keystrokes, paste, select-all+delete), `inner_render_state` emits `RenderEvent::LayoutUpdated`, which fires the observer.
3. `sync_inline_comment_blocks` → `InlineCommentsController::sync_blocks` reads `inline_height()` from each view:
   ```rust path=app/src/code/editor/inline_comment_view.rs start=277
   pub fn inline_height(&self, app: &AppContext) -> Pixels {
       let content_height = self.inner_render_state(app).as_ref(app).height().as_f32();
       Pixels::new(content_height + comment_chrome_height(&self.save_button, app))
   }
   ```
   `comment_chrome_height` is computed from named constants in `comment_editor.rs` (body padding, footer padding, footer border, outer border) plus the footer button's measured height, so the reserved space tracks button-height scaling automatically.
4. `sync_blocks` is needed — rather than each view updating its own block independently — because `reset_comment_blocks` replaces the *entire* comment block set atomically in the sumtree. You can't update one view's height in isolation; you must re-collect every view's current height and push the full set together. `sync_blocks` does this: it collects `(RenderLineLocation, Pixels, EntityId)` triples from all views, sorts them deterministically, and compares the result against `last_synced_blocks`. The equality check is critical: the observer fires on *every* body editor layout pass, including keystrokes that don't change the height (e.g. typing horizontally within a single line). Without the early-out, every keystroke in any draft would trigger a full sumtree rebuild.
5. When the set has changed, `sync_blocks` calls `CodeEditorModel::set_inline_comment_blocks`, which forwards to `RenderState::set_comment_blocks` → `LayoutAction::SetCommentBlocks`.
6. `apply_comment_blocks` → `reset_comment_blocks` rebuilds the content sumtree with `BlockItem::EmbeddedComment` entries at the new heights. The sumtree is the editor's source of truth for vertical layout: every block's Y offset is derived from the summed heights of all blocks before it, so inserting or resizing a `BlockItem::EmbeddedComment` automatically shifts every line below it.

**Height cap**: editable drafts cap at `MAX_COMMENT_HEIGHT` and scroll internally so a large draft doesn't push all diff lines off-screen. Saved cards use `UNBOUNDED_COMMENT_HEIGHT` so the full comment is always visible inline.

## Design Decision 2: Per-view isolation from the shared buffer
The editor buffer is shared across all `CodeEditorView`s open on the same file. Spacing for inline comments must not bleed into other views of that file.

**Why `BlockItem::EmbeddedComment` is its own variant**

The existing `BlockItem::TemporaryBlock` is used for removed-line diff blocks and is also per-view. The two block types must coexist and never clobber each other:
- `reset_temporary_block` (called on every diff refresh) skips `EmbeddedComment` items so comment blocks survive diff reloads.
- `reset_comment_blocks` (called from `apply_comment_blocks`) skips `TemporaryBlock` items so diff removed-line blocks survive comment updates.
- `layout_pending_edit` (buffer edits) preserves both variants explicitly.

If `EmbeddedComment` had reused `TemporaryBlock`, a diff refresh would silently erase all inline comment spacing.

**Where the write lands**

`CodeEditorModel::set_inline_comment_blocks` updates `self.render_state`, which is a `ModelHandle<RenderState>` owned exclusively by one `CodeEditorView`'s `CodeEditorModel`. Each `CodeEditorView` has its own `CodeEditorModel` and therefore its own `RenderState`. The shared buffer is never touched.

```rust path=app/src/code/editor/model.rs start=3857
pub fn set_inline_comment_blocks(
    &mut self,
    blocks: Vec<CommentBlock>,
    ctx: &mut ModelContext<Self>,
) {
    self.render_state.update(ctx, |render_state, _| {
        render_state.set_comment_blocks(blocks)
    });
}
```

**Anchor encoding for removed lines**

Current lines map to `RenderLineLocation::Current(line_number + 1)` so the block appears below the anchor line. Removed lines (which already occupy a `Temporary` slot in the content tree) map to `RenderLineLocation::Temporary { at_line, index_from_at_line }` — the same slot as the removed-line block itself. `reset_comment_blocks` inserts the comment block *after* the matching `TemporaryBlock`, so the comment renders below the removed line without a line-number offset.

## Design Decision 3: Single stable view across all modes
The old model used a single shared `CommentEditor` for drafts and separate read-only cards for saved comments. Saving called `self.reset(ctx)` on the composer, collapsing the block, then a server round-trip created a new card with the correct height. This caused a visible flash: block collapse → reinsertion.

**The new model**

Each comment has exactly one `InlineCommentView` for its entire lifetime, keyed by `CommentId`. The view owns the same `RichTextEditorView` (body editor) across three internal modes:

- `NewDraft` — editable, no saved content yet; `CommentId` is assigned at creation
- `EditingExisting` — editable, body pre-loaded with saved content
- `Saved` — selectable (read-only)

Mode transitions update `InteractionState`, footer buttons, and `saved_content` in place. The body editor's view handle is never replaced.

**Save without a view swap**

`InlineCommentView::save` emits `CommentSaved { id, line, comment_text }`. `CodeEditorView::handle_inline_comment_event` saves the comment, then calls `inline_comments.complete_save(review_comment, ctx)`, which forwards to `InlineCommentView::complete_save` on the *same* view:
```rust path=app/src/code/editor/inline_comment_view.rs start=213
pub fn complete_save(&mut self, comment: EditorReviewComment, ctx: &mut ViewContext<Self>) {
    self.saved_content = comment.comment_content;
    self.mode = InlineCommentMode::Saved;
    self.apply_mode(ctx);
    ctx.notify();
}
```
Because the view and its body editor are unchanged, the block height never collapses and there is no flash.

**When reconciliation fires**

`reconcile_saved` is called from `CodeEditorView::set_comment_locations`, which is called by `CodeReviewView::update_editor_comment_markers`. That method fires in response to `ReviewCommentBatchEvent::Changed`, emitted whenever a comment is upserted, deleted, or relocated (e.g. after a diff mode switch). Understanding this timing matters for the `update_source` guard below: batch updates can arrive while the user is mid-edit.

**Reconciliation preserves view identity**

`InlineCommentsController::reconcile_saved` matches by `CommentId`: existing IDs call `update_source` (in-place metadata/content update) and reuse their handle; only truly new IDs create a new view; IDs absent from the batch are dropped unless they are unsaved drafts.

`update_source` guards the body-editor reset behind `mode == Saved`, so an in-flight edit is never overwritten by a concurrent batch update. If the user is in `EditingExisting` mode when a batch arrives, `saved_content` is updated (so Cancel reverts to the latest server version) but the body editor is left untouched.

## Design Decision 4: Crossing the `warp_editor` crate boundary
`warp_editor` is a standalone crate with no dependency on the app crate. `InlineCommentView` and `CommentEditor` are app-crate view types, so they can't be referenced directly inside `warp_editor`'s render block interface.

The solution is to pass the view's identity — a `(WindowId, EntityId)` pair — through the `LaidOutEmbeddedItem` trait, and resolve it at render time using `app.view_with_id`. `warp_editor` never holds a typed view handle; it only stores opaque IDs.

`LaidOutInlineSavedComment` (for `InlineCommentView`) and `LaidOutEmbeddedCommentSpace` (for the legacy `CommentEditor`) both follow this pattern:
```rust path=app/src/code/editor/embedded_comment.rs start=368
fn inline_view(&self, app: &AppContext) -> Option<ViewHandle<InlineCommentView>> {
    app.view_with_id::<InlineCommentView>(self.window_id, self.view_entity_id)
}
```
The `element()` method on `LaidOutEmbeddedItem` receives an `&AppContext`, resolves the view handle there, wraps it in a `ChildView`, and returns a `RenderableHostedComment`. If the view can't be resolved (e.g. the window was closed mid-frame), it falls back to a no-op `RenderableEmbeddedCommentSpace` that preserves the block's reserved height without painting anything. Preserving the height rather than collapsing it is intentional: the block's height was set by `sync_blocks` earlier in the same frame, and clearing it would cause a one-frame content jump before the observer can correct it.

## Design Decision 5: Two height-tracking mechanisms (write-back vs. observation)
`RenderableHostedComment` has an optional `write_back_size` callback. There are two callers that use it differently, and understanding why clarifies `RenderableHostedComment`'s asymmetric construction.

**Context: the legacy `EmbeddedCommentSpace` mechanism**

`EmbeddedCommentSpace` (also in `embedded_comment.rs`) is the older mechanism used when comments were stored as markdown-style embeds in the buffer content. In that model, the comment's reserved height wasn't derived from a render-state observer — it was bootstrapped from a size stored on the `CommentEditor` view itself. `EmbeddedCommentSpace` reads `get_laid_out_size()` from the view during `layout()` and uses that stored value as the block height. The write-back path exists to keep that stored size current.

**Legacy `CommentEditor` path — write-back**: `RenderableHostedComment` calls the write-back after each layout pass to update the stored size on the view:
```rust path=app/src/code/editor/embedded_comment.rs start=189
Some(Box::new(move |measured, app| {
    if let Some(editor) = app.view_with_id::<CommentEditor>(window_id, entity_id) {
        editor.read(app, |editor, _| editor.set_laid_out_size(measured));
    }
}))
```
The next `EmbeddedCommentSpace::layout` call reads the stored size back to determine the block's reserved height.

**`InlineCommentView` path — render-state observation**: The inline path passes `None` for `write_back_size`. Height tracking goes through a completely different control flow: `InlineCommentsController::wire_view` observes the body editor's inner `RenderState`; when it changes, the observation fires `sync_inline_comment_blocks` → `sync_blocks` → `set_inline_comment_blocks`, which rebuilds the outer editor's sumtree with the updated height (Decision 1).

**Why write-back can't work for inline views**: write-back has a one-frame lag. It stores the measured height on the view during layout, but the outer editor only learns about that new height on its *next* layout pass when `EmbeddedCommentSpace::layout()` reads the stored value. For `CommentEditor`'s floating overlay, a one-frame lag is acceptable — the overlay is positioned externally and doesn't affect the sumtree. For `InlineCommentView`, the height drives sumtree layout, which determines the Y positions of every line below the block. A stale height means those lines shift by a pixel or more every time the draft grows — a visible jump on each keystroke. The observation approach fires within the same layout cycle as the body editor's layout, so the sumtree is updated before the outer editor computes its final line positions.

The write-back path is retained only to avoid a larger refactor of the legacy `EmbeddedCommentSpace` / `CommentEditor` embedding.

## Design Decision 6: Why `inline_comment_decoration_end` is needed
The problem is that the diff-hunk colored background (green for added lines, red for removed lines) must visually cover the area occupied by the inline comment editor, not stop at the bottom of the anchor line. Without any special handling, the colored rectangle ends at the last diff line, and the comment card sits against the default editor background — a visible break in the hunk's color.

The simplest fix would be to give each comment block's gutter row a copy of the parent hunk's overlay color so it joins the group naturally. But that would require passing diff-status information into the comment block rendering path, which is currently completely decoupled from diff hunk colors. The extension approach avoids that coupling by post-processing the already-computed decoration bounds.

Diff hunk colored backgrounds are painted in `element.rs` by merging consecutive gutter elements into *groups*. An element joins the current group when its line range matches the group's range and either: its overlay matches (same color), or it has no overlay at all but is part of the same diff range. The merged group is painted as a single rectangle from the group's top to its bottom.

`EmbeddedComment` blocks produce gutter entries with no overlay and a line range that no longer matches the diff hunk's range (the block is anchored *after* its line, not on it). The group collector treats this as a range mismatch and flushes the group early — leaving the comment row uncolored and the background split.

There are two places this needs fixing:

**Overlay group flushing (removed-line backgrounds)**: the group flush path in `element.rs` now checks whether a no-overlay element shares the current group's line range; if so, `end_y` is extended rather than flushing. This keeps removed-line overlay groups contiguous through comment rows in replacement hunks.

**Decoration rectangles (added-line backgrounds)**: the decoration loop separately paints `line_decoration_ranges` as rectangles. `inline_comment_decoration_end` detects whether an inline comment block starts at or very near the decoration's current bottom edge and returns the block's bottom if so:
```rust path=app/src/code/editor/element.rs start=1510
for line in comment_lines {
    if let Some(block_end) =
        inline_comment_decoration_end(start_y, end_y, line, model)
    {
        if block_end > extended_end_y {
            extended_end_y = block_end;
        }
    }
}
```
The outer loop extends the painted rectangle to `extended_end_y` instead of stopping at the bare anchor line.

## Supporting changes

**Gutter icons**: when the `EmbeddedCodeReviewComments` flag is on, saved-comment gutter indicators and diff-hunk action buttons are suppressed for any line that has an inline card or open draft. The inline card itself serves as the visual anchor.

**Horizontal pinning**: inline comment cards reserve vertical space in the content sumtree (so they scroll vertically with their anchor line) but are painted at the viewport's left edge rather than at the code's horizontal scroll position. This is done in `viewport_pinned_origin` by replacing the content rect's x origin with `ctx.bounds.origin_x()` (the visible viewport origin) before calling `child.paint`. Without this, the card would scroll horizontally with long code lines and disappear off-screen on wide diffs.

**Scroll-to-comment**: `CodeReviewView::scroll_inline_comment_into_view` uses `comment_block_content_bounds` (which queries `RenderState::comment_block_position`) to bring the full card — not just its anchor line — into the viewport when jumping from the review panel.

**`MouseStateHandle` preservation**: `set_comment_locations` now collects existing `MouseStateHandle`s by `CommentId` before clearing `comment_locations` and restores them when rebuilding the vec. Previously, a new `MouseStateHandle::default()` was created on every batch refresh, discarding all hover state. Per the WarpUI architecture, a `MouseStateHandle` must be created once during construction and reused; creating one inline during render or refresh breaks all mouse interactions for that element.

## Risks and mitigations

- Risk: `reset_temporary_block` and `reset_comment_blocks` must stay coordinated — if one forgets to preserve the other's variant, comment blocks or diff blocks disappear silently on the next diff refresh or comment sync. Mitigation: each reset skips the other's variant, and `layout_pending_edit` has explicit guards for both.
- Risk: `inline_height()` reads the inner render state before the first layout pass completes; the height is 0 at that point, causing a one-frame zero-height block. The observer fires after the first layout and corrects it, so the flash lasts at most one frame but is not fully eliminated.
- Risk: **Outdated comment filtering depends on a second feature flag.** `ReviewCommentBatch::editor_comments_for_file` only filters outdated comments when `FeatureFlag::PRCommentsSlashCommand` is enabled. If `EmbeddedCodeReviewComments` is on but `PRCommentsSlashCommand` is off, outdated line comments pass through `set_comment_locations` and create inline views, violating PRODUCT.md behavior 27. This is an open gap with no current mitigation.
- Risk: Large saved comments create tall inline blocks that can push most of the diff off-screen. Mitigation: editable drafts cap at `MAX_COMMENT_HEIGHT`; saved cards show at full height intentionally (the whole comment should be visible inline).
- Risk: The flag-off path (`active_comment_editor`, `PendingComment`) must stay intact. Mitigation: all embedded-path actions are explicitly gated on `FeatureFlag::EmbeddedCodeReviewComments.is_enabled()`.
