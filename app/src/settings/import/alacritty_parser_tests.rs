use async_io::block_on;
use virtual_fs::{Stub, VirtualFS};
use warp_core::ui::color::hex_color::coloru_from_hex_string;
use warp_core::ui::theme::AnsiColor;

use super::{
    AlacrittyColors, AlacrittyConfig, AlacrittyTheme, PrimaryAlacrittyColors, RecursivelyParseable,
};
use crate::settings::import::config::{ParseableConfig, ThemeType};

#[test]
fn test_parse_cobalt2() {
    let cobalt2_config = "# From the famous Cobalt2 sublime theme
    # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
    
    # Default colors
    [colors.primary]
    background = '#122637'
    foreground = '#ffffff'
    
    [colors.cursor]
    text = '#122637'
    cursor = '#f0cb09'
    
    # Normal colors
    [colors.normal]
    black   = '#000000'
    red     = '#ff0000'
    green   = '#37dd21'
    yellow  = '#fee409'
    blue    = '#1460d2'
    magenta = '#ff005d'
    cyan    = '#00bbbb'
    white   = '#bbbbbb'
    
    # Bright colors
    [colors.bright]
    black   = '#545454'
    red     = '#f40d17'
    green   = '#3bcf1d'
    yellow  = '#ecc809'
    blue    = '#5555ff'
    magenta = '#ff55ff'
    cyan    = '#6ae3f9'
    white   = '#ffffff'";
    let config: AlacrittyConfig =
        toml::from_str(cobalt2_config).expect("Should be able to parse toml!");
    let ThemeType::Single(theme) = config
        .colors
        .expect("Should have read colors!")
        .parse()
        .expect("Theme should have read!")
    else {
        panic!("Should not have a dark and light theme for Alacritty!")
    };

    // Check that the three primary colors are the same.
    assert_eq!(
        theme.accent().into_solid(),
        coloru_from_hex_string("#f0cb09").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.background().into_solid(),
        coloru_from_hex_string("#122637").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.foreground().into_solid(),
        coloru_from_hex_string("#ffffff").expect("Should be able to parse a color!")
    );

    // Check some terminal colors.
    assert_eq!(
        theme.terminal_colors().bright.yellow,
        AnsiColor {
            r: 0xec,
            g: 0xc8,
            b: 0x09
        }
    );
    assert_eq!(
        theme.terminal_colors().bright.cyan,
        AnsiColor {
            r: 0x6a,
            g: 0xe3,
            b: 0xf9
        }
    );
    assert_eq!(
        theme.terminal_colors().normal.yellow,
        AnsiColor {
            r: 0xfe,
            g: 0xe4,
            b: 0x09
        }
    );
    assert_eq!(
        theme.terminal_colors().normal.cyan,
        AnsiColor {
            r: 0x00,
            g: 0xbb,
            b: 0xbb
        }
    );
}

#[test]
fn test_parse_cobalt2_missing_color() {
    let cobalt2_config = "# From the famous Cobalt2 sublime theme
    # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
    
    # Default colors
    [colors.primary]
    background = '#122637'
    foreground = '#ffffff'
    
    [colors.cursor]
    text = '#122637'
    cursor = '#f0cb09'
    
    # Normal colors
    [colors.normal]
    red     = '#ff0000'
    green   = '#37dd21'
    yellow  = '#fee409'
    blue    = '#1460d2'
    magenta = '#ff005d'
    cyan    = '#00bbbb'
    white   = '#bbbbbb'
    
    # Bright colors
    [colors.bright]
    black   = '#545454'
    red     = '#f40d17'
    green   = '#3bcf1d'
    yellow  = '#ecc809'
    blue    = '#5555ff'
    magenta = '#ff55ff'
    cyan    = '#6ae3f9'
    white   = '#ffffff'";
    let config: AlacrittyConfig =
        toml::from_str(cobalt2_config).expect("Should be able to parse toml!");
    let ThemeType::Single(theme) = config
        .colors
        .expect("Should have read colors!")
        .parse()
        .expect("Theme should have read!")
    else {
        panic!("Should not have a dark and light theme for Alacritty!")
    };

    // Check that the three primary colors are the same.
    assert_eq!(
        theme.accent().into_solid(),
        coloru_from_hex_string("#f0cb09").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.background().into_solid(),
        coloru_from_hex_string("#122637").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.foreground().into_solid(),
        coloru_from_hex_string("#ffffff").expect("Should be able to parse a color!")
    );

    // Check some terminal colors.
    assert_eq!(
        theme.terminal_colors().bright.yellow,
        AnsiColor {
            r: 0xec,
            g: 0xc8,
            b: 0x09
        }
    );
    assert_eq!(
        theme.terminal_colors().bright.cyan,
        AnsiColor {
            r: 0x6a,
            g: 0xe3,
            b: 0xf9
        }
    );
    assert_eq!(
        theme.terminal_colors().normal.yellow,
        AnsiColor {
            r: 0xfe,
            g: 0xe4,
            b: 0x09
        }
    );
    assert_eq!(
        theme.terminal_colors().normal.cyan,
        AnsiColor {
            r: 0x00,
            g: 0xbb,
            b: 0xbb
        }
    );
    assert_eq!(
        theme.terminal_colors().normal.black,
        AnsiColor {
            r: 0x18,
            g: 0x18,
            b: 0x18,
        }
    );
}

