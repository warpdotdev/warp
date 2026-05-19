use std::collections::{HashMap, HashSet};

use super::{get_arguments, render_template};

fn create_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn renders_simple_substitution() {
    let template = "Hello, {{name}}!".to_string();
    let context = create_map(&[("name", "Warp")]);

    let args = get_arguments(&template);
    assert_eq!(args, vec!["name".to_string()]);

    let out = render_template(&template, &context);
    assert_eq!(out, "Hello, Warp!");
}

#[test]
fn leaves_unknown_placeholder_unchanged() {
    let template = "Hello, {{name}} and {{unknown}}!".to_string();
    let context = create_map(&[("name", "Warp")]);

    let args = get_arguments(&template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from(["name".to_string(), "unknown".to_string()])
    );

    let out = render_template(&template, &context);
    assert_eq!(out, "Hello, Warp and {{unknown}}!");
}

#[test]
fn multiple_and_repeated_arguments() {
    let template = "{{a}}-{{b}}-{{a}}";
    let context = create_map(&[("a", "X"), ("b", "Y")]);

    let args = get_arguments(template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from(["a".to_string(), "b".to_string()])
    );

    let out = render_template(template, &context);
    assert_eq!(out, "X-Y-X");
}

#[test]
fn unicode_in_names_and_text() {
    // name contains multibyte chars
    let template = "前缀 {{ab東早}} 后缀";
    let context = create_map(&[("ab東早", "值")]);

    let args = get_arguments(template);
    assert_eq!(args, vec!["ab東早".to_string()]);

    let out = render_template(template, &context);
    assert_eq!(out, "前缀 值 后缀");
}

#[test]
fn preserves_escaped_triple_braces() {
    let template = "{{{name}}} {{name}}";
    let context = create_map(&[("name", "Warp")]);

    let args = get_arguments(template);
    // Only the double-braced arg should be returned
    assert_eq!(args, vec!["name".to_string()]);

    let out = render_template(template, &context);
    // Triple braces should not be substituted by our parser; double braces should.
    assert_eq!(out, "{{{name}}} Warp");
}

#[test]
fn does_not_replace_spaced_variants() {
    // Variants with spaces are considered invalid by the parser and should remain unchanged.
    let template = "A {{ name }} B {{name }} C {{ name}} D {{name}}";
    let context = create_map(&[("name", "ok")]);

    let args = get_arguments(template);
    // Only the last {{name}} without spaces should be detected
    assert_eq!(args, vec!["name".to_string()]);

    let out = render_template(template, &context);
    assert_eq!(out, "A {{ name }} B {{name }} C {{ name}} D ok");
}

#[test]
fn handles_empty_template() {
    let template = "";
    let context = create_map(&[("unused", "value")]);

    let args = get_arguments(template);
    assert_eq!(args, Vec::<String>::new());

    let out = render_template(template, &context);
    assert_eq!(out, "");
}

#[test]
fn renders_adjacent_placeholders_without_separators() {
    let template = "{{first}}{{second}}{{third}}";
    let context = create_map(&[("first", "A"), ("second", "B"), ("third", "C")]);

    let args = get_arguments(template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from([
            "first".to_string(),
            "second".to_string(),
            "third".to_string()
        ])
    );

    let out = render_template(template, &context);
    assert_eq!(out, "ABC");
}

#[test]
fn renders_placeholders_at_start_and_end() {
    let template = "{{start}} middle {{end}}";
    let context = create_map(&[("start", "BEGIN"), ("end", "FINISH")]);

    let args = get_arguments(template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from(["start".to_string(), "end".to_string()])
    );

    let out = render_template(template, &context);
    assert_eq!(out, "BEGIN middle FINISH");
}

#[test]
fn accepts_numeric_suffixes_and_long_names_but_not_numeric_prefixes() {
    let template = "{{a1}} {{1a}} {{long_name-with-1234567890_suffix}}";
    let context = create_map(&[
        ("a1", "short"),
        ("1a", "invalid"),
        ("long_name-with-1234567890_suffix", "long"),
    ]);

    let args = get_arguments(template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from([
            "a1".to_string(),
            "long_name-with-1234567890_suffix".to_string()
        ])
    );

    let out = render_template(template, &context);
    assert_eq!(out, "short {{1a}} long");
}

#[test]
fn renders_special_values_without_reparsing_or_escaping() {
    let template = "prefix {{braces}} {{multiline}} {{empty}} suffix";
    let context = create_map(&[
        ("braces", "{{not_reparsed}} & <tag>"),
        ("multiline", "line1\nline2"),
        ("empty", ""),
    ]);

    let args = get_arguments(template);
    assert_eq!(
        args.into_iter().collect::<HashSet<String>>(),
        HashSet::from([
            "braces".to_string(),
            "multiline".to_string(),
            "empty".to_string()
        ])
    );

    let out = render_template(template, &context);
    assert_eq!(out, "prefix {{not_reparsed}} & <tag> line1\nline2  suffix");
}
