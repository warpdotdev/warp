use super::ExternalProductIcon;

#[test]
fn matches_bare_title() {
    let icon = ExternalProductIcon::from_string("Sentry");

    assert_eq!(icon.map(|i| i.get_path()), Some("bundled/svg/sentry.svg"));
}

#[test]
fn matches_title_case_insensitively() {
    let icon = ExternalProductIcon::from_string("SENTRY");

    assert_eq!(icon.map(|i| i.get_path()), Some("bundled/svg/sentry.svg"));
}

#[test]
fn matches_title_with_trailing_parenthetical_qualifier() {
    let icon = ExternalProductIcon::from_string("Sentry (OAuth)");

    assert_eq!(icon.map(|i| i.get_path()), Some("bundled/svg/sentry.svg"));
}

#[test]
fn matches_title_with_trailing_parenthetical_qualifier_and_no_space() {
    let icon = ExternalProductIcon::from_string("Sentry(OAuth)");

    assert_eq!(icon.map(|i| i.get_path()), Some("bundled/svg/sentry.svg"));
}

#[test]
fn matches_title_with_lowercase_parenthetical_qualifier() {
    let icon = ExternalProductIcon::from_string("github (oauth)");

    assert_eq!(icon.map(|i| i.get_path()), Some("bundled/svg/github.svg"));
}

#[test]
fn returns_none_for_unrelated_title_containing_a_product_name() {
    let icon = ExternalProductIcon::from_string("GitHub scraper thing");

    assert!(icon.is_none());
}

#[test]
fn returns_none_for_unknown_title_with_parenthetical_qualifier() {
    let icon = ExternalProductIcon::from_string("My Custom Server (OAuth)");

    assert!(icon.is_none());
}

#[test]
fn returns_none_for_title_that_is_only_a_parenthetical() {
    let icon = ExternalProductIcon::from_string("(OAuth)");

    assert!(icon.is_none());
}

#[test]
fn returns_none_for_unknown_title() {
    let icon = ExternalProductIcon::from_string("Not A Real Product");

    assert!(icon.is_none());
}
