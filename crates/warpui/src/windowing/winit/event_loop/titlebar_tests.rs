use winit::dpi::PhysicalPosition;
use winit::event::MouseButton;

use super::*;

#[test]
fn native_window_operation_clears_pressed_button() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    window_state.determine_click_count_and_update_button_state(MouseButton::Left);

    window_state.native_window_operation_completed();

    assert_eq!(window_state.current_mouse_button_pressed, None);
}

#[test]
fn cursor_move_after_native_window_operation_is_not_a_drag() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    window_state.determine_click_count_and_update_button_state(MouseButton::Left);

    let event = convert_cursor_moved(PhysicalPosition::new(10.0, 20.0), &mut window_state, 1.0);
    assert!(matches!(
        event,
        ConvertedEvent::Event(crate::event::Event::LeftMouseDragged { .. })
    ));

    window_state.native_window_operation_completed();

    let event = convert_cursor_moved(PhysicalPosition::new(20.0, 30.0), &mut window_state, 1.0);

    assert!(matches!(
        event,
        ConvertedEvent::Event(crate::event::Event::MouseMoved { .. })
    ));
}

#[test]
fn native_window_operation_preserves_double_click_state() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    window_state.determine_click_count_and_update_button_state(MouseButton::Left);
    window_state.native_window_operation_completed();
    window_state.determine_click_count_and_update_button_state(MouseButton::Left);
    window_state.native_window_operation_completed();

    let last_press = window_state.last_mouse_button_pressed.as_ref().unwrap();
    assert_eq!(last_press.click_count, 2);
}
