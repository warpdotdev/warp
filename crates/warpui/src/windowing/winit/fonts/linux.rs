//! Loads fonts on linux.
//!
//! Handles discovering and loading fonts on linux systems.
//! Leverages the fontconfig crate to detect all fonts
//! available on the user's device, creating handles for the fonts.
//! Handles can be converted to owned_ttf_parser::OwnedFace objects
//! by loading the fonts into memory.

use std::collections::HashMap;
use std::ffi::{c_int, CString};

use fontconfig::{
    list_fonts, sort_fonts, FontSet, Fontconfig, ObjectSet, Pattern, FC_FAMILY, FC_FILE,
    FC_FONTFORMAT, FC_FULLNAME, FC_INDEX, FC_LANG, FC_MONO, FC_SLANT, FC_SLANT_ITALIC,
    FC_SLANT_ROMAN, FC_SPACING, FC_WEIGHT, FC_WEIGHT_BLACK, FC_WEIGHT_BOLD, FC_WEIGHT_EXTRABOLD,
    FC_WEIGHT_EXTRALIGHT, FC_WEIGHT_LIGHT, FC_WEIGHT_MEDIUM, FC_WEIGHT_NORMAL, FC_WEIGHT_SEMIBOLD,
    FC_WEIGHT_THIN,
};
use itertools::Itertools;

use super::font_handle::{Error as FontDataError, FontHandle};
use super::{FontFamily, ValidateFontSupportsEn};
use crate::fonts::{FontInfo, Properties, Style, Weight};

/// Manages font detection and handle generation.
///
/// Contains our font loading object, wrapping around fontconfig::FontConfig
/// to query the available fonts on the system and return handles grouped into
/// families
pub struct FontconfigLoader {
    fc: Fontconfig,
}

impl FontconfigLoader {
    /// Creates a new FontLoader instance.
    ///
    /// # Errors
    ///
    /// Will return an Error::Init if the underlying FFI wrapper
    /// for Fontconfig fails to initialize
    pub fn new() -> Result<Self, Error> {
        if let Some(fc) = Fontconfig::new() {
            Ok(Self { fc })
        } else {
            Err(Error::Init)
        }
    }

    /// Gets a handle for a single font family.
    ///
    /// Looks up all fonts in the font family specified by `family_name`.
    /// Returns a FamilyHandle for those fonts
    ///
    /// # Errors
    /// If there are zero valid fonts within the family, this will error with
    /// Error::FamilyHasNoFonts
    ///
    /// Additionally, passing a malformed CString name (ex: a string w/ a null terminator)
    /// can trigger an Error::InvalidFontName.
    pub(super) fn get_family(&self, family_name: &str) -> Result<FamilyHandle, Error> {
        let fonts = self.query_fonts(Some(family_name))?;
        let mut family = FamilyHandle::new(family_name);
        let mut errors = Vec::<Error>::new();
        for pattern in fonts.iter() {
            match Self::parse_font(pattern, ValidateFontSupportsEn::Yes) {
                Ok(font) => family.add_font(font),
                Err(err) => errors.push(err),
            }
        }
        if !family.fonts.is_empty() {
            Ok(family)
        } else {
            Err(Error::FamilyHasNoFonts(family_name.to_string(), errors))
        }
    }

    // Gets handles for all font families present on the device.
    //
    // Searches for all available fonts on the device, and returns
    // font families for all valid results. A font is considered valid if:
    //
    // * It has a valid family_name, filename, and face_index
    // * It supports the language 'en'.
    // * It has a TTF or CFF format
    //
    // Any invalid fonts are skipped over, with logging explaining why it was skipped
    pub(super) fn get_all_families(&self) -> Result<Vec<FamilyHandle>, Error> {
        let fonts = self.query_fonts(None)?;

        let mut family_map = HashMap::new();
        for pattern in fonts.iter() {
            let font_name = pattern.name().unwrap_or("unknown");
            let Some(family_name) = pattern.get_string(FC_FAMILY).map(|name| name.to_string())
            else {
                log::warn!("could not parse font_family for font {font_name}",);
                continue;
            };

            let font_handle = match Self::parse_font(pattern, ValidateFontSupportsEn::Yes) {
                Ok(handle) => handle,
                Err(_) => continue,
            };
            family_map
                .entry(family_name.to_string())
                .or_insert_with(|| FamilyHandle::new(&family_name))
                .add_font(font_handle);
        }
        let mut results = family_map.into_values().collect::<Vec<_>>();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    }

