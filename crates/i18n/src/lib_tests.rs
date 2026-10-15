use super::*;

#[test]
fn spanish_locale_variants() {
    for locale in ["es", "es-CL", "es_ES.UTF-8", "es-419", "ES-mx", " es "] {
        assert_eq!(supported_locale(locale), "es");
    }
    for locale in ["", "en-US", "de", "espresso", "esperanto"] {
        assert_eq!(supported_locale(locale), "en");
    }
}

#[test]
fn locale_selection_returns_stable_labels_and_unknown_keys() {
    set_locale("es-CL");
    let settings = text("workspace.settings");
    assert_eq!(settings, "Configuración");
    assert_eq!(text("missing.translation"), "missing.translation");
    assert_eq!(option_label("Bold"), "Negrita");
    assert_eq!(option_label("Underline"), "Subrayado");
    assert_eq!(option_label("Dark"), "Oscuro");
    assert_eq!(option_label("English"), "Inglés");
    assert_eq!(option_label("Spanish"), "Español");
    assert_eq!(option_label("Custom option"), "Custom option");
    assert_eq!(option_label("Light"), "Claro");
    assert_eq!(font_weight_label("Light"), "Ligera");
    assert_eq!(font_weight_label("ExtraLight"), "Ultraligera");
    assert_eq!(font_weight_label("Semibold"), "Seminegrita");
    set_locale("en-US");
    assert_eq!(text("workspace.settings"), "Settings");
    assert_eq!(settings, "Configuración");
    assert_eq!(option_label("Bold"), "Bold");
    assert_eq!(font_weight_label("ExtraLight"), "ExtraLight");
}

#[test]
fn embedded_spanish_catalog_matches_english() {
    let translations = embedded_translations();
    let english = translations.get("en").expect("English catalog");
    let spanish = translations.get("es").expect("Spanish catalog");
    assert_eq!(english.len(), spanish.len());
    for key in english.keys() {
        assert!(spanish.contains_key(key), "missing Spanish key: {key}");
        assert_eq!(
            placeholders(&english[key]),
            placeholders(&spanish[key]),
            "placeholder mismatch: {key}"
        );
    }
    assert_eq!(spanish.get("workspace.settings").unwrap(), "Configuración");
}

#[test]
fn interpolation_preserves_inserted_values() {
    assert_eq!(
        interpolate("Hola, {name}", &[("name", "User".into())]),
        "Hola, User"
    );
    assert_eq!(
        interpolate(
            "{command}: {name}",
            &[("command", "echo {name}".into()), ("name", "User".into())]
        ),
        "echo {name}: User"
    );
    assert_eq!(interpolate("{unknown} {", &[]), "{unknown} {");
}

#[test]
fn spanish_catalog_does_not_add_excessive_dots() {
    let translations = embedded_translations();
    let english = &translations["en"];
    let spanish = &translations["es"];
    for (key, value) in spanish {
        assert!(
            longest_dot_run(value) <= longest_dot_run(&english[key]).max(3),
            "excessive dots in Spanish translation: {key}"
        );
    }
}

fn longest_dot_run(value: &str) -> usize {
    value
        .split(|character| character != '.')
        .map(str::len)
        .max()
        .unwrap_or_default()
}

#[test]
fn missing_translations_fall_back_to_english_then_key() {
    let translations = HashMap::from([
        (
            "en".into(),
            HashMap::from([
                ("greeting".into(), "Hello".into()),
                ("fallback".into(), "Fallback".into()),
            ]),
        ),
        (
            "es".into(),
            HashMap::from([("greeting".into(), "Hola".into())]),
        ),
    ]);
    assert_eq!(lookup(&translations, "es", "greeting"), "Hola");
    assert_eq!(lookup(&translations, "es", "fallback"), "Fallback");
    assert_eq!(lookup(&translations, "unknown", "greeting"), "Hello");
    assert_eq!(lookup(&translations, "es", "missing"), "missing");
}

#[test]
fn malformed_catalog_does_not_replace_embedded_translations() {
    let mut translations = embedded_translations();
    merge_locale_file("es: [", &mut translations);
    assert_eq!(
        lookup(&translations, "es", "workspace.settings"),
        "Configuración"
    );
    merge_locale_file(
        "es:\n  workspace:\n    settings: Custom label",
        &mut translations,
    );
    assert_eq!(
        lookup(&translations, "es", "workspace.settings"),
        "Custom label"
    );
    assert_eq!(lookup(&translations, "es", "menu.file"), "Archivo");
}

#[test]
fn macros_interpolate_explicit_and_captured_arguments() {
    let query = "echo {name}";
    assert_eq!(t!("ai_output.query_suffix", query), ": echo {name}");
    assert_eq!(t!("ai_output.query_suffix", query = query), ": echo {name}");
}

fn placeholders(value: &str) -> Vec<&str> {
    let mut names: Vec<_> = value
        .match_indices('{')
        .filter_map(|(start, _)| {
            let tail = &value[start + 1..];
            let end = tail.find('}')?;
            Some(&tail[..end])
        })
        .collect();
    names.sort_unstable();
    names
}
