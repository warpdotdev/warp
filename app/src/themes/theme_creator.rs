use std::path::PathBuf;

use anyhow::{Result, anyhow};
use deltae::*;
use kmeans_colors::{Calculate, CentroidData, Sort, get_kmeans_hamerly};
use palette::{FromColor, IntoColor, Lab, Pixel, Srgb, Srgba};
use pathfinder_color::ColorU;
use warp_errors::report_error;

use crate::util::color::hex_color::coloru_from_hex_string;

/// Uses a k-means algorithm to identify the 5 most average colors in an image.
pub fn top_colors_for_image(image_path: PathBuf) -> Result<Vec<ColorU>> {
    image::open(image_path)
        .map(|dynamic_image| {
            let image = dynamic_image.into_rgba8();
            let raw_image = image.as_raw();
            let lab: Vec<Lab> = Srgba::from_raw_slice(raw_image)
                .iter()
                .map(|x| x.into_format::<_, f32>().into_color())
                .collect();

            let result = get_kmeans_hamerly(5, 20, 5.0, false, &lab, 0_u64);
            let colors = Lab::sort_indexed_colors(&result.centroids, &result.indices);

            convert_centroids_to_hex_colors(&colors)
                .iter()
                .map(|color| match coloru_from_hex_string(color) {
                    Ok(color_u) => color_u,
                    Err(e) => {
                        report_error!(
                            anyhow::anyhow!("{e}")
                                .context("kmeans algorithm did not produce valid hex strings")
                        );
                        ColorU::black()
                    }
                })
                .collect::<Vec<_>>()
        })
        .map_err(|e| anyhow!(e))
}

fn convert_centroids_to_hex_colors<C: Calculate + Copy + IntoColor<Srgb>>(
    colors: &[CentroidData<C>],
) -> Vec<String> {
    colors
        .iter()
        .map(|color| format!("#{:x}", color.centroid.into_color().into_format::<u8>()))
        .collect::<Vec<_>>()
}

pub fn pick_accent_color_from_options(
    known_colors: &[ColorU],
    accent_options: &[ColorU],
) -> ColorU {
    if let Some(max_accent) = accent_options.iter().max_by_key(|color| {
        let accent_lab = lab_from_coloru(**color);
        known_colors
            .iter()
            .map(|known_color| {
                let known_color_lab = lab_from_coloru(*known_color);
                // Convert DeltaE to i64 to avoid floating point comparison issues. 4 decimal places is enough,
                // per DeltaE lib code.
                DeltaE::new(accent_lab, known_color_lab, DE2000)
                    .round_to(4)
                    .value()
                    * 10000.0
            })
            .sum::<f32>() as i64
    }) {
        *max_accent
    } else {
        log::warn!("At least one accent color should be provided.");
        ColorU::white()
    }
}

fn lab_from_coloru(color: ColorU) -> LabValue {
    let palette = Lab::from_color(Srgb::from_components((
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
    )));
    LabValue {
        l: palette.l,
        a: palette.a,
        b: palette.b,
    }
}

#[cfg(test)]
#[path = "theme_creator_tests.rs"]
mod tests;
