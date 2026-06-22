use super::*;

#[test]
fn ui_element_style_merge_test() {
    let style1 = UiComponentStyles {
        width: Some(24.),
        ..Default::default()
    };
    let style2 = UiComponentStyles {
        width: Some(25.),
        font_size: Some(14.),
        ..Default::default()
    };
    assert_eq!(style2, style1.merge(style2));

    let style3 = UiComponentStyles {
        font_size: Some(14.),
        ..Default::default()
    };
    let style4 = UiComponentStyles {
        width: Some(24.),
        font_size: Some(14.),
        ..Default::default()
    };
    assert_eq!(style4, style1.merge(style3));
}
