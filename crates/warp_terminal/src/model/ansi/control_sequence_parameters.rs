// The code in this file is adapted from the vte crate (an Alacritty project)
// under the Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! This module exports abstractions for parameters of control sequence actions;
//! e.g. actions to be executed after receiving a control sequence from the pty.
//!
//! Examples of such actions include repositioning the cursor, changing text
//! styles, and setting terminal modes.
use anyhow::bail;
use get_size::GetSize;
use log::trace;
use pathfinder_color::ColorU;
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, iter, str};
use thiserror::Error;
use vte::ParamsIter;

/// Terminal cursor configuration.
#[derive(Default, Debug, Eq, PartialEq, Copy, Clone, Hash)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

/// Terminal cursor shape.
#[derive(Debug, Default, Eq, PartialEq, Copy, Clone, Hash)]
pub enum CursorShape {
    /// Cursor is a block like `▒`.
    #[default]
    Block,

    /// Cursor is an underscore like `_`.
    Underline,

    /// Cursor is a vertical bar `⎸`.
    Beam,

    /// Cursor is a box like `☐`.
    HollowBlock,

    /// Invisible cursor.
    Hidden,
}

/// Terminal modes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Mode {
    /// ?1
    CursorKeys,
    /// Select 80 or 132 columns per page.
    ///
    /// CSI ? 3 h -> set 132 column font.
    /// CSI ? 3 l -> reset 80 column font.
    ///
    /// Additionally,
    ///
    /// * set margins to default positions
    /// * erases all data in page memory
    /// * resets DECLRMM to unavailable
    /// * clears data from the status line (if set to host-writable)
    #[allow(clippy::upper_case_acronyms)]
    DECCOLM,
    /// IRM Insert Mode.
    ///
    /// NB should be part of non-private mode enum.
    ///
    /// * `CSI 4 h` change to insert mode
    /// * `CSI 4 l` reset to replacement mode
    Insert,
    /// ?6
    Origin,
    /// ?7
    LineWrap,
    /// ?12
    BlinkingCursor,
    /// 20
    ///
    /// NB This is actually a private mode. We should consider adding a second
    /// enumeration for public/private modesets.
    LineFeedNewLine,
    /// ?25
    ShowCursor,
    /// ?1000
    ReportMouseClicks,
    /// ?1002
    ReportCellMouseMotion,
    /// ?1003
    ReportAllMouseMotion,
    /// ?1004
    ReportFocusInOut,
    /// ?1005
    Utf8Mouse,
    /// ?1006
    SgrMouse,
    /// ?1007
    AlternateScroll,
    /// ?1042
    UrgencyHints,
    /// ?1049, 47
    SwapScreen { save_cursor_and_clear_screen: bool },
    /// ?2004
    BracketedPaste,
    /// ?2026
    /// See https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036.
    SyncOutput,
}

impl Mode {
    /// Create mode from a primitive.
    ///
    /// TODO lots of unhandled values.
    pub fn from_primitive(intermediate: Option<&u8>, num: u16) -> Option<Mode> {
        // 0 is not a valid DEC mode.
        if num == 0 {
            return None;
        };

        let private = match intermediate {
            Some(b'?') => true,
            None => false,
            _ => return None,
        };

        if private {
            Some(match num {
                1 => Mode::CursorKeys,
                3 => Mode::DECCOLM,
                6 => Mode::Origin,
                7 => Mode::LineWrap,
                12 => Mode::BlinkingCursor,
                25 => Mode::ShowCursor,
                1000 => Mode::ReportMouseClicks,
                1002 => Mode::ReportCellMouseMotion,
                1003 => Mode::ReportAllMouseMotion,
                1004 => Mode::ReportFocusInOut,
                1005 => Mode::Utf8Mouse,
                1006 => Mode::SgrMouse,
                1007 => Mode::AlternateScroll,
                1042 => Mode::UrgencyHints,
                47 => Mode::SwapScreen {
                    save_cursor_and_clear_screen: false,
                },
                1049 => Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                },
                2004 => Mode::BracketedPaste,
                2026 => Mode::SyncOutput,
                _ => {
                    trace!("[unimplemented] primitive mode: {num}");
                    return None;
                }
            })
        } else {
            Some(match num {
                4 => Mode::Insert,
                20 => Mode::LineFeedNewLine,
                _ => return None,
            })
        }
    }
}

