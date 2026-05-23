use async_io::block_on;
use pathfinder_color::ColorU;
use plist::{Dictionary, Value};
use virtual_fs::{Stub, VirtualFS};
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::fonts::FontInfo;
use warpui::keymap::Keystroke;

use super::{color_dictionary_to_coloru, ITermTheme, ITermThemeType};
use crate::settings::import::config::{
    GlobalHotkey, HotkeyError, ImportedFont, ParseableConfig, ThemeType,
};
use crate::settings::import::iterm_parser::{
    default_dark_theme, default_light_theme, Flags, ITermKeystroke, ITermProfile,
};

fn courier_new() -> Vec<FontInfo> {
    vec![FontInfo {
        family_name: "Courier New".to_string(),
        font_names: vec![
            "CourierNewPSMT".to_string(),
            "CourierNewPSMT-Bold".to_string(),
            "CourierNew".to_string(),
        ],
        is_monospace: false,
    }]
}

#[test]
fn test_remove_default_values_from_default() {
    let default_profile = ITermProfile::default().remove_default_values();
    assert_eq!(
        default_profile.theme,
        ITermThemeType::Single(Box::default())
    );
}

#[test]
fn test_remove_default_values_from_partial_profile() {
    let partial_profile = ITermProfile {
        theme: ITermThemeType::LightAndDark {
            light: default_light_theme(),
            dark: solarized_dark_theme(),
        },
        profile_name: Some("Partial Profile".to_string()),
        ..Default::default()
    }
    .remove_default_values();
    assert_eq!(
        partial_profile.theme,
        ITermThemeType::Single(solarized_dark_theme())
    );
}

#[test]
fn test_remove_default_values_from_full_profile() {
    let solarized_profile = ITermProfile {
        theme: solarized_theme_type(),
        profile_name: Some("Solarized Profile".to_string()),
        ..Default::default()
    }
    .remove_default_values();
    assert_eq!(solarized_profile.theme, solarized_theme_type());
}

#[test]
fn test_color_dictionary_missing_alpha_to_coloru() {
    let mut fg_dict = Dictionary::from_iter([
        ("Red Component", Value::Real(0.6505126953125)),
        ("Color Space", Value::String("sRGB".to_string())),
        ("Blue Component", Value::Real(0.747039794921875)),
        ("Green Component", Value::Real(0.8780975341796875)),
    ]);
    let fg_color = color_dictionary_to_coloru(Some(&mut fg_dict))
        .expect("Should be able to parse foreground color!");
    assert_eq!(
        fg_color,
        ColorU {
            r: 166,
            g: 224,
            b: 190,
            a: 255,
        }
    );
}

#[test]
fn test_color_dictionary_to_coloru() {
    let mut fg_dict = Dictionary::from_iter([
        ("Red Component", Value::Real(0.6505126953125)),
        ("Color Space", Value::String("sRGB".to_string())),
        ("Blue Component", Value::Real(0.747039794921875)),
        ("Green Component", Value::Real(0.8780975341796875)),
        ("Alpha Component", Value::Real(0.)),
    ]);
    let fg_color = color_dictionary_to_coloru(Some(&mut fg_dict))
        .expect("Should be able to parse foreground color!");
    assert_eq!(
        fg_color,
        ColorU {
            r: 166,
            g: 224,
            b: 190,
            a: 0,
        }
    );
}

#[test]
fn test_into_warp_theme_valid() {
    let theme: WarpTheme = solarized_dark_theme()
        .into_warp_theme("", &default_dark_theme())
        .expect("Should be able to convert into WarpTheme");
    assert_eq!(
        theme.accent(),
        Fill::Solid(ColorU {
            r: 229,
            g: 136,
            b: 133,
            a: 255,
        })
    );
    assert_eq!(
        theme.background(),
        Fill::Solid(ColorU {
            r: 0,
            g: 43,
            b: 54,
            a: 255,
        })
    );
    assert_eq!(
        theme.foreground(),
        Fill::Solid(ColorU {
            r: 131,
            g: 148,
            b: 150,
            a: 255,
        })
    );
}

#[test]
fn test_into_warp_theme_invalid() {
    default_dark_theme()
        .into_warp_theme("", &default_dark_theme())
        .expect_err("Should return an error if the theme is not sufficiently configured.");
}

