/// Various metrics that apply to the entire font.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// The number of font units per em.
    ///
    /// Font sizes are usually expressed in pixels per em; e.g. `12px` means 12 pixels per em.
    pub units_per_em: u32,

    /// The maximum amount the font rises above the baseline, in font units.
    pub ascent: i16,

    /// The maximum amount the font descends below the baseline, in font units.
    ///
    /// NB: This is typically a negative value to match the definition of `sTypoDescender` in the
    /// `OS/2` table in the OpenType specification. If you are used to using Windows or Mac APIs,
    /// beware, as the sign is reversed from what those APIs return.
    pub descent: i16,

    /// Distance between baselines, in font units.
    pub line_gap: i16,
}

#[cfg(native)]
impl From<font_kit::metrics::Metrics> for Metrics {
    fn from(value: font_kit::metrics::Metrics) -> Self {
        Self {
            units_per_em: value.units_per_em,
            ascent: value.ascent as i16,
            descent: value.descent as i16,
            line_gap: value.line_gap as i16,
        }
    }
}