/// Mode for clearing line.
///
/// Relative to cursor.
#[derive(Debug, Copy, Clone)]
pub enum LineClearMode {
    /// Clear right of cursor.
    Right,
    /// Clear left of cursor.
    Left,
    /// Clear entire line.
    All,
}

/// Mode for clearing terminal.
///
/// Relative to cursor.
#[derive(Debug, Copy, Clone)]
pub enum ClearMode {
    /// Clear below cursor.
    Below,
    /// Clear above cursor.
    Above,
    /// Clear entire terminal.
    All,
    /// Clear 'saved' lines (scrollback).
    Saved,
    /// Clears all the lines in the terminal, putting the prompt on the first line.
    ResetAndClear,
    /// A synthetic mode used to clear the active block only.
    /// When it comes to interacting with the PTY, this is equivalent to a [ClearMode::ResetAndClear].
    ActiveBlock,
}

/// Mode for clearing tab stops.
#[derive(Debug, Copy, Clone)]
pub enum TabulationClearMode {
    /// Clear stop under cursor.
    Current,
    /// Clear all stops.
    All,
}

/// Standard colors.
///
/// Note: These are explicitly not given values to match the Color list, as we want this enum to
/// fit into a single byte. See the comment on `terminal::model::cell::Cell` for more details about
/// the specific memory alignment.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Foreground,
    Background,
    Cursor,
    DimBlack,
    DimRed,
    DimGreen,
    DimYellow,
    DimBlue,
    DimMagenta,
    DimCyan,
    DimWhite,
    BrightForeground,
    DimForeground,
}

impl NamedColor {
    pub fn to_bright(self) -> Self {
        match self {
            NamedColor::Foreground => NamedColor::BrightForeground,
            NamedColor::Black => NamedColor::BrightBlack,
            NamedColor::Red => NamedColor::BrightRed,
            NamedColor::Green => NamedColor::BrightGreen,
            NamedColor::Yellow => NamedColor::BrightYellow,
            NamedColor::Blue => NamedColor::BrightBlue,
            NamedColor::Magenta => NamedColor::BrightMagenta,
            NamedColor::Cyan => NamedColor::BrightCyan,
            NamedColor::White => NamedColor::BrightWhite,
            NamedColor::DimForeground => NamedColor::Foreground,
            NamedColor::DimBlack => NamedColor::Black,
            NamedColor::DimRed => NamedColor::Red,
            NamedColor::DimGreen => NamedColor::Green,
            NamedColor::DimYellow => NamedColor::Yellow,
            NamedColor::DimBlue => NamedColor::Blue,
            NamedColor::DimMagenta => NamedColor::Magenta,
            NamedColor::DimCyan => NamedColor::Cyan,
            NamedColor::DimWhite => NamedColor::White,
            val => val,
        }
    }

    pub fn to_dim(self) -> Self {
        match self {
            NamedColor::Black => NamedColor::DimBlack,
            NamedColor::Red => NamedColor::DimRed,
            NamedColor::Green => NamedColor::DimGreen,
            NamedColor::Yellow => NamedColor::DimYellow,
            NamedColor::Blue => NamedColor::DimBlue,
            NamedColor::Magenta => NamedColor::DimMagenta,
            NamedColor::Cyan => NamedColor::DimCyan,
            NamedColor::White => NamedColor::DimWhite,
            NamedColor::Foreground => NamedColor::DimForeground,
            NamedColor::BrightBlack => NamedColor::Black,
            NamedColor::BrightRed => NamedColor::Red,
            NamedColor::BrightGreen => NamedColor::Green,
            NamedColor::BrightYellow => NamedColor::Yellow,
            NamedColor::BrightBlue => NamedColor::Blue,
            NamedColor::BrightMagenta => NamedColor::Magenta,
            NamedColor::BrightCyan => NamedColor::Cyan,
            NamedColor::BrightWhite => NamedColor::White,
            NamedColor::BrightForeground => NamedColor::Foreground,
            val => val,
        }
    }

