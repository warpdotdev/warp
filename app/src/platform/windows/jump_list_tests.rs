use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

fn config(name: &str, stem: &str) -> TabConfig {
    TabConfig {
        name: name.to_string(),
        title: None,
        color: None,
        panes: Vec::new(),
        params: HashMap::new(),
        source_path: Some(PathBuf::from(format!("C:\\configs\\{stem}.toml"))),
    }
}

fn scheme() -> String {
    ChannelState::url_scheme().to_string()
}

#[test]
fn empty_input_yields_empty_output() {
    let scheme = scheme();
    assert!(tab_configs_to_entries(&[], &scheme).is_empty());
}

#[test]
fn label_and_deeplink_use_name_and_stem() {
    let scheme = scheme();
    let configs = vec![config("My Config", "my_config")];
    let entries = tab_configs_to_entries(&configs, &scheme);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "My Config");
    assert_eq!(
        entries[0].deeplink,
        format!("{scheme}://tab_config/my_config")
    );
}

#[test]
fn entries_sort_by_display_name() {
    let scheme = scheme();
    let configs = vec![
        config("Zebra", "zebra"),
        config("Alpha", "alpha"),
        config("Mango", "mango"),
    ];
    let entries = tab_configs_to_entries(&configs, &scheme);
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, vec!["Alpha", "Mango", "Zebra"]);
}

#[test]
fn same_name_distinct_stems_stay_distinct() {
    let scheme = scheme();
    let configs = vec![config("Dup", "one"), config("Dup", "two")];
    let entries = tab_configs_to_entries(&configs, &scheme);
    let deeplinks: Vec<String> = entries.iter().map(|e| e.deeplink.clone()).collect();
    assert_eq!(deeplinks.len(), 2);
    assert!(deeplinks.contains(&format!("{scheme}://tab_config/one")));
    assert!(deeplinks.contains(&format!("{scheme}://tab_config/two")));
}
