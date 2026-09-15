use pathfinder_geometry::vector::vec2f;
use serde_yaml::{Mapping, Value};
use warp_editor::content::markdown::MarkdownStyle;
use warp_editor::render::model::LaidOutEmbeddedItem;
use warpui::{EntityId, WindowId};

use super::{
    COMMENT_ID_MAPPING_KEY, ENTITY_ID_MAPPING_KEY, EmbeddedCommentSpace, EmbeddedItem as _,
    LaidOutEmbeddedCommentSpace, WINDOW_ID_MAPPING_KEY, comment_embedded_item_conversion,
};
use crate::code_review::comments::CommentId;

#[test]
fn test_comment_embedded_item_conversion_valid_input() {
    let comment_id = CommentId::new();
    let entity_id = EntityId::from_usize(123);
    let window_id = WindowId::from_usize(456);

    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String(comment_id.to_string()),
    );
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String(entity_id.to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String(window_id.to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_some());
}

#[test]
fn test_comment_embedded_item_conversion_roundtrip() {
    let comment_id = CommentId::new();
    let entity_id = EntityId::from_usize(789);
    let window_id = WindowId::from_usize(101);

    let space = EmbeddedCommentSpace::new(comment_id, entity_id, window_id);
    let mapping = space.to_mapping(MarkdownStyle::Internal);

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_some());
}

#[test]
fn test_comment_embedded_item_conversion_missing_comment_id() {
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String("123".to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String("456".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}

#[test]
fn test_comment_embedded_item_conversion_missing_entity_id() {
    let comment_id = CommentId::new();
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String(comment_id.to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String("456".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}

#[test]
fn test_comment_embedded_item_conversion_missing_window_id() {
    let comment_id = CommentId::new();
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String(comment_id.to_string()),
    );
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String("123".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}

#[test]
fn test_comment_embedded_item_conversion_invalid_uuid() {
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String("not-a-valid-uuid".to_string()),
    );
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String("123".to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String("456".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}

#[test]
fn test_comment_embedded_item_conversion_invalid_entity_id() {
    let comment_id = CommentId::new();
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String(comment_id.to_string()),
    );
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String("not-a-number".to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String("456".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}

#[test]
fn test_laid_out_embedded_comment_space_has_missing_glyphs_is_false() {
    // The comment's own text is rendered by an independent `RichTextEditorView`, not by any
    // `TextFrame` this struct holds, so it never has a missing glyph of its own to report.
    let space = LaidOutEmbeddedCommentSpace {
        size: vec2f(100., 24.),
    };
    assert!(!space.has_missing_glyphs());
}

#[test]
fn test_comment_embedded_item_conversion_invalid_window_id() {
    let comment_id = CommentId::new();
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String(COMMENT_ID_MAPPING_KEY.to_string()),
        Value::String(comment_id.to_string()),
    );
    mapping.insert(
        Value::String(ENTITY_ID_MAPPING_KEY.to_string()),
        Value::String("123".to_string()),
    );
    mapping.insert(
        Value::String(WINDOW_ID_MAPPING_KEY.to_string()),
        Value::String("not-a-number".to_string()),
    );

    let result = comment_embedded_item_conversion(mapping);
    assert!(result.is_none());
}