    // This can fail if the caller asks for a background color but self is
    // NamedColor::Foreground, for example
    pub fn to_ansi_bg_escape_code(&self) -> anyhow::Result<u8> {
        let code = match self {
            NamedColor::Black | NamedColor::DimBlack => 40,
            NamedColor::Red | NamedColor::DimRed => 41,
            NamedColor::Green | NamedColor::DimGreen => 42,
            NamedColor::Yellow | NamedColor::DimYellow => 43,
            NamedColor::Blue | NamedColor::DimBlue => 44,
            NamedColor::Magenta | NamedColor::DimMagenta => 45,
            NamedColor::Cyan | NamedColor::DimCyan => 46,
            NamedColor::White | NamedColor::DimWhite => 47,
            NamedColor::Background => 49,
            NamedColor::BrightBlack => 100,
            NamedColor::BrightRed => 101,
            NamedColor::BrightGreen => 102,
            NamedColor::BrightYellow => 103,
            NamedColor::BrightBlue => 104,
            NamedColor::BrightMagenta => 105,
            NamedColor::BrightCyan => 106,
            NamedColor::BrightWhite => 107,
            _ => bail!("{:?} is not a valid background", self),
        };
        Ok(code)
    }

    pub fn to_ansi_fg_escape_code(&self) -> anyhow::Result<u8> {
        let code = match self {
            NamedColor::Black | NamedColor::DimBlack => 30,
            NamedColor::Red | NamedColor::DimRed => 31,
            NamedColor::Green | NamedColor::DimGreen => 32,
            NamedColor::Yellow | NamedColor::DimYellow => 33,
            NamedColor::Blue | NamedColor::DimBlue => 34,
            NamedColor::Magenta | NamedColor::DimMagenta => 35,
            NamedColor::Cyan | NamedColor::DimCyan => 36,
            NamedColor::White | NamedColor::DimWhite => 37,
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => 39,
            NamedColor::BrightBlack => 90,
            NamedColor::BrightRed => 91,
            NamedColor::BrightGreen => 92,
            NamedColor::BrightYellow => 93,
            NamedColor::BrightBlue => 94,
            NamedColor::BrightMagenta => 95,
            NamedColor::BrightCyan => 96,
            NamedColor::BrightWhite => 97,
            _ => bail!("{:?} is not a valid foreground", self),
        };
        Ok(code)
    }

    pub fn into_color_index(self) -> usize {
        use NamedColor::*;
        match self {
            Black => color_index::BLACK,
            Red => color_index::RED,
            Green => color_index::GREEN,
            Yellow => color_index::YELLOW,
            Blue => color_index::BLUE,
            Magenta => color_index::MAGENTA,
            Cyan => color_index::CYAN,
            White => color_index::WHITE,
            BrightBlack => color_index::BRIGHT_BLACK,
            BrightRed => color_index::BRIGHT_RED,
            BrightGreen => color_index::BRIGHT_GREEN,
            BrightYellow => color_index::BRIGHT_YELLOW,
            BrightBlue => color_index::BRIGHT_BLUE,
            BrightMagenta => color_index::BRIGHT_MAGENTA,
            BrightCyan => color_index::BRIGHT_CYAN,
            BrightWhite => color_index::BRIGHT_WHITE,
            Foreground => color_index::FOREGROUND,
            Background => color_index::BACKGROUND,
            Cursor => color_index::CURSOR,
            DimBlack => color_index::DIM_BLACK,
            DimRed => color_index::DIM_RED,
            DimGreen => color_index::DIM_GREEN,
            DimYellow => color_index::DIM_YELLOW,
            DimBlue => color_index::DIM_BLUE,
            DimMagenta => color_index::DIM_MAGENTA,
            DimCyan => color_index::DIM_CYAN,
            DimWhite => color_index::DIM_WHITE,
            BrightForeground => color_index::BRIGHT_FOREGROUND,
            DimForeground => color_index::DIM_FOREGROUND,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "ColorU")]
//TODO write a deserializer (so #ff00aa could be used)
pub struct ColorUDef {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    #[serde(skip, default = "default_alpha")]
    pub a: u8,
}

fn default_alpha() -> u8 {
    255
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    Named(NamedColor),
    #[serde(with = "ColorUDef")]
    Spec(ColorU),
    Indexed(u8),
}

impl GetSize for Color {}

/// Terminal character attributes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Attr {
    /// Clear all special abilities.
    Reset,
    /// Bold text.
    Bold,
    /// Dim or secondary color.
    Dim,
    /// Italic text.
    Italic,
    /// Underline text.
    Underline,
    /// Underlined twice.
    DoubleUnderline,
    /// Blink cursor slowly.
    BlinkSlow,
    /// Blink cursor fast.
    BlinkFast,
    /// Invert colors.
    Reverse,
    /// Do not display characters.
    Hidden,
    /// Strikeout text.
    Strike,
    /// Cancel bold.
    CancelBold,
    /// Cancel bold and dim.
    CancelBoldDim,
    /// Cancel italic.
    CancelItalic,
    /// Cancel all underlines.
    CancelUnderline,
    /// Cancel blink.
    CancelBlink,
    /// Cancel inversion.
    CancelReverse,
    /// Cancel text hiding.
    CancelHidden,
    /// Cancel strikeout.
    CancelStrike,
    /// Set indexed foreground color.
    Foreground(Color),
    /// Set indexed background color.
    Background(Color),
}

