use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use warp_core::ui::appearance::Appearance;
use warpui::Element;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{CacheOption, Icon, Image};

#[path = "icon_data.rs"]
mod icon_data;

fn bundled_image(name: &'static str) -> Box<dyn Element> {
    Image::new(
        AssetSource::Bundled {
            path: name,
        },
        CacheOption::BySize,
    )
    .finish()
}

fn table_lookup(table: &'static [(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
    table.iter().copied().collect()
}

static FILE_EXTENSION_ICONS: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| table_lookup(icon_data::FILE_EXTENSION_ICONS));
static FILE_NAME_ICONS: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| table_lookup(icon_data::FILE_NAME_ICONS));
static FOLDER_NAME_ICONS: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| table_lookup(icon_data::FOLDER_NAME_ICONS));

/// Returns a special icon for the given file path, if any.
pub fn icon_from_file_path(path: &str, appearance: &Appearance) -> Option<Box<dyn Element>> {
    let theme = appearance.theme();
    let parsed_path = Path::new(path);
    let file_name = parsed_path.file_name().and_then(|s| s.to_str());
    let extension = parsed_path.extension().and_then(|ext| ext.to_str());

    if let Some(file_name) = file_name
        && let Some(asset) = FILE_NAME_ICONS.get(file_name.to_lowercase().as_str())
    {
        return Some(bundled_image(asset));
    }

    let image = match extension {
        Some("rs") => bundled_image("bundled/svg/file_type/rust.svg"),
        Some("json") => bundled_image("bundled/svg/file_type/json.svg"),
        Some("ts") | Some("tsx") => bundled_image("bundled/svg/file_type/typescript.svg"),
        Some("js") | Some("jsx") => bundled_image("bundled/svg/file_type/javascript.svg"),
        Some("py") => bundled_image("bundled/svg/file_type/python.svg"),
        Some("cpp") | Some("hpp") => bundled_image("bundled/svg/file_type/cpp.svg"),
        Some("go") => bundled_image("bundled/svg/file_type/go.svg"),
        Some("md") => Icon::new(
            "bundled/svg/file_type/markdown.svg",
            theme.main_text_color(theme.background()).into_solid(),
        )
        .finish(),
        Some("sh") => Icon::new(
            "bundled/svg/terminal.svg",
            theme.main_text_color(theme.background()).into_solid(),
        )
        .finish(),
        Some("kt") | Some("kts") => bundled_image("bundled/svg/file_type/kotlin.svg"),
        Some("php") => bundled_image("bundled/svg/file_type/php.svg"),
        Some("pl") | Some("pm") => bundled_image("bundled/svg/file_type/perl.svg"),
        Some("c") | Some("h") => bundled_image("bundled/svg/file_type/c.svg"),
        Some("pyx") | Some("pxd") => bundled_image("bundled/svg/file_type/cython.svg"),
        Some("swf") => bundled_image("bundled/svg/file_type/flash.svg"),
        Some("wasm") => bundled_image("bundled/svg/file_type/wasm.svg"),
        Some("zig") => bundled_image("bundled/svg/file_type/zig.svg"),
        Some("sql") => bundled_image("bundled/svg/file_type/sql.svg"),
        Some("ng") | Some("ngml") => bundled_image("bundled/svg/file_type/angular.svg"),
        Some("tf") | Some("hcl") | Some("tfvars") => {
            bundled_image("bundled/svg/file_type/terraform.svg")
        }
        Some(extension) => {
            if let Some(asset) = FILE_EXTENSION_ICONS.get(extension.to_lowercase().as_str()) {
                bundled_image(asset)
            } else {
                return None;
            }
        }
        None => {
            return None;
        }
    };
    Some(image)
}

/// Returns a special icon for the given folder path, if any.
pub fn icon_from_folder_path(path: &str) -> Option<Box<dyn Element>> {
    let folder_name = Path::new(path).file_name().and_then(|s| s.to_str())?;
    let asset = FOLDER_NAME_ICONS.get(folder_name.to_lowercase().as_str())?;
    Some(bundled_image(asset))
}

#[cfg(test)]
mod tests {
    use warp_core::ui::appearance::Appearance;

    use super::*;

    fn appearance() -> Appearance {
        Appearance::mock()
    }

    #[test]
    fn bespoke_extension_match_still_works() {
        assert!(icon_from_file_path("/repo/src/main.rs", &appearance()).is_some());
    }

    #[test]
    fn generated_extension_match_works() {
        assert!(icon_from_file_path("/repo/index.html", &appearance()).is_some());
    }

    #[test]
    fn exact_filename_match_with_no_extension() {
        assert!(icon_from_file_path("/repo/Dockerfile", &appearance()).is_some());
    }

    #[test]
    fn exact_filename_match_takes_priority_over_extension() {
        // package.json would also match the generic ".json" extension rule; the exact
        // filename rule (nodejs icon) must win.
        assert!(icon_from_file_path("/repo/package.json", &appearance()).is_some());
    }

    #[test]
    fn filename_match_is_case_insensitive() {
        assert!(icon_from_file_path("/repo/DOCKERFILE", &appearance()).is_some());
    }

    #[test]
    fn unmatched_file_returns_none() {
        assert!(icon_from_file_path("/repo/random.xyz", &appearance()).is_none());
    }

    #[test]
    fn folder_name_match() {
        assert!(icon_from_folder_path("/repo/src").is_some());
        assert!(icon_from_folder_path("/repo/node_modules").is_some());
    }

    #[test]
    fn dotfile_folder_name_match() {
        assert!(icon_from_folder_path("/repo/.git").is_some());
    }

    #[test]
    fn unmatched_folder_returns_none() {
        assert!(icon_from_folder_path("/repo/my_random_folder_name").is_none());
    }
}
