use anyhow::{anyhow, Result};

enum CurrentQuoteStrategy {
    None,
    Single,
    Double,
    AnsiC,
}

/// Unescape alias outputs in single quoting and ANSI-C quoting format.
/// For single quoting, we should take the literal meaning of all characters within
/// the quoting. For ANSI-C quoting, we need to translate escape sequences to their
/// unicode values.
///
/// Note that we don't unescape double quotes here as they require the knowledge of
/// shell variables' values. E.g. If we have the following string `echo "'$apple'"`,
/// we could unescape it without knowing what the value of $apple.
pub fn unescape_quotes(s: &str) -> Result<String> {
    let mut current_quoting = CurrentQuoteStrategy::None;

    let mut chars = s.chars().enumerate().peekable();
    let mut res = String::with_capacity(s.len());

    while let Some((idx, c)) = chars.next() {
        match (c, &current_quoting) {
            // If in single / Ansi-C quote, end the quote escaping.
            ('\'', CurrentQuoteStrategy::Single | CurrentQuoteStrategy::AnsiC) => {
                current_quoting = CurrentQuoteStrategy::None
            }
            ('\'', CurrentQuoteStrategy::None) => current_quoting = CurrentQuoteStrategy::Single,
            ('\"', CurrentQuoteStrategy::Double) => current_quoting = CurrentQuoteStrategy::None,
            ('\"', CurrentQuoteStrategy::None) => current_quoting = CurrentQuoteStrategy::Double,
            ('\\', CurrentQuoteStrategy::AnsiC) => {
                match chars.next() {
                    None => {
                        return Err(anyhow!("invalid escape at char {} in string {}", idx, s));
                    }
                    Some((_, next_character)) => {
                        // Referenced from the table here: https://en.wikipedia.org/wiki/Escape_sequences_in_C
                        res.push(match next_character {
                            'a' => '\u{07}',
                            'b' => '\u{08}',
                            'e' | 'E' => '\u{1B}',
                            'f' => '\u{0C}',
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'v' => '\u{0B}',
                            // TODO(kevin): Need to add escaping to unicode
                            // characters here. But this should be rare.
                            next_character => next_character,
                        });
                    }
                }
            }
            ('\\', CurrentQuoteStrategy::Double) => {
                match chars.next() {
                    None => {
                        return Err(anyhow!("invalid escape at char {} in string {}", idx, s));
                    }
                    Some((_, next_character)) => {
                        // The backslash retains special meaning when followed by
                        // ‘$’, ‘`’, ‘"’, ‘\’. Otherwise is treated as a literal.
                        // Referenced from here:
                        // https://www.gnu.org/software/bash/manual/html_node/Double-Quotes.html
                        match next_character {
                            '$' | '`' | '"' | '\\' => res.push(next_character),
                            // Newlines are ignored after a backslash, see:
                            // https://www.gnu.org/savannah-checkouts/gnu/bash/manual/bash.html#Escape-Character
                            '\n' => {}
                            _ => {
                                res.push('\\');
                                res.push(next_character);
                            }
                        }
                    }
                }
            }
            ('\\', CurrentQuoteStrategy::None) => match chars.next() {
                None => {
                    return Err(anyhow!("invalid escape at char {} in string {}", idx, s));
                }
                Some((_, next_character)) => match next_character {
                    // Newlines are ignored after a backslash, see:
                    // https://www.gnu.org/savannah-checkouts/gnu/bash/manual/bash.html#Escape-Character
                    '\n' => {}
                    _ => res.push(next_character),
                },
            },
            ('$', CurrentQuoteStrategy::None) => {
                match chars.peek() {
                    // ANSI-C quoting starts with $'
                    Some((_, '\'')) => {
                        current_quoting = CurrentQuoteStrategy::AnsiC;
                        chars.next();
                    }
                    _ => res.push('$'),
                }
            }
            _ => res.push(c),
        }
    }

    Ok(res)
}

#[test]
fn test_unescape_quotes() {
    assert_eq!(unescape_quotes("東方").unwrap(), "東方".to_string());
    assert_eq!(unescape_quotes(r#"$'\"\"'"#).unwrap(), r#""""#.to_string());
    assert_eq!(unescape_quotes(r#"'"'"#).unwrap(), r#"""#.to_string());
    assert_eq!(
        unescape_quotes(r#"$'foo"barbaz\'quux'"#).unwrap(),
        r#"foo"barbaz'quux"#.to_string()
    );
    // Every escape between ANSI-C quoting.
    assert_eq!(
        unescape_quotes(r"$'\a\b\v\f\n\r\t\e\E'").unwrap(),
        "\u{07}\u{08}\u{0b}\u{0c}\u{0a}\u{0d}\u{09}\u{1b}\u{1b}".to_string()
    );
    // Failure case when the escape character is at end of string.
    assert!(unescape_quotes(r"$'\").is_err());

    // Chars following escape chars should be taken literally when not currently
    // in a quote strategy.
    assert_eq!(
        unescape_quotes(r"'echo '\''hello\nworld'\'").unwrap(),
        "echo 'hello\\nworld'"
    );
}

#[test]
fn test_unescape_double_quotes() {
    assert_eq!(unescape_quotes("\"hello world\"").unwrap(), "hello world");
    assert_eq!(unescape_quotes(r#""hello\$world""#).unwrap(), "hello$world");
    assert_eq!(unescape_quotes(r#""hello\`world""#).unwrap(), "hello`world");
    assert_eq!(
        unescape_quotes(r#""hello\"world""#).unwrap(),
        "hello\"world"
    );
    assert_eq!(
        unescape_quotes(r#""hello\\world""#).unwrap(),
        "hello\\world"
    );
}

#[test]
fn test_unescape_double_quotes_nonspecial_chars() {
    assert_eq!(
        unescape_quotes(r#""hello\aworld""#).unwrap(),
        r"hello\aworld"
    );
}

#[test]
fn test_unescape_backslash_with_newline() {
    // With no quoting strategy.
    assert_eq!(unescape_quotes("hello\\\nworld").unwrap(), "helloworld");
    // With double quotes.
    assert_eq!(unescape_quotes("\"hello\\\nworld\"").unwrap(), "helloworld");
}
