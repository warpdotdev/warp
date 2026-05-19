use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::search::command_palette::conversations::search::{
    ConversationMatchResult, ConversationSearcher, FuzzyConversationSearcher, MatchedConversation,
};
use crate::search::command_palette::conversations::search_item::{
    ConversationAction, ConversationSearchItem,
};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{DataSourceSearchError, Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::workspace::Workspace;
use itertools::Itertools;
use std::collections::HashMap;
use warpui::{AppContext, Entity};

/// Sections for grouping conversations in the command palette.
#[derive(Debug, PartialEq, Eq, Hash)]
enum ConversationSection {
    ActivePane,
    OtherActive,
    Past,
}

impl ConversationSection {
    fn title(&self) -> &'static str {
        match self {
            ConversationSection::ActivePane => "Active pane conversations",
            ConversationSection::OtherActive => "Other active conversations",
            ConversationSection::Past => "Past conversations",
        }
    }

    /// Returns the ordering of the sections for display in the command palette
    /// (the command palette renders items in reverse order).
    fn reverse_order() -> [ConversationSection; 3] {
        [
            ConversationSection::Past,
            ConversationSection::OtherActive,
            ConversationSection::ActivePane,
        ]
    }

    fn for_conversation(conversation: &ConversationNavigationData) -> Self {
        if conversation.is_historical() {
            ConversationSection::Past
        } else if conversation.is_in_active_pane {
            ConversationSection::ActivePane
        } else {
            ConversationSection::OtherActive
        }
    }
}

/// Data source that produces conversations for a user to navigate to.
pub struct DataSource {
    searcher: FuzzyConversationSearcher,
    /// Whether to include extra conversation actions (i.e. new conversation & fork conversation)
    add_conversation_actions: bool,
}

impl Default for DataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl DataSource {
    pub fn new() -> Self {
        Self {
            searcher: FuzzyConversationSearcher::new(),
            add_conversation_actions: true,
        }
    }

    /// Returns a [`QueryResult`] for a conversation identified by `conversation_id`. `None` if no result was
    /// found with the given ID.
    pub fn query_result(
        conversation_id: &AIConversationId,
        app: &AppContext,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        let all_conversations = ConversationNavigationData::all_conversations(app);

        all_conversations
            .into_iter()
            .find(|conversation| &conversation.id == conversation_id)
            .map(|conversation| {
                let search_item = ConversationSearchItem::new(ConversationAction::Resume(
                    Box::new(MatchedConversation {
                        conversation,
                        match_result: ConversationMatchResult::no_match(),
                    }),
                ));
                QueryResult::from(search_item)
            })
    }

    pub fn top_n(
        &self,
        limit: usize,
        app: &AppContext,
    ) -> impl Iterator<Item = QueryResult<<Self as SyncDataSource>::Action>> {
        self.searcher
            .searchable_conversations(app)
            .into_iter()
            .k_largest_by_key(limit, |conversation| conversation.last_updated)
            .map(|conversation| {
                QueryResult::from(ConversationSearchItem::new(ConversationAction::Resume(
                    Box::new(MatchedConversation {
                        conversation,
                        match_result: ConversationMatchResult::no_match(),
                    }),
                )))
            })
    }
}

/// Get the selected conversation in the focused pane.
fn selected_conversation_in_focused_pane(app: &AppContext) -> Option<&AIConversation> {
    app.windows().active_window().and_then(|window_id| {
        app.views_of_type::<Workspace>(window_id)
            .and_then(|views| views.first().cloned())
            .and_then(|workspace| {
                workspace.read(app, |workspace, workspace_ctx| {
                    workspace.active_tab_pane_group().read(
                        workspace_ctx,
                        |pane_group, pane_group_ctx| {
                            pane_group.focused_session_view(pane_group_ctx).and_then(
                                |terminal_view| {
                                    terminal_view
                                        .as_ref(pane_group_ctx)
                                        .ai_context_model()
                                        .as_ref(pane_group_ctx)
                                        .selected_conversation(app)
                                },
                            )
                        },
                    )
                })
            })
    })
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        // When the query is empty, we want to insert special separator items between historical conversations,
        // open conversations, conversations in the active pane, and the conversation action items (i.e. new conversation & fork conversation).
        let result = if query.text.trim().is_empty() {
            let conversations = self.searcher.searchable_conversations(app);
            let mut results = Vec::new();

            // Group conversations by section.
            let mut grouped: HashMap<ConversationSection, Vec<ConversationNavigationData>> =
                HashMap::new();
            for conversation in conversations {
                let section = ConversationSection::for_conversation(&conversation);
                grouped.entry(section).or_default().push(conversation);
            }
            grouped.values_mut().for_each(|group| group.sort());

            // The command palette renders items in reverse order, so we need to add the sections in reverse order
            // and add each separator item after all of the items in the section.
            for section in ConversationSection::reverse_order() {
                if let Some(conversations) = grouped.get(&section) {
                    if !conversations.is_empty() {
                        for conversation in conversations {
                            let matched_conversation = MatchedConversation {
                                conversation: conversation.clone(),
                                match_result: ConversationMatchResult::no_match(),
                            };
                            results.push(
                                ConversationSearchItem::new(ConversationAction::Resume(Box::new(
                                    matched_conversation,
                                )))
                                .into(),
                            );
                        }
                        results.push(SeparatorSearchItem::new(section.title().to_string()).into());
                    }
                }
            }

            Ok(results)
        } else {
            self.searcher
                .search(&query.text.trim().to_lowercase(), app)
                .map_err(|err| {
                    let search_error = DataSourceSearchError {
                        message: err.to_string(),
                    };
                    Box::new(search_error) as DataSourceRunErrorWrapper
                })
        };

        // When the query is empty, we want to add the "new conversation" and "fork conversation" items.
        if self.add_conversation_actions && query.text.trim().is_empty() {
            result.map(|mut results| {
                if !cfg!(target_family = "wasm") {
                    if let Some(conversation) = selected_conversation_in_focused_pane(app) {
                        // Only surface the fork option if the selected conversation is done.
                        if conversation.status().is_done() {
                            results.push(
                                ConversationSearchItem::new(ConversationAction::Fork {
                                    conversation_id: conversation.id(),
                                    title: conversation.title().unwrap_or_default().to_string(),
                                })
                                .into(),
                            );
                        }
                    }
                }
                results.push(ConversationSearchItem::new(ConversationAction::New).into());
                results
            })
        } else {
            result
        }
    }
}

impl Entity for DataSource {
    type Event = ();
}
