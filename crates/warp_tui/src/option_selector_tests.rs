use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::{
    Appearance, OptionBadge, OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityId, EntityIdMap};
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::keymap::Keystroke;
use warpui_core::{App, AppContext, TuiView as _, TypedActionView as _, ViewHandle};

use super::{
    OptionSelectorPage, SelectorItem, TuiOptionSelector, TuiOptionSelectorAction,
    TuiOptionSelectorEvent,
};
use crate::editor_element::TuiEditorAction;
use crate::editor_interaction::TuiEditorCommand;
use crate::editor_view::TuiEditorViewAction;
use crate::test_fixtures::TestHostView;
use crate::tui_builder::TuiUiBuilder;

/// Builds an enabled row with `id` used as the label.
fn row(id: &str) -> OptionRow {
    OptionRow {
        id: id.to_string(),
        label: id.to_string(),
        harness: None,
        badge: None,
        disabled_reason: None,
    }
}

/// Builds a disabled row carrying `reason`.
fn disabled_row(id: &str, reason: &str) -> OptionRow {
    OptionRow {
        disabled_reason: Some(reason.to_string()),
        ..row(id)
    }
}

/// Builds a `Ready` snapshot over `ids` with `selected` preselected.
fn snapshot(ids: &[&str], selected: Option<&str>) -> OptionSnapshot {
    snapshot_of(ids.iter().map(|id| row(id)).collect(), selected)
}

/// Builds a `Ready` snapshot from explicit rows.
fn snapshot_of(rows: Vec<OptionRow>, selected: Option<&str>) -> OptionSnapshot {
    OptionSnapshot {
        rows,
        selected_id: selected.map(str::to_string),
        status: OptionSourceStatus::Ready,
        footer: None,
    }
}

/// Builds one selector page with shared test metadata.
fn page(snapshot: OptionSnapshot, searchable: bool) -> OptionSelectorPage {
    OptionSelectorPage {
        field_label: "Host".to_string(),
        position: (4, 6),
        prompt: "Which host should run the agents?".to_string(),
        snapshot,
        searchable,
    }
}

type CapturedEvents = Rc<RefCell<Vec<TuiOptionSelectorEvent>>>;

/// The captured events with `LayoutInvalidated` filtered out, for tests that
/// assert on the primary confirmation flow.
fn primary_events(events: &CapturedEvents) -> Vec<TuiOptionSelectorEvent> {
    events
        .borrow()
        .iter()
        .filter(|event| **event != TuiOptionSelectorEvent::LayoutInvalidated)
        .cloned()
        .collect()
}

/// Adds a selector in a fresh TUI window and captures its emitted events.
fn add_selector(app: &mut App) -> (ViewHandle<TuiOptionSelector>, CapturedEvents) {
    app.add_singleton_model(|_| Appearance::mock());
    let selector = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, TuiOptionSelector::new)
    });
    let events: CapturedEvents = Rc::new(RefCell::new(Vec::new()));
    let events_for_subscription = events.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&selector, move |_, event, _| {
            events_for_subscription.borrow_mut().push(event.clone());
        });
    });
    (selector, events)
}

/// Sets the page under the shared test header.
fn set_page(app: &mut App, selector: &ViewHandle<TuiOptionSelector>, snapshot: OptionSnapshot) {
    selector.update(app, |selector, ctx| {
        selector.set_page(page(snapshot, false), ctx);
    });
}
#[test]
fn search_editor_is_created_only_for_searchable_pages() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["auto"], Some("auto")));
        assert!(selector.read(&app, |selector, _| selector.search_field.is_none()));

        set_searchable_page(&mut app, &selector, snapshot(&["auto"], Some("auto")));
        assert!(selector.read(&app, |selector, _| selector.search_field.is_some()));
    });
}

#[test]
fn searchable_page_starts_on_the_selected_row_and_digits_still_confirm() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(&["auto", "gpt-5", "claude"], Some("gpt-5")),
        );
        assert!(selected_line(&app, &selector).contains("(2) gpt-5"));
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Search:")));

        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(3),
        );
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "claude".to_string()
            }]
        );
    });
}