#[test]
fn test_import_from_file() {
    VirtualFS::test("test_parse_iterm_from_import", |dirs, mut sandbox| {
        sandbox.mkdir("config");
        sandbox.with_files(vec![
            Stub::FileWithContent("config/base.plist", format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
<key>LoadPrefsFromCustomFolder</key><true/>
<key>PrefsCustomFolder</key><string>{}</string>
</dict></plist>", dirs.tests().join("config").to_str().unwrap_or("")).as_str()),
            Stub::FileWithContent(
            "config/com.googlecode.iterm2.plist",
            TEST_FILE,
        )]);
        let profile: ITermProfile = block_on(ITermProfile::from_file(
            dirs.tests().join("config").join("base.plist"),
        ))
        .expect("Should be able to read file!")
        .pop()
        .expect("Should have returned at least one config!")
        .remove_default_values();

        let config = profile.parse(&[]);

        let ThemeType::Single(ref warp_theme) =
            config.theme.value().as_ref().expect("Should import theme!")
        else {
            panic!("Should have read a single theme!")
        };

        assert_eq!(
            warp_theme.accent(),
            Fill::Solid(ColorU {
                r: 255,
                g: 165,
                b: 96,
                a: 255,
            })
        );
        assert_eq!(
            warp_theme.background(),
            Fill::Solid(ColorU {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            })
        );
        assert_eq!(
            warp_theme.foreground(),
            Fill::Solid(ColorU {
                r: 187,
                g: 187,
                b: 187,
                a: 255,
            })
        );

        let GlobalHotkey::QuakeMode(hotkey_mode) = config
            .hotkey_mode
            .importable_value()
            .expect("Should import a hotkey.")
            .expect("Hotkey should have parsed.")
        else {
            panic!("Hotkey should have been quake mode");
        };
        assert_eq!(
            hotkey_mode.keystroke,
            Keystroke {
                ctrl: true,
                alt: false,
                shift: false,
                cmd: true,
                meta: false,
                key: "大".to_string()
            }
        )
    });
}
#[test]
fn test_not_import_from_file() {
    VirtualFS::test("test_parse_iterm_from_import", |dirs, mut sandbox| {
        sandbox.mkdir("config");
        sandbox.with_files(vec![
            Stub::FileWithContent("config/base.plist", format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
<key>LoadPrefsFromCustomFolder</key><false/>
<key>PrefsCustomFolder</key><string>{}</string>
</dict></plist>", dirs.tests().join("config").to_str().unwrap_or("")).as_str()),
            Stub::FileWithContent(
            "config/com.googlecode.iterm2.plist",
            TEST_FILE,
        )]);
        block_on(ITermProfile::from_file(
            dirs.tests().join("config").join("base.plist"),
        ))
        .expect_err("Should not be able to read file!");
    });
}

#[test]
fn test_parse_font() {
    let test_profile = ITermProfile {
        font_name: Some("CourierNewPSMT".to_string()),
        font_size: Some("16".to_string()),
        ..Default::default()
    };
    let fonts = courier_new();
    assert_eq!(
        *test_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: Some("Courier New".to_string()),
            size: Some(16.),
        }
    );
}

#[test]
fn test_parse_font_without_size() {
    let test_profile = ITermProfile {
        font_name: Some("CourierNewPSMT".to_string()),
        ..Default::default()
    };
    let fonts = courier_new();
    assert_eq!(
        *test_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: Some("Courier New".to_string()),
            size: None,
        }
    );
}

#[test]
fn test_parse_font_with_default_size() {
    let warp_default_profile = ITermProfile {
        font_name: Some("CourierNewPSMT".to_string()),
        font_size: Some("13".to_string()),
        ..Default::default()
    };
    let fonts = courier_new();
    assert_eq!(
        *warp_default_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: Some("Courier New".to_string()),
            size: None,
        }
    );

    let iterm_default_profile = ITermProfile {
        font_name: Some("CourierNewPSMT".to_string()),
        font_size: Some("12".to_string()),
        ..Default::default()
    };
    let fonts = courier_new();
    assert_eq!(
        *iterm_default_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: Some("Courier New".to_string()),
            size: None,
        }
    );
}

