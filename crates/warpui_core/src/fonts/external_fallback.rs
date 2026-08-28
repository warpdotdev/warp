use std::hash::{Hash, Hasher};
use std::mem;
use std::sync::Arc;

use anyhow::Result;
use itertools::Itertools;

use super::{Cache, FamilyId, FontFallbackCache, FontFamilyName, FontId, GlyphId};
use crate::assets::asset_cache::Asset;
use crate::{Entity, ModelContext, SingletonEntity, text_layout};

/// Represents a font family that is lazy loaded from the web. i.e. Not a
/// bundled or system font.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalFontFamily {
    pub font_urls: Arc<Vec<String>>,
    pub name: &'static str,
}

impl Hash for ExternalFontFamily {
    // Font families should have unique names. To avoid doing unnecessary work
    // hashing all of the URLs of the fonts in the family, the hash function
    // will only hash the font family name.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

/// Specifies the different locations that can request a fallback font. Used to
/// clear the appropriate cache when a fallback font is loaded.
#[derive(Eq, PartialEq, Hash)]
pub(crate) enum RequestedFallbackFontSource {
    GlyphForChar((FontId, char)),
    Line(text_layout::CacheKeyValue),
    TextFrame(text_layout::CacheKeyValue),
}

pub(crate) struct FontBytes(pub Vec<u8>);

impl Asset for FontBytes {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(data.to_vec()))
    }

    fn size_in_bytes(&self) -> usize {
        self.0.len()
    }
}

pub enum FallbackFontEvent {
    Loaded,
}

/// Handles notifying listeners when an external fallback font has loaded.
pub struct FallbackFontModel {}

impl FallbackFontModel {
    pub(crate) fn new() -> Self {
        Self {}
    }

    pub(crate) fn loaded_fallback_font(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(FallbackFontEvent::Loaded);
    }
}

impl Entity for FallbackFontModel {
    type Event = FallbackFontEvent;
}

impl SingletonEntity for FallbackFontModel {}

impl FontFallbackCache {
    pub(super) fn app_fallback_family_for_char(&self, ch: char) -> Option<ExternalFontFamily> {
        self.fallback_font_fn
            .as_ref()
            .and_then(|fallback_font_fn| (fallback_font_fn)(ch))
    }

    /// Checks if the application specified a fallback font for the given char.
    /// If yes, the UI framework will lazy load the fallback font and trigger
    /// a re-render of the window.
    pub(crate) fn request_fallback_font_for_char(
        &self,
        ch: char,
        source: RequestedFallbackFontSource,
    ) {
        let Some(fallback_font_family) = self.app_fallback_family_for_char(ch) else {
            return;
        };

        if self
            .loaded_fallback_families
            .contains_key(fallback_font_family.name)
        {
            return;
        }

        self.requested_fallback_families
            .entry(fallback_font_family)
            .or_default()
            .push(source);
    }
}

impl Cache {
    pub(crate) fn load_fallback_family_from_bytes(
        &mut self,
        external_family: ExternalFontFamily,
        bytes: Vec<Vec<u8>>,
    ) -> Result<FamilyId> {
        let family_id = self.load_family_from_bytes(external_family.name, bytes)?;
        self.font_fallback_cache
            .loaded_fallback_families
            .insert(external_family.name, family_id);
        Ok(family_id)
    }

    pub(crate) fn set_fallback_font_fn(
        &mut self,
        fallback_font_fn: Box<dyn Fn(char) -> Option<ExternalFontFamily> + Send + Sync>,
    ) {
        self.font_fallback_cache.fallback_font_fn = Some(fallback_font_fn);
    }

    fn app_fallback_family_for_char(&self, ch: char) -> Option<ExternalFontFamily> {
        self.font_fallback_cache.app_fallback_family_for_char(ch)
    }

    /// Checks for a matching glyph in the fallback fonts loaded by the application.
    pub(crate) fn app_font_fallback(&self, ch: char, font: FontId) -> Option<(GlyphId, FontId)> {
        let external_fallback_family = self.app_fallback_family_for_char(ch)?;
        let fallback_family = self
            .font_fallback_cache
            .loaded_fallback_families
            .get(external_fallback_family.name)?;
        // We need to be careful here. `self.font_properties` is implemented
        // using a DashMap, which can deadlock if we hold a reference while
        // inserting into the map. We dereference the properties immediately
        // to avoid holding a reference, since `select_font` will insert into
        // the DashMap.
        let properties = *self.font_properties.get(&font)?;
        let fallback_font = self.select_font(*fallback_family, properties);

        self.glyph_for_char(fallback_font, ch, false)
    }

    pub(crate) fn take_requested_fallback_families(
        &self,
    ) -> impl Iterator<Item = (ExternalFontFamily, Vec<RequestedFallbackFontSource>)> {
        let result = self
            .font_fallback_cache
            .requested_fallback_families
            .iter_mut()
            .map(|mut entry| (entry.key().clone(), mem::take(entry.value_mut())))
            .collect_vec();
        self.font_fallback_cache.requested_fallback_families.clear();
        result.into_iter()
    }

    pub(crate) fn is_fallback_family_loaded(&self, family: FontFamilyName) -> bool {
        self.font_fallback_cache
            .loaded_fallback_families
            .contains_key(family)
    }
}
