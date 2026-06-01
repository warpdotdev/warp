use std::any::Any;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, ItemRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef, UniChar};
use core_graphics::display::CGSize;
use core_graphics::font::CGGlyph;
use core_text::font::{cascade_list_for_languages as ct_cascade_list_for_languages, CTFont};
use core_text::font_descriptor::{
    kCTFontFamilyNameAttribute, kCTFontLanguagesAttribute, kCTFontNameAttribute,
    kCTFontOrientationHorizontal, CTFontDescriptor, CTFontDescriptorCopyAttribute,
    SymbolicTraitAccessors, TraitAccessors,
};
use core_text::{font, font_collection, font_descriptor};
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use font_kit::font::Font;
use font_kit::loaders::core_text::NativeFont;
use futures::future::BoxFuture;
use futures::FutureExt as _;
use itertools::Itertools as _;
use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::{Vector2F, Vector2I};
use warpui_core::fonts::canvas::RasterFormat;
use warpui_core::fonts::{
    FamilyId, FontId, FontInfo, GlyphId, Metrics, Properties, RasterizedGlyph, SubpixelAlignment,
};
use warpui_core::platform::{self, FontDB as _, LineStyle, TextLayoutSystem};
use warpui_core::rendering;
use warpui_core::text_layout::{ClipConfig, StyleAndFont, TextAlignment, TextFrame};

use super::text_layout::{layout_line, layout_text};
use crate::fonts::font_kit::{properties_to_font_kit, Rasterizer};

struct FontFamily {
    name: String,
    fonts: Vec<Font>,
}

mod loader {
    use super::*;

    // Font-kit loads fonts by copying their font file into the running memory, which
    // is extremely inefficient. Thus, for system fonts, we load these fonts by reference
    // through CTFontDescriptorCreateWithAttributes function and create a dummy font-kit
    // Font interface to access its functions.
    pub fn load_system_font(font_family: &str) -> Result<FontFamily> {
        let Some(descriptors) = FontDB::descriptors_for_family(font_family) else {
            bail!(
                "could not find a non-empty font family matching one of the given names {:?}",
                font_family
            );
        };
        let mut fonts = Vec::with_capacity(descriptors.len() as usize);
        for fontdesc in descriptors.into_iter() {
            // The font size here does not affect our rendering. In CTFont, pt_size
            // is used to calculate font metrics like ascent, descent, etc. However,
            // font-kit creates its own layer of calculating font metrics at render time
            // so we just need a place-holder here for getting the CTFont object. Use
            // 16.0 here as it is consistent with https://docs.rs/core-text/19.2.0/src/core_text/font.rs.html#130
            let font = Font::from_ct_font(font::new_from_descriptor(&fontdesc, DEFAULT_FONT_SIZE));

            let glyph_id = font.glyph_for_char('m');
            if glyph_id.is_none() {
                return Err(anyhow!("font must contain a glyph for the 'm' character"));
            }

            fonts.push(font);
        }
        Ok(FontFamily {
            fonts,
            name: font_family.into(),
        })
    }

    pub fn load_all_system_fonts() -> LoadedSystemFonts {
        let collection = font_collection::create_for_all_families();
        let Some(descriptors) = collection.get_descriptors() else {
            return LoadedSystemFonts(vec![]);
        };

        let mut fonts: Vec<(FontInfo, FontFamily)> = Vec::with_capacity(descriptors.len() as usize);
        for descriptor in descriptors.iter() {
            let name = match unsafe { FontDB::get_family_name(&descriptor) } {
                Some(family) => family,
                None => {
                    log::warn!("Failed to load the font as it does not have a valid family name.");
                    continue;
                }
            };

            let internal_name = match unsafe { FontDB::get_font_name(&descriptor) } {
                Some(family) => family,
                None => {
                    log::warn!("Failed to load the font as it does not have a valid font name.");
                    continue;
                }
            };

            // We should only load languages that support english
            if !FontDB::supports_english(&descriptor) {
                continue;
            }

            if let Some(idx) = fonts.iter().position(|font| font.0.family_name == name) {
                // Some font families (e.g. Osaka) could contains both monospace
                // and variable-width fonts. To make sure the returned font family
                // info is consistent everytime, we set is_monospace to true if the
                // family contains one font that is monospace.
                if descriptor.traits().symbolic_traits().is_monospace()
                    && !fonts[idx].0.is_monospace
                {
                    // Updating here since font family names are guaranteed to be unique.
                    fonts[idx].0.is_monospace = true;
                }
                // Since we keep track of font families, add this font as a possible style.
                fonts[idx].0.font_names.push(internal_name)
            } else {
                let font_family = match load_system_font(&name) {
                    Ok(font) => font,
                    Err(err) => {
                        log::debug!("Failed to load {}: {:?}", name.as_str(), err);
                        continue;
                    }
                };

                fonts.push((
                    FontInfo {
                        family_name: name,
                        font_names: vec![internal_name],
                        is_monospace: descriptor.traits().symbolic_traits().is_monospace(),
                    },
                    font_family,
                ));
            }
        }

        LoadedSystemFonts(fonts)
    }