#[test]
fn test_parse_font_with_default_font() {
    let warp_default_profile = ITermProfile {
        font_name: Some("Hack".to_string()),
        font_size: Some("16".to_string()),
        ..Default::default()
    };
    let fonts = [FontInfo {
        family_name: "Hack".to_string(),
        font_names: vec!["Hack".to_string()],
        is_monospace: false,
    }];
    assert_eq!(
        *warp_default_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: None,
            size: Some(16.),
        }
    );

    let iterm_default_profile = ITermProfile {
        font_name: Some("Monaco".to_string()),
        font_size: Some("16".to_string()),
        ..Default::default()
    };
    let fonts = [FontInfo {
        family_name: "Monaco".to_string(),
        font_names: vec!["Monaco".to_string()],
        is_monospace: false,
    }];
    assert_eq!(
        *iterm_default_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: None,
            size: Some(16.),
        }
    );
}

#[test]
fn test_parse_invalid_font() {
    let test_profile = ITermProfile {
        font_name: Some("CourierOld".to_string()),
        ..Default::default()
    };
    let fonts = courier_new();
    assert_eq!(
        *test_profile
            .remove_default_values()
            .parse(&fonts)
            .font
            .value(),
        ImportedFont {
            family: None,
            size: None,
        }
    );
}

/// Test case with standard ASCII key and multiple modifiers
#[test]
fn test_from_iterm_keystroke_with_modifiers() {
    let iterm_keystroke = ITermKeystroke {
        modifier: (Flags::CTRL | Flags::ALT | Flags::SHIFT).bits() as u64,
        key: "a".to_string(),
    };
    let result: Result<Keystroke, HotkeyError> = iterm_keystroke.try_into();
    assert_eq!(
        result.unwrap(),
        Keystroke {
            ctrl: true,
            alt: true,
            shift: true,
            cmd: false,
            meta: false,
            key: "a".to_string(),
        }
    );
}

/// Test case with multiple modifiers and a non-ascii character
#[test]
fn test_from_iterm_keystroke_non_ascii() {
    let iterm_keystroke = ITermKeystroke {
        modifier: (Flags::CTRL | Flags::ALT | Flags::SHIFT).bits() as u64,
        key: "€".to_string(), // Euro symbol, which might have a special key representation
    };
    let result: Result<Keystroke, HotkeyError> = iterm_keystroke.try_into();
    assert_eq!(
        result.unwrap(),
        Keystroke {
            ctrl: true,
            alt: true,
            shift: true,
            cmd: false,
            meta: false,
            key: "€".to_string(),
        }
    );
}

/// Test case with multiple modifiers and a special character
#[test]
fn test_from_iterm_keystroke_function_key() {
    let iterm_keystroke = ITermKeystroke {
        modifier: (Flags::CTRL | Flags::ALT | Flags::SHIFT).bits() as u64,
        key: "\u{f704}".to_string(), // Keycode for F1
    };
    let result: Result<Keystroke, HotkeyError> = iterm_keystroke.try_into();
    assert_eq!(
        result.unwrap(),
        Keystroke {
            ctrl: true,
            alt: true,
            shift: true,
            cmd: false,
            meta: false,
            key: "f1".to_string(),
        }
    );
}

