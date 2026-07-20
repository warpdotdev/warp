use markdown_parser::{CodeBlockText, FormattedTable};
use warp_core::features::FeatureFlag;
use warpui_core::fonts::Weight;

use super::{
    BufferBlockItem, BufferTextStyle, CodeBlockType, MarkdownStyle, TextStyles,
    format_image_markdown,
};

#[test]
fn test_text_style_xor() {
    // This test makes sure that the `TextStyles` XOR implementations are updated as we add new styles.
    for style in enum_iterator::all::<BufferTextStyle>() {
        let mut with_style = TextStyles::default();

        match style {
            BufferTextStyle::Weight(weight) => {
                with_style.set_weight(Weight::from_custom_weight(Some(weight)));
            }
            BufferTextStyle::Subscript | BufferTextStyle::Superscript => {
                with_style.set_vertical_align(&style, true);
            }
            style => {
                if let Some(style_mut) = with_style.style_mut(&style) {
                    *style_mut = true;
                } else {
                    panic!("Impossible code path -- style {style:?} not handled");
                }
            }
        }

        assert!(
            (with_style ^ TextStyles::default()).colliding_style(&style),
            "Set ^ Unset = Set failed for {style:?}"
        );
        assert!(
            (TextStyles::default() ^ with_style).colliding_style(&style),
            "Unset ^ Set = Set failed for {style:?}"
        );
        assert!(
            !(with_style ^ with_style).colliding_style(&style),
            "Set ^ Set = Unset failed for {style:?}"
        );
        assert!(
            !(TextStyles::default() ^ TextStyles::default()).colliding_style(&style),
            "Unset ^ Unset = Unset failed for {style:?}"
        );

        let mut editable = with_style;

        editable ^= with_style;
        assert!(
            !editable.colliding_style(&style),
            "Set ^= Set -> Unset failed for {style:?}"
        );

        editable ^= TextStyles::default();
        assert!(
            !editable.colliding_style(&style),
            "Unset ^= Unset -> Unset failed for {style:?}"
        );

        editable ^= with_style;
        assert!(
            editable.colliding_style(&style),
            "Unset ^= Set -> Set failed for {style:?}"
        );

        editable ^= TextStyles::default();
        assert!(
            editable.colliding_style(&style),
            "Set ^= Unset -> Set failed for {style:?}"
        );
    }
}

#[test]
fn test_text_style_xor_vertical_align_transitions() {
    // Issue #14029: the XOR delta compared only `Option::is_some()`, so a direct Sub↔Sup transition
    // (both sides present) produced no vertical-align delta and the switch was dropped. The delta
    // must compare the actual `Option<VerticalAlign>` values: equal values cancel to none, differing
    // values (including Sub vs Sup) yield the target side.
    let sub = TextStyles::default().subscript();
    let sup = TextStyles::default().superscript();

    // Sub ^ Sup: differing values must produce a real delta, not cancel to none.
    let sub_to_sup = sub ^ sup;
    assert!(
        sub_to_sup.is_superscript(),
        "Sub ^ Sup should yield Sup, got {:?}",
        sub_to_sup.vertical_align
    );
    let sup_to_sub = sup ^ sub;
    assert!(
        sup_to_sub.is_subscript(),
        "Sup ^ Sub should yield Sub, got {:?}",
        sup_to_sub.vertical_align
    );

    // Same value still cancels to none.
    assert!(
        !(sub ^ sub).has_any_vertical_align(),
        "Sub ^ Sub should cancel to none"
    );
    assert!(
        !(sup ^ sup).has_any_vertical_align(),
        "Sup ^ Sup should cancel to none"
    );

    // `BitXorAssign` must match the by-value `BitXor` behavior for the direct Sub↔Sup switch.
    let mut editable = sub;
    editable ^= sup;
    assert!(
        editable.is_superscript(),
        "Sub ^= Sup should yield Sup, got {:?}",
        editable.vertical_align
    );
    let mut editable = sup;
    editable ^= sub;
    assert!(
        editable.is_subscript(),
        "Sup ^= Sub should yield Sub, got {:?}",
        editable.vertical_align
    );
}

