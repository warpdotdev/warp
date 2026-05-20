use warp_core::safe_info;
use warpui::{keymap::Keystroke, Entity, ModelContext, ModelHandle, ViewContext};

use crate::register::{valid_register_name, BLACK_HOLE_REGISTER};

/// ASCII code for backspace.
/// In Normal and Visual modes, Vim treats backspace as a leftward character motion.
const BACKSPACE_CHAR: char = '\u{8}';

/// Represents the Vim modes we currently support. Insert mode is the default instead of normal
/// mode to match the behavior of Bash and Zsh's vi mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VimMode {
    Normal,
    #[default]
    Insert,
    Visual(MotionType),
    Replace,
}

impl VimMode {
    /// Convenience method for constructing a [`ModeTransition`] at a specific insert position.
    /// Note that a non-default [`InsertPosition`] is only applicable for [`VimMode::Insert`].
    fn transition_with_motion(&self, position: InsertPosition) -> ModeTransition {
        ModeTransition {
            mode: *self,
            position,
        }
    }
}

/// A finite-state automaton for interpreting the meaning of Vim key sequences. It takes
/// individual keystrokes, performs necessary state transitions for each one, and emits a VimEvent
/// every time the corresponding editor View needs to change in turn.
#[derive(Default, Debug)]
pub struct VimFSA {
    pub mode: VimMode,
    /// The characters entered which have yet to form a completed command.
    /// See ":help showcmd" in Vim.
    showcmd: String,
    pending_action: Option<PendingAction>,
    pending_action_count: Option<String>,
    pending_operand_count: Option<String>,
    /// When you do something like "vaw", set this field after "va" was typed.
    pending_visual_object: Option<TextObjectInclusion>,
    /// This remembers the last `f`, `F`, `t`, `T` command so that it can be repeated with `;` or
    /// `,`.
    last_find_motion: Option<FindCharMotion>,
    /// Vim has char-named registers which act as separate clipboards. These can be specified on a
    /// per-command basis using the `"` command, see ':help "' in Vim.
    register: char,
    /// Holds the last [`VimEvent`] where [`VimEventType::for_dot_repeat`] returns `Some`.
    dot_repeat_event: Option<VimEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Backward,
    Forward,
}

impl Direction {
    pub fn opposite(&self) -> Self {
        match self {
            Self::Backward => Self::Forward,
            Self::Forward => Self::Backward,
        }
    }
}

/// For WordMotions, indicates whether we're moving between the word beginnings or endings.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum WordBound {
    Start,
    End,
}

/// This enum distinguishes between `w`/`W`, `b`/`B`, etc.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#word
/// or enter ":help word" in Vim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WordType {
    /// More traditional definition of words, as used by `w`, `b`, etc. which break on punctuation.
    Default,
    /// Words which include any punctuation, as used by `W`, `B`, etc. The Vim docs refer to these
    /// as "WORD" in all caps. Enter ":help WORD" in Vim.
    ///
    /// Their source code uses the term "bigword" instead. See here:
    /// https://github.com/neovim/neovim/blob/0fd8eb8aa/src/nvim/textobject.c#L486
    BigWord,
}

/// In general, word motions have upper/lowercase pairs, e.g. `w` and `W` which perform analogous
/// motions on different definitions of "word". The lowercase versions match what most other
/// programs consider a "word", and so this is named "Default". The uppercase ones redefine
/// word-breaking symbols as being word-characters, and is referred to in the Vim docs as a "WORD",
/// but the Vim source code calls them "bigwords".
impl From<char> for WordType {
    fn from(c: char) -> Self {
        if c.is_ascii_uppercase() {
            Self::BigWord
        } else {
            Self::Default
        }
    }
}

/// Word-based motions of the cursor, e.g. for the "w" and "b" commands.
#[derive(Clone, Debug)]
pub struct WordMotion {
    pub direction: Direction,
    pub bound: WordBound,
    pub word_type: WordType,
}

impl WordMotion {
    pub fn new(direction: Direction, bound: WordBound, word_type: WordType) -> Self {
        Self {
            direction,
            bound,
            word_type,
        }
    }
}

/// Line-based motions of the cursor, e.g. for the "$" and "0" commands.
/// Note that not all line-based operations have a direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineMotion {
    Start,
    FirstNonWhitespace,
    End,
}

/// Character-based motions of the cursor
/// such as "h", "j", "k", "l" and arrow keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterMotion {
    Up,
    Down,
    Left,
    Right,

    /// WrappingLeft wraps around line boundaries,
    /// whereas Left stops at line boundaries.
    WrappingLeft,

    /// WrappingRight wraps around line boundaries,
    /// whereas Right stops at line boundaries.
    WrappingRight,
}

/// Motions "+", "-", "_"
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirstNonWhitespaceMotion {
    Up,
    Down,
    DownMinusOne,
}

/// Motions for "f", "F", "t", and "T".
#[derive(Clone, Debug)]
pub struct FindCharMotion {
    pub direction: Direction,
    pub destination: FindCharDestination,
    pub is_repetition: bool,
    pub c: char,
}

impl FindCharMotion {
    fn with_opposite_direction(mut self) -> Self {
        self.direction = self.direction.opposite();
        self
    }

    /// Sets `initial_search` to false.
    fn repetition(mut self) -> Self {
        self.is_repetition = true;
        self
    }
}

/// A pending operand is an as-yet incomplete operand, which is whatever comes after an operator.
/// An example would be when `di` is entered and we're waiting for the last character, e.g. `diw`.
/// This is similar to the [`PendingAction`] enum, but these must be different enums because the
/// possible characters for pending operands is a subset of the pending actions.
#[derive(Clone, Debug)]
enum PendingOperand {
    /// Can only be ge, gg in this context
    G,
    TextObject(TextObjectInclusion),
    FindChar {
        direction: Direction,
        destination: FindCharDestination,
    },
    SquareBracket(Direction),
}

impl From<char> for PendingOperand {
    fn from(c: char) -> Self {
        match c {
            'g' => Self::G,
            'i' => Self::TextObject(TextObjectInclusion::Inner),
            'a' => Self::TextObject(TextObjectInclusion::Around),
            'f' => Self::FindChar {
                direction: Direction::Forward,
                destination: FindCharDestination::AtChar,
            },
            't' => Self::FindChar {
                direction: Direction::Forward,
                destination: FindCharDestination::BeforeChar,
            },
            'F' => Self::FindChar {
                direction: Direction::Backward,
                destination: FindCharDestination::AtChar,
            },
            'T' => Self::FindChar {
                direction: Direction::Backward,
                destination: FindCharDestination::BeforeChar,
            },
            '[' => Self::SquareBracket(Direction::Backward),
            ']' => Self::SquareBracket(Direction::Forward),
            _ => panic!("invalid char for PendingOperand: {c}"),
        }
    }
}

/// This distinguishes between `f` vs `t`, as well as `F` vs `T`.
#[derive(Clone, Copy, Debug)]
pub enum FindCharDestination {
    /// When the match is found, move the cursor _on_ that matched char.
    AtChar,
    /// When the match is found, move the cursor _before_ that matched char. Note that "before"
    /// when moving left means "one char to the _right_ of the match".
    BeforeChar,
}

/// An enum for vim actions.
/// For example: delete, change
#[derive(Clone, Debug)]
enum PendingAction {
    Operation {
        operator: VimOperator,
        pending_operand: Option<PendingOperand>,
    },
    /// the full "g" command
    G,
    /// the full "z" command (e.g. zz)
    Z,
    FindChar {
        direction: Direction,
        destination: FindCharDestination,
    },
    SquareBracket(Direction),
    SetRegister,
}

