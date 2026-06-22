use ::virtual_fs::{Stub, VirtualFS};

use super::CursorThemeCrawler;

#[test]
fn test_no_themes_found() {
    VirtualFS::test("test_no_themes_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(crawler.determine_cursor_theme(), None);
    });
}

#[test]
fn test_default_theme_found() {
    VirtualFS::test("test_default_theme_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons/default/cursors");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}

#[test]
fn test_known_theme_found() {
    VirtualFS::test("test_known_theme_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons/Yaru/cursors");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(crawler.determine_cursor_theme(), Some("Yaru".to_string()));
    });
}

#[test]
fn test_default_theme_found_via_index() {
    VirtualFS::test("test_default_theme_found_via_index", |dirs, mut sandbox| {
        sandbox.mkdir("icons/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.with_files(vec![Stub::FileWithContent(
            "icons/default/index.theme",
            r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
        )]);
        let crawler: CursorThemeCrawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}

#[test]
fn test_default_theme_is_prioritized_over_known_theme() {
    VirtualFS::test(
        "test_default_theme_is_prioritized_over_known_theme",
        |dirs, mut sandbox| {
            sandbox.mkdir("icons/Darmok/cursors");
            sandbox.mkdir("icons/Yaru/cursors");
            sandbox.mkdir("icons/default");
            sandbox.with_files(vec![Stub::FileWithContent(
                "icons/default/index.theme",
                r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
            )]);
            let crawler = CursorThemeCrawler {
                directories: vec![dirs.tests().join("icons")],
            };

            assert_eq!(
                crawler.determine_cursor_theme(),
                Some("default".to_string())
            );
        },
    );
}

#[test]
fn test_multiple_directories() {
    VirtualFS::test("test_multiple_directories", |dirs, mut sandbox| {
        sandbox.mkdir("icons2/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.with_files(vec![Stub::FileWithContent(
            "icons/default/index.theme",
            r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
        )]);
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons"), dirs.tests().join("icons2")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}
#[test]
fn test_resolution_order() {
    VirtualFS::test("test_resolution_order", |dirs, mut sandbox| {
        sandbox.mkdir("icons2/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.mkdir("icons2/default");
        sandbox.with_files(vec![
            Stub::FileWithContent(
                "icons/default/index.theme",
                r#"
                [Icon Theme]
                Inherits=Jalad
                "#,
            ),
            Stub::FileWithContent(
                "icons2/default/index.theme",
                r#"
                [Icon Theme]
                Inherits=Darmok
                "#,
            ),
        ]);

        // Case 1: we find the index file in icons first.
        // The index file points to a non-existent theme Jalad,
        // so we return None
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons"), dirs.tests().join("icons2")],
        };

        assert_eq!(crawler.determine_cursor_theme(), None);

        // Case 2: we find the index file in icons first.
        // The index file points to the valid theme Darmok,
        // so we return Some("default")
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons2"), dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}
