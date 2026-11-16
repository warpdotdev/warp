/// An iterator over lines that can efficiently return lines in a specified range.
/// This should be shared when multiple in-order ranges need to be accessed.
/// Note that you cannot request a range before the previous range that was requested.
use std::ops::Range;
pub struct LineIterator<'a, I: Iterator<Item = &'a str>> {
    lines: I,
    index: usize,
}

impl<'a, I: Iterator<Item = &'a str>> LineIterator<'a, I> {
    pub fn new(iter: I) -> Self {
        Self {
            lines: iter,
            index: 0,
        }
    }

    pub fn lines_in_range(
        &mut self,
        range: &Range<usize>,
    ) -> anyhow::Result<impl Iterator<Item = &'a str> + use<'a, '_, I>> {
        if self.index > range.start {
            return Err(anyhow::anyhow!(
                "Requested range start {} is before current index {}",
                range.start,
                self.index
            ));
        }

        if self.index < range.start {
            self.lines.nth(range.start - self.index - 1);
            self.index = range.start;
        }

        self.index = range.end;
        Ok(self.lines.by_ref().take(range.end - range.start))
    }
}
