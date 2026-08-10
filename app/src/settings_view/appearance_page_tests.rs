use warpui::App;

use super::*;
use crate::settings_view::settings_page::FilteredPageType;
use crate::test_util::settings::initialize_settings_for_tests;

/// The Tabs category as `build_page` assembles it, minus the directory-tab-colors widget, which
/// needs a view only the page can create.
fn tabs_page(app: &AppContext) -> PageType<AppearanceSettingsPageView> {
    PageType::new_categorized(
        vec![Category::new(
            "Tabs",
            AppearanceSettingsPageView::tab_settings_widgets(app),
        )],
        None,
    )
}

fn visible_widget_count(page: &PageType<AppearanceSettingsPageView>) -> usize {
    let FilteredPageType::Categorized { categories, .. } = page.get_filtered() else {
        panic!("expected a Categorized page");
    };
    categories
        .iter()
        .map(|category| category.widgets.len())
        .sum()
}

#[test]
fn auto_group_tabs_search_term_isolates_its_widget() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.update(|ctx| {
            let mut page = tabs_page(ctx);
            assert!(visible_widget_count(&page) > 1);

            assert!(page.update_filter("grouping", ctx).is_truthy());
            assert_eq!(visible_widget_count(&page), 1);
        });
    });
}

#[test]
fn clearing_the_search_restores_every_tabs_widget() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.update(|ctx| {
            let mut page = tabs_page(ctx);
            let unfiltered = visible_widget_count(&page);

            page.update_filter("grouping", ctx);
            assert_eq!(visible_widget_count(&page), 1);

            page.update_filter("", ctx);
            assert_eq!(visible_widget_count(&page), unfiltered);
        });
    });
}

#[test]
fn auto_group_tabs_widget_is_not_built_when_the_flag_is_off() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.update(|ctx| {
            let mut page = tabs_page(ctx);
            assert!(!page.update_filter("grouping", ctx).is_truthy());
            assert_eq!(visible_widget_count(&page), 0);
        });
    });
}

#[test]
fn auto_group_tabs_widget_is_not_built_without_the_grouped_tabs_flag() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(false);
    let _auto_grouping_guard = FeatureFlag::AutoTabGrouping.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.update(|ctx| {
            let mut page = tabs_page(ctx);
            assert!(!page.update_filter("grouping", ctx).is_truthy());
            assert_eq!(visible_widget_count(&page), 0);
        });
    });
}
