use super::Menu;

#[test]
fn localized_window_menu_retains_system_role() {
    assert!(Menu::new_window_menu("Ventana", vec![]).is_window_menu());
    assert!(Menu::new("Window", vec![]).is_window_menu());
    assert!(!Menu::new("Archivo", vec![]).is_window_menu());
}
