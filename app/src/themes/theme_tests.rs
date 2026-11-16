use settings_value::SettingsValue as _;

use super::*;
use crate::user_config;
use crate::util::color::OPAQUE;

fn custom_theme_json(path: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "My Theme",
        "path": path,
    })
}

fn custom_theme_from_serde_path(path: &str) -> CustomTheme {
    serde_json::from_value(custom_theme_json(path)).unwrap()
}

fn custom_theme_from_file_value_path(path: &str) -> CustomTheme {
    CustomTheme::from_file_value(&custom_theme_json(path)).unwrap()
}

fn assert_custom_theme_is_syncable(custom_theme: CustomTheme) {
    assert!(ThemeKind::Custom(custom_theme).is_custom_theme_reference_syncable());
}

fn assert_custom_theme_is_not_syncable(custom_theme: CustomTheme) {
    assert!(!ThemeKind::Custom(custom_theme).is_custom_theme_reference_syncable());
}

fn custom_theme_path_for_storage(path: &Path, theme_root: &Path) -> PathBuf {
    if path_is_absolute_or_foreign_absolute(path) {
        return portable_custom_theme_storage_string(path, theme_root)
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }

    path.to_str()
        .filter(|path| portable_stored_raw_components(path).is_some())
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

fn custom_theme_path_from_storage(path: &Path, theme_root: &Path) -> PathBuf {
    if path_is_absolute_or_foreign_absolute(path) {
        return portable_custom_theme_storage_string(path, theme_root)
            .map(|path| portable_custom_theme_path_from_stored_raw(&path, theme_root))
            .unwrap_or_else(|| path.to_path_buf());
    }

    path.to_str()
        .map(|path| portable_custom_theme_path_from_stored_raw(path, theme_root))
        .unwrap_or_else(|| path.to_path_buf())
}

#[test]
fn custom_theme_path_under_theme_root_storage_helper_returns_relative_path() {
    let root = PathBuf::from("/home/user/.local/share/warp-terminal/themes");
    let path = root.join("catppuccin/catppuccin_mocha.yml");

    assert_eq!(
        custom_theme_path_for_storage(&path, &root),
        PathBuf::from("catppuccin/catppuccin_mocha.yml")
    );
}

#[test]
fn custom_theme_relative_path_resolves_under_local_theme_root() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from("catppuccin/catppuccin_latte.yml");

    assert_eq!(
        custom_theme_path_from_storage(&stored, &root),
        root.join("catppuccin/catppuccin_latte.yml")
    );
}

#[test]
fn custom_theme_relative_parent_dir_path_is_preserved() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from("../outside.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_relative_parent_dir_path_is_not_portable() {
    let root = PathBuf::from("/Users/example/.warp/themes");

    assert!(!custom_theme_path_is_portable(
        &PathBuf::from("../outside.yml"),
        &root
    ));
}

#[test]
fn custom_theme_absolute_parent_dir_path_under_theme_root_storage_helper_preserves_path_and_rejects_portability()
 {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let path = root.join("../outside.yml");

    assert_eq!(custom_theme_path_from_storage(&path, &root), path);
    assert!(!custom_theme_path_is_portable(&path, &root));
    assert_eq!(custom_theme_path_for_storage(&path, &root), path);
}

#[test]
fn custom_theme_legacy_macos_path_is_preserved_even_when_local_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("warp-terminal/themes");
    let local = root.join("catppuccin/catppuccin_mocha.yml");
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(&local, "").unwrap();

    let stored = PathBuf::from("/Users/example/.warp/themes/catppuccin/catppuccin_mocha.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_legacy_parent_dir_path_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("warp-terminal/themes");
    let local = root.join("outside.yml");
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(&local, "").unwrap();

    let stored = PathBuf::from("/Users/example/.warp/themes/../outside.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_legacy_linux_path_is_preserved_even_when_local_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".warp/themes");
    let local = root.join("catppuccin/catppuccin_latte.yml");
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(&local, "").unwrap();

    let stored = PathBuf::from(
        "/home/user/.local/share/warp-terminal/themes/catppuccin/catppuccin_latte.yml",
    );

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_unmatched_legacy_absolute_path_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".warp/themes");
    let stored = PathBuf::from("/Users/example/.warp/themes/missing.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_windows_absolute_path_string_is_preserved() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"C:\Users\example\AppData\Roaming\warp\Warp\data\themes\mocha.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_windows_absolute_path_string_is_not_portable() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"C:\Users\example\AppData\Roaming\warp\Warp\data\themes\mocha.yml");

    assert!(!custom_theme_path_is_portable(&stored, &root));
}

