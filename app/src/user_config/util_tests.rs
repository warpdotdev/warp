use super::*;

#[test]
fn title_case_test() {
    assert_eq!("Test", title_case("test"));
    assert_eq!("Test", title_case("TEST"));
    assert_eq!("Test", title_case("Test"));
    assert_eq!("Zażółć", title_case("Zażółć"));
    assert_eq!("Zażółć", title_case("ZAŻÓŁĆ"));
}

#[test]
fn file_name_to_human_readable_name_test() {
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("solarized_dark")
    );
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("solarized_dark.yaml")
    );
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("SOLARIZED_DARK.yaml")
    );
    assert_eq!(
        "Solarizeddark",
        file_name_to_human_readable_name("solarizeddark.yaml")
    );
}
