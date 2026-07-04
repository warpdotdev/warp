// Unit tests for crates/i18n/src/lib.rs.

use super::*;

#[test]
fn test_locale_mapping() {
    // These assertions share the global CURRENT_LOCALE, so they run inside a
    // single test to avoid racing against each other under the parallel test
    // runner. The initial-default check must come first.
    assert_eq!(current_locale(), "en");
    set_locale("ru_RU.UTF-8");
    assert_eq!(current_locale(), "ru");
    set_locale("ko_KR.UTF-8");
    assert_eq!(current_locale(), "ko");
    set_locale("zh_CN.UTF-8");
    assert_eq!(current_locale(), "zh-CN");
    set_locale("de");
    assert_eq!(current_locale(), "en");
}

#[test]
fn test_interpolate() {
    let result = interpolate("Hello, {name}!", &[("name", "World".into())]);
    assert_eq!(result, "Hello, World!");
}

#[test]
fn test_interpolate_multiple() {
    let result = interpolate(
        "{greeting}, {name}!",
        &[("greeting", "Hi".into()), ("name", "Alice".into())],
    );
    assert_eq!(result, "Hi, Alice!");
}

#[test]
fn test_lookup_missing() {
    assert_eq!(lookup("nonexistent.key", "en"), TranslationLookup::Missing);
}

#[test]
fn test_flatten_value_nested() {
    let yaml = r#"
en:
  menu:
    file: "File"
    edit: "Edit"
  appearance:
    opacity: "Opacity"
"#;
    let mut translations = Translations::new();
    merge_locale_file(yaml, &mut translations);
    let en = translations.get("en").expect("en locale should exist");
    assert_eq!(en.get("menu.file"), Some(&"File".to_string()));
    assert_eq!(en.get("menu.edit"), Some(&"Edit".to_string()));
    assert_eq!(en.get("appearance.opacity"), Some(&"Opacity".to_string()));
}

#[test]
fn test_yaml_key_balance_en_equals_ru() {
    // Both locale files must have the same set of keys.
    // This prevents drift where a key is added to en.yml but forgotten in ru.yml.
    let mut en_translations = Translations::new();
    let mut ru_translations = Translations::new();

    merge_locale_file(
        include_str!("../../../resources/bundled/locales/en.yml"),
        &mut en_translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/ru.yml"),
        &mut ru_translations,
    );

    let en_keys: std::collections::BTreeSet<&String> = en_translations
        .get("en")
        .expect("en locale missing")
        .keys()
        .collect();
    let ru_keys: std::collections::BTreeSet<&String> = ru_translations
        .get("ru")
        .expect("ru locale missing")
        .keys()
        .collect();

    let en_only: Vec<_> = en_keys.difference(&ru_keys).collect();
    let ru_only: Vec<_> = ru_keys.difference(&en_keys).collect();

    assert!(
        en_only.is_empty(),
        "Keys in en.yml missing from ru.yml: {en_only:?}"
    );
    assert!(
        ru_only.is_empty(),
        "Keys in ru.yml missing from en.yml: {ru_only:?}"
    );
}

#[test]
fn test_yaml_key_balance_en_equals_ko() {
    let mut en_translations = Translations::new();
    let mut ko_translations = Translations::new();

    merge_locale_file(
        include_str!("../../../resources/bundled/locales/en.yml"),
        &mut en_translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/ko.yml"),
        &mut ko_translations,
    );

    let en_keys: std::collections::BTreeSet<&String> = en_translations
        .get("en")
        .expect("en locale missing")
        .keys()
        .collect();
    let ko_keys: std::collections::BTreeSet<&String> = ko_translations
        .get("ko")
        .expect("ko locale missing")
        .keys()
        .collect();

    let en_only: Vec<_> = en_keys.difference(&ko_keys).collect();
    let ko_only: Vec<_> = ko_keys.difference(&en_keys).collect();

    assert!(
        en_only.is_empty(),
        "Keys in en.yml missing from ko.yml: {en_only:?}"
    );
    assert!(
        ko_only.is_empty(),
        "Keys in ko.yml missing from en.yml: {ko_only:?}"
    );
}

#[test]
fn test_yaml_no_key_collisions() {
    let en = include_str!("../../../resources/bundled/locales/en.yml");
    let ru = include_str!("../../../resources/bundled/locales/ru.yml");
    let ko = include_str!("../../../resources/bundled/locales/ko.yml");
    for (raw, name) in [(en, "en.yml"), (ru, "ru.yml"), (ko, "ko.yml")] {
        let value: serde_yaml::Value = serde_yaml::from_str(raw).expect("invalid YAML");
        let mut collisions = Vec::new();
        walk_collisions(&[], &value, &mut collisions);
        assert!(
            collisions.is_empty(),
            "YAML key collision in {name}: {collisions:?}"
        );
    }
}

fn walk_collisions(path: &[&str], value: &serde_yaml::Value, out: &mut Vec<String>) {
    let serde_yaml::Value::Mapping(map) = value else {
        return;
    };
    let mut scalars: Vec<Vec<&str>> = Vec::new();
    let mut map_paths: Vec<Vec<&str>> = Vec::new();
    for (k, v) in map {
        let Some(key) = k.as_str() else { continue };
        if key == "_comment" {
            continue;
        }
        let mut child = path.to_vec();
        child.push(key);
        if matches!(v, serde_yaml::Value::Mapping(_)) {
            map_paths.push(child.clone());
            walk_collisions(&child, v, out);
        } else {
            scalars.push(child);
        }
    }
    for s in &scalars {
        if map_paths.contains(s) {
            out.push(s.join("."));
        }
    }
}

#[test]
fn test_flatten_value_with_comment() {
    let yaml = r#"
en:
  _comment: "this should be skipped"
  menu:
    file: "File"
"#;
    let mut translations = Translations::new();
    merge_locale_file(yaml, &mut translations);
    let en = translations.get("en").expect("en locale should exist");
    // _comment at root level is filtered out
    assert_eq!(en.get("_comment"), None);
    assert_eq!(en.get("menu.file"), Some(&"File".to_string()));
}