impl From<char> for PendingAction {
    fn from(c: char) -> Self {
        match c {
            'd' => Self::Operation {
                operator: VimOperator::Delete,
                pending_operand: None,
            },
            'c' => Self::Operation {
                operator: VimOperator::Change,
                pending_operand: None,
            },
            'y' => Self::Operation {
                operator: VimOperator::Yank,
                pending_operand: None,
            },
            'g' => Self::G,
            'z' => Self::Z,
            'f' => Self::FindChar {
                direction: Direction::Forward,
                destination: FindCharDestination::AtChar,
            },
            't' => Self::FindChar {
                direction: Direction::Forward,
                destination: FindCharDestination::BeforeChar,
            },
            'F' => Self::FindChar {
                direction: Direction::Backward,
                destination: FindCharDestination::AtChar,
            },
            'T' => Self::FindChar {
                direction: Direction::Backward,
                destination: FindCharDestination::BeforeChar,
            },
            '[' => Self::SquareBracket(Direction::Backward),
            ']' => Self::SquareBracket(Direction::Forward),
            '"' => Self::SetRegister,
            _ => panic!("Invalid char for PendingAction: {c}"),
        }
    }
}

/// An enum for all vim motions.
#[derive(Clone, Debug)]
pub enum VimMotion {
    Character(CharacterMotion),
    Word(WordMotion),
    Line(LineMotion),
    FirstNonWhitespace(FirstNonWhitespaceMotion),
    /// See ":help f" in Vim.
    FindChar(FindCharMotion),
    JumpToFirstLine,
    JumpToLastLine,
    /// Jump to a specific line number. See ":help G" in Vim.
    JumpToLine(u32),
    /// See ":help %" in Vim.
    JumpToMatchingBracket,
    /// See ":help [" in Vim.
    JumpToUnmatchedBracket(BracketChar),
    /// See ":help {" in Vim.
    Paragraph(Direction),
}

/// This enum mirrors Vim's "MotionType", see:
/// https://github.com/neovim/neovim/blob/0fd8eb8/src/nvim/normal.h#L19-L24
/// See ":help linewise" in Vim.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MotionType {
    Charwise,
    Linewise,
}

/// From the Vim docs on text objects:
/// > The commands that start with "a" select "a"n object including white space, the commands
/// > starting with "i" select an "inner" object without white space, or just the white space. Thus
/// > the "inner" commands always select less text than the "a" commands."
/// > See https://vimdoc.sourceforge.net/htmldoc/motion.html#text-objects
/// > or enter ":help text-objects" in Vim.
#[derive(Clone, Copy, Debug)]
pub enum TextObjectInclusion {
    /// For text objects like "a word" or "a block", etc.
    Around,
    /// For text objects like "inner word" or "inner block", etc.
    Inner,
}

/// Each text object is a "product" of an object type, e.g. word, sentence, block, and two states
/// which the Vim docs refer to as "a" or "inner". So, there's "a word" and "inner word".
#[derive(Clone, Debug)]
pub struct VimTextObject {
    pub inclusion: TextObjectInclusion,
    pub object_type: TextObjectType,
}

impl From<char> for TextObjectType {
    fn from(c: char) -> Self {
        match c {
            'w' | 'W' => TextObjectType::Word(WordType::from(c)),
            'p' => TextObjectType::Paragraph,
            '\'' | '"' | '`' => TextObjectType::Quote(QuoteType::from(c)),
            'b' | 'B' | '(' | ')' | '[' | ']' | '{' | '}' => {
                TextObjectType::Block(BracketType::from(c))
            }
            _ => panic!("Invalid char for TextObjectType: {c}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuoteType {
    Single,
    Double,
    Backtick,
}

impl QuoteType {
    pub fn is_char(&self, c: char) -> bool {
        match self {
            Self::Single => c == '\'',
            Self::Double => c == '"',
            Self::Backtick => c == '`',
        }
    }
}

impl From<char> for QuoteType {
    fn from(c: char) -> Self {
        match c {
            '\'' => Self::Single,
            '"' => Self::Double,
            '`' => Self::Backtick,
            _ => panic!("invalid char for QuoteType: {c}"),
        }
    }
}

/// Whether a bracket is opening "{" or closing "}".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BracketEnd {
    Opening,
    Closing,
}

/// What kind of bracket, either "(", "[", or "{".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BracketType {
    Parenthesis,
    CurlyBrace,
    SquareBracket,
}

impl From<char> for BracketType {
    fn from(c: char) -> Self {
        match c {
            '(' | ')' | 'b' => Self::Parenthesis,
            '[' | ']' => Self::SquareBracket,
            '{' | '}' | 'B' => Self::CurlyBrace,
            _ => panic!("Invalid char for BracketType: {c}"),
        }
    }
}

/// Struct to fully represent a bracket character.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BracketChar {
    pub(crate) end: BracketEnd,
    pub(crate) kind: BracketType,
}

impl BracketChar {
    /// Check if the raw character passed in is the same as self.
    pub fn is_char(&self, c: char) -> bool {
        match Self::try_from(c) {
            Ok(other) => *self == other,
            Err(_) => false,
        }
    }

    /// Check if the raw character passed in complements self to form a pair, e.g. "(" and ")".
    pub fn complements(&self, other: char) -> bool {
        match Self::try_from(other) {
            Ok(other) => self.kind == other.kind && self.end != other.end,
            Err(_) => false,
        }
    }
}

impl TryFrom<char> for BracketChar {
    type Error = ();

    fn try_from(c: char) -> Result<Self, Self::Error> {
        Ok(match c {
            '(' => Self {
                end: BracketEnd::Opening,
                kind: BracketType::Parenthesis,
            },
            ')' => Self {
                end: BracketEnd::Closing,
                kind: BracketType::Parenthesis,
            },
            '[' => Self {
                end: BracketEnd::Opening,
                kind: BracketType::SquareBracket,
            },
            ']' => Self {
                end: BracketEnd::Closing,
                kind: BracketType::SquareBracket,
            },
            '{' => Self {
                end: BracketEnd::Opening,
                kind: BracketType::CurlyBrace,
            },
            '}' => Self {
                end: BracketEnd::Closing,
                kind: BracketType::CurlyBrace,
            },
            _ => return Err(()),
        })
    }
}

#[derive(Clone, Debug)]
pub enum TextObjectType {
    /// Enter ":help aw" in Vim.
    Word(WordType),
    /// Enter ":help ap" in Vim.
    Paragraph,
    /// Enter ":help aquote" in Vim.
    Quote(QuoteType),
    /// Enter ":help a{" in Vim.
    Block(BracketType),
}

/// This enumerates all the positions relative to the cursor where insert mode can begin.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum InsertPosition {
    #[default]
    AtCursor,
    AfterCursor,
    LineFirstNonWhitespace,
    LineEnd,
    LineAbove,
    LineBelow,
}

/// When changing modes, and the new mode is insert mode, it isn't enough to specify that the new
/// mode is insert mode. It also needs an insert position to distinguish `i` from `a`, `I`, `A`,
/// `o`, and `O`. Therefore, this struct wraps the new mode plus the new position. Note that
/// `position` is only ever non-default for insert mode.
#[derive(Clone, Debug)]
pub struct ModeTransition {
    pub mode: VimMode,
    pub position: InsertPosition,
}

impl From<VimMode> for ModeTransition {
    fn from(mode: VimMode) -> Self {
        ModeTransition {
            mode,
            position: Default::default(),
        }
    }
}

/// Representation of a single, atomic Vim operation.
#[derive(Clone, Debug)]
pub struct VimEvent {
    event_type: VimEventType,
    count: u32,
}

impl From<VimEventType> for VimEvent {
    fn from(event_type: VimEventType) -> Self {
        VimEvent {
            event_type,
            count: 1,
        }
    }
}

/// An enumeration of all Vim operations to be done on Views.
#[derive(Clone, Debug)]
pub enum VimEventType {
    InsertChar(char),
    Navigate(VimMotion),
    ReplaceChar(Option<char>),
    ToggleCase,
    Search(Direction),
    CycleSearch(Direction),
    SearchWordAtCursor(Direction),
    KeywordPrg,
    ExCommand,
    Paste {
        direction: Direction,
        register_name: char,
    },
    JoinLine,

    ChangeMode {
        new: ModeTransition,
        old: VimMode,
    },
    Undo,
    Backspace,
    DeleteForward,
    Escape,

    Operation {
        operator: VimOperator,
        operand: VimOperand,
        register_name: char,
        /// This field is kept empty for all operators except [`VimOperator::Change`] when it is
        /// dot-repeated.
        replacement_text: String,
    },

    InsertText {
        text: String,
        position: InsertPosition,
    },