    // We use font-kit's family handle to load fonts that come with Warp as
    // these binaries are already in memory and won't increase our memory load.
    pub fn load_font_family_from_bytes(name: &str, font_bytes: Vec<Vec<u8>>) -> Result<FontFamily> {
        let mut fonts = Vec::with_capacity(font_bytes.len());

        for font in font_bytes {
            let Ok(font) = Font::from_bytes(Arc::new(font), 0) else {
                log::info!("Unable to parse font bytes for font {name:?}");
                continue;
            };

            let glyph_id = font.glyph_for_char('m');
            if glyph_id.is_none() {
                return Err(anyhow!("font must contain a glyph for the 'm' character"));
            }
            fonts.push(font);
        }

        Ok(FontFamily {
            fonts,
            name: name.to_owned(),
        })
    }
}

struct LoadedSystemFonts(Vec<(FontInfo, FontFamily)>);

impl platform::LoadedSystemFonts for LoadedSystemFonts {
    fn as_any(self: Box<Self>) -> Box<dyn Any> {
        self as Box<dyn Any>
    }
}

pub struct FontDB {
    next_family_id: AtomicUsize,
    families: HashMap<FamilyId, Family>,
    next_font_id: AtomicUsize,
    rasterizer: Rasterizer,
    font_names: DashMap<FontId, Arc<String>>,
    native_fonts: DashMap<(FontId, OrderedFloat<f32>), NativeFont>,
    fonts_by_name: DashMap<Arc<String>, FontId>,
    fallback_fonts: DashMap<FontId, Arc<Vec<FontId>>>,
    metrics: DashMap<FontId, Metrics>,
    font_selections: DashMap<(FamilyId, Properties), FontId>,
    space_advances: DashMap<FontId, Option<f64>>,
}

struct Family {
    name: String,
    font_ids: Vec<FontId>,
}

impl Default for FontDB {
    fn default() -> Self {
        Self::new()
    }
}

const DEFAULT_FONT_SIZE: f64 = 16.0;

// Returns the horizontal advance (in points) of a single space in the given font.
fn space_advance_width(font: &CTFont) -> Option<f64> {
    let space_char: UniChar = ' ' as u16;
    let mut glyph: CGGlyph = 0;

    let ok =
        unsafe { font.get_glyphs_for_characters(&space_char as *const UniChar, &mut glyph, 1) };
    if !ok || glyph == 0 {
        return None;
    }

    let mut advance = CGSize {
        width: 0.0,
        height: 0.0,
    };
    unsafe {
        font.get_advances_for_glyphs(
            kCTFontOrientationHorizontal,
            &glyph as *const CGGlyph,
            &mut advance as *mut CGSize,
            1,
        );
    }

    let width = advance.width;
    (width.is_finite() && width > 0.0).then_some(width)
}

impl FontDB {
    pub fn new() -> Self {
        Self {
            next_family_id: Default::default(),
            families: Default::default(),
            next_font_id: Default::default(),
            rasterizer: Rasterizer::new(),
            font_names: Default::default(),
            native_fonts: Default::default(),
            fonts_by_name: Default::default(),
            fallback_fonts: Default::default(),
            metrics: Default::default(),
            font_selections: Default::default(),
            space_advances: Default::default(),
        }
    }

