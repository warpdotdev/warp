//! Chart colors for the usage popover's stacked bars and row swatches.
//!
//! Colors come from the Figma "Pricing transparency" chart palette rather than
//! the app's ANSI palette, so the bars read as data-visualization segments
//! rather than terminal-themed content.

use pathfinder_color::ColorU;

const fn rgb(r: u8, g: u8, b: u8) -> ColorU {
    ColorU { r, g, b, a: 0xff }
}

/// Colors assigned to breakdown rows by position.
const CHART_PALETTE: [ColorU; 6] = [
    rgb(0xff, 0x8f, 0xfd), // magenta
    rgb(0xa5, 0xd5, 0xfe), // blue
    rgb(0xfe, 0xfd, 0xc2), // yellow
    rgb(0xd0, 0xd1, 0xfe), // cyan / lavender
    rgb(0xb4, 0xfa, 0x72), // green
    rgb(0xff, 0x82, 0x72), // red
];

/// Reserved for the orchestrator row, which keeps a fixed identity color rather
/// than taking one by position. Drawn from the same palette family so the bar
/// stays data-visualization colored.
pub const ORCHESTRATOR_COLOR: ColorU = rgb(0x7d, 0xd3, 0xd8);

/// Chart color for the row at `index` in a breakdown list, cycling once the
/// palette is exhausted. Callers must pass a stable index, which every
/// breakdown list here has since its rows are deterministically sorted before
/// rendering.
pub fn chart_color(index: usize) -> ColorU {
    CHART_PALETTE[index % CHART_PALETTE.len()]
}
