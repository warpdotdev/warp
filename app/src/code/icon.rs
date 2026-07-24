use std::path::Path;

use warp_core::ui::appearance::Appearance;
use warpui::Element;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{CacheOption, Icon, Image};

/// Returns a special icon for the given file path, if any.
pub fn icon_from_file_path(path: &str, appearance: &Appearance) -> Option<Box<dyn Element>> {
    let theme = appearance.theme();
    let parsed_path = Path::new(path);
    let extension = parsed_path.extension().and_then(|ext| ext.to_str());

    let image = match extension {
        Some("rs") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/rust.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("json") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/json.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("ts") | Some("tsx") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/typescript.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("js") | Some("jsx") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/javascript.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("py") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/python.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("cpp") | Some("hpp") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/cpp.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("go") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/go.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
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
        Some("kt") | Some("kts") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/kotlin.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("php") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/php.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("pl") | Some("pm") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/perl.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("c") | Some("h") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/c.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("pyx") | Some("pxd") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/cython.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("swf") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/flash.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("wasm") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/wasm.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("zig") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/zig.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("sql") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/sql.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("ng") | Some("ngml") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/angular.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        Some("tf") | Some("hcl") | Some("tfvars") => Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/file_type/terraform.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
        _ => {
            return None;
        }
    };
    Some(image)
}