#[test]
fn up_from_top_focuses_search_and_down_returns_to_first_row() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(&["auto", "gpt-5"], Some("auto")),
        );

        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        assert!(app.read(|ctx| selector
            .as_ref(ctx)
            .search_field
            .as_ref()
            .expect("searchable page has an editor")
            .as_ref(ctx)
            .is_focused()));
        assert!(app.read(|ctx| selector
            .as_ref(ctx)
            .interaction
            .selection
            .selected_index()
            .is_none()));

        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        assert!(selected_line(&app, &selector).contains("(1) auto"));
    });
}

#[test]
fn search_and_last_option_wrap_in_both_directions() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(&["auto", "gpt-5", "claude"], Some("auto")),
        );

        // Up from the first option focuses Search; another Up wraps to the
        // last option.
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        assert!(selected_line(&app, &selector).contains("(3) claude"));

        // Down from the last option returns to Search.
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        assert!(app.read(|ctx| selector
            .as_ref(ctx)
            .search_field
            .as_ref()
            .expect("searchable page has an editor")
            .as_ref(ctx)
            .is_focused()));
        assert!(app.read(|ctx| selector
            .as_ref(ctx)
            .interaction
            .selection
            .selected_index()
            .is_none()));
    });
}
#[test]
fn focused_search_filters_including_digits_and_enter_confirms_top_match() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(&["auto (genius)", "gpt-5", "claude"], Some("auto (genius)")),
        );
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        edit_search(
            &mut app,
            &selector,
            TuiEditorAction::InsertText("gpt-5".to_string()),
        );

        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("gpt-5")));
        assert!(!lines.iter().any(|line| line.contains("auto (genius)")));
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "gpt-5".to_string()
            }]
        );
    });
}

#[test]
fn typing_a_letter_from_the_list_focuses_and_seeds_search() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(&["auto", "gpt-5"], Some("auto")),
        );
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::FocusSearchAndInsert('g'),
        );
        assert!(app.read(|ctx| selector
            .as_ref(ctx)
            .search_field
            .as_ref()
            .expect("searchable page has an editor")
            .as_ref(ctx)
            .is_focused()));
        assert_eq!(
            app.read(|ctx| selector
                .as_ref(ctx)
                .search_field
                .as_ref()
                .expect("searchable page has an editor")
                .as_ref(ctx)
                .text(ctx)),
            "g"
        );
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("gpt-5")));
    });
}

#[test]
fn search_no_matches_and_escape_clear_are_rendered_without_moving_the_field() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_searchable_page(
            &mut app,
            &selector,
            snapshot(
                &["auto", "gpt-5", "claude", "gemini", "pareto"],
                Some("auto"),
            ),
        );
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        edit_search(
            &mut app,
            &selector,
            TuiEditorAction::InsertText("zzz".to_string()),
        );
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("Search:")));
        assert!(lines.iter().any(|line| line.contains("No matches")));

        let consumed = selector.update(&mut app, |selector, ctx| selector.handle_back(ctx));
        assert!(consumed);
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("(1) auto")));
        assert!(lines.iter().any(|line| line.contains("Search:")));
    });
}

/// Sets a searchable page under the shared test header.
fn set_searchable_page(
    app: &mut App,
    selector: &ViewHandle<TuiOptionSelector>,
    snapshot: OptionSnapshot,
) {
    selector.update(app, |selector, ctx| {
        selector.set_page(page(snapshot, true), ctx);
    });
}

/// Applies a text-field action to the active custom-text child.
fn edit_custom_text(
    app: &mut App,
    selector: &ViewHandle<TuiOptionSelector>,
    action: TuiEditorViewAction,
) {
    let field = custom_text_field(app, selector);
    field.update(app, |field, ctx| field.handle_action(&action, ctx));
}
/// Returns the selector's custom-text editor for focus and action assertions.
fn custom_text_field(
    app: &App,
    selector: &ViewHandle<TuiOptionSelector>,
) -> ViewHandle<crate::editor_view::TuiEditorView> {
    selector.read(app, |selector, _| selector.custom_text.editor.clone())
}

