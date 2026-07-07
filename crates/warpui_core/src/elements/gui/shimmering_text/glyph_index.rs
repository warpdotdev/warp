#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct GlyphIndex<T>(pub(super) T);

impl GlyphIndex<usize> {
    pub(super) fn as_f32(&self) -> GlyphIndex<f32> {
        GlyphIndex(self.0 as f32)
    }
}
