use serde_yaml::{Mapping, Value};
use warp_editor::content::markdown::MarkdownStyle;
use warpui::{EntityId, WindowId};

use super::{
    COMMENT_ID_MAPPING_KEY, ENTITY_ID_MAPPING_KEY, EmbeddedCommentSpace, EmbeddedItem as _,
    WINDOW_ID_MAPPING_KEY, comment_embedded_item_conversion,
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
