use super::compact_footer_path;

#[test]
fn compact_footer_path_preserves_short_paths() {
    assert_eq!(compact_footer_path("/erica/project"), "/erica/project");
}

#[test]
fn compact_footer_path_elides_middle_components() {
    assert_eq!(compact_footer_path("~/Documents/GitHub/warp"), "~/…/warp");
    assert_eq!(compact_footer_path("/long/path/to/project"), "/…/project");
    assert_eq!(
        compact_footer_path(r"C:\Users\erica\project"),
        r"C:\…\project"
    );
}
