// Allow the use of `LineEnding::from_current_platform` in this test file, since
// it's used in the implementation of `infer_line_ending`.
#![allow(clippy::disallowed_methods)]

use warp_core::platform::SessionPlatform;

use super::*;

#[test]
fn test_infer_line_ending_empty_file() {
    assert_eq!(
        infer_line_ending("", None),
        LineEnding::from_current_platform()
    );
}

#[test]
fn test_infer_line_ending_single_line() {
    assert_eq!(
        infer_line_ending("I have no line endings", None),
        LineEnding::from_current_platform()
    );
}

#[test]
fn test_infer_line_ending_unix_subsystem_for_windows() {
    assert_eq!(
        infer_line_ending("", Some(&SessionPlatform::WSL)),
        LineEnding::LF
    );

    assert_eq!(
        infer_line_ending("", Some(&SessionPlatform::MSYS2)),
        LineEnding::LF
    );
}

#[test]
fn test_infer_line_ending_native_platform() {
    assert_eq!(
        infer_line_ending("", Some(&SessionPlatform::Native)),
        LineEnding::from_current_platform()
    );
}

#[test]
fn test_infer_line_ending_windows() {
    assert_eq!(
        infer_line_ending("This\r\nhas\r\nlines\r\n", None),
        LineEnding::CRLF
    );
}

#[test]
fn test_infer_line_ending_nix() {
    assert_eq!(infer_line_ending("This\nhas\nlines", None), LineEnding::LF);
}

#[test]
fn test_infer_line_ending_mixed() {
    // There's no correct line ending in this case - we follow the `line-ending` crate's
    // most-common-ending approach.
    assert_eq!(
        infer_line_ending("This\r\nhas\r\nmixed\nline endings", None),
        LineEnding::CRLF
    );
    assert_eq!(
        infer_line_ending("This\nhas\r\nmixed\nline endings", None),
        LineEnding::LF
    );
    assert_eq!(
        infer_line_ending("This\r\nhas\nmixed\r\nline\nendings", None),
        LineEnding::LF
    );
}

#[test]
fn test_apply_lf() {
    let original = "This\r\nhas\nevery\rline ending";
    assert_eq!(LF::apply_to(original), "This\nhas\nevery\nline ending");
}

#[test]
fn test_apply_cr() {
    let original = "This\r\nhas\nevery\rline ending";
    assert_eq!(CR::apply_to(original), "This\rhas\revery\rline ending");
}

#[test]
fn test_apply_crlf() {
    let original = "This\r\nhas\nevery\rline ending";
    assert_eq!(
        CRLF::apply_to(original),
        "This\r\nhas\r\nevery\r\nline ending"
    );
}

#[test]
fn test_multiline_str_try_new_same_ending() {
    let s = MultilineStr::<LF>::try_new("One\nTwo").expect("Should convert successfully");
    assert_eq!(s.as_str(), "One\nTwo");
}

#[test]
fn test_multiline_str_try_new_different_ending() {
    let s = MultilineStr::<LF>::try_new("One\r\nTwo");
    assert!(s.is_err());
}

#[test]
fn test_multiline_str_try_new_mixed_ending() {
    let s = MultilineStr::<LF>::try_new("One\r\nTwo\nThree\nFour");
    assert!(s.is_err());
}

#[test]
fn test_multiline_str_new_single_line() {
    let s = MultilineStr::<LF>::try_new("One").expect("Should convert successfully");
    assert_eq!(s.as_str(), "One");
}

#[test]
fn test_multiline_str_apply_same_ending() {
    let converted = MultilineStr::<LF>::apply("One\nTwo");
    assert_eq!(
        converted,
        Cow::Borrowed(MultilineStr::new_unchecked("One\nTwo"))
    );
}

#[test]
fn test_multiline_str_apply_different_ending() {
    let converted = MultilineStr::<LF>::apply("One\r\nTwo\r\nThree");
    assert_eq!(
        converted,
        Cow::Owned(MultilineString::new_unchecked("One\nTwo\nThree"))
    )
}

#[test]
fn test_multiline_str_apply_single_line() {
    let converted = MultilineStr::<LF>::apply("One");
    assert_eq!(converted, Cow::Borrowed(MultilineStr::new_unchecked("One")));
}

#[test]
fn test_multiline_str_apply_mixed() {
    // The most common ending is already LF, *but* there are mixed endings so normalization is needed.
    let converted = MultilineStr::<LF>::apply("One\r\nTwo\nThree\nFour");
    assert_eq!(
        converted,
        Cow::Owned(MultilineString::new_unchecked("One\nTwo\nThree\nFour"))
    );
}

#[test]
fn test_any_infer_single_ending() {
    let inferred = AnyMultilineString::infer("One\nTwo\nThree");
    assert_eq!(inferred.as_str(), "One\nTwo\nThree");
    assert_eq!(inferred.line_ending(), LineEnding::LF);
}

#[test]
fn test_any_infer_single_line() {
    let inferred = AnyMultilineString::infer("One");
    assert_eq!(inferred.as_str(), "One");
    assert_eq!(inferred.line_ending(), LineEnding::from_current_platform());
}

#[test]
fn test_any_infer_mixed() {
    let inferred = AnyMultilineString::infer("One\r\nTwo\nThree\r\nFour");
    // The text should be normalized to CRLF.
    assert_eq!(inferred.as_str(), "One\r\nTwo\r\nThree\r\nFour");
    assert_eq!(inferred.line_ending(), LineEnding::CRLF);
}
