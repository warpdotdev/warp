use std::borrow::Cow;

use anyhow::{anyhow, Result};
pub mod root_view;

extern crate black_ui;
use rust_embed::RustEmbed;
use black_ui::{platform, AssetProvider};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "examples/assets"]
pub struct Assets;

// The static assets we need to load in app.
pub static ASSETS: Assets = Assets;

// Implement the AssetProvider trait here (required by App::new).
impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

fn main() -> Result<()> {
    let app_builder =
        platform::AppBuilder::new(platform::AppCallbacks::default(), Box::new(ASSETS), None);
    let _ = app_builder.run(move |ctx| {
        ctx.add_window(
            black_ui::AddWindowOptions::default(),
            root_view::RootView::new,
        );
    });

    Ok(())
}