/// Applies a shared editor action to the search child.
fn edit_search(app: &mut App, selector: &ViewHandle<TuiOptionSelector>, action: TuiEditorAction) {
    let editor = selector.read(app, |selector, _| {
        selector
            .search_field
            .clone()
            .expect("searchable page has an editor")
    });
    editor.update(app, |editor, ctx| {
        editor.handle_action(&TuiEditorViewAction::Editor(action), ctx);
    });
}

#[test]
fn set_page_recovers_a_selected_custom_text_value() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_custom_selection = snapshot(&["warp"], Some("my-host"));
        with_custom_selection.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });

        set_page(&mut app, &selector, with_custom_selection);

        let line = selected_line(&app, &selector);
        assert!(line.contains("my-host"));
        assert!(!line.contains("Custom host…"));
    });
}

/// Dispatches a selector action directly to the view.
fn act(app: &mut App, selector: &ViewHandle<TuiOptionSelector>, action: TuiOptionSelectorAction) {
    selector.update(app, |selector, ctx| selector.handle_action(&action, ctx));
}

/// Confirms the selected item through the selector-owned action.
fn confirm(app: &mut App, selector: &ViewHandle<TuiOptionSelector>) {
    act(app, selector, TuiOptionSelectorAction::ConfirmSelected);
}

/// Lays out the selector's element at `width`, returning it with its area.
fn laid_out_element(
    selector: &ViewHandle<TuiOptionSelector>,
    rendered_views: &mut EntityIdMap<Box<dyn TuiElement>>,
    width: u16,
    app: &AppContext,
) -> (Box<dyn TuiElement>, TuiRect) {
    let selector_ref = selector.as_ref(app);
    rendered_views.insert(
        selector_ref.custom_text.editor.id(),
        selector_ref.custom_text.editor.as_ref(app).render(app),
    );
    if let Some(search_field) = selector_ref.search_field.as_ref() {
        rendered_views.insert(search_field.id(), search_field.as_ref(app).render(app));
    }
    let mut element = selector_ref.render(app);
    let size = {
        let mut layout_ctx = TuiLayoutContext { rendered_views };
        element.layout(
            TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
            &mut layout_ctx,
            app,
        )
    };
    let area = TuiRect::new(0, 0, size.width.max(1), size.height.max(1));
    (element, area)
}

/// Renders the selector to a styled cell buffer at `width`.
fn render_buffer(app: &App, selector: &ViewHandle<TuiOptionSelector>, width: u16) -> TuiBuffer {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let (mut element, area) = laid_out_element(selector, &mut rendered_views, width, app);
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        }
        buffer
    })
}

/// Renders the selector to trimmed lines at `width`.
fn render_lines(app: &App, selector: &ViewHandle<TuiOptionSelector>, width: u16) -> Vec<String> {
    render_buffer(app, selector, width)
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

/// The rendered line for the selector's selected item.
fn selected_line(app: &App, selector: &ViewHandle<TuiOptionSelector>) -> String {
    let needle = app.read(|app| {
        let selector = selector.as_ref(app);
        let index = selector
            .interaction
            .selection
            .selected_index()
            .expect("a selected item");
        let digit = index - selector.interaction.scroll_offset + 1;
        let label = match selector.items()[index] {
            SelectorItem::Row(row_index) => selector.page.snapshot.rows[row_index].label.clone(),
            SelectorItem::Retry => "↻ Retry".to_string(),
            SelectorItem::CustomText => selector
                .custom_text
                .committed_value
                .clone()
                .or_else(|| match &selector.page.snapshot.footer {
                    Some(OptionFooter::CustomText { label }) => Some(label.clone()),
                    Some(OptionFooter::CreateNewAuthSecret) | None => None,
                })
                .expect("custom-text footer label"),
        };
        format!("({digit}) {label}")
    });
    render_lines(app, selector, 60)
        .into_iter()
        .find(|line| line.contains(&needle))
        .expect("a selected row")
}

#[test]
fn renders_field_label_position_prompt_and_initial_selection() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("b")));
        let lines = render_lines(&app, &selector, 60);
        // Header: field label, position in the current sequence, and prompt.
        assert!(lines[0].contains("Host"));
        assert!(lines[0].contains("←"));
        assert!(lines[0].contains("4 of 6"));
        assert!(lines[0].contains("→"));
        assert!(lines[0].ends_with("← 4 of 6 →"));
        assert!(lines[1].is_empty());
        assert!(lines[2].contains("Which host should run the agents?"));
        // The selection starts on the snapshot's current value.
        let selected = selected_line(&app, &selector);
        assert!(selected.contains("(2) b"));
        assert!(!selected.contains('❯'));

        let buffer = render_buffer(&app, &selector, 60);
        let builder = app.read(TuiUiBuilder::from_app);
        let selected = &buffer[(0, 4)];
        assert_eq!(
            selected.fg,
            builder
                .option_selector_selected_style()
                .fg
                .expect("selected option has a foreground")
        );
        assert!(selected.modifier.contains(Modifier::BOLD));
    });
}

