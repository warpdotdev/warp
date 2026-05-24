use super::*;
#[test]
fn coloru_with_opacity_test() {
    assert_eq!(
        coloru_with_opacity(ColorU::from_u32(0x000000ff), 10),
        ColorU::new(0, 0, 0, 25)
    );
    assert_eq!(
        coloru_with_opacity(ColorU::from_u32(0x000000ff), 0),
        ColorU::new(0, 0, 0, 0)
    );
    assert_eq!(
        coloru_with_opacity(ColorU::from_u32(0x000000ff), 100),
        ColorU::new(0, 0, 0, OPAQUE)
    );
}

#[test]
fn darker_lighter_test() {
    assert_eq!(
        darken(ColorU::new(255, 128, 0, OPAQUE)),
        ColorU::new(123, 62, 0, OPAQUE)
    );
    assert_eq!(
        lighten(ColorU::new(255, 128, 0, OPAQUE)),
        ColorU::new(255, 192, 128, OPAQUE)
    );
}

#[test]
fn pick_foreground_test() {
    assert_eq!(ColorU::white(), pick_foreground_color(ColorU::black()));
    assert_eq!(ColorU::black(), pick_foreground_color(ColorU::white()));
    assert_eq!(
        ColorU::white(),
        pick_foreground_color(ColorU::new(100, 100, 100, OPAQUE))
    );
}
