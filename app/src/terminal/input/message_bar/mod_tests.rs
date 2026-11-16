use warp_core::ui::Icon;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::MouseStateHandle;

use super::{ChipHorizontalAlignment, MessageItem};

#[test]
fn chip_constructor_creates_enabled_chip_with_items() {
    let item = MessageItem::chip(
        vec![
            MessageItem::text("hello"),
            MessageItem::keystroke(Default::default()),
        ],
        |_ctx| {},
        MouseStateHandle::default(),
    );

    match item {
        MessageItem::Chip {
            items, disabled, ..
        } => {
            assert_eq!(items.len(), 2);
            assert!(!disabled);
        }
        _ => panic!("expected chip variant"),
    }
}

#[test]
fn icon_constructor_creates_icon_without_color_override() {
    let item = MessageItem::icon(Icon::NewConversation);

    match item {
        MessageItem::Icon { icon, color } => {
            assert!(matches!(icon, Icon::NewConversation));
            assert!(color.is_none());
        }
        _ => panic!("expected icon variant"),
    }
}

#[test]
fn set_color_sets_icon_color_override() {
    let color = pathfinder_color::ColorU {
        r: 1,
        g: 2,
        b: 3,
        a: 255,
    };
    let mut item = MessageItem::icon(Icon::MessagePlusSquare);
    item.set_color(color);

    match item {
        MessageItem::Icon {
            color: Some(item_color),
            ..
        } => assert_eq!(item_color, color),
        _ => panic!("expected icon variant with color override"),
    }
}

#[test]
fn image_constructor_creates_image_with_correct_dimensions() {
    let item = MessageItem::image(
        AssetSource::Bundled {
            path: "bundled/svg/test.svg",
        },
        16.,
        16.,
    );

    match item {
        MessageItem::Image {
            source,
            width,
            height,
        } => {
            assert!(matches!(
                source,
                AssetSource::Bundled {
                    path: "bundled/svg/test.svg"
                }
            ));
            assert_eq!(width, 16.);
            assert_eq!(height, 16.);
        }
        _ => panic!("expected image variant"),
    }
}

#[test]
fn with_is_disabled_marks_chip_as_disabled() {
    let item = MessageItem::chip(
        vec![MessageItem::text("hello")],
        |_ctx| {},
        MouseStateHandle::default(),
    )
    .with_is_disabled(true);

    match item {
        MessageItem::Chip { disabled, .. } => assert!(disabled),
        _ => panic!("expected chip variant"),
    }
}

#[test]
fn chip_constructor_defaults_to_left_alignment() {
    let item = MessageItem::chip(
        vec![MessageItem::text("hello")],
        |_ctx| {},
        MouseStateHandle::default(),
    );

    match item {
        MessageItem::Chip {
            horizontal_alignment,
            ..
        } => assert_eq!(horizontal_alignment, ChipHorizontalAlignment::Left),
        _ => panic!("expected chip variant"),
    }
}

#[test]
fn with_horizontal_alignment_sets_right_alignment() {
    let item = MessageItem::chip(
        vec![MessageItem::text("hello")],
        |_ctx| {},
        MouseStateHandle::default(),
    )
    .with_horizontal_alignment(ChipHorizontalAlignment::Right);

    match item {
        MessageItem::Chip {
            horizontal_alignment,
            ..
        } => assert_eq!(horizontal_alignment, ChipHorizontalAlignment::Right),
        _ => panic!("expected chip variant"),
    }
}

#[test]
fn set_is_disabled_propagates_to_nested_interactive_items() {
    let mut item = MessageItem::Chip {
        items: vec![MessageItem::clickable(
            vec![MessageItem::text("nested")],
            |_ctx| {},
            MouseStateHandle::default(),
        )],
        action: std::sync::Arc::new(|_ctx| {}),
        mouse_state: MouseStateHandle::default(),
        disabled: false,
        horizontal_alignment: ChipHorizontalAlignment::default(),
    };
    item.set_is_disabled(true);

    match item {
        MessageItem::Chip {
            disabled, items, ..
        } => {
            assert!(disabled);
            match &items[0] {
                MessageItem::Clickable { disabled, .. } => assert!(*disabled),
                _ => panic!("expected nested clickable item"),
            }
        }
        _ => panic!("expected chip variant"),
    }
}
