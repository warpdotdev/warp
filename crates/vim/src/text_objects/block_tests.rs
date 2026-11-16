use unindent::Unindent;

use super::*;

const BLOCK: &str = "match BracketChar::try_from(c) {
        Ok(bracket) => todo!(),
        Err(_) => {
            let end_offset = vim_find_matching_bracket(
                buffer,
                BracketChar {
                    end: BracketEnd::Opening,
                    kind: bracket_kind,
                },
                offset,
            )?;
            let start_offset = vim_find_matching_bracket(
                buffer,
                BracketChar {
                    end: BracketEnd::Closing,
                    kind: bracket_kind,
                },
                end_offset,
            )?;
            Some(start_offset..end_offset)
        }
    }";

#[test]
fn test_vim_a_block() {
    let block = BLOCK.unindent();
    for i in 27..=29 {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::Parenthesis),
            Some(27.into()..30.into())
        );
    }
    for i in (31..=74).chain(573..=574) {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::CurlyBrace),
            Some(31.into()..575.into())
        );
    }
    for i in (75..=172).chain(266..=397).chain(491..=572) {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::CurlyBrace),
            Some(75.into()..573.into())
        );
    }
    for i in 173..=265 {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::CurlyBrace),
            Some(173.into()..266.into())
        );
    }
    for i in 398..=490 {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::CurlyBrace),
            Some(398.into()..491.into())
        );
    }
    for i in 39..=47 {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::Parenthesis),
            Some(39.into()..48.into())
        );
    }
    for i in 57..=58 {
        assert_eq!(
            vim_a_block(block.as_str(), i, BracketType::Parenthesis),
            Some(57.into()..59.into())
        );
    }
}

#[test]
fn test_vim_inner_block() {
    let block = BLOCK.unindent();
    for i in 27..=29 {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, false),
            Some(28.into()..29.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, true),
            Some(28.into()..29.into())
        );
    }
    for i in (31..=74).chain(573..=574) {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, false),
            Some(32.into()..573.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, true),
            Some(33.into()..573.into())
        );
    }
    for i in (75..=172).chain(266..=397).chain(491..=572) {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, false),
            Some(76.into()..567.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, true),
            Some(77.into()..567.into())
        );
    }
    for i in 173..=265 {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, false),
            Some(174.into()..252.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, true),
            Some(175.into()..252.into())
        );
    }
    for i in 398..=490 {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, false),
            Some(399.into()..477.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::CurlyBrace, true),
            Some(400.into()..477.into())
        );
    }
    for i in 39..=47 {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, false),
            Some(40.into()..47.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, true),
            Some(40.into()..47.into())
        );
    }
    for i in 57..=58 {
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, false),
            Some(58.into()..58.into())
        );
        assert_eq!(
            vim_inner_block(block.as_str(), i, BracketType::Parenthesis, true),
            Some(58.into()..58.into())
        );
    }
}
