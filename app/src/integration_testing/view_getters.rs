//! Various view getters with cardinality assumptions to reduce boilerplate
//! needed to get a view from the view tree.
//! We should migrate to these view getters because to make it easier to work
//! with tabs and panes. The old view getters use `tab_idx` without considering
//! how many panes are in each tab.
//! See https://github.com/warpdotdev/warp-internal/pull/4785#issue-1634862270

use warpui::integration::AssertionCallback;
use warpui::{App, Entity, View, ViewHandle, WindowId, async_assert};

use crate::ai_assistant::panel::AIAssistantPanelView;
use crate::input_suggestions::InputSuggestions;
use crate::notebooks::notebook::NotebookView;
use crate::pane_group::{PaneGroup, PaneView};
use crate::root_view::RootView;
use crate::search::command_palette::{self};
use crate::search::command_search::view::CommandSearchView;
use crate::settings_view::keybindings::KeybindingsView;
use crate::terminal::TerminalView;
use crate::terminal::input::Input;
use crate::themes::theme_chooser::ThemeChooser;
use crate::view_components::find::{Find, FindEvent, FindModel};
use crate::workflows::CategoriesView;
use crate::workflows::workflow_view::WorkflowView;
use crate::workspace::Workspace;

/// This identifier is useful when you'd like to weakly identify a terminal view
/// without actually grabbing a handle to it. Often useful when writing reusable assertions.
#[derive(Copy, Clone, Debug)]
pub enum TerminalViewIdentifier {
    /// There is only one terminal view in the entire window.
    Singleton,
    /// There is only one terminal view in the given tab index.
    SingleInTab { tab_index: usize },
    /// Fully-identified terminal view.
    Custom { tab_index: usize, pane_index: usize },
}

impl TerminalViewIdentifier {
    pub fn to_terminal_view(&self, app: &App, window_id: WindowId) -> ViewHandle<TerminalView> {
        use TerminalViewIdentifier::*;
        match self {
            Singleton => single_terminal_view(app, window_id),
            SingleInTab { tab_index } => single_terminal_view_for_tab(app, window_id, *tab_index),
            Custom {
                tab_index,
                pane_index,
            } => terminal_view(app, window_id, *tab_index, *pane_index),
        }
    }

    pub fn to_input_view(&self, app: &App, window_id: WindowId) -> ViewHandle<Input> {
        self.to_terminal_view(app, window_id)
            .read(app, |terminal, _ctx| terminal.input().to_owned())
    }
}

/// Panics if there isn't a single input suggestions view in the entire view hierarchy.
pub fn single_input_suggestions_view(
    app: &App,
    window_id: WindowId,
) -> ViewHandle<InputSuggestions> {
    singleton_view_of_type(app, window_id)
}