    /// Convenience function to parse a font from a pattern, and log appropriately
    /// if the parsing fails.
    /// If `validate` is set to [`ValidateFontSupportsEn::Yes`] an error is returned if the font does not support
    /// english.
    fn parse_font(
        pattern: Pattern<'_>,
        validate: ValidateFontSupportsEn,
    ) -> Result<FontHandle, Error> {
        FontHandle::try_from_pattern(&pattern, validate).map_err(|err| {
            let font_name = pattern.name().unwrap_or("unknown");
            match &err {
                Error::InvalidFontFormat(_) | Error::DoesNotSupportEn => {
                    log::debug!("skipping font {font_name} because of error: {err:#}")
                }
                _ => {
                    log::warn!("could not parse font {font_name}: {err:#}");
                }
            };
            err
        })
    }

    /// Returns a list of fallback fonts that match the `family_name` and given `properties`, in order of closeness.
    pub fn fallback_fonts(
        &self,
        family_name: &str,
        properties: Properties,
    ) -> Result<Vec<FontHandle>, Error> {
        let mut pattern = Pattern::new(&self.fc);
        // Though unlikely, return an `Error` if the requested family name has a null character in it.
        let name = CString::new(family_name)
            .map_err(|_| Error::InvalidFontName(family_name.to_string()))?;
        pattern.add_string(FC_FAMILY, &name);
        pattern.add_integer(FC_WEIGHT, to_fontconfig_weight(properties.weight));
        pattern.add_integer(FC_SLANT, to_fontconfig_style(properties.style));

        let mut object_set = ObjectSet::new(&self.fc);
        object_set.add(FC_FAMILY);
        object_set.add(FC_FULLNAME);
        object_set.add(FC_FILE);
        object_set.add(FC_INDEX);

        // By setting trim to true, we omit fonts that have a unicode range covered by prior fonts in chain. Doing this
        // reduces the overall set of fallback fonts we need to load.
        let sort_fonts = sort_fonts(&pattern, true /* trim */);

        // Skip the first font, since this is considered the primary "font" we're trying to match.
        let fallback_fonts = sort_fonts
            .iter()
            .skip(1)
            .filter_map(|pattern| {
                // Fallback fonts we load aren't guaranteed to support english.
                // Also, parse_font already has logging for parsing, so we log there.
                Self::parse_font(pattern, ValidateFontSupportsEn::No).ok()
            })
            .collect_vec();

        Ok(fallback_fonts)
    }

    fn query_fonts(&self, family_name: Option<&str>) -> Result<FontSet<'_>, Error> {
        let mut pattern = Pattern::new(&self.fc);
        if let Some(name) = family_name {
            // Very unlikely that someone is going to pass a font name with a \0 in,
            // but covering just in case w/ an error
            let name = CString::new(name).map_err(|_| Error::InvalidFontName(name.to_string()))?;
            pattern.add_string(FC_FAMILY, &name)
        }

        let mut object_set = ObjectSet::new(&self.fc);
        object_set.add(FC_FAMILY);
        object_set.add(FC_FULLNAME);
        object_set.add(FC_FILE);
        object_set.add(FC_INDEX);
        object_set.add(FC_SPACING);
        object_set.add(FC_LANG);
        object_set.add(FC_FONTFORMAT);

        Ok(list_fonts(&pattern, Some(&object_set)))
    }
}

impl FontHandle {
    /// Attempts to generate a FontHandle from a Fontconfig Pattern.
    ///
    /// In order to properly parse out a FontHandle, the pattern needs to have
    ///
    /// * A filename
    /// * a face_index
    ///
    /// If either of these fields are missing, an Error::MissingMetadataField will
    /// be returned
    ///
    /// Additionally, will return the following errors:
    ///
    /// * Error::DoesNotSupportEn: if the pattern is missing en as a supported language and `validate_fonts_support_en`
    ///   is set to [`ValidateFontSupportsEn::Yes`].
    /// * Error::InvalidFontFormat: if the pattern's font format is not TTF or CFF.
    fn try_from_pattern(
        value: &Pattern<'_>,
        validate_font_supports_en: ValidateFontSupportsEn,
    ) -> Result<Self, Error> {
        let file_path = value
            .filename()
            .ok_or_else(|| Error::MissingMetadataField("filename".to_owned()))?;

        let index = value
            .face_index()
            .ok_or_else(|| Error::MissingMetadataField("face_index".to_owned()))?
            as u32;

        if matches!(validate_font_supports_en, ValidateFontSupportsEn::Yes)
            && !value
                .lang_set()
                .is_some_and(|lang_set| lang_set.into_iter().any(|lang| lang == "en"))
        {
            return Err(Error::DoesNotSupportEn);
        }

        if !matches!(
            value.format(),
            Ok(fontconfig::FontFormat::TrueType) | Ok(fontconfig::FontFormat::CFF)
        ) {
            // NOTE: fontconfig::FontFormat does not impl Debug or any mapping to strings,
            // so for debugging purposes we pull the underlying string field the
            // enum is computed from.
            let font_format_str = value.get_string(FC_FONTFORMAT).unwrap_or_default();
            return Err(Error::InvalidFontFormat(font_format_str.to_string()));
        }

        let spacing = value.get_int(FC_SPACING);

        Ok(FontHandle::new(
            file_path,
            index,
            match spacing {
                None => false,
                Some(v) => v == FC_MONO,
            },
        ))
    }
}

