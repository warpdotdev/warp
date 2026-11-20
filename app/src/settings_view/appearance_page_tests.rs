use warpui::App;

use super::*;
use crate::settings_view::settings_page::{FilteredPageType, PageTitle};

#[test]
fn compact_horizontal_tabs_search_restores_rows_after_clearing() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut page = PageType::new_uncategorized(
                vec![
                    Box::new(CompactHorizontalTabsWidget::default()),
                    Box::new(TabIndicatorWidget::default()),
                    Box::new(PreserveActiveTabColorWidget::default()),
                ],
                Some(PageTitle::new("Tabs")),
            );

            page.update_filter("compact horizontal", ctx);
            let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
                panic!("expected widget list");
            };
            assert_eq!(widgets.len(), 1);
            assert_eq!(
                widgets[0].widget_id(),
                CompactHorizontalTabsWidget::static_widget_id()
            );

            page.update_filter("", ctx);
            let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
                panic!("expected widget list");
            };
            assert_eq!(widgets.len(), 3);
        });
    });
}