    // This functions the same as the family_name method in core text font descriptor, but it returns
    // None instead of panicking when the descriptor does not include the family_name attribute.
    unsafe fn get_family_name(descriptor: &ItemRef<CTFontDescriptor>) -> Option<String> {
        let value = CTFontDescriptorCopyAttribute(
            descriptor.as_concrete_TypeRef(),
            kCTFontFamilyNameAttribute,
        );
        if value.is_null() {
            return None;
        }

        let value = CFType::wrap_under_create_rule(value);
        let s = CFString::wrap_under_get_rule(value.as_CFTypeRef() as CFStringRef);
        Some(s.to_string())
    }

    unsafe fn get_font_name(descriptor: &ItemRef<CTFontDescriptor>) -> Option<String> {
        let value =
            CTFontDescriptorCopyAttribute(descriptor.as_concrete_TypeRef(), kCTFontNameAttribute);
        if value.is_null() {
            return None;
        }

        let value = CFType::wrap_under_create_rule(value);
        let s = CFString::wrap_under_get_rule(value.as_CFTypeRef() as CFStringRef);
        Some(s.to_string())
    }

    pub fn fallback_fonts(&self, font_id: FontId) -> Vec<FontId> {
        self.fallback_fonts
            .get(&font_id)
            .expect("Font fallback should not be empty")
            .to_vec()
    }

    // Check if the font family supports english.
    fn supports_english(descriptor: &ItemRef<CTFontDescriptor>) -> bool {
        unsafe {
            let value = CTFontDescriptorCopyAttribute(
                descriptor.as_concrete_TypeRef(),
                kCTFontLanguagesAttribute,
            ) as CFArrayRef;

            if value.is_null() {
                return false;
            }

            let languages: CFArray<CFString> = CFArray::wrap_under_create_rule(value);
            languages.iter().any(|s| *s == "en")
        }
    }

