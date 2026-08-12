use warpui_core::color::ColorU;

use super::{ColorMap, convert_capture_name_to_color};

fn color_map() -> ColorMap {
    ColorMap {
        keyword_color: ColorU::new(1, 2, 3, 255),
        function_color: ColorU::new(4, 5, 6, 255),
        string_color: ColorU::new(7, 8, 9, 255),
        type_color: ColorU::new(10, 11, 12, 255),
        number_color: ColorU::new(13, 14, 15, 255),
        comment_color: ColorU::new(16, 17, 18, 255),
        property_color: ColorU::new(19, 20, 21, 255),
        tag_color: ColorU::new(22, 23, 24, 255),
    }
}

#[test]
fn control_flow_captures_use_keyword_color() {
    let colors = color_map();

    assert_eq!(
        convert_capture_name_to_color("conditional", &colors),
        Some(colors.keyword_color),
    );
    assert_eq!(
        convert_capture_name_to_color("repeat", &colors),
        Some(colors.keyword_color),
    );
}