#[test]
fn custom_theme_windows_absolute_path_string_storage_helper_preserves_path() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"C:\Users\example\AppData\Roaming\warp\Warp\data\themes\mocha.yml");

    assert_eq!(custom_theme_path_for_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_windows_unc_path_string_is_preserved() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"\\server\share\warp\themes\mocha.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
#[cfg(not(windows))]
fn custom_theme_relative_backslash_path_is_preserved() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"catppuccin\mocha.yml");

    assert_eq!(custom_theme_path_from_storage(&stored, &root), stored);
}

#[test]
#[cfg(not(windows))]
fn custom_theme_relative_backslash_path_is_not_portable() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"catppuccin\mocha.yml");

    assert!(!custom_theme_path_is_portable(&stored, &root));
}

#[test]
#[cfg(not(windows))]
fn custom_theme_relative_backslash_path_storage_helper_preserves_path() {
    let root = PathBuf::from("/Users/example/.warp/themes");
    let stored = PathBuf::from(r"catppuccin\mocha.yml");

    assert_eq!(custom_theme_path_for_storage(&stored, &root), stored);
}

#[test]
fn custom_theme_serde_reads_portable_raw_path_under_theme_root() {
    let custom = custom_theme_from_serde_path("catppuccin/mocha.yml");

    assert_eq!(
        custom.path(),
        user_config::themes_dir()
            .join("catppuccin")
            .join("mocha.yml")
    );
    assert_custom_theme_is_syncable(custom);
}

#[test]
fn custom_theme_serde_preserves_unportable_raw_paths() {
    for raw_path in [
        "",
        ".",
        "./mocha.yml",
        "../outside.yml",
        "catppuccin/../mocha.yml",
        r"catppuccin\mocha.yml",
        "C:/Users/example/AppData/Roaming/warp/Warp/data/themes/mocha.yml",
        "C:themes/mocha.yml",
    ] {
        let custom = custom_theme_from_serde_path(raw_path);

        assert_eq!(custom.path(), PathBuf::from(raw_path));
        assert_custom_theme_is_not_syncable(custom);
    }
}

#[test]
fn custom_theme_settings_value_reads_portable_raw_path_under_theme_root() {
    let custom = custom_theme_from_file_value_path("catppuccin/mocha.yml");

    assert_eq!(
        custom.path(),
        user_config::themes_dir()
            .join("catppuccin")
            .join("mocha.yml")
    );
    assert_custom_theme_is_syncable(custom);
}

#[test]
fn custom_theme_settings_value_preserves_unportable_raw_paths() {
    for raw_path in [
        "",
        ".",
        "./mocha.yml",
        "../outside.yml",
        "catppuccin/../mocha.yml",
        r"catppuccin\mocha.yml",
        "C:/Users/example/AppData/Roaming/warp/Warp/data/themes/mocha.yml",
        "C:themes/mocha.yml",
    ] {
        let custom = custom_theme_from_file_value_path(raw_path);

        assert_eq!(custom.path(), PathBuf::from(raw_path));
        assert_custom_theme_is_not_syncable(custom);
    }
}

#[test]
fn custom_theme_settings_value_writes_portable_path_for_theme_root_file() {
    let root_path = user_config::themes_dir().join("my_theme.yml");
    let custom = CustomTheme::new("My Theme".to_string(), root_path);

    let value = custom.to_file_value();

    assert_eq!(value["name"], "My Theme");
    assert_eq!(value["path"], "my_theme.yml");
}

#[test]
fn custom_theme_serde_writes_portable_path_for_theme_root_file() {
    let root_path = user_config::themes_dir().join("my_theme.yml");
    let custom = CustomTheme::new("My Theme".to_string(), root_path);

    let value = serde_json::to_value(custom).unwrap();

    assert_eq!(value["name"], "My Theme");
    assert_eq!(value["path"], "my_theme.yml");
}

#[test]
fn custom_base16_theme_kind_uses_custom_theme_settings_value_path_rules() {
    let root_path = user_config::themes_dir().join("base16/ocean.yml");
    let kind = ThemeKind::CustomBase16(CustomTheme::new("Base16 Ocean".to_string(), root_path));

    assert_eq!(
        kind.to_file_value(),
        serde_json::json!({
            "custom_base_16": {
                "name": "Base16 Ocean",
                "path": "base16/ocean.yml"
            }
        })
    );
}

#[cfg(windows)]
mod windows_custom_theme_path_tests {
    use super::*;

    fn windows_theme_root() -> PathBuf {
        PathBuf::from(r"C:\Users\example\AppData\Roaming\warp\Warp\data\themes")
    }

