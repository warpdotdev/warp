//! Measures how many terminal cells a shaped Indic grapheme cluster needs,
//! based on its real Core Text advance width rather than a fixed per-script
//! guess. This is the input-time counterpart to the render-time shaping
//! that `grid_renderer.rs` already performs — see `Cell::span`/`set_span`
//! (crates/warp_terminal/src/model/grid/cell.rs) for the storage side.
//!
//! Deliberately decoupled from `AppContext`/the app-wide font `Cache`
//! singleton: `Cache`'s font-loading methods take `&mut self`, which is
//! incompatible with sharing it via `Arc` into the model layer (confirmed
//! by direct investigation — see the plan's Phase 0 execution log). The
//! real implementation instead owns a small, independent font database
//! dedicated to this one purpose.

/// Measures the number of terminal cells (1-8, matching `Cell::span`'s
/// encoding limit) a grapheme cluster string needs, relative to a single
/// cell's pixel width. Font size is fixed at construction time (not passed
/// per call): the measurer is meant to be reconstructed whenever the
/// terminal's font/zoom changes, same as `SizeInfo`/`cell_width_px` already
/// is, so `ansi_handler.rs`'s hot input path never needs to thread a font
/// size value through per character.
pub trait ClusterWidthMeasurer: Send + Sync {
    fn measure_cells(&self, cluster: &str, cell_width_px: f32) -> u8;
}

/// Always reports a single cell. Used as the default for test-only
/// `GridHandler` constructors, and anywhere Indic shaping genuinely isn't
/// available (e.g. non-macOS builds, before a platform-specific measurer
/// exists).
pub struct NoopMeasurer;

impl ClusterWidthMeasurer for NoopMeasurer {
    fn measure_cells(&self, _cluster: &str, _cell_width_px: f32) -> u8 {
        1
    }
}

// `warpui::platform::mac` (and therefore the Core Text shaping path) only
// compiles on macOS -- see `crates/warpui/src/platform/mod.rs`'s
// `#[cfg(target_os = "macos")] pub mod mac;`. Gate the real measurer the
// same way; other platforms fall back to `NoopMeasurer` at the construction
// call site.
#[cfg(target_os = "macos")]
mod mac_impl {
    use warpui::fonts::{FamilyId, Properties};
    use warpui::platform::mac::FontDB as MacFontDB;
    use warpui::platform::{FontDB as FontDBTrait, LineStyle, TextLayoutSystem as _};
    use warpui::text_layout::{ClipConfig, StyleAndFont, DEFAULT_TOP_BOTTOM_RATIO};

    use super::ClusterWidthMeasurer;

    /// Real macOS implementation, backed by a dedicated Core Text font
    /// database loaded with the terminal's configured font family.
    /// Constructed once (loading is `&mut self`); every subsequent call is
    /// `&self`-only, so no interior mutability is needed once construction
    /// completes.
    pub struct CoreTextClusterMeasurer {
        font_db: MacFontDB,
        family_id: FamilyId,
        properties: Properties,
        font_size: f32,
    }

    impl CoreTextClusterMeasurer {
        /// Loads `font_family_name` into a fresh, independent font database
        /// and returns a measurer for it, fixed at `font_size`. Returns
        /// `Err` if the family can't be resolved on the system (caller
        /// should fall back to `NoopMeasurer`).
        pub fn new(
            font_family_name: &str,
            properties: Properties,
            font_size: f32,
        ) -> anyhow::Result<Self> {
            let mut font_db = MacFontDB::default();
            let family_id = font_db.load_from_system(font_family_name)?;
            Ok(Self {
                font_db,
                family_id,
                properties,
                font_size,
            })
        }
    }

    impl ClusterWidthMeasurer for CoreTextClusterMeasurer {
        fn measure_cells(&self, cluster: &str, cell_width_px: f32) -> u8 {
            if cell_width_px <= 0.0 || cluster.is_empty() {
                return 1;
            }
            let run_length_chars = cluster.chars().count();
            let line = self.font_db.layout_line(
                cluster,
                LineStyle {
                    font_size: self.font_size,
                    line_height_ratio: 1.0,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                },
                &[(
                    0..run_length_chars,
                    StyleAndFont {
                        font_family: self.family_id,
                        properties: self.properties,
                        style: Default::default(),
                    },
                )],
                f32::MAX,
                ClipConfig::default(),
            );
            let cells = (line.width / cell_width_px).ceil() as u8;
            cells.clamp(1, 8)
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac_impl::CoreTextClusterMeasurer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_measurer_always_returns_one() {
        let measurer = NoopMeasurer;
        assert_eq!(measurer.measure_cells("ప్రభుత్వం", 10.0), 1);
        assert_eq!(measurer.measure_cells("", 10.0), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_measurer_measures_real_telugu_cluster() {
        use warpui::fonts::Properties;

        // Menlo is a bundled system font guaranteed present in CI/dev macOS
        // environments; Core Text will fall back to a Telugu-capable font
        // automatically when shaping non-Latin text, same as the render
        // path already does in grid_renderer.rs::render_indic_cluster.
        let measurer = CoreTextClusterMeasurer::new("Menlo", Properties::default(), 13.0)
            .expect("Menlo should be resolvable on any macOS system");

        // A multi-syllable Telugu word should need more than one narrow
        // monospace cell's worth of width.
        let cells = measurer.measure_cells("ప్రభుత్వం", 8.0);
        assert!(
            cells >= 2,
            "expected a multi-syllable Telugu word to need >= 2 cells at a narrow cell width, got {cells}"
        );
        assert!(cells <= 8, "measured span must stay within the 1-8 encoding limit, got {cells}");

        // A single ASCII char should fit in one narrow cell already.
        let ascii_cells = measurer.measure_cells("a", 8.0);
        assert_eq!(ascii_cells, 1);
    }
}
