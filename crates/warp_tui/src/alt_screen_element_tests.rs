use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::TerminalModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{TuiBuffer, TuiBufferExt, TuiElement, TuiPaintContext, TuiRect};
use warpui_core::keymap::Keystroke;

use super::{fallback_key_bytes, TuiAltScreenElement};

fn keystroke(key: &str) -> Keystroke {
    Keystroke {
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
        meta: false,
        key: key.to_owned(),
    }
}

/// A mock model with the alternate screen active and `bytes` already
/// processed on it (after homing + clearing, so content is deterministic).
fn alt_screen_model(bytes: &str) -> TerminalModel {
    let mut model = TerminalModel::mock(None, None);
    model.process_bytes(format!("\x1b[?1049h\x1b[H\x1b[2J{bytes}").as_str());
    assert!(model.is_alt_screen_active());
    model
}

fn alt_screen_element(model: TerminalModel) -> TuiAltScreenElement {
    TuiAltScreenElement::new(Arc::new(FairMutex::new(model)), Rc::new(StdCell::new(None)))
}

#[test]
fn renders_the_alt_screen_grid_and_places_the_cursor() {
    let element = alt_screen_element(alt_screen_model("hi"));

    // The mock model's grid is 7 columns x 10 rows (see
    // `test_utils::block_size`).
    let area = TuiRect::new(0, 0, 7, 10);
    let mut buffer = TuiBuffer::empty(area);
    let mut rendered_views = EntityIdMap::default();
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    element.render(area, &mut buffer, &mut paint_ctx);

    let lines = buffer.to_lines();
    assert_eq!(lines[0], "hi     ");

    // The terminal cursor rests after the typed text.
    assert_eq!(element.cursor_position(area, &mut paint_ctx), Some((2, 0)));
}

#[test]
fn renders_nothing_once_the_alt_screen_deactivates() {
    let mut model = alt_screen_model("hi");
    // Leaving the alternate screen returns rendering to the transcript.
    model.process_bytes("\x1b[?1049l");
    assert!(!model.is_alt_screen_active());
    let element = alt_screen_element(model);

    let area = TuiRect::new(0, 0, 7, 10);
    let mut buffer = TuiBuffer::empty(area);
    let mut rendered_views = EntityIdMap::default();
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    element.render(area, &mut buffer, &mut paint_ctx);

    assert!(buffer.to_lines().iter().all(|line| line.trim().is_empty()));
    assert_eq!(element.cursor_position(area, &mut paint_ctx), None);
}

#[test]
fn arrow_keys_honor_application_cursor_mode() {
    let mut model = alt_screen_model("");
    assert_eq!(
        TuiAltScreenElement::encode_key(&model, &keystroke("up"), "", None),
        Some(b"\x1b[A".to_vec())
    );

    // DECCKM (application cursor keys) switches arrows to SS3 encoding.
    model.process_bytes("\x1b[?1h");
    assert_eq!(
        TuiAltScreenElement::encode_key(&model, &keystroke("up"), "", None),
        Some(b"\x1bOA".to_vec())
    );
}

#[test]
fn plain_and_control_keys_encode_through_the_model() {
    let model = alt_screen_model("");
    assert_eq!(
        TuiAltScreenElement::encode_key(&model, &keystroke("q"), "q", Some("q")),
        Some(b"q".to_vec())
    );
    let mut ctrl_c = keystroke("c");
    ctrl_c.ctrl = true;
    assert_eq!(
        TuiAltScreenElement::encode_key(&model, &ctrl_c, "c", Some("c")),
        Some(vec![0x03])
    );
}

#[test]
fn mouse_reporting_requires_the_program_to_opt_in() {
    let model = alt_screen_model("");
    // No mouse tracking requested: clicks are swallowed, not forwarded.
    assert!(!TuiAltScreenElement::wants_mouse_reporting(&model));

    // Click tracking (1000) + SGR encoding (1006) opt in to forwarding.
    let mut model = alt_screen_model("");
    model.process_bytes("\x1b[?1000h\x1b[?1006h");
    assert!(TuiAltScreenElement::wants_mouse_reporting(&model));
}

#[test]
fn enter_maps_to_carriage_return() {
    assert_eq!(
        fallback_key_bytes(&keystroke("enter"), ""),
        Some(b"\r".to_vec())
    );
}

#[test]
fn escape_maps_to_esc_byte() {
    assert_eq!(
        fallback_key_bytes(&keystroke("escape"), ""),
        Some(b"\x1b".to_vec())
    );
}

#[test]
fn tab_and_backtab_map_to_tab_sequences() {
    assert_eq!(
        fallback_key_bytes(&keystroke("\t"), ""),
        Some(b"\t".to_vec())
    );
    let mut shift_tab = keystroke("\t");
    shift_tab.shift = true;
    assert_eq!(fallback_key_bytes(&shift_tab, ""), Some(b"\x1b[Z".to_vec()));
}

#[test]
fn ctrl_letter_maps_to_control_code() {
    let mut ctrl_c = keystroke("c");
    ctrl_c.ctrl = true;
    assert_eq!(fallback_key_bytes(&ctrl_c, ""), Some(vec![0x03]));

    let mut ctrl_z = keystroke("z");
    ctrl_z.ctrl = true;
    assert_eq!(fallback_key_bytes(&ctrl_z, ""), Some(vec![0x1a]));
}

#[test]
fn plain_characters_pass_through() {
    assert_eq!(
        fallback_key_bytes(&keystroke("a"), "a"),
        Some(b"a".to_vec())
    );
    // Shifted characters forward the reported (shifted) character.
    let mut shifted = keystroke("A");
    shifted.shift = true;
    assert_eq!(fallback_key_bytes(&shifted, "A"), Some(b"A".to_vec()));
}

#[test]
fn alt_prefixes_escape_byte() {
    let mut alt_x = keystroke("x");
    alt_x.alt = true;
    assert_eq!(fallback_key_bytes(&alt_x, "x"), Some(b"\x1bx".to_vec()));
}

#[test]
fn unencodable_keys_produce_no_bytes() {
    // A bare modifier-less key with no character payload has no byte encoding.
    assert_eq!(fallback_key_bytes(&keystroke("f21"), ""), None);
}
