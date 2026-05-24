use super::convert_bgra_to_rgba;

#[test]
fn test_convert_bgra_to_rgba() {
    let mut data = vec![
        0xBB, 0xCC, 0xFF, 0xAA, // BGRA pixel (Blue, Green, Red, Alpha)
        0x11, 0x22, 0x33, 0x44, // Another BGRA pixel
    ];

    convert_bgra_to_rgba(&mut data);

    // After conversion, should be RGBA (Red, Green, Blue, Alpha)
    assert_eq!(
        data,
        vec![
            0xFF, 0xCC, 0xBB, 0xAA, // RGBA pixel
            0x33, 0x22, 0x11, 0x44, // Another RGBA pixel
        ]
    );
}
