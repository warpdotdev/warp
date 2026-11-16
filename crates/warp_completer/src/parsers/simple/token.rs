#[derive(Debug, PartialEq, Eq)]
pub enum Token<'a> {
    Literal(&'a str),
    /// Whitespace excluding newline
    Whitespace(&'a str),
    /// | Operator
    Pipe,
    /// || operator
    LogicalOr,
    /// & operator
    Ampersand,
    /// && operator
    LogicalAnd,
    Semicolon,
    Newline,
    Backtick,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
    Dollar,
    SingleQuote,
    DoubleQuote,
    /// \ or `
    EscapeChar(&'a str),
    RedirectInput,
    RedirectOutput,
}

impl Token<'_> {
    pub fn as_str(&self) -> &str {
        use Token::*;
        match self {
            Literal(value) | Whitespace(value) | EscapeChar(value) => value,
            Pipe => "|",
            LogicalOr => "||",
            Ampersand => "&",
            LogicalAnd => "&&",
            Semicolon => ";",
            Newline => "\n",
            Backtick => "`",
            OpenParen => "(",
            CloseParen => ")",
            OpenCurly => "{",
            CloseCurly => "}",
            Dollar => "$",
            SingleQuote => "'",
            DoubleQuote => "\"",
            RedirectInput => "<",
            RedirectOutput => ">",
        }
    }
}
