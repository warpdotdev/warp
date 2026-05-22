use std::fmt;
use std::ops::{Index, IndexMut};

use warpui::color::ColorU;

use crate::terminal::model::ansi::color_index;
use crate::themes::theme::{AnsiColors, WarpTheme};

pub const COUNT: usize = 269;

/// Factor for automatic computation of dim colors used by terminal.
const DIM_FACTOR: f32 = 0.66;

// TODO(alokedesai): we should move this into the terminal theme.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Colors {
    pub primary: PrimaryColors,
    pub normal: NormalColors,
    pub bright: BrightColors,
    pub dim: Option<DimColors>,
    pub indexed_colors: Vec<IndexedColor>,
}

impl Colors {
    pub fn with_foreground_background_color(foreground: ColorU, background: ColorU) -> Self {
        Colors {
            primary: PrimaryColors {
                foreground,
                background,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn new(primary: PrimaryColors, normal: NormalColors, bright: BrightColors) -> Self {
        Colors {
            primary,
            normal,
            bright,
            ..Default::default()
        }
    }
}

impl From<WarpTheme> for Colors {
    fn from(theme: WarpTheme) -> Self {
        let colors = theme.terminal_colors();
        Colors::new(
            PrimaryColors::new(
                theme.foreground().into_solid(),
                theme.background().into_solid(),
            ),
            colors.normal.into(),
            colors.bright.into(),
        )
    }
}

/// List of indexed colors.
///
/// The first 16 entries are the standard ansi named colors. Items 16..232 are
/// the color cube.  Items 233..256 are the grayscale ramp. Item 256 is
/// the configured foreground color, item 257 is the configured background
/// color, item 258 is the cursor color. Following that are 8 positions for dim colors.
/// Item 267 is the bright foreground color, 268 the dim foreground.
#[derive(Copy, Clone)]
pub struct List([ColorU; COUNT]);

/// Any overrided colors that may be set via escape sequence.
#[derive(Copy, Clone)]
pub struct OverrideList([Option<ColorU>; COUNT]);

impl OverrideList {
    pub fn empty() -> Self {
        OverrideList([None; COUNT])
    }
}

impl From<&Colors> for List {
    fn from(colors: &Colors) -> List {
        // Type inference fails without this annotation.
        let mut list = List([ColorU::black(); COUNT]);

        list.fill_named(colors);
        list.fill_cube(colors);
        list.fill_gray_ramp(colors);

        list
    }
}

impl List {
    pub fn fill_named(&mut self, colors: &Colors) {
        // Normals.
        self[color_index::BLACK] = colors.normal.black;
        self[color_index::RED] = colors.normal.red;
        self[color_index::GREEN] = colors.normal.green;
        self[color_index::YELLOW] = colors.normal.yellow;
        self[color_index::BLUE] = colors.normal.blue;
        self[color_index::MAGENTA] = colors.normal.magenta;
        self[color_index::CYAN] = colors.normal.cyan;
        self[color_index::WHITE] = colors.normal.white;

        // Brights.
        self[color_index::BRIGHT_BLACK] = colors.bright.black;
        self[color_index::BRIGHT_RED] = colors.bright.red;
        self[color_index::BRIGHT_GREEN] = colors.bright.green;
        self[color_index::BRIGHT_YELLOW] = colors.bright.yellow;
        self[color_index::BRIGHT_BLUE] = colors.bright.blue;
        self[color_index::BRIGHT_MAGENTA] = colors.bright.magenta;
        self[color_index::BRIGHT_CYAN] = colors.bright.cyan;
        self[color_index::BRIGHT_WHITE] = colors.bright.white;
        self[color_index::BRIGHT_FOREGROUND] = colors
            .primary
            .bright_foreground
            .unwrap_or(colors.primary.foreground);

        // Foreground and background.
        self[color_index::FOREGROUND] = colors.primary.foreground;
        self[color_index::BACKGROUND] = colors.primary.background;

        // Dims.
        self[color_index::DIM_FOREGROUND] = colors
            .primary
            .dim_foreground
            .unwrap_or_else(|| dim(colors.primary.foreground));
        match colors.dim {
            Some(ref dim) => {
                self[color_index::DIM_BLACK] = dim.black;
                self[color_index::DIM_RED] = dim.red;
                self[color_index::DIM_GREEN] = dim.green;
                self[color_index::DIM_YELLOW] = dim.yellow;
                self[color_index::DIM_BLUE] = dim.blue;
                self[color_index::DIM_MAGENTA] = dim.magenta;
                self[color_index::DIM_CYAN] = dim.cyan;
                self[color_index::DIM_WHITE] = dim.white;
            }
            None => {
                self[color_index::DIM_BLACK] = dim(colors.normal.black);
                self[color_index::DIM_RED] = dim(colors.normal.red);
                self[color_index::DIM_GREEN] = dim(colors.normal.green);
                self[color_index::DIM_YELLOW] = dim(colors.normal.yellow);
                self[color_index::DIM_BLUE] = dim(colors.normal.blue);
                self[color_index::DIM_MAGENTA] = dim(colors.normal.magenta);
                self[color_index::DIM_CYAN] = dim(colors.normal.cyan);
                self[color_index::DIM_WHITE] = dim(colors.normal.white);
            }
        }
    }

    pub fn fill_cube(&mut self, colors: &Colors) {
        let mut index: usize = 16;
        // Build colors.
        for r in 0..6 {
            for g in 0..6 {
                for b in 0..6 {
                    // Override colors 16..232 with the config (if present).
                    if let Some(indexed_color) = colors
                        .indexed_colors
                        .iter()
                        .find(|ic| ic.index() == index as u8)
                    {
                        self[index] = indexed_color.color;
                    } else {
                        self[index] = ColorU::new(
                            if r == 0 { 0 } else { r * 40 + 55 },
                            if g == 0 { 0 } else { g * 40 + 55 },
                            if b == 0 { 0 } else { b * 40 + 55 },
                            255,
                        );
                    }
                    index += 1;
                }
            }
        }

        debug_assert!(index == 232);
    }

    pub fn fill_gray_ramp(&mut self, colors: &Colors) {
        let mut index: usize = 232;

        for i in 0..24 {
            // Index of the color is number of named colors + number of cube colors + i.
            let color_index = 16 + 216 + i;

            // Override colors 232..256 with the config (if present).
            if let Some(indexed_color) = colors
                .indexed_colors
                .iter()
                .find(|ic| ic.index() == color_index)
            {
                self[index] = indexed_color.color;
                index += 1;
                continue;
            }

            let value = i * 10 + 8;
            self[index] = ColorU::new(value, value, value, 255);
            index += 1;
        }

        debug_assert!(index == 256);
    }
}

impl fmt::Debug for List {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("List[..]")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrimaryColors {
    pub foreground: ColorU,
    pub background: ColorU,
    pub bright_foreground: Option<ColorU>,
    pub dim_foreground: Option<ColorU>,
}

impl PrimaryColors {
    pub fn new(foreground: ColorU, background: ColorU) -> Self {
        PrimaryColors {
            foreground,
            background,
            ..Default::default()
        }
    }
}

impl Default for PrimaryColors {
    fn default() -> Self {
        PrimaryColors {
            background: ColorU {
                r: 0x1d,
                g: 0x1f,
                b: 0x21,
                a: 0xff,
            },
            foreground: ColorU {
                r: 0xc5,
                g: 0xc8,
                b: 0xc6,
                a: 0xff,
            },
            bright_foreground: Default::default(),
            dim_foreground: Default::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NormalColors {
    pub black: ColorU,
    pub red: ColorU,
    pub green: ColorU,
    pub yellow: ColorU,
    pub blue: ColorU,
    pub magenta: ColorU,
    pub cyan: ColorU,
    pub white: ColorU,
}

impl NormalColors {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        black: ColorU,
        red: ColorU,
        green: ColorU,
        yellow: ColorU,
        blue: ColorU,
        magenta: ColorU,
        cyan: ColorU,
        white: ColorU,
    ) -> Self {
        NormalColors {
            black,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
            white,
        }
    }
}
impl From<AnsiColors> for NormalColors {
    fn from(colors: AnsiColors) -> Self {
        Self::new(
            colors.black.into(),
            colors.red.into(),
            colors.green.into(),
            colors.yellow.into(),
            colors.blue.into(),
            colors.magenta.into(),
            colors.cyan.into(),
            colors.white.into(),
        )
    }
}

impl Default for NormalColors {
    fn default() -> Self {
        NormalColors {
            black: ColorU {
                r: 0x1d,
                g: 0x1f,
                b: 0x21,
                a: 0xff,
            },
            red: ColorU {
                r: 0xcc,
                g: 0x66,
                b: 0x66,
                a: 0xff,
            },
            green: ColorU {
                r: 0xb5,
                g: 0xbd,
                b: 0x68,
                a: 0xff,
            },
            yellow: ColorU {
                r: 0xf0,
                g: 0xc6,
                b: 0x74,
                a: 0xff,
            },
            blue: ColorU {
                r: 0x81,
                g: 0xa2,
                b: 0xbe,
                a: 0xff,
            },
            magenta: ColorU {
                r: 0xb2,
                g: 0x94,
                b: 0xbb,
                a: 0xff,
            },
            cyan: ColorU {
                r: 0x8a,
                g: 0xbe,
                b: 0xb7,
                a: 0xff,
            },
            white: ColorU {
                r: 0xc5,
                g: 0xc8,
                b: 0xc6,
                a: 0xff,
            },
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BrightColors {
    pub black: ColorU,
    pub red: ColorU,
    pub green: ColorU,
    pub yellow: ColorU,
    pub blue: ColorU,
    pub magenta: ColorU,
    pub cyan: ColorU,
    pub white: ColorU,
}

impl BrightColors {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        black: ColorU,
        red: ColorU,
        green: ColorU,
        yellow: ColorU,
        blue: ColorU,
        magenta: ColorU,
        cyan: ColorU,
        white: ColorU,
    ) -> Self {
        BrightColors {
            black,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
            white,
        }
    }
}
impl From<AnsiColors> for BrightColors {
    fn from(colors: AnsiColors) -> Self {
        Self::new(
            colors.black.into(),
            colors.red.into(),
            colors.green.into(),
            colors.yellow.into(),
            colors.blue.into(),
            colors.magenta.into(),
            colors.cyan.into(),
            colors.white.into(),
        )
    }
}

impl Default for BrightColors {
    fn default() -> Self {
        BrightColors {
            black: ColorU {
                r: 0x66,
                g: 0x66,
                b: 0x66,
                a: 0xff,
            },
            red: ColorU {
                r: 0xd5,
                g: 0x4e,
                b: 0x53,
                a: 0xff,
            },
            green: ColorU {
                r: 0xb9,
                g: 0xca,
                b: 0x4a,
                a: 0xff,
            },
            yellow: ColorU {
                r: 0xe7,
                g: 0xc5,
                b: 0x47,
                a: 0xff,
            },
            blue: ColorU {
                r: 0x7a,
                g: 0xa6,
                b: 0xda,
                a: 0xff,
            },
            magenta: ColorU {
                r: 0xc3,
                g: 0x97,
                b: 0xd8,
                a: 0xff,
            },
            cyan: ColorU {
                r: 0x70,
                g: 0xc0,
                b: 0xb1,
                a: 0xff,
            },
            white: ColorU {
                r: 0xea,
                g: 0xea,
                b: 0xea,
                a: 0xff,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimColors {
    pub black: ColorU,
    pub red: ColorU,
    pub green: ColorU,
    pub yellow: ColorU,
    pub blue: ColorU,
    pub magenta: ColorU,
    pub cyan: ColorU,
    pub white: ColorU,
}

impl Default for DimColors {
    fn default() -> Self {
        DimColors {
            black: ColorU {
                r: 0x13,
                g: 0x14,
                b: 0x15,
                a: 0xff,
            },
            red: ColorU {
                r: 0x86,
                g: 0x43,
                b: 0x43,
                a: 0xff,
            },
            green: ColorU {
                r: 0x77,
                g: 0x7c,
                b: 0x44,
                a: 0xff,
            },
            yellow: ColorU {
                r: 0x9e,
                g: 0x82,
                b: 0x4c,
                a: 0xff,
            },
            blue: ColorU {
                r: 0x55,
                g: 0x6a,
                b: 0x7d,
                a: 0xff,
            },
            magenta: ColorU {
                r: 0x75,
                g: 0x61,
                b: 0x7b,
                a: 0xff,
            },
            cyan: ColorU {
                r: 0x5b,
                g: 0x7d,
                b: 0x78,
                a: 0xff,
            },
            white: ColorU {
                r: 0x82,
                g: 0x84,
                b: 0x82,
                a: 0xff,
            },
        }
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
struct ColorIndex(u8);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IndexedColor {
    pub color: ColorU,

    index: ColorIndex,
}

impl IndexedColor {
    #[inline]
    pub fn index(&self) -> u8 {
        self.index.0
    }
}

impl Index<usize> for List {
    type Output = ColorU;

    #[inline]
    fn index(&self, idx: usize) -> &Self::Output {
        &self.0[idx]
    }
}

impl IndexMut<usize> for List {
    #[inline]
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
        &mut self.0[idx]
    }
}

impl Index<u8> for List {
    type Output = ColorU;

    #[inline]
    fn index(&self, idx: u8) -> &Self::Output {
        &self.0[idx as usize]
    }
}

impl IndexMut<u8> for List {
    #[inline]
    fn index_mut(&mut self, idx: u8) -> &mut Self::Output {
        &mut self.0[idx as usize]
    }
}

impl Index<usize> for OverrideList {
    type Output = Option<ColorU>;

    #[inline]
    fn index(&self, idx: usize) -> &Self::Output {
        &self.0[idx]
    }
}

impl IndexMut<usize> for OverrideList {
    #[inline]
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
        &mut self.0[idx]
    }
}

impl Index<u8> for OverrideList {
    type Output = Option<ColorU>;

    #[inline]
    fn index(&self, idx: u8) -> &Self::Output {
        &self.0[idx as usize]
    }
}

impl IndexMut<u8> for OverrideList {
    #[inline]
    fn index_mut(&mut self, idx: u8) -> &mut Self::Output {
        &mut self.0[idx as usize]
    }
}

/// Returns a dimmed version of `color` based on a fixed dim-factor
/// that is suitable for terminal colors.
pub(super) fn dim(color: ColorU) -> ColorU {
    mult_coloru(color, DIM_FACTOR)
}

fn mult_coloru(color: ColorU, rhs: f32) -> ColorU {
    let r = (color.r as f32 * rhs).clamp(0., 255.);
    let g = (color.g as f32 * rhs).clamp(0., 255.);
    let b = (color.b as f32 * rhs).clamp(0., 255.);
    ColorU::new(r as u8, g as u8, b as u8, color.a)
}
