use bitflags::bitflags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CellFlags: u8 {
        const BOLD      = 0b0000_0001;
        const DIM       = 0b0000_0010;
        const ITALIC    = 0b0000_0100;
        const UNDERLINE = 0b0000_1000;
        const INVERSE   = 0b0001_0000;
        const HIDDEN    = 0b0010_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAttr {
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Default for CellAttr {
    fn default() -> Self {
        Self {
            fg: Color::Default,
            bg: Color::Default,
            flags: CellFlags::empty(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub attr: CellAttr,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            attr: CellAttr::default(),
        }
    }
}

impl Cell {
    pub fn with_char(ch: char, attr: CellAttr) -> Self {
        Self { ch, attr }
    }
}