/// A handle containing information necessary to load all font faces in a family.
pub(super) struct FamilyHandle {
    name: String,
    fonts: Vec<FontHandle>,
}

impl FamilyHandle {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            fonts: vec![],
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn add_font(&mut self, font: FontHandle) {
        self.fonts.push(font);
    }

    /// Consumes the Family Handle into a FontFamily object.
    pub fn into_family(self) -> Result<FontFamily, Error> {
        self.into_info_and_family().map(|(_, family)| family)
    }

    /// Converts the [`FamilyHandle`] into a [`FontInfo`], [`FontFamily`] pair.
    pub fn into_info_and_family(self) -> Result<(FontInfo, FontFamily), Error> {
        let mut fonts = Vec::<FontHandle>::with_capacity(self.fonts.len());
        let mut errors = Vec::<Error>::new();
        let mut is_monospace = false;
        let name = self.name;

        for handle in self.fonts {
            match handle.validate_font_data() {
                Ok(_) => {
                    is_monospace |= handle.is_monospace();
                    fonts.push(handle);
                }
                Err(err) => errors.push(Error::FontData(err)),
            }
        }
        if !fonts.is_empty() {
            Ok((
                FontInfo {
                    family_name: name.clone(),
                    is_monospace,
                },
                FontFamily { fonts, name },
            ))
        } else {
            Err(Error::FamilyHasNoFonts(name, errors))
        }
    }
}

/// Errors associated with loading fonts.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The FontLoader cannot be initialized b/c the underlying
    /// Fontconfig ffi handle failed to init.
    #[error("Failed to initialize Fontconfig ffi handle")]
    Init,

    /// The user has passed a malformed CString font name.
    #[error("Invalid Font Name {0}")]
    InvalidFontName(String),

    /// A font could not be parsed into a handle b/c it is missing
    /// an important metadata field.
    #[error("Could not parse font, missing metadata field {0}")]
    MissingMetadataField(String),

    /// A font does not have a valid font format (either TTF or CFF)
    #[error("Invalid font format '{0}'")]
    InvalidFontFormat(String),

    /// A font does not support the language en
    #[error("Font does not support language en")]
    DoesNotSupportEn,

    /// A font family has been requested, but there are no valid
    /// fonts for that family
    #[error("Font family {0} does not contain any valid fonts")]
    FamilyHasNoFonts(String, Vec<Error>),

    // When the underlying font handle has trouble loading data.
    #[error("Failed to load font data")]
    FontData(#[from] FontDataError),
}

fn to_fontconfig_weight(weight: Weight) -> c_int {
    match weight {
        Weight::Thin => FC_WEIGHT_THIN,
        Weight::ExtraLight => FC_WEIGHT_EXTRALIGHT,
        Weight::Light => FC_WEIGHT_LIGHT,
        Weight::Normal => FC_WEIGHT_NORMAL,
        Weight::Medium => FC_WEIGHT_MEDIUM,
        Weight::Semibold => FC_WEIGHT_SEMIBOLD,
        Weight::Bold => FC_WEIGHT_BOLD,
        Weight::ExtraBold => FC_WEIGHT_EXTRABOLD,
        Weight::Black => FC_WEIGHT_BLACK,
    }
}

fn to_fontconfig_style(style: Style) -> c_int {
    match style {
        Style::Normal => FC_SLANT_ROMAN,
        Style::Italic => FC_SLANT_ITALIC,
    }
}