#[test]
fn test_read_activation_keystroke() {
    // 63239 = 0xf707, which is the character for f4.
    VirtualFS::test("test_read_activation_keystroke", |dirs, mut sandbox| {
        sandbox.mkdir("config");
        sandbox.with_files(vec![
            Stub::FileWithContent("config/base.plist", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
	<key>Hotkey</key>
	<true/>
	<key>HotkeyChar</key>
	<integer>63239</integer>
	<key>HotkeyModifiers</key>
	<integer>0</integer>
	<key>Default Bookmark Guid</key>
	<string>a</string>
	<key>New Bookmarks</key>
		<array>
			<dict>
			<key>Name</key>
			<string>test</string>
			<key>Guid</key>
			<string>a</string>
			</dict>
		</array>
</dict></plist>")]);
        let profile: ITermProfile = block_on(ITermProfile::from_file(
            dirs.tests().join("config").join("base.plist"),
        ))
        .expect("Should be able to read file!")
        .pop()
        .expect("Should have returned at least one config!")
        .remove_default_values();

        assert_eq!(
            profile
                .activation_keystroke
                .expect("Should have read an activation keystroke")
                .key,
            "\u{f707}"
        )
    });
}

/// A solarized dark/light theme combo from iTerm
fn solarized_theme_type() -> ITermThemeType {
    ITermThemeType::LightAndDark {
        light: solarized_light_theme(),
        dark: solarized_dark_theme(),
    }
}

// Used the following regex to create:
// Real -> Value::Real
// String -> Value::String
// ("[a-zA-Z ]*"): ([0-9.a-zA-Z:\(\)" ]*) -> ($1, $2)
// Some\(\{([0-9.a-zA-Z:\(\)", ]*)\}\) -> Some(Dictionary::from_iter([$1]))
// Value::String\("(.*)"\) -> Value::String("$1".to_string())
// terminal_colors: [ -> terminal_colors: vec![

/// Solarized light theme
fn solarized_light_theme() -> Box<ITermTheme> {
    Box::new(ITermTheme {
        terminal_colors: vec![
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.027450980392156862)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.25882352941176473)),
                ("Green Component", Value::Real(0.21176470588235294)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8627450980392157)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.1843137254901961)),
                ("Green Component", Value::Real(0.19607843137254902)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5215686274509804)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Green Component", Value::Real(0.6)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7098039215686275)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Green Component", Value::Real(0.5372549019607843)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.14901960784313725)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8235294117647058)),
                ("Green Component", Value::Real(0.5450980392156862)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8274509803921568)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5098039215686274)),
                ("Green Component", Value::Real(0.21176470588235294)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.16470588235294117)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.596078431372549)),
                ("Green Component", Value::Real(0.6313725490196078)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9333333333333333)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8352941176470589)),
                ("Green Component", Value::Real(0.9098039215686274)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.21176470588235294)),
                ("Green Component", Value::Real(0.16862745098039217)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.796078431372549)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.08627450980392157)),
                ("Green Component", Value::Real(0.29411764705882354)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.34509803921568627)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.4588235294117647)),
                ("Green Component", Value::Real(0.43137254901960786)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.396078431372549)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5137254901960784)),
                ("Green Component", Value::Real(0.4823529411764706)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5137254901960784)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5882352941176471)),
                ("Green Component", Value::Real(0.5803921568627451)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.4235294117647059)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7686274509803922)),
                ("Green Component", Value::Real(0.44313725490196076)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5764705882352941)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.6313725490196078)),
                ("Green Component", Value::Real(0.6313725490196078)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9921568627450981)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8901960784313725)),
                ("Green Component", Value::Real(0.9647058823529412)),
            ])),
        ],
        foreground: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.396078431372549)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.5137254901960784)),
            ("Green Component", Value::Real(0.4823529411764706)),
        ])),
        background: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.9921568627450981)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.8901960784313725)),
            ("Green Component", Value::Real(0.9647058823529412)),
        ])),
        cursor: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.396078431372549)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.5137254901960784)),
            ("Green Component", Value::Real(0.4823529411764706)),
        ])),
    })
}

fn solarized_dark_theme() -> Box<ITermTheme> {
    Box::new(ITermTheme {
        terminal_colors: vec![
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.027450980392156862)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.25882352941176473)),
                ("Green Component", Value::Real(0.21176470588235294)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8627450980392157)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.1843137254901961)),
                ("Green Component", Value::Real(0.19607843137254902)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5215686274509804)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Green Component", Value::Real(0.6)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7098039215686275)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Green Component", Value::Real(0.5372549019607843)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.14901960784313725)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8235294117647058)),
                ("Green Component", Value::Real(0.5450980392156862)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8274509803921568)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5098039215686274)),
                ("Green Component", Value::Real(0.21176470588235294)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.16470588235294117)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.596078431372549)),
                ("Green Component", Value::Real(0.6313725490196078)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9333333333333333)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8352941176470589)),
                ("Green Component", Value::Real(0.9098039215686274)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.21176470588235294)),
                ("Green Component", Value::Real(0.16862745098039217)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.796078431372549)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.08627450980392157)),
                ("Green Component", Value::Real(0.29411764705882354)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.34509803921568627)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.4588235294117647)),
                ("Green Component", Value::Real(0.43137254901960786)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.396078431372549)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5137254901960784)),
                ("Green Component", Value::Real(0.4823529411764706)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5137254901960784)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5882352941176471)),
                ("Green Component", Value::Real(0.5803921568627451)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.4235294117647059)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7686274509803922)),
                ("Green Component", Value::Real(0.44313725490196076)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.5764705882352941)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.6313725490196078)),
                ("Green Component", Value::Real(0.6313725490196078)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9921568627450981)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8901960784313725)),
                ("Green Component", Value::Real(0.9647058823529412)),
            ])),
        ],
        foreground: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.5137254901960784)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.5882352941176471)),
            ("Green Component", Value::Real(0.5803921568627451)),
        ])),
        background: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.0)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.21176470588235294)),
            ("Green Component", Value::Real(0.16862745098039217)),
        ])),
        cursor: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.8979712128639221)),
            ("Color Space", Value::String("P3".to_string())),
            ("Blue Component", Value::Real(0.5218176245689392)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.5329258441925049)),
        ])),
    })
}

