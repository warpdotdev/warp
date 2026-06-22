use std::cmp::Ordering;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct Span {
    start: usize,
    end: usize,
}

impl From<(usize, usize)> for Span {
    fn from((start, end): (usize, usize)) -> Span {
        Span::new(start, end)
    }
}

impl From<&Span> for Span {
    fn from(span: &Span) -> Span {
        *span
    }
}

impl From<Option<Span>> for Span {
    fn from(input: Option<Span>) -> Span {
        input.unwrap_or_else(|| Span::new(0, 0))
    }
}

impl From<Span> for std::ops::Range<usize> {
    fn from(input: Span) -> std::ops::Range<usize> {
        let start = input.start;
        let end = input.end;

        std::ops::Range { start, end }
    }
}

impl Span {
    /// Creates a new `Span` that has 0 start and 0 end.
    pub fn unknown() -> Span {
        Span::new(0, 0)
    }

    pub fn for_char(pos: usize) -> Span {
        Span {
            start: pos,
            end: pos + 1,
        }
    }

    pub fn until(&self, other: impl Into<Span>) -> Span {
        let other = other.into();

        Span::new(self.start, other.end)
    }

    pub fn from_list(list: &[impl HasSpan]) -> Span {
        let mut iterator = list.iter();

        match iterator.next() {
            None => Span::new(0, 0),
            Some(first) => {
                let last = iterator.last().unwrap_or(first);

                Span::new(first.span().start, last.span().end)
            }
        }
    }

    pub fn new(start: usize, end: usize) -> Span {
        assert!(
            end >= start,
            "Can't create a Span whose end < start, start={start}, end={end}"
        );

        Span { start, end }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn skip(&self, n_chars: usize) -> Span {
        Span::new(self.start + n_chars, self.end)
    }

    pub fn distance(&self) -> usize {
        self.end - self.start
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        let start = self.start;
        let end = self.end;

        &source[start..end]
    }
}

impl PartialOrd<usize> for Span {
    fn partial_cmp(&self, other: &usize) -> Option<Ordering> {
        (self.end - self.start).partial_cmp(other)
    }
}

impl PartialEq<usize> for Span {
    fn eq(&self, other: &usize) -> bool {
        (self.end - self.start) == *other
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Spanned<T> {
    pub span: Span,
    pub item: T,
}

impl<T> Spanned<T> {
    pub fn map<U>(self, input: impl FnOnce(T) -> U) -> Spanned<U> {
        let span = self.span;

        let mapped = input(self.item);
        mapped.spanned(span)
    }
}

pub trait SpannedItem: Sized {
    fn spanned(self, span: impl Into<Span>) -> Spanned<Self> {
        Spanned {
            item: self,
            span: span.into(),
        }
    }

    fn spanned_unknown(self) -> Spanned<Self> {
        Spanned {
            item: self,
            span: Span::unknown(),
        }
    }
}

impl<T> SpannedItem for T {}

impl<T> std::ops::Deref for Spanned<T> {
    type Target = T;

    /// Shorthand to deref to the contained value
    fn deref(&self) -> &T {
        &self.item
    }
}

pub trait HasSpan {
    fn span(&self) -> Span;
}

impl<T, E> HasSpan for Result<T, E>
where
    T: HasSpan,
{
    fn span(&self) -> Span {
        match self {
            Result::Ok(val) => val.span(),
            Result::Err(_) => Span::unknown(),
        }
    }
}

impl<T> HasSpan for Spanned<T> {
    fn span(&self) -> Span {
        self.span
    }
}

pub trait IntoSpanned {
    type Output: HasFallibleSpan;

    fn into_spanned(self, span: impl Into<Span>) -> Self::Output;
}

impl<T: HasFallibleSpan> IntoSpanned for T {
    type Output = T;
    fn into_spanned(self, _span: impl Into<Span>) -> Self::Output {
        self
    }
}

pub trait HasFallibleSpan {
    fn maybe_span(&self) -> Option<Span>;
}

impl HasFallibleSpan for bool {
    fn maybe_span(&self) -> Option<Span> {
        None
    }
}

impl HasFallibleSpan for () {
    fn maybe_span(&self) -> Option<Span> {
        None
    }
}

impl<T> HasFallibleSpan for T
where
    T: HasSpan,
{
    fn maybe_span(&self) -> Option<Span> {
        Some(HasSpan::span(self))
    }
}

#[cfg(test)]
#[path = "meta_tests.rs"]
mod tests;