    pub fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.families.get(&id).map(|s| s.name.clone())
    }

    fn create_new_family_id(&self) -> FamilyId {
        FamilyId(self.next_family_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Return fallback descriptors for font/language list.
    /// Heavily inspired by crossfont's implementation:
    /// https://github.com/alacritty/crossfont/blob/d3515de22494c6fa70d84d2a9264c10097e303bd/src/darwin/mod.rs#L288
    fn cascade_list_for_languages(&self, ct_font: &CTFont, languages: &[&str]) -> Vec<FontId> {
        // Convert language type &Vec<String> -> CFArray.
        let langarr: CFArray<CFString> = {
            let tmp: Vec<CFString> = languages
                .iter()
                .map(|language| CFString::new(language))
                .collect();
            CFArray::from_CFTypes(&tmp)
        };

        // CFArray of CTFontDescriptorRef (again).
        let list = ct_cascade_list_for_languages(ct_font, &langarr);

        let mut fallback_fonts: Vec<FontId> = list
            .into_iter()
            .filter_map(|fontdesc| self.descriptor_to_font_id(fontdesc))
            .collect();

        // While .Apple Symbols Fallback is not a valid font. Apple Symbols is and it provides
        // many fallback characters. This implementation is consistent with Alacritty:
        // See: https://github.com/alacritty/crossfont/blob/d3515de22494c6fa70d84d2a9264c10097e303bd/src/darwin/mod.rs#L91
        if let Some(font) = FontDB::descriptors_for_family("Apple Symbols")
            .as_ref()
            .and_then(|descriptor| descriptor.into_iter().next())
            .and_then(|font_descriptor| self.descriptor_to_font_id(font_descriptor))
        {
            fallback_fonts.push(font);
        }

        fallback_fonts
    }

    // Get a list of CTFontDescriptors for a font family.
    fn descriptors_for_family(name: &str) -> Option<CFArray<CTFontDescriptor>> {
        let attributes: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[(
            CFString::new("NSFontFamilyAttribute"),
            CFString::new(name).as_CFType(),
        )]);

        let descriptor = font_descriptor::new_from_attributes(&attributes);
        let collection_descriptors = &CFArray::from_CFTypes(&[descriptor]);
        let collection = font_collection::new_from_descriptors(collection_descriptors);

        collection.get_descriptors()
    }

    // Convert a CTFontDescriptor to font_id. This function does not load fallback fonts
    // and assumes the descriptor refers to a valid system font.
    fn descriptor_to_font_id(&self, fontdesc: ItemRef<CTFontDescriptor>) -> Option<FontId> {
        let name = match unsafe { FontDB::get_family_name(&fontdesc) } {
            Some(family) => family,
            None => {
                log::warn!("Failed to load the font as it does not have a valid family name.");
                return None;
            }
        };

        let font_name = match unsafe { FontDB::get_font_name(&fontdesc) } {
            Some(name) => name,
            None => {
                log::warn!("Failed to load the font as it does not have a valid name.");
                return None;
            }
        };

        // We should not load fonts with name that starts with dot
        // https://developer.apple.com/videos/play/wwdc2019/227/?time=200
        (!name.starts_with('.')).then(|| {
            // Check if the fallback font is in cache.
            match self.fonts_by_name.entry(Arc::new(font_name)) {
                Entry::Occupied(entry) => return *entry.get(),
                Entry::Vacant(_) => (),
            }

            // We need to push font after releasing the entry of the dashmap to prevent deadlocks.
            self.push_font(Font::from_ct_font(font::new_from_descriptor(
                &fontdesc,
                DEFAULT_FONT_SIZE,
            )))
        })
    }

    pub fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId {
        match self.font_selections.entry((family_id, properties)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let family = &self
                    .families
                    .get(&family_id)
                    .expect("FamilyId must correspond to a valid family");
                let candidates = family
                    .font_ids
                    .iter()
                    .map(|font_id| self.font(*font_id).properties())
                    .collect::<Vec<_>>();

                let font_id = {
                    if let Ok(idx) = font_kit::matching::find_best_match(
                        &candidates,
                        &properties_to_font_kit(properties),
                    ) {
                        self.font(family.font_ids[idx]).properties();
                        family.font_ids[idx]
                    } else {
                        font_kit::matching::find_best_match(&candidates, &Default::default())
                            .map(|idx| family.font_ids[idx])
                            .unwrap_or(family.font_ids[0])
                    }
                };

                // Make sure we've loaded fallback fonts for the selected font.
                if !self.fallback_fonts.contains_key(&font_id) {
                    self.fallback_fonts.insert(
                        font_id,
                        Arc::new(self.cascade_list_for_languages(
                            &self.font(font_id).native_font(),
                            &["en"],
                        )),
                    );
                }
                *entry.insert(font_id)
            }
        }
    }

    pub fn font(&self, font_id: FontId) -> Arc<Font> {
        self.rasterizer.font_for_id(font_id)
    }

    pub fn native_font(&self, font_id: FontId, size: f32) -> NativeFont {
        match self.native_fonts.entry((font_id, OrderedFloat(size))) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry
                .insert(
                    self.rasterizer
                        .font_for_id(font_id)
                        .native_font()
                        .clone_with_font_size(size as f64),
                )
                .clone(),
        }
    }

    /// Returns the horizontal advance of a space character at the given size, or `None` if the
    /// advance could not be measured. Uses a cached reference advance (measured at
    /// `DEFAULT_FONT_SIZE`) scaled linearly to `size`.
    pub fn space_advance_width(&self, font_id: FontId, size: f32) -> Option<f64> {
        let stored = *self.space_advances.get(&font_id)?;
        stored.map(|a| a * size as f64 / DEFAULT_FONT_SIZE)
    }

    pub fn font_id_for_native_font(&self, native_font: NativeFont) -> FontId {
        let postscript_name = native_font.postscript_name();
        if let Some(font_id) = self.fonts_by_name.get(&postscript_name).as_ref() {
            return *font_id.value();
        }

        self.push_font(Font::from_ct_font(native_font))
    }

    fn push_font(&self, font: Font) -> FontId {
        let name = Arc::new(font.postscript_name().unwrap());
        let font_id = FontId(self.next_font_id.fetch_add(1, Ordering::SeqCst));

        let ct_font = font.native_font().clone_with_font_size(DEFAULT_FONT_SIZE);
        let advance = space_advance_width(&ct_font);

        self.rasterizer.insert(font_id, Arc::new(font));
        self.font_names.insert(font_id, name.clone());
        self.fonts_by_name.insert(name, font_id);
        self.space_advances.insert(font_id, advance);
        font_id
    }

    fn insert_font_family(&mut self, font_family: FontFamily) -> Result<FamilyId> {
        if let Some(family_id) = self.family_id_for_name(&font_family.name) {
            return Ok(family_id);
        }

        let font_ids = font_family
            .fonts
            .into_iter()
            .map(|font| self.push_font(font));

        let family_id = self.create_new_family_id();
        self.families.insert(
            family_id,
            Family {
                name: font_family.name,
                font_ids: font_ids.collect(),
            },
        );

        Ok(family_id)
    }
}