    VisualOperator {
        operator: VimOperator,
        motion_type: MotionType,
        register_name: char,
    },
    VisualPaste {
        motion_type: MotionType,
        read_register_name: char,
        write_register_name: char,
    },
    VisualTextObject(VimTextObject),
    GotoDefinition,
    FindReferences,
    ShowHover,
    /// Center the current line vertically in the viewport. Triggered by `zz`.
    CenterCursorVertically,
    /// Move cursor and scroll viewport down by half a page. Triggered by `<C-d>`.
    ScrollHalfPageDown,
    /// Move cursor and scroll viewport up by half a page. Triggered by `<C-u>`.
    ScrollHalfPageUp,
}

impl VimEventType {
    /// This method determines which event types will respect the `.` command. If they are not
    /// repeatable, return None. Most events just repeat themselves exactly. Some map to other
    /// event types. Entering insert mode, for example, starts as [`VimEventType::ChangeMode`], but
    /// when it's repeated does a different event [`VimEventType::InsertText`] which inserts the
    /// text all at once.
    fn for_dot_repeat(&self) -> Option<Self> {
        match self {
            VimEventType::ReplaceChar(_)
            | VimEventType::ToggleCase
            | VimEventType::Paste { .. }
            | VimEventType::InsertText { .. }
            | VimEventType::JoinLine
            | VimEventType::DeleteForward => Some(self.clone()),
            VimEventType::Operation { operator, .. }
                if *operator == VimOperator::Change || *operator == VimOperator::Delete =>
            {
                Some(self.clone())
            }
            VimEventType::ChangeMode {
                new:
                    ModeTransition {
                        mode: VimMode::Insert,
                        position,
                    },
                ..
            } => Some(VimEventType::InsertText {
                text: String::new(),
                position: *position,
            }),
            VimEventType::ChangeMode {
                new:
                    ModeTransition {
                        mode: VimMode::Replace,
                        ..
                    },
                ..
            } => Some(VimEventType::ReplaceChar(None)),
            VimEventType::Operation { .. }
            | VimEventType::ChangeMode { .. }
            | VimEventType::Navigate(_)
            | VimEventType::Search(_)
            | VimEventType::CycleSearch(_)
            | VimEventType::SearchWordAtCursor(_)
            | VimEventType::KeywordPrg
            | VimEventType::ExCommand
            | VimEventType::InsertChar(_)
            | VimEventType::Undo
            | VimEventType::VisualOperator { .. }
            | VimEventType::VisualPaste { .. }
            | VimEventType::VisualTextObject(_)
            | VimEventType::Backspace
            | VimEventType::Escape
            | VimEventType::GotoDefinition
            | VimEventType::FindReferences
            | VimEventType::ShowHover
            | VimEventType::CenterCursorVertically
            | VimEventType::ScrollHalfPageDown
            | VimEventType::ScrollHalfPageUp => None,
        }
    }
}

/// "Operators" are `d`, `c`, `y`, etc.
/// See https://vimdoc.sourceforge.net/htmldoc/motion.html#operator
/// or enter ":help operator" in Vim.
/// They must have an operand, see [`VimOperand`], to become a complete operation, see
/// [`VimEventType::Operation`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VimOperator {
    Delete,
    Change,
    Yank,
    ToggleCase,
    Uppercase,
    Lowercase,
    ToggleComment,
}

impl From<char> for VimOperator {
    fn from(c: char) -> Self {
        match c {
            'd' | 'D' | 'x' | 'X' => Self::Delete,
            'c' | 'C' | 's' | 'S' => Self::Change,
            'y' | 'Y' => Self::Yank,
            '~' => Self::ToggleCase,
            'u' => Self::Lowercase,
            'U' => Self::Uppercase,
            _ => panic!("invalid char for VimOperator: {c}"),
        }
    }
}

/// Whatever can come after an operator, see [`VimOperator`].
#[derive(Clone, Debug)]
pub enum VimOperand {
    Motion {
        motion_type: MotionType,
        motion: VimMotion,
    },
    TextObject(VimTextObject),
    Line,
}

impl VimFSA {
    pub fn new() -> Self {
        Self {
            mode: VimMode::default(),
            showcmd: String::new(),
            pending_action: None,
            pending_action_count: None,
            pending_operand_count: None,
            pending_visual_object: None,
            last_find_motion: None,
            // When doing an operation that reads/writes to a register, the default (unnamed) register
            // is called ".
            register: '"',
            dot_repeat_event: None,
        }
    }

    pub fn clear(&mut self) {
        self.showcmd = String::new();
        self.pending_action = None;
        self.pending_action_count = None;
        self.pending_operand_count = None;
        self.pending_visual_object = None;
        self.register = '"';
    }

    pub fn interrupt(&mut self) {
        self.clear();
        match self.mode {
            VimMode::Replace | VimMode::Visual(_) => {
                self.mode = VimMode::Normal;
            }
            VimMode::Insert | VimMode::Normal => {}
        }
    }

