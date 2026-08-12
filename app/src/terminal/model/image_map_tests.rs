use pathfinder_geometry::vector::Vector2F;

use super::*;

fn virtual_placement(cols: usize, rows: usize) -> VirtualPlacement {
    VirtualPlacement {
        cols,
        rows,
        image_size: Vector2F::new(cols as f32, rows as f32),
    }
}

#[test]
fn add_virtual_placement_replaces_the_previous_placement_for_the_image() {
    let mut map = ImageMap::default();

    map.add_virtual_placement(1, virtual_placement(4, 2));
    map.add_virtual_placement(1, virtual_placement(16, 4));

    let placement = map.virtual_placement(1).expect("placement should exist");
    assert_eq!(placement.cols, 16);
    assert_eq!(placement.rows, 4);
}

#[test]
fn evict_image_removes_only_its_virtual_placement() {
    let mut map = ImageMap::default();
    map.add_virtual_placement(1, virtual_placement(4, 2));
    map.add_virtual_placement(2, virtual_placement(8, 8));

    map.evict_image(1);

    assert!(map.virtual_placement(1).is_none());
    assert!(map.virtual_placement(2).is_some());
}

#[test]
fn evict_all_images_removes_virtual_placements() {
    let mut map = ImageMap::default();
    map.add_virtual_placement(1, virtual_placement(4, 2));

    map.evict_all_images();

    assert!(map.virtual_placement(1).is_none());
}
