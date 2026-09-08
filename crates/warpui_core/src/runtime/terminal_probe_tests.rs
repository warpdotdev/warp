use super::*;

#[test]
fn parses_bel_terminated_replies() {
    let replies = b"\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:0000/0000/0000\x07\x1b[?6c";
    let colors = parse_replies(replies);
    assert_eq!(
        colors.fg,
        Some(ProbedRgb {
            r: 255,
            g: 255,
            b: 255
        })
    );
    assert_eq!(colors.bg, Some(ProbedRgb { r: 0, g: 0, b: 0 }));
}

#[test]
fn parses_st_terminated_replies() {
    let replies = b"\x1b]11;rgb:fdfd/e3e3/d2d2\x1b\\";
    let colors = parse_replies(replies);
    assert_eq!(colors.fg, None);
    assert_eq!(
        colors.bg,
        Some(ProbedRgb {
            r: 253,
            g: 227,
            b: 210
        })
    );
}

#[test]
fn parses_two_digit_components() {
    let replies = b"\x1b]11;rgb:28/2a/36\x07";
    assert_eq!(
        parse_replies(replies).bg,
        Some(ProbedRgb {
            r: 40,
            g: 42,
            b: 54
        })
    );
}

#[test]
fn parses_rgba_payload_ignoring_alpha() {
    let replies = b"\x1b]11;rgba:ffff/0000/8080/cccc\x07";
    assert_eq!(
        parse_replies(replies).bg,
        Some(ProbedRgb {
            r: 255,
            g: 0,
            b: 128
        })
    );
}

#[test]
fn scales_single_and_mixed_width_components() {
    // 1-digit components scale by 0xf, e.g. `f` -> 255, `8` -> 136.
    assert_eq!(parse_scaled_component("f"), Some(255));
    assert_eq!(parse_scaled_component("8"), Some(136));
    assert_eq!(parse_scaled_component("ffff"), Some(255));
    assert_eq!(parse_scaled_component("808"), Some(128));
    assert_eq!(parse_scaled_component(""), None);
    assert_eq!(parse_scaled_component("fffff"), None);
    assert_eq!(parse_scaled_component("zz"), None);
}

#[test]
fn ignores_malformed_payloads() {
    assert_eq!(parse_replies(b"\x1b]11;?\x07").bg, None);
    assert_eq!(parse_replies(b"\x1b]11;rgb:ff/ff\x07").bg, None);
    assert_eq!(parse_replies(b"\x1b]11;#282a36\x07").bg, None);
    assert_eq!(parse_replies(b"").bg, None);
}

#[test]
fn classifies_luminance_with_rec601_luma() {
    let white = ProbedRgb {
        r: 255,
        g: 255,
        b: 255,
    };
    let black = ProbedRgb { r: 0, g: 0, b: 0 };
    // Saturated blue is dark despite its high blue channel; saturated green
    // is light despite two zero channels — the luma weights dominate.
    let blue = ProbedRgb { r: 0, g: 0, b: 255 };
    let green = ProbedRgb { r: 0, g: 255, b: 0 };
    assert!(white.is_light());
    assert!(!black.is_light());
    assert!(!blue.is_light());
    assert!(green.is_light());
}

#[test]
fn background_luminance_prefers_probed_background() {
    let light = ProbedTerminalColors {
        fg: None,
        bg: Some(ProbedRgb {
            r: 253,
            g: 246,
            b: 227,
        }),
    };
    assert_eq!(light.background_luminance(), BackgroundLuminance::Light);

    let dark = ProbedTerminalColors {
        fg: None,
        bg: Some(ProbedRgb {
            r: 40,
            g: 42,
            b: 54,
        }),
    };
    assert_eq!(dark.background_luminance(), BackgroundLuminance::Dark);
}

#[test]
fn detects_da1_reply_sentinel() {
    assert!(contains_da1_reply(b"\x1b[?6c"));
    assert!(contains_da1_reply(b"\x1b]11;rgb:00/00/00\x07\x1b[?65;1;9c"));
    // A cursor-position report is not the sentinel.
    assert!(!contains_da1_reply(b"\x1b[?1;2R"));
    // An incomplete reply is not the sentinel yet.
    assert!(!contains_da1_reply(b"\x1b[?65;1"));
    assert!(!contains_da1_reply(b""));
    // A non-DA1 CSI followed by a real DA1 reply still matches.
    assert!(contains_da1_reply(b"\x1b[?1;2R\x1b[?6c"));
}

#[test]
fn colorfgbg_heuristic() {
    assert_eq!(colorfgbg_luminance("15;0"), BackgroundLuminance::Dark);
    assert_eq!(colorfgbg_luminance("0;15"), BackgroundLuminance::Light);
    assert_eq!(
        colorfgbg_luminance("15;default;0"),
        BackgroundLuminance::Dark
    );
    assert_eq!(colorfgbg_luminance("8"), BackgroundLuminance::Dark);
    assert_eq!(colorfgbg_luminance("7"), BackgroundLuminance::Light);
    assert_eq!(
        colorfgbg_luminance("15;default"),
        BackgroundLuminance::Unknown
    );
    assert_eq!(colorfgbg_luminance(""), BackgroundLuminance::Unknown);
}
