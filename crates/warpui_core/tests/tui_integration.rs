//! Headless integration tests for the in-core TUI backend, ported from the
//! legacy `warpui_tui` crate's `repo_explorer_integration` tests.
//!
//! The legacy tests `#[path]`-included the repo_explorer example's view, which
//! wired up the real `repo_metadata` model. `repo_metadata` depends on
//! `warp_core`, which uses GUI-only `warpui_core` surface and therefore cannot
//! coexist with the `tui` feature in one build graph, so these tests use a
//! self-contained directory model instead — preserving the original shape:
//! a real model registered through the shared core, indexed asynchronously on
//! the background executor, observed by the root view, rendered through the
//! `TuiPresenter`, and navigated via typed actions dispatched through the
//! shared core.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, TuiBufferExt, TuiElement, TuiEventHandler, TuiFlex, TuiParentElement, TuiRect,
    TuiStyle, TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, ModelHandle, TuiView, TypedActionView, UpdateModel,
    ViewContext, ViewHandle, WindowId,
};

/// The indexing lifecycle of [`DirectoryModel`].
enum IndexState {
    Indexing,
    Indexed,
}

/// A minimal model holding a directory listing, indexed off-thread.
struct DirectoryModel {
    path: PathBuf,
    entries: Vec<(String, bool)>,
    state: IndexState,
}

impl Entity for DirectoryModel {
    type Event = ();
}

/// The typed action the view handles, dispatched through the shared core.
#[derive(Debug, Clone, Copy)]
enum NavAction {
    SelectNext,
    SelectPrev,
}

/// The root TUI view: a header + status sourced from the model, the entry
/// list with a selection marker, and key bindings that dispatch typed actions.
struct ExplorerView {
    model: ModelHandle<DirectoryModel>,
    selected: usize,
    quit: Rc<Cell<bool>>,
}

impl ExplorerView {
    fn entries(&self, ctx: &AppContext) -> Vec<(String, bool)> {
        self.model.as_ref(ctx).entries.clone()
    }

    fn selected(&self) -> usize {
        self.selected
    }
}

impl Entity for ExplorerView {
    type Event = ();
}

impl TuiView for ExplorerView {
    fn ui_name() -> &'static str {
        "ExplorerView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let model = self.model.as_ref(ctx);
        let entries = &model.entries;
        let selected = self.selected.min(entries.len().saturating_sub(1));

        let header_style = TuiStyle::default().add_modifier(Modifier::BOLD);
        let selected_style = TuiStyle::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let hint_style = TuiStyle::default().add_modifier(Modifier::DIM);

        let mut rows: Vec<Box<dyn TuiElement>> = Vec::new();
        rows.push(Box::new(
            TuiText::new(format!("explorer · {}", display_name(&model.path)))
                .with_style(header_style)
                .truncate(),
        ));
        let status = match model.state {
            IndexState::Indexing => "status: indexing…".to_owned(),
            IndexState::Indexed => {
                format!("status: indexed · {} entries", entries.len())
            }
        };
        rows.push(Box::new(TuiText::new(status).truncate()));
        rows.push(Box::new(TuiText::new(" ")));

        if entries.is_empty() {
            rows.push(Box::new(
                TuiText::new("(no indexed entries yet)").with_style(hint_style),
            ));
        }
        for (index, (name, is_dir)) in entries.iter().enumerate() {
            let marker = if index == selected { "› " } else { "  " };
            let suffix = if *is_dir { "/" } else { "" };
            let style = if index == selected {
                selected_style
            } else {
                TuiStyle::default()
            };
            rows.push(Box::new(
                TuiText::new(format!("{marker}{name}{suffix}"))
                    .with_style(style)
                    .truncate(),
            ));
        }

        rows.push(Box::new(TuiText::new(" ")));
        rows.push(Box::new(
            TuiText::new("j/↓ next · k/↑ prev · q quit")
                .with_style(hint_style)
                .truncate(),
        ));

        let body = TuiFlex::column().with_children(rows);

        // Wire keyboard input: navigation keys dispatch a typed action through
        // the shared core; quit keys flip the shared quit flag the runtime
        // polls.
        let quit_for_q = self.quit.clone();
        let quit_for_esc = self.quit.clone();
        let handler = TuiEventHandler::new(body.finish())
            .on_key("j", |_, ctx, _| {
                ctx.dispatch_typed_action(NavAction::SelectNext)
            })
            .on_key("down", |_, ctx, _| {
                ctx.dispatch_typed_action(NavAction::SelectNext)
            })
            .on_key("k", |_, ctx, _| {
                ctx.dispatch_typed_action(NavAction::SelectPrev)
            })
            .on_key("up", |_, ctx, _| {
                ctx.dispatch_typed_action(NavAction::SelectPrev)
            })
            .on_key("q", move |_, _, _| quit_for_q.set(true))
            .on_key("escape", move |_, _, _| quit_for_esc.set(true));

        Box::new(handler)
    }
}

impl TypedActionView for ExplorerView {
    type Action = NavAction;