    pub fn state(&self) -> VimState<'_> {
        VimState {
            mode: self.mode,
            showcmd: &self.showcmd,
        }
    }

    /// For processing Vim keypresses that are represented by a single char.
    fn typed_character(&mut self, c: char) -> Option<VimEvent> {
        self.showcmd.push(c);
        let event_type = match self.mode {
            VimMode::Insert => self.handle_insert_char(c),
            VimMode::Normal => match self.pending_action.clone() {
                Some(pending_action) => self.handle_normal_pending_action(c, pending_action)?,
                None => self.handle_normal_nothing_pending(c)?,
            },
            VimMode::Visual(motion_type) => self.handle_visual_command(c, motion_type)?,
            VimMode::Replace => {
                self.mode = VimMode::Normal;
                VimEventType::ReplaceChar(Some(c))
            }
        };
        let count = self.compute_event_count(c, &event_type);

        self.clear();

        self.dot_repeat_event = event_type
            .for_dot_repeat()
            .map(|event_type| VimEvent { event_type, count })
            .or(self.dot_repeat_event.take());
        Some(VimEvent { event_type, count })
    }

    /// Like Self::typed_character, but for keypresses that aren't representable by a single char.
    fn keypress(&mut self, keystroke: &str) -> Option<VimEvent> {
        let event = match keystroke {
            "escape" => match self.mode {
                VimMode::Normal => {
                    self.clear();
                    VimEventType::Escape.into()
                }
                VimMode::Replace | VimMode::Visual(_) => {
                    self.change_mode(VimMode::Normal.into()).into()
                }
                VimMode::Insert => self.exit_insert_mode(),
            },
            "backspace" => match self.mode {
                VimMode::Insert => self.handle_insert_mode_backspace().into(),
                VimMode::Visual(_) | VimMode::Normal => {
                    return self.typed_character(BACKSPACE_CHAR)
                }
                VimMode::Replace => self.change_mode(VimMode::Normal.into()).into(),
            },
            "enter" | "shift-enter" | "numpadenter" => match self.mode {
                VimMode::Insert => VimEventType::InsertChar('\n').into(),
                VimMode::Normal | VimMode::Visual(_) => VimEventType::Navigate(
                    VimMotion::FirstNonWhitespace(FirstNonWhitespaceMotion::Down),
                )
                .into(),
                VimMode::Replace => self.change_mode(VimMode::Normal.into()).into(),
            },
            "tab" | "shift-tab" => match self.mode {
                VimMode::Insert => VimEventType::InsertChar('\t').into(),
                _ => return None,
            },
            "delete" => match self.mode {
                VimMode::Insert => self.handle_insert_mode_delete().into(),
                VimMode::Normal | VimMode::Visual(_) => self.typed_character('x')?,
                VimMode::Replace => self.change_mode(VimMode::Normal.into()).into(),
            },
            "ctrl-d" => match self.mode {
                VimMode::Normal | VimMode::Visual(_) => {
                    let count = self.get_action_count().unwrap_or(1);
                    self.clear();
                    VimEvent {
                        event_type: VimEventType::ScrollHalfPageDown,
                        count,
                    }
                }
                _ => return None,
            },
            "ctrl-u" => match self.mode {
                VimMode::Normal | VimMode::Visual(_) => {
                    let count = self.get_action_count().unwrap_or(1);
                    self.clear();
                    VimEvent {
                        event_type: VimEventType::ScrollHalfPageUp,
                        count,
                    }
                }
                _ => return None,
            },
            _ => return None,
        };
        Some(event)
    }

    /// We need to consider counts in two positions, e.g. `2d3w`. The `2` and `3` are called
    /// "action count" and "operand count" respectively. The final count will be ther product of
    /// the two counts.
    fn compute_event_count(&self, c: char, event_type: &VimEventType) -> u32 {
        let this_action_count = self.get_action_count();
        match (c, event_type) {
            // Dot-repeat is a special case where a None count takes the count of the repeated
            // event, and specifying a count overrides it. Note that this means `1.` and `.` are
            // not the same!
            //
            // Replace mode is another special case where we remember the count
            // that was entered when switching into replace mode.
            ('.', _) | (_, VimEventType::ReplaceChar(_)) => {
                this_action_count.unwrap_or_else(|| {
                    self.dot_repeat_event
                        .as_ref()
                        .map(|event| event.count)
                        .unwrap_or(1)
                })
            }
            _ => this_action_count.unwrap_or(1) * self.get_operand_count().unwrap_or(1),
        }
    }

    /// Exiting insert mode is simple when it had been entered without a count, you just change the
    /// mode. However, if there was a count, we need to repeat the text that was entered n - 1
    /// times.
    fn exit_insert_mode(&mut self) -> VimEvent {
        // First, check if insert mode had been entered with a count > 1, and extract the necessary
        // data. All that data is stored in `[Self::dot_repeat_event]`.
        let number_repeated_text_to_insert = self
            .dot_repeat_event
            .as_ref()
            .filter(|event| event.count > 1)
            .and_then(|event| match &event.event_type {
                VimEventType::InsertText { text, position } => {
                    let position = match position {
                        InsertPosition::LineAbove | InsertPosition::LineBelow => *position,
                        _ => InsertPosition::AtCursor,
                    };
                    Some((text.to_owned(), position, event.count - 1))
                }
                _ => None,
            });
        match number_repeated_text_to_insert {
            Some((text, position, count)) => {
                self.mode = VimMode::Normal;
                VimEvent {
                    event_type: VimEventType::InsertText { text, position },
                    count,
                }
            }
            None => self.change_mode(VimMode::Normal.into()).into(),
        }
    }

    /// [`Self::typed_character`] dispatches to this method if [`Self::pending_action`] is None and
    /// we're in [`VimMode::Normal`].
    fn handle_normal_nothing_pending(&mut self, c: char) -> Option<VimEventType> {
        let event =
            match c {
                'h' | 'j' | 'k' | 'l' | ' ' | BACKSPACE_CHAR => {
                    VimEventType::Navigate(VimMotion::Character(char_motion_for_character(c)))
                }
                'w' | 'W' | 'b' | 'B' | 'e' | 'E' => {
                    VimEventType::Navigate(VimMotion::Word(word_motion_for_character(c)))
                }
                '^' | '$' => VimEventType::Navigate(VimMotion::Line(line_motion_for_character(c))),
                '+' | '-' | '_' => VimEventType::Navigate(VimMotion::FirstNonWhitespace(
                    first_nonwhitespace_motion_for_char(c),
                )),
                '0' => match self.pending_action_count {
                    None => VimEventType::Navigate(VimMotion::Line(LineMotion::Start)),
                    Some(_) => {
                        // append "0" to the digit string
                        self.pending_action_count
                            .get_or_insert_with(String::new)
                            .push(c);
                        return None;
                    }
                },
                '1'..='9' => {
                    // append digit to the digit string
                    self.pending_action_count
                        .get_or_insert_with(String::new)
                        .push(c);
                    return None;
                }
                'i' => self
                    .change_mode(VimMode::Insert.transition_with_motion(InsertPosition::AtCursor)),
                'a' => self.change_mode(
                    VimMode::Insert.transition_with_motion(InsertPosition::AfterCursor),
                ),
                'o' => self
                    .change_mode(VimMode::Insert.transition_with_motion(InsertPosition::LineBelow)),
                'O' => self
                    .change_mode(VimMode::Insert.transition_with_motion(InsertPosition::LineAbove)),
                'I' => self.change_mode(
                    VimMode::Insert.transition_with_motion(InsertPosition::LineFirstNonWhitespace),
                ),
                'A' => self
                    .change_mode(VimMode::Insert.transition_with_motion(InsertPosition::LineEnd)),
                'x' => self.create_operation(
                    VimOperator::Delete,
                    VimOperand::Motion {
                        motion: VimMotion::Character(CharacterMotion::Right),
                        motion_type: MotionType::Charwise,
                    },
                ),
                'X' => self.create_operation(
                    VimOperator::Delete,
                    VimOperand::Motion {
                        motion: VimMotion::Character(CharacterMotion::Left),
                        motion_type: MotionType::Charwise,
                    },
                ),
                'r' => self.change_mode(VimMode::Replace.into()),
                'g' | 'd' | 'c' | 'y' | 'z' | 'f' | 'F' | 't' | 'T' | '[' | ']' | '"' => {
                    self.pending_action = Some(PendingAction::from(c));
                    return None;
                }
                'D' => self.create_operation(
                    VimOperator::Delete,
                    VimOperand::Motion {
                        motion: VimMotion::Line(LineMotion::End),
                        motion_type: MotionType::Charwise,
                    },
                ),
                'C' => {
                    self.mode = VimMode::Insert;
                    self.create_operation(
                        VimOperator::Change,
                        VimOperand::Motion {
                            motion: VimMotion::Line(LineMotion::End),
                            motion_type: MotionType::Charwise,
                        },
                    )
                }
                // Note that "Y" is not a built-in command in Vim, but it is an NeoVim. NeoVim added it
                // because it is an obvious analogy to "C" and "D". It only takes 4 lines of code for
                // us to implement.
                'Y' => self.create_operation(VimOperator::Yank, VimOperand::Line),
                's' => {
                    self.mode = VimMode::Insert;
                    self.create_operation(
                        VimOperator::Change,
                        VimOperand::Motion {
                            motion: VimMotion::Character(CharacterMotion::Right),
                            motion_type: MotionType::Charwise,
                        },
                    )
                }
                'S' => {
                    self.mode = VimMode::Insert;
                    self.create_operation(VimOperator::Change, VimOperand::Line)
                }
                'p' => VimEventType::Paste {
                    direction: Direction::Forward,
                    register_name: self.register,
                },
                'P' => VimEventType::Paste {
                    direction: Direction::Backward,
                    register_name: self.register,
                },
                'J' => VimEventType::JoinLine,
                'K' => VimEventType::KeywordPrg,
                'u' => VimEventType::Undo,
                '~' => VimEventType::ToggleCase,
                '/' => VimEventType::Search(Direction::Forward),
                '?' => VimEventType::Search(Direction::Backward),
                'n' => VimEventType::CycleSearch(Direction::Forward),
                'N' => VimEventType::CycleSearch(Direction::Backward),
                '*' => VimEventType::SearchWordAtCursor(Direction::Forward),
                '#' => VimEventType::SearchWordAtCursor(Direction::Backward),
                ':' => VimEventType::ExCommand,
                'G' => match self.get_action_count() {
                    Some(line_number) => VimEventType::Navigate(VimMotion::JumpToLine(line_number)),
                    None => VimEventType::Navigate(VimMotion::JumpToLastLine),
                },
                '%' => VimEventType::Navigate(VimMotion::JumpToMatchingBracket),
                '{' => VimEventType::Navigate(VimMotion::Paragraph(Direction::Backward)),
                '}' => VimEventType::Navigate(VimMotion::Paragraph(Direction::Forward)),
                ';' => {
                    if let Some(motion) = &self.last_find_motion {
                        VimEventType::Navigate(VimMotion::FindChar(motion.clone().repetition()))
                    } else {
                        self.clear();
                        return None;
                    }
                }
                ',' => {
                    if let Some(motion) = &self.last_find_motion {
                        VimEventType::Navigate(VimMotion::FindChar(
                            motion.clone().repetition().with_opposite_direction(),
                        ))
                    } else {
                        self.clear();
                        return None;
                    }
                }
                '.' => {
                    if let Some(event) = &self.dot_repeat_event {
                        event.event_type.clone()
                    } else {
                        return None;
                    }
                }
                'v' => self.change_mode(VimMode::Visual(MotionType::Charwise).into()),
                'V' => self.change_mode(VimMode::Visual(MotionType::Linewise).into()),
                _ => {
                    self.clear();
                    return None;
                }
            };
        Some(event)
    }

    /// [`Self::typed_character`] dispatches to this method if [`Self::pending_action`] is Some and
    /// we're in [`VimMode::Normal`].
    fn handle_normal_pending_action(
        &mut self,
        c: char,
        action: PendingAction,
    ) -> Option<VimEventType> {
        let event = match action {
            PendingAction::Z => match c {
                'z' => VimEventType::CenterCursorVertically,
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::G => match c {
                'e' | 'E' => VimEventType::Navigate(VimMotion::Word(WordMotion {
                    direction: Direction::Backward,
                    bound: WordBound::End,
                    word_type: WordType::from(c),
                })),
                'g' => VimEventType::Navigate(VimMotion::JumpToFirstLine),
                'd' => VimEventType::GotoDefinition,
                'h' => VimEventType::ShowHover,
                'r' => VimEventType::FindReferences,
                // For two-character operations (g~, gu, gU, gc),
                // replace the existing pending action, the generic G,
                // with a more specific pending action.
                '~' | 'u' | 'U' => {
                    self.pending_action = Some(PendingAction::Operation {
                        operator: VimOperator::from(c),
                        pending_operand: None,
                    });
                    return None;
                }
                'c' => {
                    self.pending_action = Some(PendingAction::Operation {
                        operator: VimOperator::ToggleComment,
                        pending_operand: None,
                    });
                    return None;
                }
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::FindChar {
                direction,
                destination,
            } => {
                let motion = FindCharMotion {
                    direction,
                    destination,
                    is_repetition: false,
                    c,
                };
                self.last_find_motion = Some(motion.clone());
                VimEventType::Navigate(VimMotion::FindChar(motion))
            }
            PendingAction::SquareBracket(direction) => match c {
                '(' | ')' | '{' | '}' | '[' | ']' => {
                    VimEventType::Navigate(VimMotion::JumpToUnmatchedBracket(BracketChar {
                        end: match direction {
                            Direction::Backward => BracketEnd::Closing,
                            Direction::Forward => BracketEnd::Opening,
                        },
                        kind: BracketType::from(c),
                    }))
                }
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::SetRegister => {
                // Accept this register name only if it's valid.
                if valid_register_name(c) {
                    self.register = c;
                }
                self.pending_action = None;
                return None;
            }
            PendingAction::Operation {
                operator,
                pending_operand,
            } => {
                let event = match pending_operand {
                    Some(operand) => self.handle_normal_pending_operand(c, operator, operand)?,
                    None => self.handle_normal_pending_operation(c, operator)?,
                };
                if operator == VimOperator::Change {
                    self.mode = VimMode::Insert;
                }
                event
            }
        };

        Some(event)
    }

    fn handle_normal_pending_operation(
        &mut self,
        c: char,
        operator: VimOperator,
    ) -> Option<VimEventType> {
        let event_type = match c {
            'd' if operator == VimOperator::Delete => {
                self.create_operation(operator, VimOperand::Line)
            }
            'c' if operator == VimOperator::Change => {
                self.create_operation(operator, VimOperand::Line)
            }
            'y' if operator == VimOperator::Yank => {
                self.create_operation(operator, VimOperand::Line)
            }
            // Support gcc (toggle comment line)
            'c' if operator == VimOperator::ToggleComment => {
                self.create_operation(operator, VimOperand::Line)
            }
            'i' | 'a' | 'g' | 'f' | 'F' | 't' | 'T' | '[' | ']' => {
                self.pending_action = Some(PendingAction::Operation {
                    operator,
                    pending_operand: Some(PendingOperand::from(c)),
                });
                return None;
            }
            'h' | 'l' | ' ' | BACKSPACE_CHAR => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Character(char_motion_for_character(c)),
                    motion_type: MotionType::Charwise,
                },
            ),
            'j' | 'k' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Character(char_motion_for_character(c)),
                    motion_type: MotionType::Linewise,
                },
            ),
            'w' | 'W' | 'b' | 'B' | 'e' | 'E' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Word(match operator {
                        VimOperator::Change => closed_word_motion_for_character(c),
                        _ => word_motion_for_character(c),
                    }),
                    motion_type: MotionType::Charwise,
                },
            ),
            '^' | '$' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Line(line_motion_for_character(c)),
                    motion_type: MotionType::Charwise,
                },
            ),
            '+' | '-' | '_' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::FirstNonWhitespace(first_nonwhitespace_motion_for_char(c)),
                    motion_type: MotionType::Linewise,
                },
            ),
            'G' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: match self.get_operand_count() {
                        Some(line_number) => VimMotion::JumpToLine(line_number),
                        None => VimMotion::JumpToLastLine,
                    },
                    motion_type: MotionType::Linewise,
                },
            ),
            '{' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Paragraph(Direction::Backward),
                    motion_type: MotionType::Linewise,
                },
            ),
            '}' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::Paragraph(Direction::Forward),
                    motion_type: MotionType::Linewise,
                },
            ),
            '0' => match self.pending_operand_count {
                None => self.create_operation(
                    operator,
                    VimOperand::Motion {
                        motion: VimMotion::Line(LineMotion::Start),
                        motion_type: MotionType::Charwise,
                    },
                ),
                Some(_) => {
                    // append "0" to the digit string
                    self.pending_operand_count
                        .get_or_insert_with(String::new)
                        .push(c);
                    return None;
                }
            },
            '1'..='9' => {
                // append digit to the digit string
                self.pending_operand_count
                    .get_or_insert_with(String::new)
                    .push(c);
                return None;
            }
            '%' => self.create_operation(
                operator,
                VimOperand::Motion {
                    motion: VimMotion::JumpToMatchingBracket,
                    motion_type: MotionType::Charwise,
                },
            ),
            ';' => {
                if let Some(motion) = &self.last_find_motion {
                    self.create_operation(
                        operator,
                        VimOperand::Motion {
                            motion: VimMotion::FindChar(motion.clone().repetition()),
                            motion_type: MotionType::Charwise,
                        },
                    )
                } else {
                    self.clear();
                    return None;
                }
            }
            ',' => {
                if let Some(motion) = &self.last_find_motion {
                    self.create_operation(
                        operator,
                        VimOperand::Motion {
                            motion: VimMotion::FindChar(
                                motion.clone().repetition().with_opposite_direction(),
                            ),
                            motion_type: MotionType::Charwise,
                        },
                    )
                } else {
                    self.clear();
                    return None;
                }
            }
            _ => {
                self.clear();
                return None;
            }
        };
        Some(event_type)
    }

    fn handle_normal_pending_operand(
        &mut self,
        c: char,
        operator: VimOperator,
        operand: PendingOperand,
    ) -> Option<VimEventType> {
        let event_type = match operand {
            PendingOperand::G => match c {
                'e' | 'E' => self.create_operation(
                    operator,
                    VimOperand::Motion {
                        motion: VimMotion::Word(WordMotion {
                            direction: Direction::Backward,
                            bound: WordBound::End,
                            word_type: WordType::from(c),
                        }),
                        motion_type: MotionType::Charwise,
                    },
                ),
                'g' => self.create_operation(
                    operator,
                    VimOperand::Motion {
                        motion: VimMotion::JumpToFirstLine,
                        motion_type: MotionType::Linewise,
                    },
                ),
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingOperand::FindChar {
                direction,
                destination,
            } => {
                let motion = FindCharMotion {
                    direction,
                    destination,
                    is_repetition: false,
                    c,
                };
                self.last_find_motion = Some(motion.clone());
                self.create_operation(
                    operator,
                    VimOperand::Motion {
                        motion: VimMotion::FindChar(FindCharMotion {
                            direction,
                            destination,
                            is_repetition: false,
                            c,
                        }),
                        motion_type: MotionType::Charwise,
                    },
                )
            }
            PendingOperand::TextObject(inclusion) => match c {
                'w' | 'W' | 'p' | '\'' | '"' | '`' | 'b' | 'B' | '(' | ')' | '[' | ']' | '{'
                | '}' => self.create_operation(
                    operator,
                    VimOperand::TextObject(VimTextObject {
                        inclusion,
                        object_type: TextObjectType::from(c),
                    }),
                ),
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingOperand::SquareBracket(direction) => match c {
                '(' | ')' | '{' | '}' | '[' | ']' => self.create_operation(
                    operator,
                    VimOperand::Motion {
                        motion: VimMotion::JumpToUnmatchedBracket(BracketChar {
                            end: match direction {
                                Direction::Backward => BracketEnd::Closing,
                                Direction::Forward => BracketEnd::Opening,
                            },
                            kind: BracketType::from(c),
                        }),
                        motion_type: MotionType::Charwise,
                    },
                ),
                _ => {
                    self.clear();
                    return None;
                }
            },
        };
        Some(event_type)
    }

    /// [`Self::typed_character`] dispatches to this method if we're in [`VimMode::Visual`].
    fn handle_visual_command(&mut self, c: char, motion_type: MotionType) -> Option<VimEventType> {
        let event_type = match self.pending_action.clone() {
            Some(pending_action) => self.handle_visual_pending_action(c, pending_action)?,
            None => match self.pending_visual_object {
                Some(inclusion) => match c {
                    'w' | 'W' | 'p' | '\'' | '"' | '`' | 'b' | 'B' | '(' | ')' | '[' | ']'
                    | '{' | '}' => {
                        if c == 'p' {
                            self.mode = VimMode::Visual(MotionType::Linewise);
                        }
                        VimEventType::VisualTextObject(VimTextObject {
                            inclusion,
                            object_type: TextObjectType::from(c),
                        })
                    }
                    _ => {
                        self.clear();
                        return None;
                    }
                },
                None => self.handle_visual_nothing_pending(c, motion_type)?,
            },
        };
        Some(event_type)
    }

    fn handle_visual_nothing_pending(
        &mut self,
        c: char,
        motion_type: MotionType,
    ) -> Option<VimEventType> {
        let event_type = match c {
            'h' | 'j' | 'k' | 'l' | ' ' | BACKSPACE_CHAR => {
                VimEventType::Navigate(VimMotion::Character(char_motion_for_character(c)))
            }
            'w' | 'W' | 'b' | 'B' | 'e' | 'E' => {
                VimEventType::Navigate(VimMotion::Word(word_motion_for_character(c)))
            }
            '^' | '$' => VimEventType::Navigate(VimMotion::Line(line_motion_for_character(c))),
            '+' | '-' | '_' => VimEventType::Navigate(VimMotion::FirstNonWhitespace(
                first_nonwhitespace_motion_for_char(c),
            )),
            '0' => match self.pending_action_count {
                None => VimEventType::Navigate(VimMotion::Line(LineMotion::Start)),
                Some(_) => {
                    // append "0" to the digit string
                    self.pending_action_count
                        .get_or_insert_with(String::new)
                        .push(c);
                    return None;
                }
            },
            '1'..='9' => {
                // append digit to the digit string
                self.pending_action_count
                    .get_or_insert_with(String::new)
                    .push(c);
                return None;
            }
            'G' => match self.get_action_count() {
                Some(line_number) => VimEventType::Navigate(VimMotion::JumpToLine(line_number)),
                None => VimEventType::Navigate(VimMotion::JumpToLastLine),
            },
            '%' => VimEventType::Navigate(VimMotion::JumpToMatchingBracket),
            '{' => VimEventType::Navigate(VimMotion::Paragraph(Direction::Backward)),
            '}' => VimEventType::Navigate(VimMotion::Paragraph(Direction::Forward)),
            ';' => {
                if let Some(motion) = &self.last_find_motion {
                    VimEventType::Navigate(VimMotion::FindChar(motion.clone().repetition()))
                } else {
                    self.clear();
                    return None;
                }
            }
            ',' => {
                if let Some(motion) = &self.last_find_motion {
                    VimEventType::Navigate(VimMotion::FindChar(
                        motion.clone().repetition().with_opposite_direction(),
                    ))
                } else {
                    self.clear();
                    return None;
                }
            }
            'd' | 'D' | 'y' | 'Y' | 'x' | 'X' | '~' | 'u' | 'U' => {
                let event_type = self.create_visual_operator(c, motion_type);
                self.mode = VimMode::Normal;
                event_type
            }
            'c' | 'C' | 's' | 'S' => {
                let event_type = self.create_visual_operator(c, motion_type);
                self.mode = VimMode::Insert;
                event_type
            }
            'p' | 'P' => {
                self.mode = VimMode::Normal;
                let write_register_name = if c == 'p' {
                    self.register
                } else {
                    BLACK_HOLE_REGISTER
                };
                VimEventType::VisualPaste {
                    motion_type,
                    read_register_name: self.register,
                    write_register_name,
                }
            }
            'g' | 'z' | 'f' | 'F' | 't' | 'T' | '[' | ']' | '"' => {
                self.pending_action = Some(PendingAction::from(c));
                return None;
            }
            'i' => {
                self.pending_visual_object = Some(TextObjectInclusion::Inner);
                return None;
            }
            'a' => {
                self.pending_visual_object = Some(TextObjectInclusion::Around);
                return None;
            }
            'v' => match motion_type {
                MotionType::Charwise => self.change_mode(VimMode::Normal.into()),
                MotionType::Linewise => {
                    self.change_mode(VimMode::Visual(MotionType::Charwise).into())
                }
            },
            'V' => match motion_type {
                MotionType::Linewise => self.change_mode(VimMode::Normal.into()),
                MotionType::Charwise => {
                    self.change_mode(VimMode::Visual(MotionType::Linewise).into())
                }
            },
            _ => return None,
        };
        Some(event_type)
    }

    fn handle_visual_pending_action(
        &mut self,
        c: char,
        pending_action: PendingAction,
    ) -> Option<VimEventType> {
        let event_type = match pending_action {
            PendingAction::Z => match c {
                'z' => VimEventType::CenterCursorVertically,
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::G => match c {
                'e' | 'E' => VimEventType::Navigate(VimMotion::Word(WordMotion {
                    direction: Direction::Backward,
                    bound: WordBound::End,
                    word_type: WordType::from(c),
                })),
                'g' => VimEventType::Navigate(VimMotion::JumpToFirstLine),
                'c' => {
                    let motion_type = match self.mode {
                        VimMode::Visual(mt) => mt,
                        _ => MotionType::Charwise,
                    };
                    self.mode = VimMode::Normal;
                    VimEventType::VisualOperator {
                        operator: VimOperator::ToggleComment,
                        motion_type,
                        register_name: self.register,
                    }
                }
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::FindChar {
                direction,
                destination,
            } => {
                let motion = FindCharMotion {
                    direction,
                    destination,
                    is_repetition: false,
                    c,
                };
                self.last_find_motion = Some(motion.clone());
                VimEventType::Navigate(VimMotion::FindChar(motion))
            }
            PendingAction::SquareBracket(direction) => match c {
                '(' | ')' | '{' | '}' | '[' | ']' => {
                    VimEventType::Navigate(VimMotion::JumpToUnmatchedBracket(BracketChar {
                        end: match direction {
                            Direction::Backward => BracketEnd::Closing,
                            Direction::Forward => BracketEnd::Opening,
                        },
                        kind: BracketType::from(c),
                    }))
                }
                _ => {
                    self.clear();
                    return None;
                }
            },
            PendingAction::SetRegister => {
                // Accept this register name only if it's valid.
                if valid_register_name(c) {
                    self.register = c;
                }
                self.pending_action = None;
                return None;
            }
            _ => {
                self.clear();
                return None;
            }
        };
        Some(event_type)
    }

    /// Helper function to change modes.
    /// (1) Updates `self.mode`, and
    /// (2) returns a VimEvent to be handled externally.
    fn change_mode(&mut self, mode_trans: ModeTransition) -> VimEventType {
        let old_mode = self.mode;
        self.mode = mode_trans.mode;
        VimEventType::ChangeMode {
            new: mode_trans,
            old: old_mode,
        }
    }

    /// Parse an unsigned integer from self.pending_action_count.
    fn get_action_count(&self) -> Option<u32> {
        self.pending_action_count
            .as_ref()
            .and_then(|s| s.parse().ok())
    }

    /// Parse an unsigned integer from self.pending_operand_count.
    fn get_operand_count(&self) -> Option<u32> {
        self.pending_operand_count
            .as_ref()
            .and_then(|s| s.parse().ok())
    }

    /// This is for when the view needs to initiate a switch to insert mode, e.g. when the user
    /// does an interaction, say selecting text with the mouse, which doesn't make sense in the
    /// current mode.
    fn force_insert_mode(&mut self) {
        self.clear();
        self.mode = VimMode::Insert;
    }

    fn create_operation(&self, operator: VimOperator, operand: VimOperand) -> VimEventType {
        VimEventType::Operation {
            operator,
            operand,
            register_name: self.register,
            replacement_text: String::new(),
        }
    }

    fn create_visual_operator(&self, c: char, motion_type: MotionType) -> VimEventType {
        // Capitalized operators will ignore if we had been in charwise visual mode, and become
        // linewise.
        // 'U' (for Uppercase) is a special case of a capitalized charwise operator.
        let motion_type = if c.is_ascii_uppercase() && c != 'U' {
            MotionType::Linewise
        } else {
            motion_type
        };
        VimEventType::VisualOperator {
            operator: VimOperator::from(c),
            motion_type,
            register_name: self.register,
        }
    }

    /// When in insert mode, we need to keep track of chars entered for dot-repeat. How this
    /// happens depends on the [`VimEventType`] variant we have in [`Self::dot_repeat_event`].
    fn dot_repeat_text_mut(&mut self) -> Option<&mut String> {
        match &mut self.dot_repeat_event {
            Some(VimEvent {
                event_type: VimEventType::InsertText { ref mut text, .. },
                ..
            })
            | Some(VimEvent {
                event_type:
                    VimEventType::Operation {
                        operator: VimOperator::Change,
                        replacement_text: ref mut text,
                        ..
                    },
                ..
            }) => Some(text),
            _ => None,
        }
    }

    /// Makes sure we append to the [`Self::dot_repeat_event`] accordingly.
    fn handle_insert_char(&mut self, c: char) -> VimEventType {
        if let Some(text) = self.dot_repeat_text_mut() {
            text.push(c);
        }
        VimEventType::InsertChar(c)
    }

    /// Makes sure we pop from the [`Self::dot_repeat_event`] accordingly.
    fn handle_insert_mode_backspace(&mut self) -> VimEventType {
        if let Some(text) = self.dot_repeat_text_mut() {
            text.pop();
        }
        VimEventType::Backspace
    }

    /// Handles delete forward in insert mode without contaminating registers.
    fn handle_insert_mode_delete(&mut self) -> VimEventType {
        // Track delete-forward events for dot-repeat
        if self.dot_repeat_event.is_none() {
            self.dot_repeat_event = Some(VimEvent {
                event_type: VimEventType::DeleteForward,
                count: 1,
            });
        }
        VimEventType::DeleteForward
    }
}

/// Match typed characters with their associated word motion
fn word_motion_for_character(c: char) -> WordMotion {
    let word_type = WordType::from(c);
    match c {
        'w' => WordMotion::new(Direction::Forward, WordBound::Start, word_type),
        'W' => WordMotion::new(Direction::Forward, WordBound::Start, word_type),
        'b' => WordMotion::new(Direction::Backward, WordBound::Start, word_type),
        'B' => WordMotion::new(Direction::Backward, WordBound::Start, word_type),
        'e' => WordMotion::new(Direction::Forward, WordBound::End, word_type),
        'E' => WordMotion::new(Direction::Forward, WordBound::End, word_type),
        _ => panic!("could not match character {c} with a word motion"),
    }
}

/// Match typed characters with a closed word motion.
/// This means forward motions (w/W and e/E) will both match to e/E.
fn closed_word_motion_for_character(c: char) -> WordMotion {
    let word_type = WordType::from(c);
    match c {
        'w' | 'e' => WordMotion::new(Direction::Forward, WordBound::End, word_type),
        'W' | 'E' => WordMotion::new(Direction::Forward, WordBound::End, word_type),
        'b' => WordMotion::new(Direction::Backward, WordBound::Start, word_type),
        'B' => WordMotion::new(Direction::Backward, WordBound::Start, word_type),
        _ => panic!("could not match character {c} with a closed word motion"),
    }
}

/// Match typed characters with their associated line motion
fn line_motion_for_character(c: char) -> LineMotion {
    match c {
        '0' => LineMotion::Start,
        '^' => LineMotion::FirstNonWhitespace,
        '$' => LineMotion::End,
        _ => panic!("could not match character {c} with a line motion"),
    }
}

/// Match typed characters with their associated character motion
fn char_motion_for_character(c: char) -> CharacterMotion {
    match c {
        'h' => CharacterMotion::Left,
        'l' => CharacterMotion::Right,
        'k' => CharacterMotion::Up,
        'j' => CharacterMotion::Down,
        ' ' => CharacterMotion::WrappingRight,
        BACKSPACE_CHAR => CharacterMotion::WrappingLeft,
        _ => panic!("could not match character {c} with a single-character motion"),
    }
}

fn first_nonwhitespace_motion_for_char(c: char) -> FirstNonWhitespaceMotion {
    match c {
        '+' => FirstNonWhitespaceMotion::Down,
        '-' => FirstNonWhitespaceMotion::Up,
        '_' => FirstNonWhitespaceMotion::DownMinusOne,
        _ => panic!("could not match character {c} with a first nonwhitespace motion"),
    }
}

/// A single struct encapsulating the publicly accessible state from the VimFSA.
pub struct VimState<'a> {
    pub mode: VimMode,
    pub showcmd: &'a str,
}

/// This struct is a wrapper around the VimFSA that turns it into a warpui::Entity. We want to keep
/// the VimFSA independent of our UI framework, so anything involving warpui should live here
/// instead.
#[derive(Default)]
pub struct VimModel {
    fsa: VimFSA,
}

impl VimModel {
    pub fn new() -> Self {
        Self { fsa: VimFSA::new() }
    }

    pub fn state(&self) -> VimState<'_> {
        self.fsa.state()
    }

    pub fn typed_character(&mut self, c: char, ctx: &mut ModelContext<Self>) {
        if let Some(event) = self.fsa.typed_character(c) {
            ctx.emit(event);
        }
    }

    pub fn keypress(&mut self, keystroke: &Keystroke, ctx: &mut ModelContext<Self>) {
        if let Some(event) = self.fsa.keypress(keystroke.normalized().as_str()) {
            ctx.emit(event);
        }
    }

    pub fn force_insert_mode(&mut self, ctx: &mut ModelContext<Self>) {
        self.fsa.force_insert_mode();
        ctx.notify();
    }

    pub fn interrupt(&mut self, ctx: &mut ModelContext<Self>) {
        self.fsa.interrupt();
        ctx.notify();
    }
}

impl Entity for VimModel {
    type Event = VimEvent;
}

pub trait VimSubscriber {
    fn handle_vim_event(
        &mut self,
        _handle: ModelHandle<VimModel>,
        event: &VimEvent,
        ctx: &mut ViewContext<Self>,
    );
}

impl<T> VimSubscriber for T
where
    T: VimHandler,
{
    fn handle_vim_event(
        &mut self,
        _handle: ModelHandle<VimModel>,
        event: &VimEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        safe_info!(
            safe: ("Handling vim event, count {:?}", event.count),
            full: ("Handling vim event type {:?}, count {:?}", event.event_type, event.count)
        );
        match &event.event_type {
            VimEventType::InsertChar(c) => self.insert_char(*c, ctx),
            VimEventType::Navigate(motion) => match motion {
                VimMotion::Character(motion) => self.navigate_char(event.count, motion, ctx),
                VimMotion::Word(motion) => self.navigate_word(event.count, motion, ctx),
                VimMotion::Line(motion) => self.navigate_line(event.count, motion, ctx),
                VimMotion::FirstNonWhitespace(motion) => {
                    self.first_nonwhitespace_motion(event.count, motion, ctx)
                }
                VimMotion::FindChar(motion) => self.find_char(event.count, motion, ctx),
                VimMotion::Paragraph(direction) => {
                    self.navigate_paragraph(event.count, direction, ctx)
                }
                VimMotion::JumpToFirstLine => self.jump_to_first_line(ctx),
                VimMotion::JumpToLastLine => self.jump_to_last_line(ctx),
                VimMotion::JumpToLine(line_number) => self.jump_to_line(*line_number, ctx),
                VimMotion::JumpToMatchingBracket => self.jump_to_matching_bracket(ctx),
                VimMotion::JumpToUnmatchedBracket(bracket) => {
                    self.jump_to_unmatched_bracket(bracket, ctx)
                }
            },
            VimEventType::Operation {
                operator,
                operand,
                register_name,
                replacement_text,
            } => self.operation(
                operator,
                event.count,
                operand,
                *register_name,
                replacement_text.as_str(),
                ctx,
            ),
            VimEventType::ReplaceChar(Some(c)) => self.replace_char(*c, event.count, ctx),
            VimEventType::ReplaceChar(_) => {}
            VimEventType::Paste {
                direction,
                register_name,
            } => self.paste(event.count, direction, *register_name, ctx),
            VimEventType::InsertText { text, position } => {
                self.insert_text(text.as_str(), position, event.count, ctx)
            }
            VimEventType::JoinLine => self.join_line(event.count, ctx),
            VimEventType::Undo => self.undo(ctx),
            VimEventType::ToggleCase => self.toggle_case(event.count, ctx),
            VimEventType::Search(direction) => self.search(direction, ctx),
            VimEventType::CycleSearch(direction) => self.cycle_search(direction, ctx),
            VimEventType::SearchWordAtCursor(direction) => {
                self.search_word_at_cursor(direction, ctx)
            }
            VimEventType::KeywordPrg => self.keyword_prg(ctx),
            VimEventType::ExCommand => self.ex_command(ctx),
            VimEventType::VisualOperator {
                operator,
                motion_type,
                register_name,
            } => self.visual_operator(operator, *motion_type, *register_name, ctx),
            VimEventType::VisualPaste {
                motion_type,
                read_register_name,
                write_register_name,
            } => self.visual_paste(*motion_type, *read_register_name, *write_register_name, ctx),
            VimEventType::VisualTextObject(text_object) => {
                self.visual_text_object(text_object, ctx)
            }
            // Escape is idempotent
            VimEventType::Escape => self.escape(ctx),
            VimEventType::ChangeMode { new, old } => self.change_mode(old, new, ctx),
            VimEventType::Backspace => self.backspace(ctx),
            VimEventType::DeleteForward => self.delete_forward(ctx),
            VimEventType::GotoDefinition => self.goto_definition(ctx),
            VimEventType::FindReferences => self.find_references(ctx),
            VimEventType::ShowHover => self.show_hover(ctx),
            VimEventType::CenterCursorVertically => self.center_cursor_vertically(ctx),
            VimEventType::ScrollHalfPageDown => self.scroll_half_page_down(event.count, ctx),
            VimEventType::ScrollHalfPageUp => self.scroll_half_page_up(event.count, ctx),
        };
    }
}

/// To be implemented by Views that support Vim keybindings.
pub trait VimHandler {
    /// A character to be inserted to the buffer.
    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>);
    /// A one-character motion of the cursor.
    fn navigate_char(
        &mut self,
        count: u32,
        character_motion: &CharacterMotion,
        ctx: &mut ViewContext<Self>,
    );
    /// Word-related motion of the cursor.
    fn navigate_word(&mut self, count: u32, word_motion: &WordMotion, ctx: &mut ViewContext<Self>);
    /// Motions within the current line: 0, ^, $
    fn navigate_line(&mut self, count: u32, line_motion: &LineMotion, ctx: &mut ViewContext<Self>);
    fn first_nonwhitespace_motion(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ViewContext<Self>,
    );
    /// Motions to a particular character on the current line.
    fn find_char(
        &mut self,
        occurrence_count: u32,
        find_char_motion: &FindCharMotion,
        ctx: &mut ViewContext<Self>,
    );
    /// Navigate by paragraph: { and }.
    fn navigate_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        ctx: &mut ViewContext<Self>,
    );
    /// For all "operator commands", e.g. d, c, y. See ":help operator" in Vim, or click here:
    /// https://vimdoc.sourceforge.net/htmldoc/motion.html#operator
    fn operation(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        register_name: char,
        replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    );
    /// Replace a character with another.
    /// If `char_count` is greater than the number of characters remaining in the current line,
    /// the replace operation is cancelled.
    fn replace_char(&mut self, c: char, char_count: u32, ctx: &mut ViewContext<Self>);
    /// Switch between upper/lowercase for character on cursor.
    /// Even if `char_count` is greater than the number of characters remaining in the current line,
    /// only characters in the current line are toggled.
    fn toggle_case(&mut self, char_count: u32, ctx: &mut ViewContext<Self>);
    /// Open the search experience for the relevant context, e.g. in a shell, that may be searching
    /// shell history, while in notebooks, that may be searching the editor buffer.
    fn search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>);
    /// Cycle through matches for the existing search query (if any). Mapped to 'n'/'N'.
    fn cycle_search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>);
    /// Initiate a search for the word nearest to the cursor. If the cursor is inside a word,
    /// search that. If it is on whitespace or punctuation, go forward to the nearest word.
    fn search_word_at_cursor(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>);
    /// Bring up the menu (command-line mode) to accept an ex-command. See ":help :" in Vim.
    fn ex_command(&mut self, ctx: &mut ViewContext<Self>);
    /// Execute `keywordprg`, i.e. Vim's `K` command. See ":help K" or ":help keywordprg"
    fn keyword_prg(&mut self, ctx: &mut ViewContext<Self>);
    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        motion_type: MotionType,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    );
    fn visual_paste(
        &mut self,
        motion_type: MotionType,
        read_register_name: char,
        write_register_name: char,
        ctx: &mut ViewContext<Self>,
    );
    fn visual_text_object(&mut self, text_object: &VimTextObject, ctx: &mut ViewContext<Self>);
    fn jump_to_first_line(&mut self, ctx: &mut ViewContext<Self>);
    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>);
    fn jump_to_line(&mut self, line_number: u32, ctx: &mut ViewContext<Self>);
    fn jump_to_matching_bracket(&mut self, ctx: &mut ViewContext<Self>);
    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ViewContext<Self>);
    fn paste(
        &mut self,
        count: u32,
        direction: &Direction,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    );
    fn insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ViewContext<Self>,
    );
    /// Join the current line to the next one.
    fn join_line(&mut self, count: u32, ctx: &mut ViewContext<Self>);

    fn undo(&mut self, ctx: &mut ViewContext<Self>);
    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>);
    fn backspace(&mut self, ctx: &mut ViewContext<Self>);
    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>);
    fn escape(&mut self, ctx: &mut ViewContext<Self>);
    /// Go to the definition of the symbol under cursor (gd).
    fn goto_definition(&mut self, _ctx: &mut ViewContext<Self>) {}
    /// Find references of the symbol under cursor (gr).
    fn find_references(&mut self, _ctx: &mut ViewContext<Self>) {}
    /// Show hover information for the symbol under cursor (gh).
    fn show_hover(&mut self, _ctx: &mut ViewContext<Self>) {}
    /// Center the current line vertically in the viewport (zz).
    fn center_cursor_vertically(&mut self, _ctx: &mut ViewContext<Self>) {}
    /// Move the cursor down `count` half-pages and scroll the viewport (`<C-d>`).
    fn scroll_half_page_down(&mut self, _count: u32, _ctx: &mut ViewContext<Self>) {}
    /// Move the cursor up `count` half-pages and scroll the viewport (`<C-u>`).
    fn scroll_half_page_up(&mut self, _count: u32, _ctx: &mut ViewContext<Self>) {}
}
