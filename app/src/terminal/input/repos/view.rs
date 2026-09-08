//! Inline repos menu view for switching between indexed repos.

#[cfg(feature = "local_fs")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
#[cfg(feature = "local_fs")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "local_fs")]
use warpui::SingletonEntity;
use warpui::elements::ChildView;
use warpui::{Element, Entity, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::blocklist::agent_view::AgentViewController;
#[cfg(feature = "local_fs")]
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::search::data_source::{Query, QueryFilter};
use crate::search::mixer::{AddAsyncSourceOptions, SearchMixer};
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::repos::AcceptRepo;
#[cfg(feature = "local_fs")]
use crate::terminal::input::repos::data_source::GitSummaryCache;
use crate::terminal::input::repos::data_source::RepoMenuDataSource;
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

/// Events emitted by InlineReposMenuView.
#[derive(Debug, Clone)]
pub enum InlineReposMenuEvent {
    /// User accepted a repo (hit enter).
    NavigateToRepo { path: PathBuf },
    /// User dismissed the menu.
    Dismissed,
}

pub struct InlineReposMenuView {
    menu_view: ViewHandle<InlineMenuView<AcceptRepo>>,
    mixer: ModelHandle<SearchMixer<AcceptRepo>>,
    input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
    /// Git summaries shared with the data source, populated in the background.
    #[cfg(feature = "local_fs")]
    git_summaries: GitSummaryCache,
}

impl InlineReposMenuView {
    pub fn new(
        input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        let git_summaries: GitSummaryCache = Arc::new(Mutex::new(HashMap::new()));

        #[cfg(feature = "local_fs")]
        let data_source = {
            let git_summaries = git_summaries.clone();
            ctx.add_model(move |_| RepoMenuDataSource::new(git_summaries))
        };
        #[cfg(not(feature = "local_fs"))]
        let data_source = ctx.add_model(|_| RepoMenuDataSource::new());

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptRepo>::new();
            mixer.add_async_source(
                data_source.clone(),
                [QueryFilter::Repos],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: true,
                    run_when_unfiltered: false,
                },
                ctx,
            );
            mixer
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new(
                mixer.clone(),
                positioner.clone(),
                &input_suggestions_model,
                agent_view_controller,
                ctx,
            )
        });

        ctx.subscribe_to_view(&menu_view, |_, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem {
                item,
                cmd_or_ctrl_shift_enter: _,
            } => {
                ctx.emit(InlineReposMenuEvent::NavigateToRepo {
                    path: item.path.clone(),
                });
            }
            InlineMenuEvent::SelectedItem { .. }
            | InlineMenuEvent::NoResults
            | InlineMenuEvent::TabChanged => (),
            InlineMenuEvent::Dismissed => {
                ctx.emit(InlineReposMenuEvent::Dismissed);
            }
        });

        ctx.subscribe_to_model(
            &input_suggestions_model,
            |me, input_suggestions_model, event, ctx| {
                let InputSuggestionsModeEvent::ModeChanged { .. } = event;
                if input_suggestions_model.as_ref(ctx).is_repos_menu() {
                    me.mixer.update(ctx, |mixer, ctx| {
                        mixer.run_query(
                            repos_query(me.input_buffer_model.as_ref(ctx).current_value()),
                            ctx,
                        );
                    });
                    // Kick off the background git-summary load so branch/diff
                    // stats fill in without blocking the initial repo list.
                    #[cfg(feature = "local_fs")]
                    me.load_git_summaries_in_background(ctx);
                }
            },
        );

        ctx.subscribe_to_model(input_buffer_model, |me, _, event, ctx| {
            if me.input_suggestions_model.as_ref(ctx).is_repos_menu() {
                let InputBufferUpdateEvent { new_content, .. } = event;
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.run_query(repos_query(new_content), ctx);
                });
            }
        });

        Self {
            menu_view,
            mixer,
            input_suggestions_model,
            input_buffer_model: input_buffer_model.clone(),
            #[cfg(feature = "local_fs")]
            git_summaries,
        }
    }

    /// Loads git summaries (branch + diff stats) for all known repos in the
    /// background, then re-runs the query so the freshly-loaded data renders.
    ///
    /// The initial repo list is shown immediately without git data (see
    /// [`RepoMenuDataSource`]); this fills it in a moment later. We load all
    /// summaries and refresh once (rather than per-repo) to avoid repeatedly
    /// resetting the user's selection while data streams in.
    #[cfg(feature = "local_fs")]
    fn load_git_summaries_in_background(&self, ctx: &mut ViewContext<Self>) {
        let paths: Vec<PathBuf> = PersistedWorkspace::as_ref(ctx)
            .workspaces()
            .map(|m| m.path)
            .collect();
        if paths.is_empty() {
            return;
        }

        ctx.spawn(
            async move {
                use crate::util::git::get_repo_git_summary;
                let futures = paths.into_iter().map(|path| async move {
                    let summary = get_repo_git_summary(&path).await;
                    (path, summary)
                });
                futures::future::join_all(futures).await
            },
            move |me, results, ctx| {
                let mut updated = false;
                {
                    let mut cache = me.git_summaries.lock().unwrap_or_else(|e| e.into_inner());
                    for (path, summary) in results {
                        if let Some(summary) = summary {
                            cache.insert(path, summary);
                            updated = true;
                        }
                    }
                }

                // Only refresh if the repos menu is still open, so we don't
                // stomp on whatever the user is doing next.
                if updated && me.input_suggestions_model.as_ref(ctx).is_repos_menu() {
                    me.mixer.update(ctx, |mixer, ctx| {
                        mixer.run_query(
                            repos_query(me.input_buffer_model.as_ref(ctx).current_value()),
                            ctx,
                        );
                    });
                }
            },
        );
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_down(ctx));
    }

    pub fn accept_selected_item(&self, cmd_or_ctrl_enter: bool, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(cmd_or_ctrl_enter, ctx));
    }
}

/// Build a Query that includes the Repos filter so the async source runs.
fn repos_query(text: &str) -> Query {
    Query {
        text: text.to_owned(),
        filters: HashSet::from([QueryFilter::Repos]),
    }
}

impl View for InlineReposMenuView {
    fn ui_name() -> &'static str {
        "InlineReposMenuView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineReposMenuView {
    type Event = InlineReposMenuEvent;
}