#[test]
fn test_parse_cobalt2_missing_section() {
    let cobalt2_config = "# From the famous Cobalt2 sublime theme
    # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
    
    # Default colors
    [colors.primary]
    background = '#122637'
    foreground = '#ffffff'
    
    [colors.cursor]
    text = '#122637'
    cursor = '#f0cb09'";
    let config: AlacrittyConfig =
        toml::from_str(cobalt2_config).expect("Should be able to parse toml!");
    let ThemeType::Single(theme) = config
        .colors
        .expect("Should have read colors!")
        .parse()
        .expect("Theme should have read!")
    else {
        panic!("Should not have a dark and light theme for Alacritty!")
    };

    // Check that the three primary colors are the same.
    assert_eq!(
        theme.accent().into_solid(),
        coloru_from_hex_string("#f0cb09").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.background().into_solid(),
        coloru_from_hex_string("#122637").expect("Should be able to parse a color!")
    );
    assert_eq!(
        theme.foreground().into_solid(),
        coloru_from_hex_string("#ffffff").expect("Should be able to parse a color!")
    );
}

#[test]
fn test_parse_cobalt2_bad_terminal_color() {
    let cobalt2_config = "# From the famous Cobalt2 sublime theme
    # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
    
    # Default colors
    [colors.primary]
    background = '#122637'
    foreground = '#ffffff'
    
    [colors.cursor]
    text = '#122637'
    cursor = '#f0cb09'
    
    # Normal colors
    [colors.normal]
    red     = '#ff000'
    green   = '#37dd21'
    yellow  = '#fee409'
    blue    = '#1460d2'
    magenta = '#ff005d'
    cyan    = '#00bbbb'
    white   = '#bbbbbb'
    
    # Bright colors
    [colors.bright]
    black   = '#545454'
    red     = '#f40d17'
    green   = '#3bcf1d'
    yellow  = '#ecc809'
    blue    = '#5555ff'
    magenta = '#ff55ff'
    cyan    = '#6ae3f9'
    white   = '#ffffff'";
    let config: AlacrittyConfig =
        toml::from_str(cobalt2_config).expect("Should be able to parse toml!");
    let _ = config
        .colors
        .expect("Should have read colors!")
        .parse()
        .expect_err("Theme should not have read!");
}

#[test]
fn test_parse_cobalt2_no_colors() {
    let cobalt2_config = "# From the famous Cobalt2 sublime theme
    # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
    
    # Default colors
    [colors.primary]
    background = ''
    foreground = ''
    
    [colors.cursor]
    text = ''
    cursor = ''";
    let config: AlacrittyConfig =
        toml::from_str(cobalt2_config).expect("Should be able to parse toml!");
    let _ = config
        .colors
        .expect("Should have read colors!")
        .parse()
        .expect_err("Theme should not have read!");
}