const TEST_FILE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
	<key>AppleAntiAliasingThreshold</key>
	<integer>1</integer>
	<key>ApplePressAndHoldEnabled</key>
	<false/>
	<key>AppleScrollAnimationEnabled</key>
	<integer>0</integer>
	<key>AppleSmoothFixedFontsSizeThreshold</key>
	<integer>1</integer>
	<key>AppleWindowTabbingMode</key>
	<string>manual</string>
	<key>Default Bookmark Guid</key>
	<string>F6D2AFD9-675C-4392-A2FE-5BF298108F8E</string>
	<key>HapticFeedbackForEsc</key>
	<false/>
	<key>HotkeyMigratedFromSingleToMulti</key>
	<true/>
	<key>New Bookmarks</key>
	<array>
		<dict>
			<key>ASCII Anti Aliased</key>
			<true/>
			<key>Ambiguous Double Width</key>
			<false/>
			<key>Ansi 0 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.25882352941176473</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.21176470588235294</real>
				<key>Red Component</key>
				<real>0.027450980392156862</real>
			</dict>
			<key>Ansi 0 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.30978869999999997</real>
				<key>Green Component</key>
				<real>0.30978869999999997</real>
				<key>Red Component</key>
				<real>0.30978869999999997</real>
			</dict>
			<key>Ansi 0 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.30978869999999997</real>
				<key>Green Component</key>
				<real>0.30978869999999997</real>
				<key>Red Component</key>
				<real>0.30978869999999997</real>
			</dict>
			<key>Ansi 1 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.18431372549019609</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.19607843137254902</real>
				<key>Red Component</key>
				<real>0.86274509803921573</real>
			</dict>
			<key>Ansi 1 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>0.4235294</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 1 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>0.4235294</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 10 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.45882352941176469</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.43137254901960786</real>
				<key>Red Component</key>
				<real>0.34509803921568627</real>
			</dict>
			<key>Ansi 10 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.67277030000000004</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>0.80941479999999999</real>
			</dict>
			<key>Ansi 10 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.67277030000000004</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>0.80941479999999999</real>
			</dict>
			<key>Ansi 11 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.51372549019607838</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.4823529411764706</real>
				<key>Red Component</key>
				<real>0.396078431372549</real>
			</dict>
			<key>Ansi 11 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.7996491</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 11 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.7996491</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 12 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.58823529411764708</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.58039215686274515</real>
				<key>Red Component</key>
				<real>0.51372549019607838</real>
			</dict>
			<key>Ansi 12 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.9982605</real>
				<key>Green Component</key>
				<real>0.86277559999999998</real>
				<key>Red Component</key>
				<real>0.71165029999999996</real>
			</dict>
			<key>Ansi 12 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.9982605</real>
				<key>Green Component</key>
				<real>0.86277559999999998</real>
				<key>Red Component</key>
				<real>0.71165029999999996</real>
			</dict>
			<key>Ansi 13 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.7686274509803922</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.44313725490196076</real>
				<key>Red Component</key>
				<real>0.42352941176470588</real>
			</dict>
			<key>Ansi 13 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99652090000000004</real>
				<key>Green Component</key>
				<real>0.61330589999999996</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 13 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99652090000000004</real>
				<key>Green Component</key>
				<real>0.61330589999999996</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 14 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.63137254901960782</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.63137254901960782</real>
				<key>Red Component</key>
				<real>0.57647058823529407</real>
			</dict>
			<key>Ansi 14 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99703969999999997</real>
				<key>Green Component</key>
				<real>0.87631029999999999</real>
				<key>Red Component</key>
				<real>0.87591359999999996</real>
			</dict>
			<key>Ansi 14 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99703969999999997</real>
				<key>Green Component</key>
				<real>0.87631029999999999</real>
				<key>Red Component</key>
				<real>0.87591359999999996</real>
			</dict>
			<key>Ansi 15 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.8901960784313725</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.96470588235294119</real>
				<key>Red Component</key>
				<real>0.99215686274509807</real>
			</dict>
			<key>Ansi 15 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>1</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 15 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>1</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 2 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.0</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.59999999999999998</real>
				<key>Red Component</key>
				<real>0.52156862745098043</real>
			</dict>
			<key>Ansi 2 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>0.65882350000000001</real>
			</dict>
			<key>Ansi 2 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>0.65882350000000001</real>
			</dict>
			<key>Ansi 3 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.0</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.53725490196078429</real>
				<key>Red Component</key>
				<real>0.70980392156862748</real>
			</dict>
			<key>Ansi 3 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.71372550000000001</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 3 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.71372550000000001</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 4 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.82352941176470584</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.54509803921568623</real>
				<key>Red Component</key>
				<real>0.14901960784313725</real>
			</dict>
			<key>Ansi 4 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99607840000000003</real>
				<key>Green Component</key>
				<real>0.79607839999999996</real>
				<key>Red Component</key>
				<real>0.58823530000000002</real>
			</dict>
			<key>Ansi 4 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99607840000000003</real>
				<key>Green Component</key>
				<real>0.79607839999999996</real>
				<key>Red Component</key>
				<real>0.58823530000000002</real>
			</dict>
			<key>Ansi 5 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.50980392156862742</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.21176470588235294</real>
				<key>Red Component</key>
				<real>0.82745098039215681</real>
			</dict>
			<key>Ansi 5 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99215690000000001</real>
				<key>Green Component</key>
				<real>0.4509804</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 5 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99215690000000001</real>
				<key>Green Component</key>
				<real>0.4509804</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 6 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.59607843137254901</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.63137254901960782</real>
				<key>Red Component</key>
				<real>0.16470588235294117</real>
			</dict>
			<key>Ansi 6 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99607840000000003</real>
				<key>Green Component</key>
				<real>0.77254900000000004</real>
				<key>Red Component</key>
				<real>0.77647060000000001</real>
			</dict>
			<key>Ansi 6 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.99607840000000003</real>
				<key>Green Component</key>
				<real>0.77254900000000004</real>
				<key>Red Component</key>
				<real>0.77647060000000001</real>
			</dict>
			<key>Ansi 7 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.83529411764705885</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.90980392156862744</real>
				<key>Red Component</key>
				<real>0.93333333333333335</real>
			</dict>
			<key>Ansi 7 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.93353169999999996</real>
				<key>Green Component</key>
				<real>0.93353169999999996</real>
				<key>Red Component</key>
				<real>0.93353169999999996</real>
			</dict>
			<key>Ansi 7 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.93353169999999996</real>
				<key>Green Component</key>
				<real>0.93353169999999996</real>
				<key>Red Component</key>
				<real>0.93353169999999996</real>
			</dict>
			<key>Ansi 8 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.21176470588235294</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.16862745098039217</real>
				<key>Red Component</key>
				<real>0.0</real>
			</dict>
			<key>Ansi 8 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.4862745</real>
				<key>Green Component</key>
				<real>0.4862745</real>
				<key>Red Component</key>
				<real>0.4862745</real>
			</dict>
			<key>Ansi 8 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.4862745</real>
				<key>Green Component</key>
				<real>0.4862745</real>
				<key>Red Component</key>
				<real>0.4862745</real>
			</dict>
			<key>Ansi 9 Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.086274509803921567</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.29411764705882354</real>
				<key>Red Component</key>
				<real>0.79607843137254897</real>
			</dict>
			<key>Ansi 9 Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.69019609999999998</real>
				<key>Green Component</key>
				<real>0.71372550000000001</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Ansi 9 Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.69019609999999998</real>
				<key>Green Component</key>
				<real>0.71372550000000001</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>BM Growl</key>
			<true/>
			<key>Background Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.8901960784313725</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.96470588235294119</real>
				<key>Red Component</key>
				<real>0.99215686274509807</real>
			</dict>
			<key>Background Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.0</real>
				<key>Green Component</key>
				<real>0.0</real>
				<key>Red Component</key>
				<real>0.0</real>
			</dict>
			<key>Background Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.0</real>
				<key>Green Component</key>
				<real>0.0</real>
				<key>Red Component</key>
				<real>0.0</real>
			</dict>
			<key>Background Image Location</key>
			<string>/Users/noahspector/Downloads/IMG_8414.jpg</string>
			<key>Background Image Mode</key>
			<integer>0</integer>
			<key>Cursor Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.51372549019607838</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.4823529411764706</real>
				<key>Red Component</key>
				<real>0.396078431372549</real>
			</dict>
			<key>Cursor Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>0.64705880000000005</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Cursor Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.37647059999999999</real>
				<key>Green Component</key>
				<real>0.64705880000000005</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Cursor Guide Color</key>
			<dict>
				<key>Alpha Component</key>
				<real>0.25</real>
				<key>Blue Component</key>
				<real>1</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.9268307089805603</real>
				<key>Red Component</key>
				<real>0.70213186740875244</real>
			</dict>
			<key>Cursor Guide Color (Dark)</key>
			<dict>
				<key>Alpha Component</key>
				<real>0.25</real>
				<key>Blue Component</key>
				<real>0.99125725030899048</real>
				<key>Color Space</key>
				<string>P3</string>
				<key>Green Component</key>
				<real>0.92047786712646484</real>
				<key>Red Component</key>
				<real>0.74862593412399292</real>
			</dict>
			<key>Cursor Guide Color (Light)</key>
			<dict>
				<key>Alpha Component</key>
				<real>0.25</real>
				<key>Blue Component</key>
				<real>0.99125725030899048</real>
				<key>Color Space</key>
				<string>P3</string>
				<key>Green Component</key>
				<real>0.92047786712646484</real>
				<key>Red Component</key>
				<real>0.74862593412399292</real>
			</dict>
			<key>Cursor Text Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.83529411764705885</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.90980392156862744</real>
				<key>Red Component</key>
				<real>0.93333333333333335</real>
			</dict>
			<key>Cursor Text Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>1</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Cursor Text Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>1</real>
				<key>Green Component</key>
				<real>1</real>
				<key>Red Component</key>
				<real>1</real>
			</dict>
			<key>Custom Command</key>
			<string>No</string>
			<key>Custom Directory</key>
			<string>No</string>
			<key>Default Bookmark</key>
			<string>No</string>
			<key>Description</key>
			<string>Default</string>
			<key>Disable Window Resizing</key>
			<true/>
			<key>Faint Text Alpha</key>
			<real>0.5</real>
			<key>Faint Text Alpha (Dark)</key>
			<real>0.5</real>
			<key>Faint Text Alpha (Light)</key>
			<real>0.5</real>
			<key>Flashing Bell</key>
			<false/>
			<key>Foreground Color</key>
			<dict>
				<key>Blue Component</key>
				<real>0.51372549019607838</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.4823529411764706</real>
				<key>Red Component</key>
				<real>0.396078431372549</real>
			</dict>
			<key>Foreground Color (Dark)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.73333334922790527</real>
				<key>Green Component</key>
				<real>0.73333334922790527</real>
				<key>Red Component</key>
				<real>0.73333334922790527</real>
			</dict>
			<key>Foreground Color (Light)</key>
			<dict>
				<key>Blue Component</key>
				<real>0.73333334922790527</real>
				<key>Green Component</key>
				<real>0.73333334922790527</real>
				<key>Red Component</key>
				<real>0.73333334922790527</real>
			</dict>
			<key>Guid</key>
			<string>F6D2AFD9-675C-4392-A2FE-5BF298108F8E</string>
			<key>Horizontal Spacing</key>
			<real>1</real>
			<key>Idle Code</key>
			<integer>0</integer>
			<key>Jobs to Ignore</key>
			<array>
				<string>rlogin</string>
				<string>ssh</string>
				<string>slogin</string>
				<string>telnet</string>
			</array>
			<key>Minimum Contrast</key>
			<real>0.0</real>
			<key>Minimum Contrast (Dark)</key>
			<real>0.0</real>
			<key>Minimum Contrast (Light)</key>
			<real>0.0</real>
			<key>Mouse Reporting</key>
			<true/>
			<key>Name</key>
			<string>Awesome Profile</string>
			<key>Use Separate Colors for Light and Dark Mode</key>
			<true/>
			<key>Use Tab Color</key>
			<false/>
			<key>Use Tab Color (Dark)</key>
			<false/>
			<key>Use Tab Color (Light)</key>
			<false/>
			<key>Use Underline Color</key>
			<false/>
			<key>Use Underline Color (Dark)</key>
			<false/>
			<key>Use Underline Color (Light)</key>
			<false/>
			<key>Vertical Spacing</key>
			<real>1</real>
			<key>Visual Bell</key>
			<true/>
			<key>Window Type</key>
			<integer>0</integer>
			<key>Working Directory</key>
			<string>/Users/noahspector</string>
		</dict>
		<dict>
			<key>Custom Command</key>
			<string>Custom Shell</string>
			<key>Custom Directory</key>
			<string>No</string>
			<key>Default Bookmark</key>
			<string>No</string>
			<key>Description</key>
			<string>Default</string>
			<key>Disable Window Resizing</key>
			<true/>
			<key>Flashing Bell</key>
			<false/>
			<key>Foreground Color</key>
			<dict>
				<key>Alpha Component</key>
				<real>1</real>
				<key>Blue Component</key>
				<real>0.062745101749897003</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.062745101749897003</real>
				<key>Red Component</key>
				<real>0.062745101749897003</real>
			</dict>
			<key>Foreground Color (Dark)</key>
			<dict>
				<key>Alpha Component</key>
				<real>1</real>
				<key>Blue Component</key>
				<real>0.86198854446411133</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.86199951171875</real>
				<key>Red Component</key>
				<real>0.86197912693023682</real>
			</dict>
			<key>Foreground Color (Light)</key>
			<dict>
				<key>Alpha Component</key>
				<real>1</real>
				<key>Blue Component</key>
				<real>0.062745098039215685</real>
				<key>Color Space</key>
				<string>sRGB</string>
				<key>Green Component</key>
				<real>0.062745098039215685</real>
				<key>Red Component</key>
				<real>0.062745098039215685</real>
			</dict>
			<key>Guid</key>
			<string>B24C1F3C-7A69-413F-97E1-1AC22CA7EC5D</string>
			<key>Has Hotkey</key>
			<true/>
			<key>Horizontal Spacing</key>
			<real>1</real>
			<key>HotKey Activated By Modifier</key>
			<false/>
			<key>HotKey Alternate Shortcuts</key>
			<array/>
			<key>HotKey Characters</key>
			<string>大</string>
			<key>HotKey Characters Ignoring Modifiers</key>
			<string>大</string>
			<key>HotKey Key Code</key>
			<integer>106</integer>
			<key>HotKey Modifier Activation</key>
			<integer>0</integer>
			<key>HotKey Modifier Flags</key>
			<integer>1310720</integer>
			<key>HotKey Window Animates</key>
			<true/>
			<key>HotKey Window AutoHides</key>
			<true/>
			<key>HotKey Window Dock Click Action</key>
			<integer>0</integer>
			<key>HotKey Window Floats</key>
			<false/>
			<key>HotKey Window Reopens On Activation</key>
			<false/>
			<key>Mouse Reporting</key>
			<true/>
			<key>Name</key>
			<string>Hotkey Window</string>
			<key>Non Ascii Font</key>
			<string>Monaco 12</string>
			<key>Non-ASCII Anti Aliased</key>
			<true/>
			<key>Normal Font</key>
			<string>Monaco 12</string>
			<key>Option Key Sends</key>
			<integer>0</integer>
			<key>Prompt Before Closing 2</key>
			<false/>
			<key>Right Option Key Sends</key>
			<integer>0</integer>
			<key>Rows</key>
			<integer>25</integer>
			<key>Screen</key>
			<integer>-1</integer>
			<key>Window Type</key>
			<integer>2</integer>
			<key>Working Directory</key>
			<string>/Users/noahspector</string>
		</dict>
	</array>
	<key>P3</key>
	<true/>
	<key>SoundForEsc</key>
	<false/>
	<key>ToolbeltTools</key>
	<array>
		<string>Snippets</string>
	</array>
	<key>VisualIndicatorForEsc</key>
	<false/>
	<key>kCPKSelectionViewPreferredModeKey</key>
	<integer>3</integer>
	<key>kCPKSelectionViewShowHSBTextFieldsKey</key>
	<false/>
</dict>
</plist>";