#[test]
fn up_and_down_move_the_selection_and_enter_confirms_it() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        assert!(selected_line(&app, &selector).contains('b'));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        assert!(selected_line(&app, &selector).contains('a'));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "b".to_string()
            }],
        );
    });
}

#[test]
fn digits_confirm_the_corresponding_visible_row() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(3),
        );
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "c".to_string()
            }],
        );
    });
}

#[test]
fn digits_are_viewport_relative_in_scrolled_lists() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let ids: Vec<String> = (0..12).map(|i| format!("row-{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        set_page(&mut app, &selector, snapshot(&id_refs, Some("row-0")));
        // Scroll two rows down; digit 1 now confirms the third row,
        // and the clipped top renders an overflow marker.
        act(&mut app, &selector, TuiOptionSelectorAction::ScrollBy(2));
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.trim() == "↑"));
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(1),
        );
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "row-2".to_string()
            }],
        );
    });
}

#[test]
fn navigation_scrolls_to_keep_the_selection_visible() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let ids: Vec<String> = (0..12).map(|i| format!("row-{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        set_page(&mut app, &selector, snapshot(&id_refs, Some("row-0")));
        for _ in 0..9 {
            act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        }
        // The selection scrolled beyond the first viewport.
        assert!(selected_line(&app, &selector).contains("row-9"));
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.trim() == "↑"));
    });
}

#[test]
fn list_viewport_shows_six_rows_and_arrow_overflow_markers() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_page(
            &mut app,
            &selector,
            snapshot(&["a", "b", "c", "d", "e", "f", "g", "h"], Some("a")),
        );
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("(6) f")));
        assert!(!lines.iter().any(|line| line.contains("(7) g")));
        assert!(lines.iter().any(|line| line.trim() == "↓"));

        act(&mut app, &selector, TuiOptionSelectorAction::ScrollBy(2));
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.trim() == "↑"));
        assert!(lines.iter().any(|line| line.contains("(1) c")));
    });
}

#[test]
fn disabled_rows_are_selectable_but_not_confirmable() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(
            &mut app,
            &selector,
            snapshot_of(
                vec![
                    row("a"),
                    disabled_row("b", "Disabled by your administrator"),
                ],
                Some("a"),
            ),
        );
        // The disabled row can be selected and shows its reason
        // …
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        let line = selected_line(&app, &selector);
        assert!(line.contains('b'));
        assert!(line.contains("Disabled by your administrator"));
        // … but neither Enter, its digit, nor a click confirms it.
        confirm(&mut app, &selector);
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(2),
        );
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(primary_events(&events).is_empty());
    });
}

#[test]
fn loading_and_empty_states_render_non_selectable_status_rows() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut loading = snapshot(&["Skip (advanced)"], None);
        loading.status = OptionSourceStatus::Loading;
        set_page(&mut app, &selector, loading);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Loading…")));

        let mut empty = snapshot_of(Vec::new(), None);
        empty.status = OptionSourceStatus::Empty {
            message: "No harnesses available".to_string(),
        };
        set_page(&mut app, &selector, empty);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("No harnesses available")));
        // Nothing is confirmable in an empty list.
        confirm(&mut app, &selector);
        assert!(primary_events(&events).is_empty());
    });
}

