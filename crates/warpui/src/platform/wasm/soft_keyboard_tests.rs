use super::*;

#[test]
fn test_map_insert_text() {
    let event = HiddenInputEvent::InsertText {
        text: "hello".to_string(),
    };
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::TextInserted(s)) if s == "hello"));
}

#[test]
fn test_map_backspace() {
    let event = HiddenInputEvent::Backspace;
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::Backspace)));
}

#[test]
fn test_map_delete() {
    let event = HiddenInputEvent::Delete;
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::Backspace)));
}

#[test]
fn test_map_blur() {
    let event = HiddenInputEvent::Blur;
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::KeyboardDismissed)));
}

#[test]
fn test_map_keydown_enter() {
    let event = HiddenInputEvent::KeyDown {
        key: "Enter".to_string(),
    };
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::KeyDown(key)) if key == "Enter"));
}

#[test]
fn test_map_unicode_insert() {
    let event = HiddenInputEvent::InsertText {
        text: "👋🌍".to_string(),
    };
    let result = map_hidden_input_event(event);
    assert!(matches!(result, Some(SoftKeyboardInput::TextInserted(s)) if s == "👋🌍"));
}
