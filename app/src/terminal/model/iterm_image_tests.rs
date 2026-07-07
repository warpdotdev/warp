use super::*;

#[test]
fn parse_invalid_iterm_image_dimensions() {
    assert_eq!(parse_iterm_image_dimensions(b"a123"), None);
    assert_eq!(parse_iterm_image_dimensions(b"123p"), None);
    assert_eq!(parse_iterm_image_dimensions(b"123p%"), None);
}

#[test]
fn parse_valid_iterm_image_dimensions() {
    assert_eq!(
        parse_iterm_image_dimensions(b"123"),
        Some((123, ITermImageDimensionUnit::Cell))
    );
    assert_eq!(
        parse_iterm_image_dimensions(b"123px"),
        Some((123, ITermImageDimensionUnit::Pixel))
    );
    assert_eq!(
        parse_iterm_image_dimensions(b"90%"),
        Some((90, ITermImageDimensionUnit::Percent))
    );
}

#[test]
fn clamps_percentage_image_width() {
    assert_eq!(
        parse_iterm_image_dimensions(b"123%"),
        Some((100, ITermImageDimensionUnit::Percent))
    );
    assert_eq!(
        parse_iterm_image_dimensions(b"99%"),
        Some((99, ITermImageDimensionUnit::Percent))
    );
}