#[test]
fn test_text_style_xor_weight_transitions() {
    // Sibling sweep of the vertical-align miss-cause: `weight` is likewise an enum-valued Option
    // (`Bold` vs `Light` vs `Medium` …) whose delta compared only `is_some()`, so a direct
    // weight-to-weight switch also dropped to no delta. Two distinct weights must produce a delta of
    // the target weight; equal weights cancel to none.
    let mut bold = TextStyles::default();
    bold.set_weight(Weight::Bold);
    let mut light = TextStyles::default();
    light.set_weight(Weight::Light);

    let bold_to_light = bold ^ light;
    assert_eq!(
        bold_to_light.get_custom_weight(),
        light.get_custom_weight(),
        "Bold ^ Light should yield Light"
    );

    assert!(
        (bold ^ bold).get_custom_weight().is_none(),
        "Bold ^ Bold should cancel to none"
    );

    let mut editable = bold;
    editable ^= light;
    assert_eq!(
        editable.get_custom_weight(),
        light.get_custom_weight(),
        "Bold ^= Light should yield Light"
    );
}

#[test]
fn test_formatted_table_round_trip() {
    let input = "Name\tAge\nAlice\t30\nBob\t25\n";
    let table = FormattedTable::from_internal_format(input);
    assert_eq!(table.headers.len(), 2);
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.to_internal_format(), input);
}

#[test]
fn test_formatted_table_single_column() {
    let input = "Header\nValue";
    let table = FormattedTable::from_internal_format(input);
    assert_eq!(table.headers.len(), 1);
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.to_internal_format(), "Header\nValue\n");
}

#[test]
fn test_formatted_table_empty_input() {
    let table = FormattedTable::from_internal_format("");
    assert!(table.headers.is_empty());
    assert!(table.rows.is_empty());
}

#[test]
fn test_mermaid_code_block_type_respects_feature_flag() {
    let markdown = CodeBlockText {
        lang: "mermaid".to_string(),
        code: "graph TD\nA --> B\n".to_string(),
    };

    let _disabled = FeatureFlag::MarkdownMermaid.override_enabled(false);
    assert_eq!(
        CodeBlockType::from(&markdown),
        CodeBlockType::Code {
            lang: "mermaid".to_string(),
        }
    );

    drop(_disabled);

    let _enabled = FeatureFlag::MarkdownMermaid.override_enabled(true);
    assert_eq!(CodeBlockType::from(&markdown), CodeBlockType::Mermaid);
}

#[test]
fn test_formatted_table_normalize_shape() {
    let input = "A\tB\tC\nX";
    let mut table = FormattedTable::from_internal_format(input);
    assert_eq!(table.rows[0].len(), 1);
    table.normalize_shape();
    assert_eq!(table.headers.len(), 3);
    assert_eq!(table.rows[0].len(), 3);
}

#[test]
fn format_image_markdown_preserves_title() {
    // No title -> canonical pre-title form.
    assert_eq!(
        format_image_markdown("alt", "src.png", None),
        "![alt](src.png)"
    );

    // Empty title is equivalent to no title (product invariant 4).
    assert_eq!(
        format_image_markdown("alt", "src.png", Some("")),
        "![alt](src.png)"
    );

    // Non-empty title is re-serialized with double quotes.
    assert_eq!(
        format_image_markdown("alt", "src.png", Some("caption")),
        "![alt](src.png \"caption\")"
    );

    // Literal double quotes in the title are escaped with a backslash so the
    // round-trip remains lossless.
    assert_eq!(
        format_image_markdown("alt", "src.png", Some("a \"quoted\" caption")),
        "![alt](src.png \"a \\\"quoted\\\" caption\")"
    );
}

#[test]
fn buffer_block_image_as_markdown_preserves_title() {
    let untitled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: None,
    };
    assert_eq!(
        &*untitled.as_markdown(MarkdownStyle::Internal),
        "![A dog](dog.png)"
    );

    let titled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex, my dog".to_string()),
    };
    assert_eq!(
        &*titled.as_markdown(MarkdownStyle::Internal),
        "![A dog](dog.png \"Rex, my dog\")"
    );
}

#[test]
fn buffer_block_image_partial_eq_considers_title() {
    let untitled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: None,
    };
    let titled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex".to_string()),
    };
    assert_ne!(untitled, titled);

    let titled_again = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex".to_string()),
    };
    assert_eq!(titled, titled_again);
}
