pub use warp_features::*;
use warpui::platform::menu::{CustomMenuItem, MenuItem, MenuItemPropertyChanges};
fn feature_flag_menu_item(flag: FeatureFlag) -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        &format!("{flag:?}"),
        move |_| {
            // toggling the flag
            flag.set_enabled(!flag.is_enabled())
        },
        move |_props, _ctx| MenuItemPropertyChanges {
            checked: Some(flag.is_enabled()),
            ..Default::default()
        },
        None,
    ))
}

pub fn runtime_flags_menu_items() -> Vec<MenuItem> {
    if !FeatureFlag::RuntimeFeatureFlags.is_enabled() {
        return Vec::new();
    }

    RUNTIME_FEATURE_FLAGS
        .iter()
        .map(|flag| feature_flag_menu_item(*flag))
        .collect()
}