/// Identifiers which can be assigned to a graphic character set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CharsetIndex {
    /// Default set, is designated as ASCII at startup.
    #[default]
    G0,
    G1,
    G2,
    G3,
}

/// Standard or common character sets which can be designated as G0-G3.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StandardCharset {
    #[default]
    Ascii,
    SpecialCharacterAndusizeDrawing,
}

impl StandardCharset {
    /// Switch/Map character to the active charset. Ascii is the common case and
    /// for that we want to do as little as possible.
    #[inline]
    #[allow(dead_code)]
    pub fn map(self, c: char) -> char {
        match self {
            StandardCharset::Ascii => c,
            StandardCharset::SpecialCharacterAndusizeDrawing => match c {
                '`' => '◆',
                'a' => '▒',
                'b' => '\t',
                'c' => '\u{000c}',
                'd' => '\r',
                'e' => '\n',
                'f' => '°',
                'g' => '±',
                'h' => '\u{2424}',
                'i' => '\u{000b}',
                'j' => '┘',
                'k' => '┐',
                'l' => '┌',
                'm' => '└',
                'n' => '┼',
                'o' => '⎺',
                'p' => '⎻',
                'q' => '─',
                'r' => '⎼',
                's' => '⎽',
                't' => '├',
                'u' => '┤',
                'v' => '┴',
                'w' => '┬',
                'x' => '│',
                'y' => '≤',
                'z' => '≥',
                '{' => 'π',
                '|' => '≠',
                '}' => '£',
                '~' => '·',
                _ => c,
            },
        }
    }
}

