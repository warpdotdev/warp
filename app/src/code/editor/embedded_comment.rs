use std::any::Any;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use pathfinder_geometry::vector::{vec2f, Vector2F};
use serde_yaml::Mapping;
use uuid::Uuid;
use warp_editor::content::markdown::MarkdownStyle;
use warp_editor::editor::EmbeddedItemModel;
use warp_editor::render::element::{RenderContext, RenderableBlock};
use warp_editor::render::layout::TextLayout;
use warp_editor::render::model::viewport::ViewportItem;
use warp_editor::render::model::{
    BlockSpacing, EmbeddedItem, EmbeddedItemHTMLRepresentation, EmbeddedItemRichFormat,
    LaidOutEmbeddedItem, RenderLineLocation, RenderState,
};
use warpui::elements::ChildView;
use warpui::event::DispatchedEvent;
use warpui::units::Pixels;
use warpui::{
    AppContext, Element, EntityId, EventContext, LayoutContext, SizeConstraint, ViewHandle,
    WindowId,
};

use crate::code::editor::comment_editor::{CommentEditor, MAX_COMMENT_HEIGHT};
use crate::code::editor::inline_comment_view::InlineCommentView;
use crate::code_review::comments::CommentId;

/// Height constraint for a saved inline comment card: effectively unconstrained so the card
/// reserves its full laid-out height and the whole comment is visible inline without scrolling.
/// Uses `f32::INFINITY` rather than a large finite constant so the constraint is semantically
/// correct (no maximum) and avoids overflow if layout code accumulates heights.
const UNBOUNDED_COMMENT_HEIGHT: f32 = f32::INFINITY;

/// Height of the first visible line used for `first_line_bound` in inline comment blocks.
/// Matches `ButtonSize::Small.button_height()` at the default appearance (24px).
const INLINE_COMMENT_FIRST_LINE_HEIGHT: f32 = 24.0;

const COMMENT_ID_MAPPING_KEY: &str = "comment_id";
const ENTITY_ID_MAPPING_KEY: &str = "entity_id";
const WINDOW_ID_MAPPING_KEY: &str = "window_id";

fn viewport_pinned_origin(viewport_item: &ViewportItem, ctx: &RenderContext) -> Vector2F {
    let content_rect = viewport_item.content_bounds(ctx);
    let mut origin = content_rect.origin();
    origin.set_x(ctx.bounds.origin_x());
    origin
}

#[derive(Debug)]
pub struct EmbeddedCommentSpace {
    // We unfortunately need to store a string version of the ID
    // in order to return it in EmbeddedItem::hashed_id()
    id_string: String,
    editor_entity_id: EntityId,
    window_id: WindowId,
}

impl EmbeddedCommentSpace {
    fn new(id: CommentId, editor_entity_id: EntityId, window_id: WindowId) -> Self {
        Self {
            id_string: id.to_string(),
            editor_entity_id,
            window_id,
        }
    }

    // Fetch the underlying comment editor view
    fn get_comment_editor(&self, app: &AppContext) -> Option<ViewHandle<CommentEditor>> {
        app.view_with_id::<CommentEditor>(self.window_id, self.editor_entity_id)
    }
}

impl EmbeddedItem for EmbeddedCommentSpace {
    fn layout(&self, _text_layout: &TextLayout, app: &AppContext) -> Box<dyn LaidOutEmbeddedItem> {
        let comment_editor = self.get_comment_editor(app);
        if comment_editor.is_none() {
            log::error!(
                "EmbeddedComment can't layout missing comment editor for comment ID {:?}",
                self.id_string
            );
        };

        let size = comment_editor
            .and_then(|editor| editor.read(app, |editor, _ctx| editor.get_laid_out_size()))
            .unwrap_or_else(|| {
                log::error!(
                    "Didn't find laid out size for editor ID {:?}",
                    self.id_string
                );
                Vector2F::new(100.0, 24.0)
            });

        Box::new(LaidOutEmbeddedCommentSpace::new(
            self.editor_entity_id,
            self.window_id,
            size,
        ))
    }

    fn hashed_id(&self) -> &str {
        self.id_string.as_str()
    }

    fn to_mapping(&self, _style: MarkdownStyle) -> Mapping {
        let mut map = Mapping::new();
        let comment_id = self.id_string.clone();
        let editor_entity_id = self.editor_entity_id.to_string();
        let window_id = self.window_id.to_string();
        map.insert(COMMENT_ID_MAPPING_KEY.into(), comment_id.into());
        map.insert(ENTITY_ID_MAPPING_KEY.into(), editor_entity_id.into());
        map.insert(WINDOW_ID_MAPPING_KEY.into(), window_id.into());
        map
    }

    fn to_rich_format(&self, app: &AppContext) -> EmbeddedItemRichFormat<'_> {
        let text = if let Some(editor) = self.get_comment_editor(app) {
            editor.read(app, |editor, app| editor.comment_text(app))
        } else {
            String::new()
        };

        EmbeddedItemRichFormat {
            plain_text: text.to_string(),
            html: EmbeddedItemHTMLRepresentation {
                element_name: "div",
                content: text.to_string(),
                attributes: HashMap::new(),
            },
        }
    }
}

