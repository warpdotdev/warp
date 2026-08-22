use super::*;

fn presentation(
    id: &str,
    is_selectable: bool,
    is_key_connected: bool,
    is_profile_default: bool,
) -> TuiModelPickerPresentation {
    TuiModelPickerPresentation {
        id: id.into(),
        title: id.to_owned(),
        is_selectable,
        is_key_connected,
        is_profile_default,
        discount_percentage: None,
    }
}

#[test]
fn empty_query_prefers_active_model_and_filtered_query_prefers_best_match() {
    let presentations = vec![
        presentation("auto", true, false, false),
        presentation("gpt-4", true, false, false),
        presentation("gpt-5", true, false, false),
    ];

    assert_eq!(
        preferred_selection_id(&presentations, &LLMId::from("gpt-4"), true),
        Some(LLMId::from("gpt-4"))
    );
    assert_eq!(
        preferred_selection_id(&presentations, &LLMId::from("gpt-4"), false),
        Some(LLMId::from("gpt-5"))
    );
}

#[test]
fn model_selection_skips_disabled_rows() {
    let presentations = vec![
        presentation("auto", true, false, false),
        presentation("gpt-5", true, false, false),
        presentation("disabled", false, false, false),
    ];

    assert_eq!(
        preferred_selection_id(&presentations, &LLMId::from("disabled"), true),
        Some(LLMId::from("gpt-5"))
    );
    assert_eq!(
        preferred_selection_id(&presentations, &LLMId::from("auto"), false),
        Some(LLMId::from("gpt-5"))
    );
}

#[test]
fn snapshot_marks_only_key_connected_models() {
    let connected = snapshot_row(&presentation("gpt-5", true, true, false));
    assert_eq!(connected.state_suffix.as_deref(), Some("(key connected)"));
    let hosted = snapshot_row(&presentation("auto", true, false, false));
    assert_eq!(hosted.state_suffix, None);
}

#[test]
fn snapshot_marks_the_profile_default_model() {
    let default = snapshot_row(&presentation("auto", true, false, true));
    assert_eq!(default.state_suffix.as_deref(), Some("(default)"));

    let connected_default = snapshot_row(&presentation("gpt-5", true, true, true));
    assert_eq!(
        connected_default.state_suffix.as_deref(),
        Some("(default) (key connected)")
    );
}
