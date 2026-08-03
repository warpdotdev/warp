use super::*;

fn create_empty_element() -> Box<dyn Element> {
    Box::new(super::super::Empty::new())
}

fn create_header(width: TableColumnWidth) -> TableHeader {
    TableHeader::new(create_empty_element()).with_width(width)
}

fn create_test_state() -> TableStateHandle {
    TableStateHandle::new(0, |_, _| vec![])
}

// ============================================================================
// Construction Validation Tests
// ============================================================================

#[test]
fn test_table_construction_with_row_render_fn() {
    let state = TableStateHandle::new(3, |_, _| {
        vec![
            create_empty_element(),
            create_empty_element(),
            create_empty_element(),
        ]
    });
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Fixed(100.0)),
    ]);

    assert_eq!(table.total_row_count(), 3);
    assert_eq!(table.column_count(), 3);
}

// ============================================================================
// Column Width Computation Tests
// ============================================================================

#[test]
fn test_compute_column_widths_fixed_only() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Fixed(150.0)),
        create_header(TableColumnWidth::Fixed(50.0)),
    ]);

    let widths = table.compute_column_widths(500.0, &[0.0, 0.0, 0.0]);
    assert_eq!(widths, vec![100.0, 150.0, 50.0]);
}

#[test]
fn test_compute_column_widths_flex_only() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Flex(1.0)),
        create_header(TableColumnWidth::Flex(2.0)),
        create_header(TableColumnWidth::Flex(1.0)),
    ]);

    let widths = table.compute_column_widths(400.0, &[0.0, 0.0, 0.0]);
    assert_eq!(widths, vec![100.0, 200.0, 100.0]);
}

#[test]
fn test_compute_column_widths_fraction() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Fraction(0.25)),
        create_header(TableColumnWidth::Fraction(0.5)),
        create_header(TableColumnWidth::Fraction(0.25)),
    ]);

    let widths = table.compute_column_widths(400.0, &[0.0, 0.0, 0.0]);
    assert_eq!(widths, vec![100.0, 200.0, 100.0]);
}

#[test]
fn test_compute_column_widths_mixed() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Flex(1.0)),
        create_header(TableColumnWidth::Fraction(0.2)),
    ]);

    let widths = table.compute_column_widths(500.0, &[0.0, 0.0, 0.0]);
    assert_eq!(widths[0], 100.0);
    assert_eq!(widths[2], 100.0);
    assert_eq!(widths[1], 300.0);
}

#[test]
fn test_compute_column_widths_intrinsic_scaling() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Intrinsic),
        create_header(TableColumnWidth::Intrinsic),
    ]);

    let widths = table.compute_column_widths(100.0, &[150.0, 150.0]);
    assert_eq!(widths[0], 50.0);
    assert_eq!(widths[1], 50.0);
}

#[test]
fn test_compute_column_widths_intrinsic_no_scaling_needed() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0).with_headers(vec![
        create_header(TableColumnWidth::Intrinsic),
        create_header(TableColumnWidth::Intrinsic),
    ]);

    let widths = table.compute_column_widths(400.0, &[100.0, 100.0]);
    assert_eq!(widths[0], 100.0);
    assert_eq!(widths[1], 100.0);
}

#[test]
fn test_compute_column_widths_empty_table() {
    let state = create_test_state();
    let table = Table::new(state, 800.0, 500.0);

    let widths = table.compute_column_widths(500.0, &[]);
    assert!(widths.is_empty());
}

// ============================================================================
// TableStateHandle Tests
// ============================================================================

#[test]
fn test_table_state_handle_new() {
    let state = create_test_state();
    assert!(state.column_widths().is_empty());
}

#[test]
fn test_table_state_handle_shared_across_clones() {
    let state = create_test_state();
    let state_clone = state.clone();

    {
        let mut inner = state.inner.borrow_mut();
        inner.column_widths = vec![100.0, 200.0];
    }

    assert_eq!(state_clone.column_widths(), vec![100.0, 200.0]);
}

#[test]
fn test_table_state_handle_row_count() {
    let state = TableStateHandle::new(100, |_, _| vec![]);
    assert_eq!(state.row_count(), 100);

    state.set_row_count(200);
    assert_eq!(state.row_count(), 200);
}

// ============================================================================
// Helper Function Tests
// ============================================================================

#[test]
fn test_compute_column_lefts() {
    let widths = vec![100.0, 150.0, 50.0];
    let lefts = Table::compute_column_lefts(&widths);
    assert_eq!(lefts, vec![0.0, 100.0, 250.0]);
}

#[test]
fn test_compute_column_lefts_empty() {
    let widths: Vec<f32> = vec![];
    let lefts = Table::compute_column_lefts(&widths);
    assert!(lefts.is_empty());
}

#[test]
fn test_compute_column_lefts_single() {
    let widths = vec![100.0];
    let lefts = Table::compute_column_lefts(&widths);
    assert_eq!(lefts, vec![0.0]);
}

// ============================================================================
// TableConfig Tests
// ============================================================================

#[test]
fn test_table_config_default() {
    let config = TableConfig::default();
    assert_eq!(config.border_width, 1.0);
    assert_eq!(config.cell_padding, 8.0);
    assert!(config.row_background.alternating.is_none());
    assert_eq!(config.vertical_sizing, TableVerticalSizing::Viewported);
}

