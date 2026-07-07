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
    LaidOutEmbeddedItem, RenderState,
};
use warpui::event::DispatchedEvent;
use warpui::units::Pixels;
use warpui::{AppContext, EntityId, EventContext, LayoutContext, ViewHandle, WindowId};

use crate::code::editor::comment_editor::CommentEditor;
use crate::code_review::comments::CommentId;

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

        Box::new(LaidOutEmbeddedCommentSpace { size })
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
}

impl LaidOutEmbeddedItem for LaidOutEmbeddedCommentSpace {
    fn height(&self) -> Pixels {
        Pixels::new(self.size.y())
    }

    fn size(&self) -> Vector2F {
        self.size
    }

    fn first_line_bound(&self) -> Vector2F {
        vec2f(self.size.x(), 24.0)
    }

    fn element(
        &self,
        _state: &RenderState,
        viewport_item: ViewportItem,
        _model: Option<&dyn EmbeddedItemModel>,
        _ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        // Just create a spacer - no child view rendering here
        Box::new(RenderableEmbeddedCommentSpace::new(viewport_item))
    }

    fn spacing(&self) -> BlockSpacing {
        BlockSpacing::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct RenderableEmbeddedCommentSpace {
    viewport_item: ViewportItem,
}

impl RenderableEmbeddedCommentSpace {
    pub(crate) fn new(viewport_item: ViewportItem) -> Self {
        Self { viewport_item }
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

    fn is_embedded_comment(&self) -> bool {
        true
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
