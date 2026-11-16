use enum_iterator::Sequence;
use markdown_parser::weight::CustomWeight;

/// Header sizes for formatted text blocks.
#[derive(Eq, PartialEq, Clone, Copy, Debug, Hash, Sequence)]
pub enum BlockHeaderSize {
    Header1,
    Header2,
    Header3,
    Header4,
    Header5,
    Header6,
}

impl BlockHeaderSize {
    /// Get font size multiplication ratio for this heading level.
    pub fn font_size_multiplication_ratio(self) -> f32 {
        // The WHATWG HTML living standard is a useful starting point for these, but we don't
        // follow it exactly:
        // https://html.spec.whatwg.org/multipage/rendering.html#sections-and-headings
        match self {
            Self::Header1 => 2.25,
            Self::Header2 => 1.8,
            Self::Header3 => 1.5,
            Self::Header4 => 1.0,
            Self::Header5 => 0.83,
            Self::Header6 => 0.67,
        }
    }

    /// Font weight for this heading level.
    pub fn font_weight(self) -> Option<CustomWeight> {
        match self {
            Self::Header1 | Self::Header2 | Self::Header3 | Self::Header4 => {
                Some(CustomWeight::Semibold)
            }
            Self::Header5 | Self::Header6 => None,
        }
    }

    /// A text label for this heading, in the format `Heading $N`.
    pub fn label(self) -> &'static str {
        match self {
            BlockHeaderSize::Header1 => "Heading 1",
            BlockHeaderSize::Header2 => "Heading 2",
            BlockHeaderSize::Header3 => "Heading 3",
            BlockHeaderSize::Header4 => "Heading 4",
            BlockHeaderSize::Header5 => "Heading 5",
            BlockHeaderSize::Header6 => "Heading 6",
        }
    }
}

impl From<BlockHeaderSize> for usize {
    fn from(header_size: BlockHeaderSize) -> Self {
        match header_size {
            BlockHeaderSize::Header1 => 1,
            BlockHeaderSize::Header2 => 2,
            BlockHeaderSize::Header3 => 3,
            BlockHeaderSize::Header4 => 4,
            BlockHeaderSize::Header5 => 5,
            BlockHeaderSize::Header6 => 6,
        }
    }
}

impl TryFrom<usize> for BlockHeaderSize {
    type Error = ();

    fn try_from(header_size: usize) -> Result<Self, Self::Error> {
        match header_size {
            1 => Ok(BlockHeaderSize::Header1),
            2 => Ok(BlockHeaderSize::Header2),
            3 => Ok(BlockHeaderSize::Header3),
            4 => Ok(BlockHeaderSize::Header4),
            5 => Ok(BlockHeaderSize::Header5),
            6 => Ok(BlockHeaderSize::Header6),
            _ => Err(()),
        }
    }
}