pub fn single_input_suggestions_view_for_tab(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> ViewHandle<InputSuggestions> {
    single_input_view_for_tab(app, window_id, tab_index)
        .read(app, |input, _| input.input_suggestions().to_owned())
}

/// Panics if there isn't a single find view in the entire view hierarchy.
pub fn single_find_view<T: FindModel + Entity<Event = FindEvent>>(
    app: &App,
    window_id: WindowId,
) -> ViewHandle<Find<T>> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single input view in the entire view hierarchy.
pub fn single_input_view(app: &App, window_id: WindowId) -> ViewHandle<Input> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single input view in the given tab.
pub fn single_input_view_for_tab(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> ViewHandle<Input> {
    single_terminal_view_for_tab(app, window_id, tab_index)
        .read(app, |terminal, _ctx| terminal.input().to_owned())
}

/// Panics if there isn't a single terminal view in the entire view hierarchy.
pub fn single_terminal_view(app: &App, window_id: WindowId) -> ViewHandle<TerminalView> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single terminal view in the given tab.
pub fn single_terminal_view_for_tab(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> ViewHandle<TerminalView> {
    pane_group_view(app, window_id, tab_index).read(app, |pane_group, ctx| {
        let num_terminal_views = pane_group.terminal_pane_ids().count();
        assert_eq!(num_terminal_views, 1, "window_id={window_id}, tab_index={tab_index} doesn't have a single terminal view. Has {num_terminal_views} terminal views instead");
        pane_group.terminal_view_at_pane_index(0, ctx).unwrap().to_owned()
    })
}

/// Panics if there isn't a single terminal pane view in the given tab.
pub fn single_terminal_pane_view_for_tab(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> ViewHandle<PaneView<TerminalView>> {
    pane_group_view(app, window_id, tab_index).read(app, |pane_group, _ctx| {
        let num_terminal_views = pane_group.terminal_pane_ids().count();
        assert_eq!(num_terminal_views, 1, "window_id={window_id}, tab_index={tab_index} doesn't have a single terminal pane view. Has {num_terminal_views} pane views instead");
        pane_group.terminal_pane_view_at_pane_index(0).unwrap().to_owned()
    })
}

/// Panics if there isn't a single terminal view in the given tab and pane index.
pub fn terminal_view(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> ViewHandle<TerminalView> {
    pane_group_view(app, window_id, tab_index).read(app, |pane_group, ctx| {
        pane_group.terminal_view_at_pane_index(pane_index, ctx).unwrap_or_else(|| panic!("terminal_view should exist for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index}")).to_owned()
    })
}

/// Panics if there isn't a single terminal view in the given tab and pane index.
pub fn input_view(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> ViewHandle<Input> {
    terminal_view(app, window_id, tab_index, pane_index)
        .read(app, |terminal_view, _| terminal_view.input().to_owned())
}

/// Panics if there isn't a notebook view at the given tab and pane index.
pub fn notebook_view(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> ViewHandle<NotebookView> {
    pane_group_view(app, window_id, tab_index).read(
        app,
        |pane_group, ctx| match pane_group.notebook_view_at_pane_index(pane_index, ctx) {
            Some(pane) => pane.clone(),
            None => panic!("notebook view should exist for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index}")
        },
    )
}

/// Panics if there isn't a workflow view at the given tab and pane index.
pub fn workflow_view(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> ViewHandle<WorkflowView> {
    pane_group_view(app, window_id, tab_index).read(
        app,
        |pane_group, ctx| match pane_group.workflow_view_at_pane_index(pane_index, ctx) {
            Some(pane) => pane.clone(),
            None => panic!("workflow view should exist for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index}")
        },
    )
}

/// Panics if there isn't a single pane group for the given tab.
pub fn pane_group_view(app: &App, window_id: WindowId, tab_index: usize) -> ViewHandle<PaneGroup> {
    workspace_view(app, window_id).read(app, |workspace, _ctx| {
        workspace
            .get_pane_group_view(tab_index)
            .unwrap_or_else(|| {
                panic!(
                    "pane_group view should exist for window_id={window_id}, tab_index={tab_index}"
                )
            })
            .to_owned()
    })
}

/// Panics if there isn't a single theme chooser view in the view hierarchy.
pub fn theme_chooser_view(app: &App, window_id: WindowId) -> ViewHandle<ThemeChooser> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single single keybindings view in the view hierarchy.
pub fn keybindings_view(app: &App, window_id: WindowId) -> ViewHandle<KeybindingsView> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single workflows view in the view hierarchy.
pub fn workflow_categories_view(app: &App, window_id: WindowId) -> ViewHandle<CategoriesView> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single ai assistant panel view in the view hierarchy.
pub fn ai_assistant_panel_view(app: &App, window_id: WindowId) -> ViewHandle<AIAssistantPanelView> {
    singleton_view_of_type(app, window_id)
}

/// Panics if there isn't a single workspace view in the view hierarchy.
pub fn workspace_view(app: &App, window_id: WindowId) -> ViewHandle<Workspace> {
    root_view(app, window_id).read(app, |root_view, _ctx| {
        root_view.workspace_view().cloned().unwrap_or_else(|| {
            panic!("root_view should have a workspace view for window_id={window_id}")
        })
    })
}

/// Panics if there isn't a single root view in the view hierarchy.
pub fn root_view(app: &App, window_id: WindowId) -> ViewHandle<RootView> {
    app.root_view(window_id)
        .unwrap_or_else(|| panic!("root view for window_id={window_id} does not exist"))
}

/// Returns a [`ViewHandle`] to the command palette contained within `WindowId`.
/// #Panics if there isn't a command palette view in the view hierarchy.
pub fn command_palette_view(app: &App, window_id: WindowId) -> ViewHandle<command_palette::View> {
    let workspace = singleton_view_of_type::<Workspace>(app, window_id);
    workspace.read(app, |workspace, _ctx| workspace.command_palette_view())
}

/// Returns a [`ViewHandle`] to the command search view contained within `WindowId`.
/// #Panics if there isn't a command search view view in the view hierarchy.
/// Note that the command search view is implemented at the workspace level, so there should only
/// be one in a given workspace view/window.
pub fn command_search_view(app: &App, window_id: WindowId) -> ViewHandle<CommandSearchView> {
    singleton_view_of_type(app, window_id)
}

pub fn assert_no_views_of_type<T: View>() -> AssertionCallback {
    Box::new(move |app, window_id| async_assert!(app.views_of_type::<T>(window_id).is_none()))
}

// Useful helper to get a singleton view (per window) without having to expose
// getters on Workspace, etc. Instead of using this directly in a test, write a helper
// to extract the view you want for better ergonomics.
fn singleton_view_of_type<T: View>(app: &App, window_id: WindowId) -> ViewHandle<T> {
    let views_of_type = app
        .views_of_type(window_id)
        .expect("there's at least one view of type");
    let num_views_of_type = views_of_type.len();
    assert_eq!(
        num_views_of_type, 1,
        "window_id={window_id} doesn't have a single view of type T. Has {num_views_of_type} views instead"
    );
    views_of_type.first().unwrap().clone()
}
