use super::TuiConstrainedBox;
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiFlex, TuiLayoutContext, TuiSize, TuiText,
};
use crate::{App, EntityIdMap};

/// A column of three single-row children (natural height 3).
fn three_row_column() -> Box<dyn TuiElement> {
    TuiFlex::column()
        .child(TuiText::new("A").finish())
        .child(TuiText::new("B").finish())
        .child(TuiText::new("C").finish())
        .finish()
}

#[test]
fn caps_child_height_to_max_rows() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut boxed = TuiConstrainedBox::new(three_row_column()).with_max_rows(2);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = boxed.layout(TuiConstraint::loose(TuiSize::new(1, 10)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(1, 2));
        });
    });
}

#[test]
fn passes_through_when_uncapped() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut boxed = TuiConstrainedBox::new(three_row_column());
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = boxed.layout(TuiConstraint::loose(TuiSize::new(1, 10)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(1, 3));
        });
    });
}

#[test]
fn min_cols_floors_narrow_child_to_minimum_width() {
    // A text child whose natural width (1 char) is less than min_cols (10);
    // the box should force it to the minimum.
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut boxed = TuiConstrainedBox::new(TuiText::new("A").finish()).with_min_cols(10);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = boxed.layout(
                TuiConstraint::loose(TuiSize::new(80, 24)),
                &mut ctx,
                app_ctx,
            );
            assert_eq!(size.width, 10, "min_cols should floor narrow child to 10");
        });
    });
}

#[test]
fn min_and_max_cols_pin_child_to_fixed_width() {
    // Setting min == max == 48 produces a fixed-width column regardless of
    // content width — the primary use-case for the zero-state text column.
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut boxed = TuiConstrainedBox::new(TuiText::new("short").finish())
                .with_min_cols(48)
                .with_max_cols(48);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = boxed.layout(
                TuiConstraint::loose(TuiSize::new(200, 24)),
                &mut ctx,
                app_ctx,
            );
            assert_eq!(size.width, 48, "min=max=48 should pin width to exactly 48");
        });
    });
}

#[test]
fn min_cols_clamps_gracefully_on_narrow_terminal() {
    // When the parent only offers 20 cols and min_cols=48, the box should
    // clamp the min to the available space (20) and not panic.
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut boxed = TuiConstrainedBox::new(TuiText::new("short").finish())
                .with_min_cols(48)
                .with_max_cols(48);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = boxed.layout(
                TuiConstraint::loose(TuiSize::new(20, 24)),
                &mut ctx,
                app_ctx,
            );
            assert_eq!(
                size.width, 20,
                "should clamp to available 20 cols on a narrow terminal"
            );
        });
    });
}