impl crate::platform::FontDB for FontDB {
    fn load_from_bytes(&mut self, name: &str, bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        let family = loader::load_font_family_from_bytes(name, bytes)?;
        self.insert_font_family(family)
    }

    fn load_from_system(&mut self, font_family: &str) -> Result<FamilyId> {
        let family = loader::load_system_font(font_family)?;
        self.insert_font_family(family)
    }

    fn load_all_system_fonts(&self) -> BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        async { Box::new(loader::load_all_system_fonts()) as Box<dyn platform::LoadedSystemFonts> }
            .boxed()
    }

    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, crate::fonts::FontInfo)> {
        let loaded_system_fonts: Box<LoadedSystemFonts> = loaded_system_fonts
            .as_any()
            .downcast()
            .expect("should not fail to downcast to concrete type");

        loaded_system_fonts
            .0
            .into_iter()
            .flat_map(|(font_info, family)| {
                let family_id = self.insert_font_family(family).ok()?;
                Some((Some(family_id), font_info))
            })
            .collect_vec()
    }

    fn fallback_fonts(&self, _ch: char, font_id: FontId) -> Vec<FontId> {
        self.fallback_fonts(font_id)
    }

    fn load_family_name_from_id(&self, id: FamilyId) -> Option<String> {
        self.load_family_name_from_id(id)
    }

    fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId {
        self.select_font(family_id, properties)
    }

    fn font_metrics(&self, font_id: FontId) -> Metrics {
        match self.metrics.entry(font_id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => *entry.insert(self.font(font_id).metrics().into()),
        }
    }

    fn glyph_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Vector2I> {
        Ok(self.font(font_id).advance(glyph_id)?.to_i32())
    }

    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        point_size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI> {
        self.rasterizer
            .glyph_raster_bounds(font_id, point_size, glyph_id, scale, glyph_config)
    }

    fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
        Ok(self.font(font_id).typographic_bounds(glyph_id)?.to_i32())
    }

    fn rasterize_glyph(
        &self,
        font_id: FontId,
        point_size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph> {
        self.rasterizer.rasterize_glyph(
            font_id,
            point_size,
            glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }

    fn glyph_for_char(&self, font: FontId, char: char) -> Option<GlyphId> {
        self.font(font).glyph_for_char(char)
    }

    fn family_id_for_name(&self, name: &str) -> Option<FamilyId> {
        self.families
            .iter()
            .find(|(_, f)| f.name == name)
            .map(|(id, _)| *id)
    }

    fn text_layout_system(&self) -> &dyn TextLayoutSystem {
        self
    }
}

impl crate::platform::TextLayoutSystem for FontDB {
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        _max_width: f32,
        clip_config: ClipConfig,
    ) -> crate::text_layout::Line {
        layout_line(text, line_style, style_runs, self, clip_config)
    }

    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        layout_text(
            text,
            line_style,
            style_runs,
            self,
            max_width,
            max_height,
            alignment,
            first_line_head_indent,
        )
    }
}
