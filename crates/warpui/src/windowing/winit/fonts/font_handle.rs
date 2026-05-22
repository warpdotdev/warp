// Neither macOS nor wasm make use of the "load font from path" functionality,
// and so there's a lot of unused code in here.  Instead of marking each of the
// relevant functions with allow(dead_code), we'll do it at the module level
// instead for simplicity.
#![cfg_attr(
    any(target_os = "macos", target_os = "windows", target_family = "wasm"),
    allow(dead_code)
)]

use std::fs::File;
use std::path::PathBuf;

use owned_ttf_parser::{AsFaceRef, Face, FaceParsingError, OwnedFace};

/// A handle that wraps around a font face.
pub struct FontHandle {
    data: FontData,
}

/// Source data for a font to be loaded within the winit font system.
pub enum FontData {
    /// The font is to be loaded via bytes. This should be used sparingly since it requires loading the font into
    /// memory.
    Bytes(OwnedFace),
    /// The font identified at the given `path` and `index` will be loaded.
    /// NOTE the font will never be loaded into memory. Instead, data from the font will be read via a memory-mapped
    /// file.
    Path {
        path: PathBuf,
        index: u32,
        is_monospace: bool,
    },
}

impl FontData {
    /// Returns an [`Error`] if the [`FontData`] does not map to a valid font.
    ///
    /// A font is considered valid iff:
    /// * The file referenced by [`FontData::Path`] exists and can be read.
    /// * The data can be parsed into a valid [`ttf_parser::Face`].
    /// * The font face contains a glyph for the 'm' character.
    fn validate(&self) -> Result<(), Error> {
        match self {
            FontData::Bytes(_) => Ok(()),
            FontData::Path { path, index, .. } => {
                let file = File::open(path).map_err(|e| Error::Load {
                    path: path.clone(),
                    io_error: e,
                })?;
                let mmap = unsafe {
                    memmap2::Mmap::map(&file).map_err(|e| Error::Load {
                        path: path.clone(),
                        io_error: e,
                    })?
                };
                let face = Face::parse(&mmap, *index).map_err(|e| Error::Parse {
                    path: path.clone(),
                    parse_error: e,
                })?;

                if face.as_face_ref().glyph_index('m').is_none() {
                    Err(Error::Validate { path: path.clone() })
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl FontHandle {
    pub fn new(path: impl Into<PathBuf>, index: u32, is_monospace: bool) -> Self {
        Self {
            data: FontData::Path {
                path: path.into(),
                index,
                is_monospace,
            },
        }
    }

    pub fn is_monospace(&self) -> bool {
        match &self.data {
            FontData::Path { is_monospace, .. } => *is_monospace,
            FontData::Bytes(face) => face.as_face_ref().is_monospaced(),
        }
    }

    /// Validates the [`FontHandle`] is a parseable font.
    pub fn validate_font_data(&self) -> Result<(), Error> {
        self.data.validate()
    }

    pub(super) fn into_data(self) -> FontData {
        self.data
    }

    #[allow(dead_code)]
    pub(super) fn data(&self) -> &FontData {
        &self.data
    }
}

impl From<OwnedFace> for FontHandle {
    fn from(value: OwnedFace) -> Self {
        Self {
            data: FontData::Bytes(value),
        }
    }
}

/// Errors associated with loading fonts
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to load font data due to an underlying std::io::Error
    #[error("Error loading font data for font {path}")]
    Load {
        path: PathBuf,
        io_error: std::io::Error,
    },

    /// Failed to parse the underlying data into a valid font
    #[error("Error parsing font data for font {path}")]
    Parse {
        path: PathBuf,
        parse_error: FaceParsingError,
    },

    /// A font was properly loaded, but did not have a codepoint
    /// for the letter m, indicating it would not work within Warp.
    #[error("Font {path} does not have a valid codepoint for the letter m")]
    Validate { path: PathBuf },
}