#[derive(Debug)]
pub struct LaidOutEmbeddedCommentSpace {
    pub size: Vector2F,
    editor_entity_id: EntityId,
    window_id: WindowId,
}

impl LaidOutEmbeddedCommentSpace {
    pub fn new(editor_entity_id: EntityId, window_id: WindowId, size: Vector2F) -> Self {
        Self {
            size,
            editor_entity_id,
            window_id,
        }
    }

    fn comment_editor(&self, app: &AppContext) -> Option<ViewHandle<CommentEditor>> {
        app.view_with_id::<CommentEditor>(self.window_id, self.editor_entity_id)
    }
}

impl LaidOutEmbeddedItem for LaidOutEmbeddedCommentSpace {
    fn height(&self) -> Pixels {
        Pixels::new(self.size.y())
    }

    fn size(&self) -> Vector2F {
        self.size
    }

    fn first_line_bound(&self) -> Vector2F {
        vec2f(self.size.x(), INLINE_COMMENT_FIRST_LINE_HEIGHT)
    }

    fn element(
        &self,
        _state: &RenderState,
        viewport_item: ViewportItem,
        _model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        // Host the comment editor's element in-tree so it occupies real vertical space at its line
        // and scrolls with the surrounding content. If the editor handle can't be resolved, fall
        // back to a no-op spacer that still reserves the block's height. This legacy path hosts a
        // markdown embed (not a per-view comment block), so it carries no comment anchor.
        match self.comment_editor(ctx) {
            Some(editor) => {
                let child = ChildView::new(&editor).finish();
                let window_id = self.window_id;
                let entity_id = self.editor_entity_id;
                Box::new(RenderableHostedComment::new(
                    viewport_item,
                    child,
                    MAX_COMMENT_HEIGHT,
                    None,
                    Some(Box::new(move |measured, app| {
                        if let Some(editor) =
                            app.view_with_id::<CommentEditor>(window_id, entity_id)
                        {
                            editor.read(app, |editor, _| editor.set_laid_out_size(measured));
                        }
                    })),
                ))
            }
            None => Box::new(RenderableEmbeddedCommentSpace::new(viewport_item, None)),
        }
    }

    fn spacing(&self) -> BlockSpacing {
        BlockSpacing::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

type SizeWriteBack = Box<dyn Fn(Vector2F, &AppContext)>;

/// Hosts an inline comment view's element (either the active composer or a saved-comment card)
/// inside a per-view inline comment block. It lays out the child against the block's content width
/// (capped at `max_height`), optionally writes the measured size back via `write_back_size` (the
/// legacy composer reserves space from its last measured size; inline cards derive their reserved
/// height from their render state instead), and paints and routes events to the child in content
/// space (so it scrolls with its anchored line).
pub struct RenderableHostedComment {
    viewport_item: ViewportItem,
    child: Box<dyn Element>,
    max_height: f32,
    /// The anchor of the inline comment block hosting this child, when known. Surfaced through
    /// [`RenderableBlock::embedded_comment_location`] so gutter rendering can classify comment
    /// rows without a reverse lookup into the content tree.
    embedded_comment_location: Option<RenderLineLocation>,
    /// Size write-back used only by the legacy [`LaidOutEmbeddedCommentSpace`] / [`CommentEditor`]
    /// path. After each layout pass the measured size is written back to the view so
    /// `EmbeddedCommentSpace::layout` can read it on the next frame. `InlineCommentView` always
    /// passes `None` here and uses the render-state observation path instead (see
    /// `InlineCommentsController::wire_view`). Remove when the legacy path is cleaned up.
    write_back_size: Option<SizeWriteBack>,
}

impl RenderableHostedComment {
    fn new(
        viewport_item: ViewportItem,
        child: Box<dyn Element>,
        max_height: f32,
        embedded_comment_location: Option<RenderLineLocation>,
        write_back_size: Option<SizeWriteBack>,
    ) -> Self {
        Self {
            viewport_item,
            child,
            max_height,
            embedded_comment_location,
            write_back_size,
        }
    }
}

impl RenderableBlock for RenderableHostedComment {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, ctx: &mut LayoutContext, app: &AppContext) {
        let width = self.viewport_item.content_size.x();
        let measured = self.child.layout(
            SizeConstraint::new(vec2f(0., 0.), vec2f(width, self.max_height)),
            ctx,
            app,
        );

        if let Some(write_back_size) = &self.write_back_size {
            write_back_size(measured, app);
        }
    }

    fn paint(&mut self, _model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        let origin = viewport_pinned_origin(&self.viewport_item, ctx);
        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);
        self.child.paint(origin, ctx.paint, app);
        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn embedded_comment_location(&self) -> Option<RenderLineLocation> {
        self.embedded_comment_location
    }
}

pub struct RenderableEmbeddedCommentSpace {
    viewport_item: ViewportItem,
    /// See [`RenderableHostedComment::embedded_comment_location`].
    embedded_comment_location: Option<RenderLineLocation>,
}

impl RenderableEmbeddedCommentSpace {
    pub(crate) fn new(
        viewport_item: ViewportItem,
        embedded_comment_location: Option<RenderLineLocation>,
    ) -> Self {
        Self {
            viewport_item,
            embedded_comment_location,
        }
    }
}

impl RenderableBlock for RenderableEmbeddedCommentSpace {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, _ctx: &mut LayoutContext, _app: &AppContext) {
        // No-op: this is just a spacer, the actual editor is laid out by EditorWrapper
    }

