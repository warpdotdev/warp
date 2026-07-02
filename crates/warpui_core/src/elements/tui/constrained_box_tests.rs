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
