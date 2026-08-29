use super::{LayoutLeaf, LayoutNode, PaneId, SplitStep, parse_window_layout, split_steps};

#[test]
fn parses_a_single_pane() {
    let layout = parse_window_layout("80x24,0,0,3").unwrap();
    assert_eq!(layout.pane_ids(), vec![PaneId::from("%3")]);
}

#[test]
fn parses_checksum_prefixed_side_by_side_split() {
    let layout = parse_window_layout("b25d,80x24,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
    assert_eq!(
        layout.pane_ids(),
        vec![PaneId::from("%0"), PaneId::from("%1")]
    );
    match layout {
        LayoutNode::Split {
            horizontal,
            children,
            ..
        } => {
            assert!(horizontal);
            assert_eq!(children.len(), 2);
        }
        LayoutNode::Leaf(_) => panic!("expected split"),
    }
}

#[test]
fn split_steps_emit_one_side_by_side_from_first_leaf() {
    let layout = parse_window_layout("80x24,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
    assert_eq!(
        split_steps(&layout),
        vec![SplitStep {
            parent: vec![PaneId::from("%0")],
            new_pane: PaneId::from("%1"),
            side_by_side: true,
            parent_size: 40,
            new_size: 39,
        }]
    );
}

#[test]
fn stacked_split_is_not_side_by_side() {
    let layout = parse_window_layout("80x24,0,0[40x12,0,0,0,40x11,0,13,2]").unwrap();
    assert_eq!(
        split_steps(&layout),
        vec![SplitStep {
            parent: vec![PaneId::from("%0")],
            new_pane: PaneId::from("%2"),
            side_by_side: false,
            parent_size: 12,
            new_size: 11,
        }]
    );
}

#[test]
fn nested_split_preserves_tree_order_and_sizes() {
    let layout =
        parse_window_layout("80x24,0,0{40x24,0,0,0,40x24,40,0[40x12,40,0,1,40x11,40,13,2]}")
            .unwrap();
    assert_eq!(
        layout.pane_ids(),
        vec![PaneId::from("%0"), PaneId::from("%1"), PaneId::from("%2")]
    );
    assert_eq!(
        split_steps(&layout),
        vec![
            SplitStep {
                parent: vec![PaneId::from("%0")],
                new_pane: PaneId::from("%1"),
                side_by_side: true,
                parent_size: 40,
                new_size: 40,
            },
            SplitStep {
                parent: vec![PaneId::from("%1")],
                new_pane: PaneId::from("%2"),
                side_by_side: false,
                parent_size: 12,
                new_size: 11,
            },
        ]
    );
}

#[test]
fn non_even_three_way_split_keeps_proportions() {
    let layout = parse_window_layout("90x24,0,0{30x24,0,0,0,20x24,31,0,1,39x24,52,0,2}").unwrap();
    assert_eq!(
        split_steps(&layout),
        vec![
            SplitStep {
                parent: vec![PaneId::from("%0")],
                new_pane: PaneId::from("%1"),
                side_by_side: true,
                parent_size: 30,
                new_size: 20,
            },
            SplitStep {
                parent: vec![PaneId::from("%1")],
                new_pane: PaneId::from("%2"),
                side_by_side: true,
                parent_size: 20,
                new_size: 39,
            },
        ]
    );
}

#[test]
fn nested_first_outer_child_splits_beside_the_subtree() {
    let layout =
        parse_window_layout("80x24,0,0{40x24,0,0[40x12,0,0,0,40x11,40,13,1],39x24,41,0,2}")
            .unwrap();
    assert_eq!(
        layout.pane_ids(),
        vec![PaneId::from("%0"), PaneId::from("%1"), PaneId::from("%2")]
    );
    assert_eq!(
        split_steps(&layout),
        vec![
            SplitStep {
                parent: vec![PaneId::from("%0")],
                new_pane: PaneId::from("%1"),
                side_by_side: false,
                parent_size: 12,
                new_size: 11,
            },
            SplitStep {
                parent: vec![PaneId::from("%0"), PaneId::from("%1")],
                new_pane: PaneId::from("%2"),
                side_by_side: true,
                parent_size: 40,
                new_size: 39,
            },
        ]
    );
}

#[test]
fn nested_middle_outer_child_splits_beside_the_subtree() {
    let layout = parse_window_layout(
        "80x24,0,0{20x24,0,0,0,40x24,21,0[40x12,21,0,1,40x11,21,13,2],19x24,62,0,3}",
    )
    .unwrap();
    assert_eq!(
        layout.pane_ids(),
        vec![
            PaneId::from("%0"),
            PaneId::from("%1"),
            PaneId::from("%2"),
            PaneId::from("%3"),
        ]
    );
    assert_eq!(
        split_steps(&layout),
        vec![
            SplitStep {
                parent: vec![PaneId::from("%0")],
                new_pane: PaneId::from("%1"),
                side_by_side: true,
                parent_size: 20,
                new_size: 40,
            },
            SplitStep {
                parent: vec![PaneId::from("%1")],
                new_pane: PaneId::from("%2"),
                side_by_side: false,
                parent_size: 12,
                new_size: 11,
            },
            SplitStep {
                parent: vec![PaneId::from("%1"), PaneId::from("%2")],
                new_pane: PaneId::from("%3"),
                side_by_side: true,
                parent_size: 40,
                new_size: 19,
            },
        ]
    );
}

#[test]
fn missing_from_layout_returns_stale_pane_ids() {
    let layout = parse_window_layout("80x24,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
    assert_eq!(
        super::missing_from_layout(
            &[PaneId::from("%0"), PaneId::from("%1"), PaneId::from("%9")],
            &layout,
        ),
        vec![PaneId::from("%9")]
    );
}

#[test]
fn leaf_geometry_is_preserved() {
    let layout = parse_window_layout("40x12,1,2,7").unwrap();
    match layout {
        LayoutNode::Leaf(LayoutLeaf {
            width,
            height,
            x,
            y,
            pane_id,
        }) => {
            assert_eq!((width, height, x, y), (40, 12, 1, 2));
            assert_eq!(pane_id, PaneId::from("%7"));
        }
        LayoutNode::Split { .. } => panic!("expected leaf"),
    }
}
