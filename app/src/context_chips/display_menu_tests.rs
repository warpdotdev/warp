use super::query_matches_existing_name;

#[test]
fn query_matches_existing_name_is_ascii_case_insensitive() {
    let names = ["main", "feature/Foo"];
    assert!(query_matches_existing_name(names, "main"));
    assert!(query_matches_existing_name(names, "Main"));
    assert!(query_matches_existing_name(names, "MAIN"));
    assert!(query_matches_existing_name(names, "feature/foo"));
    assert!(query_matches_existing_name(names, "FEATURE/FOO"));
}

#[test]
fn query_matches_existing_name_returns_false_when_no_overlap() {
    let names = ["main", "feature/foo"];
    assert!(!query_matches_existing_name(names, "develop"));
    assert!(!query_matches_existing_name(names, "feature/bar"));
}

#[test]
fn query_matches_existing_name_returns_false_for_empty_input() {
    let names: [&str; 0] = [];
    assert!(!query_matches_existing_name(names, "main"));
}

#[test]
fn query_matches_existing_name_works_with_owned_strings() {
    let names = [String::from("main"), String::from("Develop")];
    assert!(query_matches_existing_name(names.iter(), "Main"));
    assert!(query_matches_existing_name(names.iter(), "develop"));
}
