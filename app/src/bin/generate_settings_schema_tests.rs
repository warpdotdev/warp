use super::*;

#[test]
fn surface_annotation_matches_setting_schema_entry_metadata() {
    ensure_settings_linked();

    for entry in inventory::iter::<SettingSchemaEntry> {
        let surfaces = (entry.surfaces_fn)();
        let annotation = setting_surface_names(surfaces);
        let annotation_names: HashSet<&str> = annotation.iter().filter_map(Value::as_str).collect();

        assert_eq!(
            annotation_names.contains("gui"),
            surfaces.includes(SettingsMode::Gui),
            "GUI surface mismatch for {}",
            entry.storage_key
        );
        assert_eq!(
            annotation_names.contains("tui"),
            surfaces.includes(SettingsMode::Tui),
            "TUI surface mismatch for {}",
            entry.storage_key
        );
        assert_eq!(
            annotation_names.len(),
            usize::from(surfaces.includes(SettingsMode::Gui))
                + usize::from(surfaces.includes(SettingsMode::Tui)),
            "unexpected surface annotation for {}",
            entry.storage_key
        );
    }
}