pub fn attrs_from_sgr_parameters(params: &mut ParamsIter<'_>) -> Vec<Option<Attr>> {
    let mut attrs = Vec::with_capacity(params.size_hint().0);

    while let Some(param) = params.next() {
        let attr = match param {
            [0] => Some(Attr::Reset),
            [1] => Some(Attr::Bold),
            [2] => Some(Attr::Dim),
            [3] => Some(Attr::Italic),
            [4, 0] => Some(Attr::CancelUnderline),
            [4, 2] => Some(Attr::DoubleUnderline),
            [4, ..] => Some(Attr::Underline),
            [5] => Some(Attr::BlinkSlow),
            [6] => Some(Attr::BlinkFast),
            [7] => Some(Attr::Reverse),
            [8] => Some(Attr::Hidden),
            [9] => Some(Attr::Strike),
            [21] => Some(Attr::CancelBold),
            [22] => Some(Attr::CancelBoldDim),
            [23] => Some(Attr::CancelItalic),
            [24] => Some(Attr::CancelUnderline),
            [25] => Some(Attr::CancelBlink),
            [27] => Some(Attr::CancelReverse),
            [28] => Some(Attr::CancelHidden),
            [29] => Some(Attr::CancelStrike),
            [30] => Some(Attr::Foreground(Color::Named(NamedColor::Black))),
            [31] => Some(Attr::Foreground(Color::Named(NamedColor::Red))),
            [32] => Some(Attr::Foreground(Color::Named(NamedColor::Green))),
            [33] => Some(Attr::Foreground(Color::Named(NamedColor::Yellow))),
            [34] => Some(Attr::Foreground(Color::Named(NamedColor::Blue))),
            [35] => Some(Attr::Foreground(Color::Named(NamedColor::Magenta))),
            [36] => Some(Attr::Foreground(Color::Named(NamedColor::Cyan))),
            [37] => Some(Attr::Foreground(Color::Named(NamedColor::White))),
            [38] => {
                let mut iter = params.map(|param| param[0]);
                parse_sgr_color(&mut iter).map(Attr::Foreground)
            }
            [38, params @ ..] => {
                let rgb_start = if params.len() > 4 { 2 } else { 1 };
                let rgb_iter = params[rgb_start..].iter().copied();
                let mut iter = iter::once(params[0]).chain(rgb_iter);

                parse_sgr_color(&mut iter).map(Attr::Foreground)
            }
            [39] => Some(Attr::Foreground(Color::Named(NamedColor::Foreground))),
            [40] => Some(Attr::Background(Color::Named(NamedColor::Black))),
            [41] => Some(Attr::Background(Color::Named(NamedColor::Red))),
            [42] => Some(Attr::Background(Color::Named(NamedColor::Green))),
            [43] => Some(Attr::Background(Color::Named(NamedColor::Yellow))),
            [44] => Some(Attr::Background(Color::Named(NamedColor::Blue))),
            [45] => Some(Attr::Background(Color::Named(NamedColor::Magenta))),
            [46] => Some(Attr::Background(Color::Named(NamedColor::Cyan))),
            [47] => Some(Attr::Background(Color::Named(NamedColor::White))),
            [48] => {
                let mut iter = params.map(|param| param[0]);
                parse_sgr_color(&mut iter).map(Attr::Background)
            }
            [48, params @ ..] => {
                let rgb_start = if params.len() > 4 { 2 } else { 1 };
                let rgb_iter = params[rgb_start..].iter().copied();
                let mut iter = iter::once(params[0]).chain(rgb_iter);

                parse_sgr_color(&mut iter).map(Attr::Background)
            }
            [49] => Some(Attr::Background(Color::Named(NamedColor::Background))),
            [90] => Some(Attr::Foreground(Color::Named(NamedColor::BrightBlack))),
            [91] => Some(Attr::Foreground(Color::Named(NamedColor::BrightRed))),
            [92] => Some(Attr::Foreground(Color::Named(NamedColor::BrightGreen))),
            [93] => Some(Attr::Foreground(Color::Named(NamedColor::BrightYellow))),
            [94] => Some(Attr::Foreground(Color::Named(NamedColor::BrightBlue))),
            [95] => Some(Attr::Foreground(Color::Named(NamedColor::BrightMagenta))),
            [96] => Some(Attr::Foreground(Color::Named(NamedColor::BrightCyan))),
            [97] => Some(Attr::Foreground(Color::Named(NamedColor::BrightWhite))),
            [100] => Some(Attr::Background(Color::Named(NamedColor::BrightBlack))),
            [101] => Some(Attr::Background(Color::Named(NamedColor::BrightRed))),
            [102] => Some(Attr::Background(Color::Named(NamedColor::BrightGreen))),
            [103] => Some(Attr::Background(Color::Named(NamedColor::BrightYellow))),
            [104] => Some(Attr::Background(Color::Named(NamedColor::BrightBlue))),
            [105] => Some(Attr::Background(Color::Named(NamedColor::BrightMagenta))),
            [106] => Some(Attr::Background(Color::Named(NamedColor::BrightCyan))),
            [107] => Some(Attr::Background(Color::Named(NamedColor::BrightWhite))),
            _ => None,
        };
        attrs.push(attr);
    }

    attrs
}

/// Parse a color specifier from list of attributes.
fn parse_sgr_color(params: &mut dyn Iterator<Item = u16>) -> Option<Color> {
    match params.next() {
        Some(2) => Some(Color::Spec(ColorU::new(
            u8::try_from(params.next()?).ok()?,
            u8::try_from(params.next()?).ok()?,
            u8::try_from(params.next()?).ok()?,
            0xff,
        ))),
        Some(5) => Some(Color::Indexed(u8::try_from(params.next()?).ok()?)),
        _ => None,
    }
}

/// Defines the varieties of prompt marker sequences we can process.
#[derive(Copy, Clone, Debug)]
pub enum PromptMarker {
    /// A marker indicating that the shell is starting to write out
    /// a prompt of the given kind.
    StartPrompt { kind: PromptKind },
    /// A marker indicating that the shell has finished writing out
    /// the in-progress prompt.
    EndPrompt,
}