#[test]
fn failed_state_offers_a_retry_row_that_emits_retry_requested() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut failed = snapshot(&["Skip (advanced)"], None);
        failed.status = OptionSourceStatus::Failed {
            message: "Unable to load secrets".to_string(),
        };
        set_page(&mut app, &selector, failed);
        let lines = render_lines(&app, &selector, 60);
        assert!(lines
            .iter()
            .any(|line| line.contains("Unable to load secrets")));
        assert!(lines.iter().any(|line| line.contains("Retry")));
        // The Retry row is reachable by keyboard.
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::RetryRequested],
        );
    });
}

#[test]
fn custom_text_editor_trims_validates_and_submits() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        // The footer renders and confirming it opens the one-line editor.
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Custom host…")));
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));

        // Whitespace-only input stays editable with a concise error
        //.
        edit_custom_text(
            &mut app,
            &selector,
            TuiEditorViewAction::Editor(TuiEditorAction::InsertChar(' ')),
        );
        confirm(&mut app, &selector);
        assert!(primary_events(&events).is_empty());
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Enter a value to continue.")));

        // Valid input is trimmed and submitted.
        for c in "my-host ".chars() {
            edit_custom_text(
                &mut app,
                &selector,
                TuiEditorViewAction::Editor(TuiEditorAction::InsertChar(c)),
            );
        }
        edit_custom_text(
            &mut app,
            &selector,
            TuiEditorViewAction::Command(TuiEditorCommand::Backspace),
        );
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::CustomTextSubmitted {
                value: "my-host".to_string()
            }],
        );
        assert!(!custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));
        let line = selected_line(&app, &selector);
        assert!(line.contains("my-host"));
        assert!(!line.contains("Custom host…"));

        // Editing the custom option again starts from the submitted value.
        confirm(&mut app, &selector);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Custom host…: my-host")));
    });
}

#[test]
fn back_cancels_custom_text_editing_before_leaving_the_page() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));
        // The first Back unwinds editing and is consumed; the next one isn't.
        let consumed = selector.update(&mut app, |selector, ctx| selector.handle_back(ctx));
        assert!(consumed);
        assert!(!custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));
        let consumed = selector.update(&mut app, |selector, ctx| selector.handle_back(ctx));
        assert!(!consumed);
    });
}

/// Arrow navigation leaves list state untouched while custom text owns focus.
#[test]
fn arrows_do_not_navigate_the_list_while_editing_custom_text() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["a", "b", "c", "d", "e", "f", "g", "h"], Some("a"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        let custom_text_index = 8;

        for action in [
            TuiOptionSelectorAction::MoveUp,
            TuiOptionSelectorAction::MoveDown,
        ] {
            act(
                &mut app,
                &selector,
                TuiOptionSelectorAction::SelectItem(custom_text_index),
            );
            let before = selector.read(&app, |selector, _| {
                (
                    selector.interaction.selection.selected_index(),
                    selector.interaction.scroll_offset,
                )
            });

            act(&mut app, &selector, action);

            let after = selector.read(&app, |selector, _| {
                (
                    selector.interaction.selection.selected_index(),
                    selector.interaction.scroll_offset,
                )
            });
            assert_eq!(after, before);
            assert!(custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));
        }
    });
}

#[test]
fn create_new_auth_secret_footer_is_ignored() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["Skip (advanced)"], None);
        with_footer.footer = Some(OptionFooter::CreateNewAuthSecret);
        set_page(&mut app, &selector, with_footer);
        // Resource creation is out of scope in the TUI: the
        // footer contributes no navigable item.
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .all(|line| !line.contains("New API key")));
    });
}

/// Page replacement invalidates any host-cached selector measurement.
#[test]
fn set_page_invalidates_layout() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);

        set_page(&mut app, &selector, snapshot(&["a"], Some("a")));

        assert!(events
            .borrow()
            .contains(&TuiOptionSelectorEvent::LayoutInvalidated));
    });
}
#[test]
fn layout_invalidated_is_emitted_only_when_overflow_markers_toggle() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(
            &mut app,
            &selector,
            snapshot(&["a", "b", "c", "d", "e", "f", "g", "h"], Some("a")),
        );
        events.borrow_mut().clear();

        // Moves within the viewport do not scroll, so nothing is emitted.
        for _ in 0..5 {
            act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        }
        assert!(!events
            .borrow()
            .contains(&TuiOptionSelectorEvent::LayoutInvalidated));

        // Scrolling past the viewport reveals the `↑` marker: one event.
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| **event == TuiOptionSelectorEvent::LayoutInvalidated)
                .count(),
            1,
        );
    });
}

