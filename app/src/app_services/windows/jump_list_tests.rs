use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

fn tab_config(name: &str, file_name: Option<&str>) -> TabConfig {
    TabConfig {
        name: name.to_owned(),
        title: None,
        color: None,
        panes: Vec::new(),
        params: HashMap::new(),
        source_path: file_name.map(|file_name| PathBuf::from(r"C:\tab_configs").join(file_name)),
    }
}

fn task(title: &str, url: &str) -> JumpListTask {
    JumpListTask {
        title: title.to_owned(),
        url: url.to_owned(),
    }
}

#[test]
fn new_window_comes_first_even_without_tab_configs() {
    assert_eq!(
        jump_list_tasks(&[], "warp"),
        [task("New Window", "warp://action/new_window?path=~")]
    );
}

#[test]
fn tab_configs_keep_their_order_and_menu_labels() {
    let configs = [
        tab_config("Dev", Some("dev_a.toml")),
        tab_config("Ops", Some("ops.toml")),
        tab_config("Dev", Some("dev_b.toml")),
    ];

    assert_eq!(
        jump_list_tasks(&configs, "warppreview"),
        [
            task("New Window", "warppreview://action/new_window?path=~"),
            task("Dev", "warppreview://tab_config/dev_a"),
            task("Ops", "warppreview://tab_config/ops"),
            task("Dev (1)", "warppreview://tab_config/dev_b"),
        ]
    );
}

#[test]
fn tab_config_stems_are_encoded_for_the_deeplink_decoder() {
    let configs = [tab_config("Mine", Some("a+b & c=d.toml"))];

    assert_eq!(
        jump_list_tasks(&configs, "warp")[1].url,
        "warp://tab_config/a%2Bb%20%26%20c%3Dd"
    );
}

#[test]
fn configs_without_a_file_are_skipped_without_shifting_labels() {
    let configs = [tab_config("Dev", None), tab_config("Dev", Some("dev.toml"))];

    assert_eq!(
        jump_list_tasks(&configs, "warp")[1..],
        [task("Dev (1)", "warp://tab_config/dev")]
    );
}