#[derive(Debug, Error)]
pub enum PromptMarkerParseError {
    #[error("unknown parameter encountered")]
    UnknownParam,
    #[error("malformed option encountered")]
    MalformedOption,
}

impl TryFrom<&[&[u8]]> for PromptMarker {
    type Error = PromptMarkerParseError;

    /// Try to parse prompt marker information from an OSC 133
    /// sequence.
    ///
    /// See the "semantic prompts" spec from terminal-wg for more
    /// details on the grammar and parameters:
    /// https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md
    fn try_from(params: &[&[u8]]) -> Result<Self, Self::Error> {
        match params.first() {
            Some(&b"A") => Ok(PromptMarker::StartPrompt {
                kind: PromptKind::Initial,
            }),
            Some(&b"B") => Ok(PromptMarker::EndPrompt),
            Some(&b"P") => {
                // Default to "Initial" as the kind, if one is not specified as an option.
                let mut kind = PromptKind::Initial;
                // Loop through and parse out any options, which are expected to be of the form
                // "key=value".  We ignore unknown options, but return an error for any malformed
                // ones.
                for param in &params[1..] {
                    let Some(eq_index) = param.iter().position(|byte| byte == &b"="[0]) else {
                        return Err(Self::Error::MalformedOption);
                    };
                    if eq_index + 1 >= param.len() {
                        return Err(Self::Error::MalformedOption);
                    }
                    let key = &param[..eq_index];
                    let value = &param[eq_index + 1..];
                    // "k" represents the prompt kind; try to parse the value into our
                    // PromptKind enum.
                    if let b"k" = key {
                        if let Ok(k) = PromptKind::try_from(value) {
                            kind = k;
                        }
                    }
                }
                Ok(PromptMarker::StartPrompt { kind })
            }
            _ => Err(Self::Error::UnknownParam),
        }
    }
}

/// An enumeration of the kinds of prompts we support.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// The initial (left) prompt.
    Initial,
    /// The right-side prompt.
    Right,
}

#[derive(Debug, Error)]
pub enum PromptKindParseError {
    #[error("unknown value")]
    UnknownValue,
}

impl TryFrom<&[u8]> for PromptKind {
    type Error = PromptKindParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value {
            b"i" => Ok(PromptKind::Initial),
            b"r" => Ok(PromptKind::Right),
            _ => Err(Self::Error::UnknownValue),
        }
    }
}

/// Maximum byte length of a URI carried by an OSC 8 hyperlink. Sequences
/// whose URI exceeds this cap are dropped at parse time before the URI
/// `String` is allocated, so a hostile sender can't trigger an arbitrarily
/// large heap allocation.
pub const MAX_URI_BYTES: usize = 4096;

/// A hyperlink declared via the OSC 8 escape sequence.
///
/// Format: `OSC 8 ; params ; URI ST`, where `params` is a colon-separated
/// list of `key=value` pairs and the URI is the destination. An empty URI
/// closes the active hyperlink. Currently only the `id` parameter is defined
/// by the spec; it lets terminals visually group runs of cells that belong
/// to the same logical link (we parse it but treat it as a hint only).
///
/// Reference: <https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda>
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hyperlink {
    /// Optional identifier declared by the emitter. Parsed and stored, but
    /// not used for cross-run grouping in the current implementation.
    pub id: Option<String>,
    /// The URI the link points to.
    pub uri: String,
}

#[derive(Debug, Error)]
pub enum HyperlinkParseError {
    #[error("hyperlink URI is not valid UTF-8")]
    InvalidUtf8,
    #[error("hyperlink URI exceeds the {MAX_URI_BYTES} byte cap (got {len})")]
    UriTooLong { len: usize },
    #[error("hyperlink param is malformed (no '=' found)")]
    MalformedParam,
}

