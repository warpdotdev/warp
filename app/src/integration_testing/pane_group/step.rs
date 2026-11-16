use warpui::integration::TestStep;

use crate::integration_testing::view_getters::pane_group_view;
use crate::pane_group::tree::Direction;

/// Close a specific pane by its index within a tab.
pub fn close_pane_by_index(tab_index: usize, pane_index: usize) -> TestStep {
    TestStep::new("Close pane by index").with_action(move |app, _, _| {
        let active_window = app
            .read(|ctx| ctx.windows().active_window())
            .expect("no active window");

        let pg = pane_group_view(app, active_window, tab_index);

        let pane_id = pg.read(app, |pane_group, _ctx| {
            pane_group
                .pane_id_from_index(pane_index)
                .expect("missing pane index")
        });

        pg.update(app, |pane_group, ctx| {
            pane_group.close_pane(pane_id, ctx);
        });
    })
}

/// Move one pane relative to another by pane indices within a tab.
pub fn move_pane_by_indices(
    tab_index: usize,
    from_pane_index: usize,
    to_pane_index: usize,
    direction: Direction,
) -> TestStep {
    TestStep::new("Move pane by indices").with_action(move |app, _, _| {
        let active_window = app
            .read(|ctx| ctx.windows().active_window())
            .expect("window exists");

        let pg = pane_group_view(app, active_window, tab_index);

        let (from_id, to_id) = pg.read(app, |pane_group, _ctx| {
            let a = pane_group
                .pane_id_from_index(from_pane_index)
                .expect("missing from pane index");
            let b = pane_group
                .pane_id_from_index(to_pane_index)
                .expect("missing to pane index");
            (a, b)
        });

        pg.update(app, |pane_group, ctx| {
            pane_group.move_pane(from_id, to_id, direction, ctx);
        });
    })
}
