use std::borrow::Cow;

use anyhow::{Result, anyhow};
use pathfinder_geometry::vector::vec2f;
pub mod root_view;

extern crate warpui;
use rust_embed::RustEmbed;
use warpui::platform::WindowBounds;
use warpui::{AssetProvider, platform};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "examples/assets"]
pub struct Assets;

pub static ASSETS: Assets = Assets;

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

#[derive(Debug, Clone, Default)]
pub struct CaptureConfig {
    pub capture_screenshots: bool,
    pub capture_baseline: bool,
}

fn parse_args() -> CaptureConfig {
    let args: Vec<String> = std::env::args().collect();
    let mut config = CaptureConfig::default();

    for arg in args.iter() {
        match arg.as_str() {
            "--capture-screenshots" => config.capture_screenshots = true,
            "--capture-baseline" => {
                config.capture_screenshots = true;
                config.capture_baseline = true;
            }
            "--help" | "-h" => {
                println!("Table Sample Example - Screenshot Testing");
                println!("\nUsage: table-sample [OPTIONS]");
                println!("\nOptions:");
                println!("  --capture-screenshots  Capture screenshots of all demos");
                println!("  --capture-baseline     Capture and save as baseline screenshots");
                println!("  --help, -h             Show this help message");
                std::process::exit(0);
            }
            _ => {}
        }
    }

    config
}

fn main() -> Result<()> {
    env_logger::builder().format_timestamp_millis().init();
    let capture_config = parse_args();

    if capture_config.capture_screenshots {
        println!("📸 Screenshot capture mode enabled");
        if capture_config.capture_baseline {
            println!("📁 Baseline mode: screenshots will be saved as reference images");
        }
    }

    let app_builder =
        platform::AppBuilder::new(platform::AppCallbacks::default(), Box::new(ASSETS), None);
    let _ = app_builder.run(move |ctx| {
        root_view::init(ctx);
        let window_options = warpui::AddWindowOptions {
            window_bounds: WindowBounds::ExactSize(vec2f(1000.0, 800.0)),
            window_style: if capture_config.capture_screenshots {
                warpui::platform::WindowStyle::NotStealFocus
            } else {
                warpui::platform::WindowStyle::Normal
            },
            ..Default::default()
        };
        let config = capture_config.clone();
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
        let (window_id, _root) = ctx.add_window(window_options, move |view_ctx| {
            root_view::RootView::new(view_ctx, config)
        });
        #[cfg(target_os = "macos")]
        if capture_config.capture_screenshots {
            // Make it visible for rendering but keep z-index
            ctx.windows()
                .show_window_and_focus_app_without_ordering_front(window_id);
        }
    });

    Ok(())
}
