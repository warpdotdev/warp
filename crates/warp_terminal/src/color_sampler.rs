use std::collections::HashMap;

use pathfinder_color::ColorU;

/// Stores count of occurrences of distinct colors as a grid is rendered, which we can use to
/// compute the most common background color of a grid and color-match other UI elements against
/// it.
#[derive(Debug, Default)]
pub struct ColorSampler {
    counts: HashMap<ColorU, usize>,
    total_samples: usize,
}

impl ColorSampler {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
            total_samples: 0,
        }
    }

    pub fn sample(&mut self, color: ColorU) {
        self.total_samples += 1;
        // Sample every 8th color (cell).
        if !self.total_samples.is_multiple_of(8) {
            return;
        }

        let color = if color.is_fully_transparent() {
            // Sample all fully transparent colors as the same color even if they have differing
            // rgb values, cause that makes no difference in the rendered "color".
            ColorU::transparent_black()
        } else {
            color
        };

        *self.counts.entry(color).or_default() += 1;
    }

    pub fn most_common(&self) -> Option<ColorU> {
        self.counts
            .iter()
            .max_by_key(|&(_, &count)| count)
            .map(|(&color, _)| color)
    }

    pub fn reset(&mut self) {
        self.counts.clear();
        self.total_samples = 0;
    }
}