    fn paint(&mut self, _model: &RenderState, _ctx: &mut RenderContext, _app: &AppContext) {
        // No-op: this is just empty space, the actual editor is painted by EditorWrapper
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        // No interactivity: events are handled by the editor rendered by EditorWrapper
        false
    }

    fn embedded_comment_location(&self) -> Option<RenderLineLocation> {
        self.embedded_comment_location
    }
}

/// An already-laid-out inline block hosting a saved-comment [`InlineCommentView`] (the read-only
/// card). It mirrors [`LaidOutEmbeddedCommentSpace`] but hosts the saved card rather than the active
/// composer, resolving the view by its window + entity id so the host stays independent of the
/// app-crate view type at the `warp_editor` boundary.
#[derive(Debug)]
pub struct LaidOutInlineSavedComment {
    pub size: Vector2F,
    view_entity_id: EntityId,
    window_id: WindowId,
    /// The anchor of the comment block hosting this card, forwarded to the renderable so gutter
    /// rendering can classify the row without a reverse lookup into the content tree.
    location: RenderLineLocation,
}

impl LaidOutInlineSavedComment {
    pub fn new(
        view_entity_id: EntityId,
        window_id: WindowId,
        size: Vector2F,
        location: RenderLineLocation,
    ) -> Self {
        Self {
            size,
            view_entity_id,
            window_id,
            location,
        }
    }

    fn inline_view(&self, app: &AppContext) -> Option<ViewHandle<InlineCommentView>> {
        app.view_with_id::<InlineCommentView>(self.window_id, self.view_entity_id)
    }
}

impl LaidOutEmbeddedItem for LaidOutInlineSavedComment {
    fn height(&self) -> Pixels {
        Pixels::new(self.size.y())
    }

    fn size(&self) -> Vector2F {
        self.size
    }

    fn first_line_bound(&self) -> Vector2F {
        vec2f(self.size.x(), INLINE_COMMENT_FIRST_LINE_HEIGHT)
    }

    fn element(
        &self,
        _state: &RenderState,
        viewport_item: ViewportItem,
        _model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        match self.inline_view(ctx) {
            Some(view) => {
                let child = ChildView::new(&view).finish();
                // No size write-back: the inline card's reserved height is derived from its body
                // editor's render state (see `InlineCommentView::inline_height`).
                Box::new(RenderableHostedComment::new(
                    viewport_item,
                    child,
                    UNBOUNDED_COMMENT_HEIGHT,
                    Some(self.location),
                    None,
                ))
            }
            None => Box::new(RenderableEmbeddedCommentSpace::new(
                viewport_item,
                Some(self.location),
            )),
        }
    }

    fn spacing(&self) -> BlockSpacing {
        BlockSpacing::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The embedded item transformation for comments.
#[cfg_attr(not(test), allow(unused))] // TODO(CODE-1464): use this
pub(super) fn comment_embedded_item_conversion(
    mut mapping: serde_yaml::Mapping,
) -> Option<Arc<dyn EmbeddedItem>> {
    use serde_yaml::Value;
    let Some(Value::String(comment_uuid)) =
        mapping.remove(&Value::String(COMMENT_ID_MAPPING_KEY.to_string()))
    else {
        log::error!("Unable to deserialize embedded comment ID");
        return None;
    };
    let Some(Value::String(entity_id)) =
        mapping.remove(&Value::String(ENTITY_ID_MAPPING_KEY.to_string()))
    else {
        log::error!("Unable to deserialize embedded comment entity ID");
        return None;
    };
    let Some(Value::String(window_id)) =
        mapping.remove(&Value::String(WINDOW_ID_MAPPING_KEY.to_string()))
    else {
        log::error!("Unable to deserialize embedded comment window ID");
        return None;
    };

    let comment_id = CommentId::from_uuid(
        Uuid::from_str(&comment_uuid)
            .inspect_err(|e| {
                log::error!("Unable to parse comment ID {comment_uuid}: {e:?}");
            })
            .ok()?,
    );
    let entity_id = EntityId::from_usize(
        entity_id
            .parse::<usize>()
            .inspect_err(|e| {
                log::error!("Unable to parse entity ID {entity_id}: {e:?}");
            })
            .ok()?,
    );
    let window_id = WindowId::from_usize(
        window_id
            .parse::<usize>()
            .inspect_err(|e| {
                log::error!("Unable to parse entity ID {window_id}: {e:?}");
            })
            .ok()?,
    );
    Some(Arc::new(EmbeddedCommentSpace::new(
        comment_id, entity_id, window_id,
    )))
}

#[cfg(test)]
#[path = "embedded_comment_tests.rs"]
mod tests;