#[test]
fn layout_invalidated_is_emitted_when_the_custom_text_error_row_toggles() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        events.borrow_mut().clear();

        // An empty submit adds the validation-error row.
        confirm(&mut app, &selector);
        assert!(events
            .borrow()
            .contains(&TuiOptionSelectorEvent::LayoutInvalidated));
        events.borrow_mut().clear();

        // Typing clears the error row.
        edit_custom_text(
            &mut app,
            &selector,
            TuiEditorViewAction::Editor(TuiEditorAction::InsertChar('x')),
        );
        assert!(events
            .borrow()
            .contains(&TuiOptionSelectorEvent::LayoutInvalidated));
    });
}

#[test]
fn snapshot_refresh_preserves_the_selected_row() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        // The selected row survives a catalog refresh that reorders rows
        //.
        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(snapshot(&["c", "a"], Some("a")), ctx);
        });
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "c".to_string()
            }],
        );
    });
}

#[test]
fn snapshot_refresh_falls_back_to_the_selected_value_when_the_selection_vanishes() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        // "b" disappears from the catalog; the selection falls back to the
        // snapshot's current value rather than silently confirming anything
        //.
        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(snapshot(&["a", "x"], Some("a")), ctx);
        });
        confirm(&mut app, &selector);
        assert_eq!(
            primary_events(&events),
            [TuiOptionSelectorEvent::Confirmed {
                id: "a".to_string()
            }],
        );
    });
}

/// Dispatches `event` to the selector's freshly rendered and painted element
/// tree, returning whether it was handled.
fn dispatch(app: &App, selector: &ViewHandle<TuiOptionSelector>, event: &TuiEvent) -> bool {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let (mut element, area) = laid_out_element(selector, &mut rendered_views, 60, app);
        // Paint so the tree retains geometry and the scene supports hit
        // testing during dispatch.
        let scene = {
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            Rc::new(paint_ctx.scene.clone())
        };
        let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(event, &mut event_ctx, app)
    })
}

/// Builds an unmodified key-down event for `key`.
fn key_down(key: &str) -> TuiEvent {
    TuiEvent::KeyDown {
        keystroke: Keystroke {
            key: key.to_string(),
            ..Default::default()
        },
        chars: String::new(),
        details: Default::default(),
        is_composing: false,
    }
}

#[test]
fn enter_and_numpad_enter_are_consumed_by_the_selector_element() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a"], Some("a")));
        for key in ["enter", "numpadenter"] {
            assert!(dispatch(&app, &selector, &key_down(key)), "{key}");
        }
    });
}

#[test]
fn paste_is_consumed_only_while_editing_custom_text() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        let paste = TuiEvent::Paste {
            text: "my-host\nsecond line".to_string(),
        };
        // Ignored while the list (not the editor) is active.
        assert!(!dispatch(&app, &selector, &paste));

        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(custom_text_field(&app, &selector).read(&app, |field, _| field.is_focused()));
        // The editor consumes the paste (only the first line's printable
        // characters are inserted; the editor is single-line).
        assert!(dispatch(&app, &selector, &paste));
        // The shared editor consumes the paste event, but its single-line
        // policy inserts nothing when the first line is empty.
        let control_only = TuiEvent::Paste {
            text: "\nsecond line".to_string(),
        };
        assert!(dispatch(&app, &selector, &control_only));
    });
}

#[test]
fn badges_render_next_to_their_rows() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let rows = vec![
            OptionRow {
                badge: Some(OptionBadge::Default),
                ..row("team-host")
            },
            OptionRow {
                badge: Some(OptionBadge::Connected),
                ..row("worker-1")
            },
            OptionRow {
                badge: Some(OptionBadge::Recent),
                ..row("old-host")
            },
        ];
        set_page(&mut app, &selector, snapshot_of(rows, Some("team-host")));
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("(default)")));
        assert!(lines.iter().any(|line| line.contains("(connected)")));
        assert!(lines.iter().any(|line| line.contains("(recent)")));
    });
}