#[test]
fn test_merge_left() {
    let mut colors = AlacrittyTheme {
        primary: Some(PrimaryAlacrittyColors {
            foreground: Some("#ffffff".to_string()),
            background: Some("#000000".to_string()),
        }),
        normal: Some(AlacrittyColors {
            black: Some("#000000".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let second_colors = AlacrittyTheme {
        primary: Some(PrimaryAlacrittyColors {
            foreground: None,
            background: Some("#ff0000".to_string()),
        }),
        normal: Some(AlacrittyColors {
            red: Some("#ff1111".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    colors = colors.merge_left(second_colors);
    let primary = colors
        .primary
        .clone()
        .expect("Primary colors should be present");
    let normal = colors
        .normal
        .clone()
        .expect("Normal colors should be present");

    // Check all four combinations of present and absent in the inner struct.
    assert_eq!(primary.foreground, Some("#ffffff".to_string()));
    assert_eq!(primary.background, Some("#000000".to_string()));
    assert_eq!(normal.black, Some("#000000".to_string()));
    assert_eq!(normal.red, Some("#ff1111".to_string()));
    assert_eq!(normal.white, None);

    // Check that None values are preserved.
    assert!(colors.bright.is_none());
}

/// This is a unit test that tests reading from the default config location with one import.
#[test]
fn test_parse_cobalt2_from_import() {
    VirtualFS::test("test_parse_cobalt2_from_import", |dirs, mut sandbox| {
        sandbox.mkdir("config");
        sandbox.with_files(vec![
            Stub::FileWithContent(
                "config/config.toml",
                format!(
                    "
            import = [{:?}]
            ",
                    dirs.tests().join("config").join("Cobalt2.toml")
                )
                .as_str(),
            ),
            Stub::FileWithContent(
                "config/Cobalt2.toml",
                "# From the famous Cobalt2 sublime theme
                # Source  https//github.com/wesbos/cobalt2/tree/master/Cobalt2
                
                # Default colors
                [colors.primary]
                background = '#122637'
                foreground = '#ffffff'
                
                [colors.cursor]
                text = '#122637'
                cursor = '#f0cb09'
                
                # Normal colors
                [colors.normal]
                black   = '#000000'
                red     = '#ff0000'
                green   = '#37dd21'
                yellow  = '#fee409'
                blue    = '#1460d2'
                magenta = '#ff005d'
                cyan    = '#00bbbb'
                white   = '#bbbbbb'
                
                # Bright colors
                [colors.bright]
                black   = '#545454'
                red     = '#f40d17'
                green   = '#3bcf1d'
                yellow  = '#ecc809'
                blue    = '#5555ff'
                magenta = '#ff55ff'
                cyan    = '#6ae3f9'
                white   = '#ffffff'",
            ),
        ]);
        let config: AlacrittyConfig = block_on(AlacrittyConfig::from_file(
            dirs.tests().join("config").join("config.toml"),
        ))
        .expect("Should be able to read file!")
        .pop()
        .expect("Should have returned at least one config!");
        let ThemeType::Single(theme) = config
            .colors
            .expect("Should have read colors!")
            .parse()
            .expect("Theme should have read!")
        else {
            panic!("Should not have a dark and light theme for Alacritty!")
        };

        // Check that the three primary colors are the same.
        assert_eq!(
            theme.accent().into_solid(),
            coloru_from_hex_string("#f0cb09").expect("Should be able to parse a color!")
        );
        assert_eq!(
            theme.background().into_solid(),
            coloru_from_hex_string("#122637").expect("Should be able to parse a color!")
        );
        assert_eq!(
            theme.foreground().into_solid(),
            coloru_from_hex_string("#ffffff").expect("Should be able to parse a color!")
        );

        // Check some terminal colors.
        assert_eq!(
            theme.terminal_colors().bright.yellow,
            AnsiColor {
                r: 0xec,
                g: 0xc8,
                b: 0x09
            }
        );
        assert_eq!(
            theme.terminal_colors().bright.cyan,
            AnsiColor {
                r: 0x6a,
                g: 0xe3,
                b: 0xf9
            }
        );
        assert_eq!(
            theme.terminal_colors().normal.yellow,
            AnsiColor {
                r: 0xfe,
                g: 0xe4,
                b: 0x09
            }
        );
        assert_eq!(
            theme.terminal_colors().normal.cyan,
            AnsiColor {
                r: 0x00,
                g: 0xbb,
                b: 0xbb
            }
        );
    });
}
