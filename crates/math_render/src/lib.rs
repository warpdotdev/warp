//! Typeset LaTeX math to self-contained SVG.
//!
//! This is the math analogue of `mermaid_to_svg`: it turns LaTeX math source
//! (the contents of `$...$` / `$$...$$` spans, without the delimiters) into an
//! SVG document string whose glyphs are embedded as outline paths, so the
//! result renders through `usvg`/`resvg` with no font dependencies.
//!
//! Typesetting is done by the RaTeX engine (KaTeX-level LaTeX coverage). On
//! parse failure an error is returned; callers are expected to fall back to
//! showing the raw LaTeX source.

use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;

const PADDING: f64 = 4.0;
const STROKE_WIDTH: f64 = 1.5;

#[derive(Debug, thiserror::Error)]
pub enum MathRenderError {
    #[error("failed to parse LaTeX math: {0}")]
    Parse(String),
    #[error("invalid color: {0}")]
    InvalidColor(String),
}

/// Render LaTeX math source to a self-contained SVG document.
///
/// * `latex` — math-mode LaTeX source, without `$`/`$$` delimiters.
/// * `display` — display style (`$$...$$`) vs. inline text style (`$...$`).
/// * `color` — glyph color as a CSS-style string (e.g. `#e0e0e0`), typically
///   the theme's foreground color so math matches the surrounding text.
/// * `font_size` — target em size in logical pixels; sets the SVG's intrinsic
///   size so the equation renders at the same scale as surrounding text.
pub fn render_math_to_svg(
    latex: &str,
    display: bool,
    color: &str,
    font_size: f64,
) -> Result<String, MathRenderError> {
    let color =
        Color::parse(color).ok_or_else(|| MathRenderError::InvalidColor(color.to_string()))?;
    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let layout_options = LayoutOptions::default().with_style(style).with_color(color);
    let svg_options = SvgOptions {
        font_size,
        padding: PADDING,
        stroke_width: STROKE_WIDTH,
        embed_glyphs: true,
        font_dir: String::new(),
    };

    let ast = parse(latex).map_err(|error| MathRenderError::Parse(error.to_string()))?;
    let layout_box = layout(&ast, &layout_options);
    let display_list = to_display_list(&layout_box);
    Ok(render_to_svg(&display_list, &svg_options))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