#[test]
fn test_table_column_width_default() {
    let width = TableColumnWidth::default();
    assert!(matches!(width, TableColumnWidth::Flex(1.0)));
}

// ============================================================================
// Virtualization Tests
// ============================================================================

#[test]
fn test_sumtree_initialization_with_row_count() {
    let state = TableStateHandle::new(100, |_, _| vec![]);
    state.set_row_count(100);

    let inner = state.inner.borrow();
    assert_eq!(inner.row_count, 100);
}

#[test]
fn test_sumtree_row_height_invalidation() {
    let state = TableStateHandle::new(10, |_, _| vec![]);

    {
        let mut inner = state.inner.borrow_mut();
        let mut tree = SumTree::new();
        for _ in 0..10 {
            tree.push(TableRowItem {
                height: Some(Pixels::new(50.0)),
            });
        }
        inner.rows = tree;
        inner.last_measured_row_index = 9;
    }

    state.invalidate_row_height(5);

    let inner = state.inner.borrow();
    let mut cursor = inner.rows.cursor::<RowCount, ()>();
    cursor.seek(&RowCount(5), sum_tree::SeekBias::Right);
    if let Some(item) = cursor.item() {
        assert!(item.height.is_none());
    }
    assert!(inner.last_measured_row_index <= 4);
}

#[test]
fn test_scroll_to_row_updates_scroll_offset() {
    let state = TableStateHandle::new(100, |_, _| vec![]);

    state.scroll_to_row(50, None);

    let inner = state.inner.borrow();
    assert_eq!(inner.scroll_top.row_index.0, 50);
    assert_eq!(inner.scroll_top.offset_from_start, Pixels::zero());
}

#[test]
fn test_scroll_to_row_with_offset() {
    let state = TableStateHandle::new(100, |_, _| vec![]);

    state.scroll_to_row(25, Some(Pixels::new(10.0)));

    let inner = state.inner.borrow();
    assert_eq!(inner.scroll_top.row_index.0, 25);
    assert_eq!(inner.scroll_top.offset_from_start, Pixels::new(10.0));
}

#[test]
fn test_approximate_height_with_measured_rows() {
    let state = TableStateHandle::new(10, |_, _| vec![]);

    {
        let mut inner = state.inner.borrow_mut();
        inner.header_height = Pixels::new(40.0);
        let mut tree = SumTree::new();
        for _ in 0..10 {
            tree.push(TableRowItem {
                height: Some(Pixels::new(30.0)),
            });
        }
        inner.rows = tree;
    }

    let inner = state.inner.borrow();
    let approx_height = inner.approximate_height();
    assert_eq!(approx_height, Pixels::new(340.0));
}

#[test]
fn test_approximate_height_estimates_unmeasured_rows() {
    let state = TableStateHandle::new(10, |_, _| vec![]);

    {
        let mut inner = state.inner.borrow_mut();
        inner.header_height = Pixels::new(40.0);
        let mut tree = SumTree::new();
        for i in 0..10 {
            tree.push(TableRowItem {
                height: if i < 5 { Some(Pixels::new(30.0)) } else { None },
            });
        }
        inner.rows = tree;
    }

    let inner = state.inner.borrow();
    let approx_height = inner.approximate_height();
    assert_eq!(approx_height, Pixels::new(340.0));
}

#[test]
fn test_visible_start_row_idx_tracking() {
    let state = TableStateHandle::new(100, |_, _| vec![]);

    {
        let mut inner = state.inner.borrow_mut();
        inner.visible_start_row_idx = 42;
    }

    let inner = state.inner.borrow();
    assert_eq!(inner.visible_start_row_idx, 42);
}

#[test]
fn test_max_scroll_offset_respects_viewport() {
    let state = TableStateHandle::new(10, |_, _| vec![]);

    {
        let mut inner = state.inner.borrow_mut();
        inner.header_height = Pixels::new(40.0);
        inner.viewport_height = Pixels::new(200.0);
        let mut tree = SumTree::new();
        for _ in 0..10 {
            tree.push(TableRowItem {
                height: Some(Pixels::new(50.0)),
            });
        }
        inner.rows = tree;
    }

    let inner = state.inner.borrow();
    let max_scroll_px = inner.header_height + inner.rows.summary().height - inner.viewport_height;
    assert!(max_scroll_px > Pixels::zero());
}

// ============================================================================
// Multiple Layout Tests
// ============================================================================

#[test]
fn test_visible_row_count_starts_at_zero() {
    let state = TableStateHandle::new(5, |_, _| {
        vec![create_empty_element(), create_empty_element()]
    });
    let table = Table::new(state, 400.0, 300.0).with_headers(vec![
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Fixed(100.0)),
    ]);

    assert_eq!(table.visible_row_count(), 0);
}

#[test]
fn test_children_vector_is_initially_empty() {
    let state = TableStateHandle::new(10, |_, _| {
        vec![create_empty_element(), create_empty_element()]
    });
    let table = Table::new(state, 400.0, 300.0).with_headers(vec![
        create_header(TableColumnWidth::Fixed(100.0)),
        create_header(TableColumnWidth::Fixed(100.0)),
    ]);

    assert_eq!(table.visible_row_count(), 0);
}
