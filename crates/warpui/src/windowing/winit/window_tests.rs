use super::{BackdropType, WindowBackdrop, winit_backdrop};

#[test]
fn maps_every_window_backdrop_to_winit() {
    let cases = [
        (WindowBackdrop::None, BackdropType::None),
        (WindowBackdrop::Auto, BackdropType::Auto),
        (WindowBackdrop::Mica, BackdropType::MainWindow),
        (WindowBackdrop::Acrylic, BackdropType::TransientWindow),
        (WindowBackdrop::MicaAlt, BackdropType::TabbedWindow),
    ];

    for (backdrop, expected) in cases {
        assert_eq!(winit_backdrop(backdrop), expected);
    }
}
