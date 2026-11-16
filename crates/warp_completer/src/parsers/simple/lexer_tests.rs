use super::*;

#[test]
fn test_lexer() {
    let source = r#"ls | rm -rf || touch 'hello.txt' &
cat "Hello $(ls -la)" && echo `ps \`;  {echo Goodbye😀}"#;
    let tokens: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.item)
        .collect();

    assert_eq!(
        tokens,
        [
            Token::Literal("ls"),
            Token::Whitespace(" "),
            Token::Pipe,
            Token::Whitespace(" "),
            Token::Literal("rm"),
            Token::Whitespace(" "),
            Token::Literal("-rf"),
            Token::Whitespace(" "),
            Token::LogicalOr,
            Token::Whitespace(" "),
            Token::Literal("touch"),
            Token::Whitespace(" "),
            Token::SingleQuote,
            Token::Literal("hello.txt"),
            Token::SingleQuote,
            Token::Whitespace(" "),
            Token::Ampersand,
            Token::Newline,
            Token::Literal("cat"),
            Token::Whitespace(" "),
            Token::DoubleQuote,
            Token::Literal("Hello"),
            Token::Whitespace(" "),
            Token::Dollar,
            Token::OpenParen,
            Token::Literal("ls"),
            Token::Whitespace(" "),
            Token::Literal("-la"),
            Token::CloseParen,
            Token::DoubleQuote,
            Token::Whitespace(" "),
            Token::LogicalAnd,
            Token::Whitespace(" "),
            Token::Literal("echo"),
            Token::Whitespace(" "),
            Token::Backtick,
            Token::Literal("ps"),
            Token::Whitespace(" "),
            Token::EscapeChar("\\"),
            Token::Backtick,
            Token::Semicolon,
            Token::Whitespace("  "),
            Token::OpenCurly,
            Token::Literal("echo"),
            Token::Whitespace(" "),
            Token::Literal("Goodbye😀"),
            Token::CloseCurly,
        ]
    );
}

#[test]
fn test_spans() {
    let source = "ls -la && echo Hello' World'$(cat  😀.txt";
    let spans: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.span)
        .collect();

    assert_eq!(
        spans,
        [
            Span::new(0, 2),   // ls
            Span::new(2, 3),   // space
            Span::new(3, 6),   // -la
            Span::new(6, 7),   // space
            Span::new(7, 9),   // &&
            Span::new(9, 10),  // space
            Span::new(10, 14), // echo
            Span::new(14, 15), // space
            Span::new(15, 20), // Hello
            Span::new(20, 21), // '
            Span::new(21, 22), // space
            Span::new(22, 27), // World
            Span::new(27, 28), // '
            Span::new(28, 29), // $
            Span::new(29, 30), // (
            Span::new(30, 33), // cat
            Span::new(33, 35), // double space
            Span::new(35, 43), // 😀.txt (😀 is 4 bytes long)
        ]
    );
}

#[test]
fn test_escaped_tokens() {
    let source = r"\\\||\&&\\&&||";
    let tokens: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.item)
        .collect();

    assert_eq!(
        tokens,
        [
            Token::EscapeChar("\\"),
            Token::EscapeChar("\\"),
            Token::EscapeChar("\\"),
            Token::Pipe,
            Token::Pipe,
            Token::EscapeChar("\\"),
            Token::Ampersand,
            Token::Ampersand,
            Token::EscapeChar("\\"),
            Token::EscapeChar("\\"),
            Token::LogicalAnd,
            Token::LogicalOr,
        ]
    )
}

#[test]
fn test_escaped_token_spans() {
    let source = r"\\\||\&&\\&&||";
    let spans: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.span)
        .collect();

    assert_eq!(
        spans,
        [
            Span::new(0, 1),   // \
            Span::new(1, 2),   // \
            Span::new(2, 3),   // \
            Span::new(3, 4),   // |
            Span::new(4, 5),   // |
            Span::new(5, 6),   // \
            Span::new(6, 7),   // &
            Span::new(7, 8),   // &
            Span::new(8, 9),   // \
            Span::new(9, 10),  // \
            Span::new(10, 12), // &&
            Span::new(12, 14), // ||
        ]
    );
}

#[test]
fn test_multiple_whitespace() {
    let source = " \t  |\t  ";
    let tokens: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.item)
        .collect();

    assert_eq!(
        tokens,
        [
            Token::Whitespace(" \t  "),
            Token::Pipe,
            Token::Whitespace("\t  "),
        ]
    )
}

#[test]
fn test_backtick_escape_char() {
    let source = r#"& "$HOME\Downloads\Warp` Setup.exe" /SP- /SILENT `t`"#;
    let tokens: Vec<_> = Lexer::new(source, EscapeChar::Backtick, false)
        .map(|t| (t.item, t.span))
        .collect();

    assert_eq!(
        tokens,
        [
            (Token::Ampersand, Span::new(0, 1)),
            (Token::Whitespace(" "), Span::new(1, 2)),
            (Token::DoubleQuote, Span::new(2, 3)),
            (Token::Dollar, Span::new(3, 4)),
            (Token::Literal(r"HOME\Downloads\Warp"), Span::new(4, 23)),
            (Token::EscapeChar("`"), Span::new(23, 24)),
            (Token::Whitespace(" "), Span::new(24, 25)),
            (Token::Literal("Setup.exe"), Span::new(25, 34)),
            (Token::DoubleQuote, Span::new(34, 35)),
            (Token::Whitespace(" "), Span::new(35, 36)),
            (Token::Literal("/SP-"), Span::new(36, 40)),
            (Token::Whitespace(" "), Span::new(40, 41)),
            (Token::Literal("/SILENT"), Span::new(41, 48)),
            (Token::Whitespace(" "), Span::new(48, 49)),
            (Token::EscapeChar("`"), Span::new(49, 50)),
            (Token::Literal("t"), Span::new(50, 51)),
            (Token::EscapeChar("`"), Span::new(51, 52)),
        ]
    )
}

#[test]
fn test_single_quote_as_literals() {
    let source = r#"I'd like to edit app/src"#;
    let tokens: Vec<_> = Lexer::new(source, EscapeChar::Backslash, true)
        .map(|t| (t.item, t.span))
        .collect();

    assert_eq!(
        tokens,
        [
            (Token::Literal("I'd"), Span::new(0, 3)),
            (Token::Whitespace(" "), Span::new(3, 4)),
            (Token::Literal("like"), Span::new(4, 8)),
            (Token::Whitespace(" "), Span::new(8, 9)),
            (Token::Literal("to"), Span::new(9, 11)),
            (Token::Whitespace(" "), Span::new(11, 12)),
            (Token::Literal("edit"), Span::new(12, 16)),
            (Token::Whitespace(" "), Span::new(16, 17)),
            (Token::Literal("app/src"), Span::new(17, 24)),
        ]
    )
}