    #[test]
    fn custom_theme_windows_theme_root_path_serializes_with_slashes() {
        let root = windows_theme_root();
        let path = root.join("catppuccin").join("mocha.yml");

        assert_eq!(
            custom_theme_path_for_storage(&path, &root),
            PathBuf::from("catppuccin/mocha.yml")
        );
        assert_eq!(
            portable_custom_theme_storage_string(&path, &root).as_deref(),
            Some("catppuccin/mocha.yml")
        );
    }

    #[test]
    fn custom_theme_windows_slash_stored_path_resolves_under_theme_root() {
        let root = windows_theme_root();
        let stored = PathBuf::from("catppuccin/mocha.yml");

        assert_eq!(
            custom_theme_path_from_storage(&stored, &root),
            root.join("catppuccin").join("mocha.yml")
        );
        assert_eq!(
            portable_custom_theme_path_from_stored_raw("catppuccin/mocha.yml", &root),
            root.join("catppuccin").join("mocha.yml")
        );
    }

    #[test]
    fn custom_theme_windows_theme_root_path_is_portable() {
        let root = windows_theme_root();
        let path = root.join("catppuccin").join("mocha.yml");

        assert!(custom_theme_path_is_portable(&path, &root));
    }

    #[test]
    fn custom_theme_windows_raw_unportable_stored_paths_are_preserved() {
        let root = windows_theme_root();

        for raw_path in [
            r"catppuccin\mocha.yml",
            "C:/Users/example/AppData/Roaming/warp/Warp/data/themes/mocha.yml",
            "C:themes/mocha.yml",
        ] {
            assert_eq!(
                portable_custom_theme_path_from_stored_raw(raw_path, &root),
                PathBuf::from(raw_path)
            );
        }
    }

    #[test]
    fn custom_theme_windows_raw_unportable_paths_are_not_portable() {
        let root = windows_theme_root();

        for raw_path in [
            r"catppuccin\mocha.yml",
            "C:/Users/example/AppData/Roaming/warp/Warp/data/themes/mocha.yml",
            "C:themes/mocha.yml",
        ] {
            assert!(!custom_theme_path_is_portable(
                &PathBuf::from(raw_path),
                &root
            ));
        }
    }

    #[test]
    fn custom_theme_windows_settings_value_serializes_theme_root_file_with_slashes() {
        let root_path = user_config::themes_dir()
            .join("catppuccin")
            .join("mocha.yml");
        let custom = CustomTheme::new("Mocha".to_string(), root_path);

        let value = custom.to_file_value();

        assert_eq!(value["path"], "catppuccin/mocha.yml");
    }

    #[test]
    fn custom_theme_windows_serde_serializes_theme_root_file_with_slashes() {
        let root_path = user_config::themes_dir()
            .join("catppuccin")
            .join("mocha.yml");
        let custom = CustomTheme::new("Mocha".to_string(), root_path);

        let value = serde_json::to_value(custom).unwrap();

        assert_eq!(value["path"], "catppuccin/mocha.yml");
    }
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn in_memory_theme_generation_test() {
    let mountains_bg_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "assets",
        "async",
        "jpg",
        "mountains.jpg",
    ]
    .iter()
    .collect();

    let mut in_memory_theme = warpui::r#async::block_on(InMemoryThemeOptions::new(
        "mountains".to_string(),
        mountains_bg_path.clone(),
    ))
    .unwrap();

    let mountains_bg_path_string = mountains_bg_path.to_str().unwrap_or_default().to_owned();
    assert_eq!(
        in_memory_theme.theme(),
        WarpTheme::new(
            // the theme defaults to the 0th bg color
            ColorU::new(35, 31, 44, OPAQUE).into(),
            // this background color makes it a "dark" theme, so the foreground is white
            ColorU::white(),
            // the most distinct accent color is 3rd one
            ColorU::new(238, 203, 111, OPAQUE).into(),
            None,
            Some(Details::Darker),
            dark_mode_colors(),
            Some(Image {
                source: AssetSource::LocalFile {
                    path: mountains_bg_path_string.clone(),
                    content_version: None,
                },
                opacity: 30,
            }),
            Some("mountains".to_string()),
        )
    );

    in_memory_theme.chosen_bg_color_index = 2;

    assert_eq!(
        in_memory_theme.theme(),
        WarpTheme::new(
            // now the background is the 2nd one
            ColorU::new(229, 142, 113, OPAQUE).into(),
            // changing the background color made this a light theme
            ColorU::black(),
            // now the 4th color is the most distinct color
            ColorU::new(193, 217, 212, OPAQUE).into(),
            None,
            Some(Details::Lighter),
            light_mode_colors(),
            Some(Image {
                source: AssetSource::LocalFile {
                    path: mountains_bg_path_string,
                    content_version: None,
                },
                opacity: 30,
            }),
            Some("mountains".to_string()),
        )
    );
}