impl Hyperlink {
    /// Parse the params from an OSC 8 sequence into a `Hyperlink`.
    ///
    /// `params` is the slice of fields *after* the leading `b"8"` identifier
    /// — i.e. for `OSC 8 ; id=foo ; https://example.com ST`, callers should
    /// pass `&[b"id=foo", b"https://example.com"]`.
    ///
    /// The vte parser splits OSC bytes on `;`, so a URI containing `;` arrives
    /// split across multiple slice elements. The contract is therefore: the
    /// URI is **always** the `b";"`-rejoin of `params[1..]`. Implementations
    /// that follow the simpler "two fields after the identifier" mental shortcut
    /// silently drop URIs containing `;` (matrix params, `jsessionid`, percent-
    /// encoded payloads); see the `uri_with_semicolons_is_rejoined` test below.
    ///
    /// Returns `Ok(None)` for the closing form (empty URI) and `Ok(Some(_))`
    /// for an opening form. Unknown `key=value` params are ignored. The
    /// `MAX_URI_BYTES` cap is checked on the raw bytes before allocating
    /// the URI `String`.
    pub fn parse_osc_params(params: &[&[u8]]) -> Result<Option<Self>, HyperlinkParseError> {
        // Close-form shapes: empty slice, single empty field, or two empty fields.
        if params.is_empty() {
            return Ok(None);
        }
        let params_field = params[0];
        let uri_parts = &params[1..];

        // Compute the URI's total byte length without allocating, so the
        // length cap rejects oversized inputs before we grow a String.
        let uri_len = if uri_parts.is_empty() {
            0
        } else {
            uri_parts.iter().map(|part| part.len()).sum::<usize>()
                + uri_parts.len().saturating_sub(1) // semicolons rejoined between parts
        };

        if uri_len == 0 {
            return Ok(None);
        }
        if uri_len > MAX_URI_BYTES {
            return Err(HyperlinkParseError::UriTooLong { len: uri_len });
        }

        // Rejoin URI parts with `;` (the vte parser split them).
        let mut uri_bytes = Vec::with_capacity(uri_len);
        for (i, part) in uri_parts.iter().enumerate() {
            if i > 0 {
                uri_bytes.push(b';');
            }
            uri_bytes.extend_from_slice(part);
        }
        let uri = String::from_utf8(uri_bytes).map_err(|_| HyperlinkParseError::InvalidUtf8)?;

        let mut id: Option<String> = None;
        if !params_field.is_empty() {
            for pair in params_field.split(|byte| *byte == b':') {
                if pair.is_empty() {
                    continue;
                }
                let Some(eq_idx) = pair.iter().position(|byte| *byte == b'=') else {
                    return Err(HyperlinkParseError::MalformedParam);
                };
                let key = &pair[..eq_idx];
                let value = &pair[eq_idx + 1..];
                if key == b"id" {
                    let value =
                        str::from_utf8(value).map_err(|_| HyperlinkParseError::InvalidUtf8)?;
                    id = Some(value.to_owned());
                }
            }
        }

        Ok(Some(Hyperlink { id, uri }))
    }
}

#[cfg(test)]
mod hyperlink_parse_tests {
    use super::*;

    #[test]
    fn open_with_no_params() {
        let parsed = Hyperlink::parse_osc_params(&[b"", b"https://example.com"]).unwrap();
        assert_eq!(
            parsed,
            Some(Hyperlink {
                id: None,
                uri: "https://example.com".to_owned(),
            })
        );
    }

    #[test]
    fn open_with_id_param() {
        let parsed = Hyperlink::parse_osc_params(&[b"id=foo", b"https://example.com"]).unwrap();
        assert_eq!(parsed.unwrap().id.as_deref(), Some("foo"));
    }

    #[test]
    fn close_canonical() {
        // Canonical close: `OSC 8 ; ; ST` -> two empty fields after identifier.
        let parsed = Hyperlink::parse_osc_params(&[b"", b""]).unwrap();
        assert_eq!(parsed, None);
    }

    #[test]
    fn close_single_empty_field() {
        // Some emitters write `OSC 8 ; ST` -> one empty field.
        let parsed = Hyperlink::parse_osc_params(&[b""]).unwrap();
        assert_eq!(parsed, None);
    }

    #[test]
    fn close_zero_fields() {
        // Defensive: empty params slice (caller didn't strip a leading b"8").
        let parsed = Hyperlink::parse_osc_params(&[]).unwrap();
        assert_eq!(parsed, None);
    }

