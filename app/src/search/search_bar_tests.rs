use settings::Setting as _;
use warpui::{platform::WindowStyle, App, SingletonEntity as _};

use super::{
    CreateQueryResultRendererFn, QueryResult, QueryResultIndex, QueryResultRenderer, SearchBar,
    SearchBarPlaceholder, SearchBarState, SearchMixer, SearchResultOrdering,
};
use crate::{
    localization,
    settings::{AppLanguage, LanguageSettings},
    test_util::terminal::initialize_app_for_terminal_view,
};

#[derive(Clone, Debug)]
struct TestAction;

fn unused_result_renderer(
    _result_index: QueryResultIndex,
    _result: QueryResult<TestAction>,
) -> QueryResultRenderer<TestAction> {
    unreachable!("search results are not rendered in this test")
}

#[test]
fn localized_placeholder_updates_after_language_change() {
    const PLACEHOLDER_KEY: &str = "search.command_search.placeholder";

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (_, search_bar) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mixer = ctx.add_model(|_| SearchMixer::<TestAction>::new());
            let state = ctx.add_model(|_| SearchBarState::new(SearchResultOrdering::TopDown));
            let create_renderer: CreateQueryResultRendererFn<TestAction> = unused_result_renderer;

            SearchBar::new(
                mixer,
                state,
                SearchBarPlaceholder::localized(PLACEHOLDER_KEY),
                create_renderer,
                ctx,
            )
        });

        let english_placeholder = search_bar.read(&app, |search_bar, ctx| {
            editor_placeholder_text(search_bar, ctx)
        });
        let expected_english = app.update(|ctx| localization::text_for_app(ctx, PLACEHOLDER_KEY));
        assert_eq!(
            english_placeholder.as_deref(),
            Some(expected_english.as_str())
        );

        search_bar.update(&mut app, |search_bar, ctx| {
            search_bar.editor_handle.update(ctx, |editor, ctx| {
                editor.set_placeholder_text("stale placeholder", ctx);
            });
        });
        app.update(crate::localization::notify_locale_changed);
        let refreshed_placeholder = search_bar.read(&app, |search_bar, ctx| {
            editor_placeholder_text(search_bar, ctx)
        });
        assert_eq!(
            refreshed_placeholder.as_deref(),
            Some(expected_english.as_str())
        );

        LanguageSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .app_language
                .set_value(AppLanguage::SimplifiedChinese, ctx)
                .expect("test app language should be configurable");
        });

        let chinese_placeholder = search_bar.read(&app, |search_bar, ctx| {
            editor_placeholder_text(search_bar, ctx)
        });
        let expected_chinese = app.update(|ctx| localization::text_for_app(ctx, PLACEHOLDER_KEY));
        assert_eq!(
            chinese_placeholder.as_deref(),
            Some(expected_chinese.as_str())
        );
        assert_ne!(english_placeholder, chinese_placeholder);
    });
}

fn editor_placeholder_text(
    search_bar: &SearchBar<TestAction>,
    ctx: &warpui::AppContext,
) -> Option<String> {
    search_bar.editor_handle.read(ctx, |editor, _| {
        editor.placeholder_text("").map(str::to_owned)
    })
}
