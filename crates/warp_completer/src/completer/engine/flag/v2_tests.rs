use super::*;
use crate::meta::SpannedItem;
use crate::signatures::Opt;

fn opt(names: &[&str]) -> Opt {
    Opt {
        name: names.iter().map(|name| name.to_string()).collect(),
        ..Default::default()
    }
}

fn command_with_options(name: &str, options: Vec<Opt>) -> Command {
    Command {
        name: name.to_string(),
        options,
        ..Default::default()
    }
}

fn flag_location(command_name: &str, token: &str) -> Spanned<LocationType> {
    LocationType::Flag {
        command_name: command_name.to_string().spanned_unknown(),
        flag_name: Some(token.to_string().spanned_unknown()),
    }
    .spanned_unknown()
}

/// Regression test for #9820: a CMake definition such as `-DWITH_TESTS` is not a bundle of
/// short-hand flags, so it must not exactly-match an option just because its final character
/// (`S`) happens to be a known short flag. That spurious exact match is what colored some
/// `-D...=...` definitions as options while leaving others plain.
#[test]
fn attached_value_token_is_not_treated_as_short_flag_bundle() {
    let cmake = command_with_options("cmake", vec![opt(&["-S"]), opt(&["-D"]), opt(&["-G"])]);
    let location = flag_location("cmake", "-DWITH_TESTS");

    let suggestions = complete(MatchStrategy::CaseSensitive, &location, Some(&cmake));

    assert!(
        !suggestions
            .iter()
            .any(|matched| matched.suggestion.replacement == "-DWITH_TESTS"),
        "token whose characters are not all known short flags must not exact-match as a \
         short-flag bundle: {suggestions:?}"
    );
}

/// Genuine short-flag bundles (every character is a known short flag) keep their
/// current-token exact match so the Describe API can annotate e.g. `ssh -Xv`.
#[test]
fn genuine_short_flag_bundle_still_matches_current_token() {
    let ssh = command_with_options("ssh", vec![opt(&["-X"]), opt(&["-v"]), opt(&["-A"])]);
    let location = flag_location("ssh", "-Xv");

    let suggestions = complete(MatchStrategy::CaseSensitive, &location, Some(&ssh));

    assert!(
        suggestions
            .iter()
            .any(|matched| matched.suggestion.replacement == "-Xv"),
        "a token made entirely of known short flags must keep its current-token match: \
         {suggestions:?}"
    );
}
