use anyhow::anyhow;
use std::any::Any;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use bytes::Bytes;
use mermaid_to_svg::{MermaidError, MermaidTheme};
use warpui_core::assets::asset_cache::{
    AssetCache, AssetSource, AssetState, AsyncAssetId, AsyncAssetType,
};
use warpui_core::image_cache::ImageType;
use warpui_core::units::{IntoPixels, Pixels};
use warpui_core::{AppContext, SingletonEntity};

use crate::render::layout::TextLayout;
use crate::render::model::{BlockSpacing, ImageBlockConfig};

const DEFAULT_MERMAID_HEIGHT_LINE_MULTIPLIER: f32 = 10.0;
const FAILED_MERMAID_HEIGHT_LINE_MULTIPLIER: f32 = 2.0;

struct MermaidDiagramAsset;

impl AsyncAssetType for MermaidDiagramAsset {}

pub fn mermaid_asset_source(source: &str) -> AssetSource {
    let source = source.to_string();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let id = format!("light:{:x}", hasher.finish());
    let fetch_source = source.clone();

    AssetSource::Async {
        id: AsyncAssetId::new::<MermaidDiagramAsset>(id),
        fetch: Arc::new(move || {
            let source = fetch_source.clone();
            Box::pin(async move { render_mermaid_asset_bytes(&source) })
        }),
    }
}

fn render_mermaid_asset_bytes(source: &str) -> anyhow::Result<Bytes> {
    render_mermaid_asset_bytes_with(source, |source, theme| {
        mermaid_to_svg::render_mermaid_to_svg(source, Some(theme))
    })
}

fn render_mermaid_asset_bytes_with(
    source: &str,
    render: impl FnOnce(&str, &MermaidTheme) -> Result<String, MermaidError>,
) -> anyhow::Result<Bytes> {
    let theme = MermaidTheme::light();
    let svg = catch_unwind(AssertUnwindSafe(|| render(source, &theme))).map_err(|payload| {
        anyhow!(
            "Mermaid renderer panicked: {}",
            panic_message(payload.as_ref())
        )
    })??;

    Ok(Bytes::from(svg.into_bytes()))
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return message.to_string();
    }

    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }

    "unknown panic payload".to_string()
}
pub fn mermaid_diagram_layout(
    source: &str,
    layout: &TextLayout,
    spacing: BlockSpacing,
    app: &AppContext,
) -> (AssetSource, ImageBlockConfig) {
    let asset_source = mermaid_asset_source(source);
    let config = mermaid_diagram_config(&asset_source, layout, spacing, app);

    (asset_source, config)
}

fn mermaid_diagram_config(
    asset_source: &AssetSource,
    layout: &TextLayout,
    spacing: BlockSpacing,
    app: &AppContext,
) -> ImageBlockConfig {
    let max_width = layout.max_width() - spacing.x_axis_offset();
    let (width, height) = mermaid_diagram_size(asset_source, max_width, app).unwrap_or_else(|| {
        let height = layout.rich_text_styles().base_line_height()
            * mermaid_diagram_fallback_height_line_multiplier(asset_source, app).into_pixels();
        (max_width, height)
    });
    ImageBlockConfig {
        width,
        height,
        spacing,
    }
}

fn mermaid_diagram_fallback_height_line_multiplier(
    asset_source: &AssetSource,
    app: &AppContext,
) -> f32 {
    let asset_cache = AssetCache::as_ref(app);
    match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
        AssetState::FailedToLoad(_) => FAILED_MERMAID_HEIGHT_LINE_MULTIPLIER,
        AssetState::Loading { .. } | AssetState::Loaded { .. } | AssetState::Evicted => {
            DEFAULT_MERMAID_HEIGHT_LINE_MULTIPLIER
        }
    }
}
fn mermaid_diagram_size(
    asset_source: &AssetSource,
    max_width: Pixels,
    app: &AppContext,
) -> Option<(Pixels, Pixels)> {
    let asset_cache = AssetCache::as_ref(app);
    let AssetState::Loaded { data } = asset_cache.load_asset::<ImageType>(asset_source.clone())
    else {
        return None;
    };
    let ImageType::Svg { svg } = data.as_ref() else {
        return None;
    };
    let intrinsic_size = svg.size();
    let intrinsic_width = intrinsic_size.width();
    let intrinsic_height = intrinsic_size.height();
    if intrinsic_width <= 0. || intrinsic_height <= 0. {
        return None;
    }
    let width = max_width;
    let height = Pixels::new(width.as_f32() * intrinsic_height / intrinsic_width);
    Some((width, height))
}

#[cfg(test)]
#[path = "mermaid_diagram_tests.rs"]
mod tests;