    #[test]
    fn unknown_keys_in_params_are_ignored() {
        let parsed =
            Hyperlink::parse_osc_params(&[b"unknown=val:id=bar", b"https://example.com"]).unwrap();
        let hyperlink = parsed.unwrap();
        assert_eq!(hyperlink.id.as_deref(), Some("bar"));
        assert_eq!(hyperlink.uri, "https://example.com");
    }

    #[test]
    fn multiple_params_separated_by_colons() {
        let parsed =
            Hyperlink::parse_osc_params(&[b"id=foo:hover=true:other=ignored", b"https://x"])
                .unwrap();
        assert_eq!(parsed.unwrap().id.as_deref(), Some("foo"));
    }

    #[test]
    fn malformed_param_without_equals_is_rejected() {
        let result = Hyperlink::parse_osc_params(&[b"badparam", b"https://example.com"]);
        assert!(matches!(result, Err(HyperlinkParseError::MalformedParam)));
    }

    /// Anti-regression: vte splits OSC params on `;`, so a URI with literal
    /// `;` arrives across multiple slice elements. The parser must rejoin them.
    /// Failing this test is the cardinal indicator of a "two-field shortcut"
    /// implementation that silently drops valid URIs.
    #[test]
    fn uri_with_semicolons_is_rejoined() {
        let parsed = Hyperlink::parse_osc_params(&[
            b"",
            b"https://example.com/a?x=1",
            b"y=2",
            b"jsessionid=abc",
        ])
        .unwrap();
        let hyperlink = parsed.unwrap();
        assert_eq!(
            hyperlink.uri,
            "https://example.com/a?x=1;y=2;jsessionid=abc"
        );
    }

    #[test]
    fn non_utf8_uri_is_rejected() {
        // Invalid UTF-8 byte sequence in the URI part.
        let result = Hyperlink::parse_osc_params(&[b"", &[0xff, 0xfe, 0xfd]]);
        assert!(matches!(result, Err(HyperlinkParseError::InvalidUtf8)));
    }

    #[test]
    fn over_length_uri_is_rejected_without_allocation() {
        // Allocate the input as a borrowed slice — the function must reject
        // before allocating its own String of comparable size.
        let big = vec![b'x'; MAX_URI_BYTES + 1];
        let result = Hyperlink::parse_osc_params(&[b"", big.as_slice()]);
        match result {
            Err(HyperlinkParseError::UriTooLong { len }) => {
                assert_eq!(len, MAX_URI_BYTES + 1);
            }
            other => panic!("expected UriTooLong, got {other:?}"),
        }
    }

    #[test]
    fn empty_segments_in_params_are_skipped() {
        // Defensive: a stray colon in the params field shouldn't flag the entry as malformed.
        let parsed =
            Hyperlink::parse_osc_params(&[b"id=foo::id=bar", b"https://example.com"]).unwrap();
        // The last `id=` wins.
        assert_eq!(parsed.unwrap().id.as_deref(), Some("bar"));
    }
}

pub mod color_index {
    pub const BLACK: usize = 0;
    pub const RED: usize = 1;
    pub const GREEN: usize = 2;
    pub const YELLOW: usize = 3;
    pub const BLUE: usize = 4;
    pub const MAGENTA: usize = 5;
    pub const CYAN: usize = 6;
    pub const WHITE: usize = 7;
    pub const BRIGHT_BLACK: usize = 8;
    pub const BRIGHT_RED: usize = 9;
    pub const BRIGHT_GREEN: usize = 10;
    pub const BRIGHT_YELLOW: usize = 11;
    pub const BRIGHT_BLUE: usize = 12;
    pub const BRIGHT_MAGENTA: usize = 13;
    pub const BRIGHT_CYAN: usize = 14;
    pub const BRIGHT_WHITE: usize = 15;
    pub const FOREGROUND: usize = 256;
    pub const BACKGROUND: usize = 257;
    pub const CURSOR: usize = 258;
    pub const DIM_BLACK: usize = 259;
    pub const DIM_RED: usize = 260;
    pub const DIM_GREEN: usize = 261;
    pub const DIM_YELLOW: usize = 262;
    pub const DIM_BLUE: usize = 263;
    pub const DIM_MAGENTA: usize = 264;
    pub const DIM_CYAN: usize = 265;
    pub const DIM_WHITE: usize = 266;
    pub const BRIGHT_FOREGROUND: usize = 267;
    pub const DIM_FOREGROUND: usize = 268;
}
