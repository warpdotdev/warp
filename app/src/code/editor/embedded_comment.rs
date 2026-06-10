use std::any::Any;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use pathfinder_geometry::vector::{vec2f, Vector2F};
use serde_yaml::Mapping;
use uuid::Uuid;
use warp_editor::content::markdown::MarkdownStyle;
use warp_editor::editor::EmbeddedItemModel;
use warp_editor::render::element::RenderableBlock;
use warp_editor::render::layout::TextLayout;
use warp_editor::render::model::viewport::ViewportItem;
use warp_editor::render::model::{
    BlockSpacing, EmbeddedItem, EmbeddedItemHTMLRepresentation, EmbeddedItemRichFormat,
    LaidOutEmbeddedItem, RenderState,
};
use warpui::elements::ChildView;
use warpui::units::Pixels;
use warpui::{AppContext, Element, EntityId, ViewHandle, WindowId};

use crate::code::editor::comment_editor::{CommentEditor, MAX_COMMENT_HEIGHT};
use crate::code::editor::inline_comment_view::InlineCommentView;
use crate::code_review::comments::CommentId;
use crate::editor::view_zone::{RenderableViewZone, RenderableViewZoneSpacer, ViewZoneXOrigin};

/// Upper bound on a saved inline comment card's laid-out height. Unlike the composer (capped at
/// [`MAX_COMMENT_HEIGHT`] so its draft scrolls internally), a saved card reserves its full height so
/// the whole comment is visible/scrollable inline; this is just a generous layout constraint.
const UNBOUNDED_COMMENT_HEIGHT: f32 = 100_000.0;

/// Height of the first visible line used for `first_line_bound` in inline comment blocks.
/// Matches `ButtonSize::Small.button_height()` at the default appearance (24px).
const INLINE_COMMENT_FIRST_LINE_HEIGHT: f32 = 24.0;

const COMMENT_ID_MAPPING_KEY: &str = "comment_id";
const ENTITY_ID_MAPPING_KEY: &str = "entity_id";
const WINDOW_ID_MAPPING_KEY: &str = "window_id";

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

    /// The rendered body text of the comment editor hosted by this inline block, resolved through
    /// the block's hosted child (not the view's own composer handle). Returns `None` if the hosted
    /// editor can no longer be resolved.
    #[cfg(feature = "integration_tests")]
    pub fn rendered_body_for_test(&self, app: &AppContext) -> Option<String> {
        self.comment_editor(app)
            .map(|editor| editor.read(app, |editor, app| editor.comment_text(app)))
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
        // back to a no-op spacer that still reserves the block's height.
        match self.comment_editor(ctx) {
            Some(editor) => {
                let child = ChildView::new(&editor).finish();
                let window_id = self.window_id;
                let entity_id = self.editor_entity_id;
                Box::new(RenderableViewZone::new(
                    viewport_item,
                    child,
                    MAX_COMMENT_HEIGHT,
                    ViewZoneXOrigin::ViewportPinned,
                    Box::new(move |measured, app| {
                        if let Some(editor) =
                            app.view_with_id::<CommentEditor>(window_id, entity_id)
                        {
                            editor.read(app, |editor, _| editor.set_laid_out_size(measured));
                        }
                    }),
                ))
            }
            None => Box::new(RenderableViewZoneSpacer::new(viewport_item)),
        }
    }

    fn spacing(&self) -> BlockSpacing {
        BlockSpacing::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
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
}

impl LaidOutInlineSavedComment {
    pub fn new(view_entity_id: EntityId, window_id: WindowId, size: Vector2F) -> Self {
        Self {
            size,
            view_entity_id,
            window_id,
        }
    }

    fn inline_view(&self, app: &AppContext) -> Option<ViewHandle<InlineCommentView>> {
        app.view_with_id::<InlineCommentView>(self.window_id, self.view_entity_id)
    }

    /// The rendered body text of the saved card hosted by this inline block, resolved through the
    /// block's hosted child. Returns `None` if the hosted view can no longer be resolved.
    #[cfg(feature = "integration_tests")]
    pub fn rendered_body_for_test(&self, app: &AppContext) -> Option<String> {
        self.inline_view(app)
            .map(|view| view.read(app, |view, app| view.rendered_body(app)))
    }

    /// Whether the saved card hosted by this inline block embeds a diff snippet (resolved through
    /// the block's hosted child). `Some(false)` proves the inline card shows only its body, never
    /// the redundant diff snippet the bottom-panel card renders. `None` if the view can no longer
    /// be resolved.
    #[cfg(feature = "integration_tests")]
    pub fn embeds_diff_snippet_for_test(&self, app: &AppContext) -> Option<bool> {
        self.inline_view(app)
            .map(|view| view.read(app, |view, _| view.embeds_diff_snippet_for_test()))
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
                let window_id = self.window_id;
                let entity_id = self.view_entity_id;
                Box::new(RenderableViewZone::new(
                    viewport_item,
                    child,
                    UNBOUNDED_COMMENT_HEIGHT,
                    ViewZoneXOrigin::ViewportPinned,
                    Box::new(move |measured, app| {
                        if let Some(view) =
                            app.view_with_id::<InlineCommentView>(window_id, entity_id)
                        {
                            view.read(app, |view, _| view.set_laid_out_size(measured));
                        }
                    }),
                ))
            }
            None => Box::new(RenderableViewZoneSpacer::new(viewport_item)),
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
