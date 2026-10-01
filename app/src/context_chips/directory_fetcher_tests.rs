use super::*;

fn create_directory_item(name: &str, directory_type: DirectoryType) -> DirectoryItem {
    DirectoryItem {
        name: name.to_string(),
        directory_type,
    }
}

#[test]
fn test_sort_comparison_total_order() {
    // Test that sort_menu_items produces consistent ordering that prevents panics
    // by verifying the expected Directory < TextFile < OtherFile hierarchy

    // Test basic type ordering with different combinations
    let mut items1 = vec![
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("text.txt", DirectoryType::TextFile),
    ];
    sort_menu_items(&mut items1);
    assert_eq!(items1[0].directory_type, DirectoryType::Directory);
    assert_eq!(items1[1].directory_type, DirectoryType::TextFile);

    let mut items2 = vec![
        create_directory_item("text.txt", DirectoryType::TextFile),
        create_directory_item("binary.exe", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut items2);
    assert_eq!(items2[0].directory_type, DirectoryType::TextFile);
    assert_eq!(items2[1].directory_type, DirectoryType::OtherFile);

    let mut items3 = vec![
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("binary.exe", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut items3);
    assert_eq!(items3[0].directory_type, DirectoryType::Directory);
    assert_eq!(items3[1].directory_type, DirectoryType::OtherFile);

    // Test that sort_menu_items is consistent - calling it multiple times
    // on the same data should produce the same result
    let test_items = vec![
        create_directory_item("binary.exe", DirectoryType::OtherFile),
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("text.txt", DirectoryType::TextFile),
    ];

    let mut items_copy1 = test_items.clone();
    let mut items_copy2 = test_items.clone();

    sort_menu_items(&mut items_copy1);
    sort_menu_items(&mut items_copy2);

    // Both sorts should produce identical results
    assert_eq!(items_copy1, items_copy2);

    // Verify the expected ordering: Directory, TextFile, OtherFile
    assert_eq!(items_copy1[0].directory_type, DirectoryType::Directory);
    assert_eq!(items_copy1[1].directory_type, DirectoryType::TextFile);
    assert_eq!(items_copy1[2].directory_type, DirectoryType::OtherFile);
}

#[test]
fn test_sort_same_types_alphabetically() {
    let mut dirs = vec![
        create_directory_item("zebra", DirectoryType::Directory),
        create_directory_item("alpha", DirectoryType::Directory),
        create_directory_item("beta", DirectoryType::Directory),
    ];
    sort_menu_items(&mut dirs);
    assert_eq!(dirs[0].name, "alpha");
    assert_eq!(dirs[1].name, "beta");
    assert_eq!(dirs[2].name, "zebra");

    let mut texts = vec![
        create_directory_item("z.txt", DirectoryType::TextFile),
        create_directory_item("a.rs", DirectoryType::TextFile),
        create_directory_item("m.py", DirectoryType::TextFile),
    ];
    sort_menu_items(&mut texts);
    assert_eq!(texts[0].name, "a.rs");
    assert_eq!(texts[1].name, "m.py");
    assert_eq!(texts[2].name, "z.txt");

    let mut others = vec![
        create_directory_item("z.bin", DirectoryType::OtherFile),
        create_directory_item("a.exe", DirectoryType::OtherFile),
        create_directory_item("m.dll", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut others);
    assert_eq!(others[0].name, "a.exe");
    assert_eq!(others[1].name, "m.dll");
    assert_eq!(others[2].name, "z.bin");
}

#[test]
fn test_sort_single_item() {
    let mut items = vec![create_directory_item("single", DirectoryType::Directory)];
    sort_menu_items(&mut items);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "single");
}