    fn handle_action(&mut self, action: &NavAction, ctx: &mut ViewContext<Self>) {
        let count = self.entries(ctx).len();
        if count == 0 {
            return;
        }
        match action {
            NavAction::SelectNext => {
                self.selected = (self.selected + 1).min(count - 1);
            }
            NavAction::SelectPrev => {
                self.selected = self.selected.saturating_sub(1);
            }
        }
        // Mark the view dirty so the runtime repaints with the new selection.
        ctx.notify();
    }
}

/// The display name for a directory entry: its final path component, falling
/// back to the full path when there is no file name.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Scans `path` into (display name, is_dir) entries — directories first, then
/// alphabetically — so the rendered list is deterministic and navigable.
fn scan(path: &Path) -> Vec<(String, bool)> {
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| {
                    let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
                    (entry.file_name().to_string_lossy().into_owned(), is_dir)
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
}

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// Registers the model, indexes `dir` on the background executor and awaits
/// the scan on the shared runtime, then installs the root view (observing the
/// model so it redraws on change).
async fn bootstrap(
    app: &mut App,
    dir: PathBuf,
    quit: Rc<Cell<bool>>,
) -> (WindowId, ViewHandle<ExplorerView>) {
    let model = app.add_model(|_| DirectoryModel {
        path: dir.clone(),
        entries: Vec::new(),
        state: IndexState::Indexing,
    });

    // Index on the background executor — the same `spawn` plumbing GUI views
    // use — then apply the results through the shared core, notifying
    // observers.
    let scan_dir = dir.clone();
    let (tx, rx) = futures::channel::oneshot::channel();
    app.background_executor()
        .spawn(async move {
            let _ = tx.send(scan(&scan_dir));
        })
        .detach();
    let entries = rx.await.expect("the background scan completes");
    app.update(|ctx| {
        ctx.update_model(&model, |model, mctx| {
            model.entries = entries;
            model.state = IndexState::Indexed;
            mctx.notify();
        });
    });

    let model_for_view = model.clone();
    let (window_id, root) = app.update(|ctx| {
        ctx.add_tui_window(window_options(), |view_ctx| {
            // Redraw whenever the model changes — the same observation
            // primitive GUI views use.
            view_ctx.observe(&model_for_view, |_view, _model, ctx| ctx.notify());
            ExplorerView {
                model: model_for_view.clone(),
                selected: 0,
                quit: quit.clone(),
            }
        })
    });

    (window_id, root)
}

#[test]
fn buffer_reflects_real_model_state() {
    App::test((), |mut app| async move {
        let dir = std::env::current_dir().expect("cwd");
        let quit = Rc::new(Cell::new(false));

        let (_window_id, root) = bootstrap(&mut app, dir.clone(), quit).await;

        // Render into a buffer tall enough to hold the indexed entries.
        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 100, 120);
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let text = frame.buffer.to_lines().join("\n");

        assert!(
            text.contains(&display_name(&dir)),
            "buffer should render the header sourced from the model:\n{text}"
        );
        assert!(
            text.contains("status: indexed"),
            "buffer should reflect the model's indexed state:\n{text}"
        );

        // The rendered list must be sourced from the model: take the model's
        // own first entry and assert it appears in the painted buffer.
        let (first_name, entry_count) = app.read(|ctx| {
            root.read(ctx, |view, ctx| {
                let entries = view.entries(ctx);
                (entries.first().map(|(name, _)| name.clone()), entries.len())
            })
        });
        assert!(entry_count > 0, "the indexed directory should have entries");
        let first_name = first_name.expect("there is at least one entry");
        assert!(
            text.contains(&first_name),
            "buffer should render the model's first entry {first_name:?}:\n{text}"
        );
    });
}

#[test]
fn typed_nav_action_changes_rendered_buffer() {
    App::test((), |mut app| async move {
        let dir = std::env::current_dir().expect("cwd");
        let quit = Rc::new(Cell::new(false));

        let (window_id, root) = bootstrap(&mut app, dir, quit).await;

        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 80, 40);

        // First frame: selection starts at entry 0.
        let before = app.update(|ctx| presenter.present(ctx, &root, area));
        let before_lines = before.buffer.to_lines();
        let selected_before = app.read(|ctx| root.read(ctx, |view, _| view.selected()));
        assert_eq!(
            selected_before, 0,
            "selection should start at the first entry"
        );

        // Dispatch the typed action through the shared core, exactly as the
        // runtime does when a navigation key is pressed.
        app.dispatch_typed_action(window_id, &[root.id()], &NavAction::SelectNext);

        let selected_after = app.read(|ctx| root.read(ctx, |view, _| view.selected()));
        assert_eq!(
            selected_after, 1,
            "SelectNext dispatched through the shared core should advance the selection"
        );

        // Second frame: re-present; the runtime would call invalidate() first
        // but the standalone presenter falls back to a fresh render.
        let after = app.update(|ctx| presenter.present(ctx, &root, area));
        let after_lines = after.buffer.to_lines();
        assert_ne!(
            before_lines, after_lines,
            "the typed action should change the rendered buffer"
        );

        // The selection marker '›' should sit on a different row after
        // navigation.
        let marker_row = |lines: &[String]| lines.iter().position(|line| line.contains('›'));
        assert_ne!(
            marker_row(&before_lines),
            marker_row(&after_lines),
            "the selection marker should move to a different row"
        );
    });
}
