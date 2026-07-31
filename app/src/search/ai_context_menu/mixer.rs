use std::collections::HashSet;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{ModelContext, ModelHandle};

use super::code::data_source::CodeSymbolCache;
use super::core::AIContextMenuCategory;
use crate::cloud_object::ObjectType;
use crate::code_review::diff_state::DiffMode;
use crate::search::data_source::{Query, QueryFilter};
#[cfg(not(target_family = "wasm"))]
use crate::search::mixer::AddAsyncSourceOptions;
use crate::search::mixer::SearchMixer;

/// Debounce applied to the file and code sources, whose work scales with repo size.
#[cfg(not(target_family = "wasm"))]
const LOCAL_SEARCH_DEBOUNCE: Duration = Duration::from_millis(50);

pub type AIContextMenuMixer = SearchMixer<AIContextMenuSearchableAction>;

/// Long-lived handles the `@` menu's data sources need but the surface-neutral
/// menu state cannot own.
pub struct AtContextMenuSourceContext {
    /// Cache backing the Code category's symbol search. `None` on surfaces that
    /// do not offer code symbols, and in WASM builds where the cache has no
    /// outline to read.
    pub code_symbol_cache: Option<ModelHandle<CodeSymbolCache>>,
    /// Directory the file and skill sources are scoped to. The GUI resolves this
    /// from the active window's session; the TUI passes its own session's
    /// directory, since it has no active window.
    pub working_directory: Option<LocalOrRemotePath>,
}

/// The query shape the `@` menu runs. Sources are selected by installation
/// rather than by filter, so every query is unfiltered.
pub fn at_context_menu_query(text: &str) -> Query {
    Query {
        text: text.to_owned(),
        filters: HashSet::new(),
    }
}

/// Installs the data sources backing a single category.
///
/// Callers reset the mixer first and run the query afterwards, so this is safe
/// to call once per category or in a loop across several.
#[cfg_attr(target_family = "wasm", allow(unused_variables))]
pub fn install_sources_for_category(
    mixer: &mut AIContextMenuMixer,
    category: AIContextMenuCategory,
    source_context: &AtContextMenuSourceContext,
    ctx: &mut ModelContext<AIContextMenuMixer>,
) {
    match category {
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::CurrentFolderFiles => {
            let source = super::files::data_source::file_data_source_for_pwd(
                source_context.working_directory.as_ref(),
                ctx,
            );
            mixer.add_async_source(source, [QueryFilter::Files], local_search_options(), ctx);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::RepoFiles => {
            mixer.add_async_source(
                super::files::data_source::file_data_source_for_current_repo(
                    source_context.working_directory.clone(),
                ),
                [QueryFilter::Files],
                local_search_options(),
                ctx,
            );
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Code => {
            let Some(cache) = &source_context.code_symbol_cache else {
                return;
            };
            mixer.add_async_source(
                super::code::data_source::code_data_source(cache.as_ref(ctx)),
                [QueryFilter::Code],
                local_search_options(),
                ctx,
            );
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Commands => {
            let source = ctx.add_model(|_| super::commands::data_source::CommandDataSource::new());
            mixer.add_sync_source(source, [QueryFilter::Commands]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Blocks => {
            let source = ctx.add_model(|_| super::blocks::data_source::BlockDataSource::new());
            mixer.add_sync_source(source, [QueryFilter::Blocks]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Workflows => {
            let source =
                ctx.add_model(|_| super::workflows::data_source::WorkflowDataSource::new());
            mixer.add_sync_source(source, [QueryFilter::Workflows]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Notebooks => {
            let source =
                ctx.add_model(|_| super::notebooks::data_source::NotebookDataSource::new(false));
            mixer.add_sync_source(source, [QueryFilter::Notebooks]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Plans => {
            let source =
                ctx.add_model(|_| super::notebooks::data_source::NotebookDataSource::new(true));
            mixer.add_sync_source(source, [QueryFilter::Notebooks]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Rules => {
            let source = ctx.add_model(|_| super::rules::data_source::RulesDataSource::new());
            mixer.add_sync_source(source, [QueryFilter::Rules]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::DiffSet => {
            let source = ctx.add_model(|_| super::diffset::data_source::DiffSetDataSource);
            mixer.add_sync_source(source, [QueryFilter::DiffSets]);
        }
        #[cfg(not(target_family = "wasm"))]
        AIContextMenuCategory::Skills => {
            let working_directory = source_context.working_directory.clone();
            let source = ctx.add_model(|_| {
                super::skills::data_source::SkillsDataSource::new(working_directory)
            });
            mixer.add_sync_source(source, [QueryFilter::Skills]);
        }
        AIContextMenuCategory::Conversations => {
            let source =
                ctx.add_model(|_| super::conversations::data_source::ConversationDataSource);
            mixer.add_sync_source(source, [QueryFilter::Conversations]);
        }
        // Categories in the enum that no data source backs yet. In WASM builds
        // this also absorbs every category above except Conversations, since
        // their sources are unavailable there.
        AIContextMenuCategory::Diffs
        | AIContextMenuCategory::Docs
        | AIContextMenuCategory::Tasks
        | AIContextMenuCategory::Servers
        | AIContextMenuCategory::Terminal
        | AIContextMenuCategory::Web
        | AIContextMenuCategory::RecentDiff
        | AIContextMenuCategory::RecentBlock => {}
        #[cfg(target_family = "wasm")]
        AIContextMenuCategory::CurrentFolderFiles
        | AIContextMenuCategory::RepoFiles
        | AIContextMenuCategory::Code
        | AIContextMenuCategory::Commands
        | AIContextMenuCategory::Blocks
        | AIContextMenuCategory::Workflows
        | AIContextMenuCategory::Notebooks
        | AIContextMenuCategory::Plans
        | AIContextMenuCategory::Rules
        | AIContextMenuCategory::DiffSet
        | AIContextMenuCategory::Skills => {}
    }
}

/// Installs the data sources for every category the menu currently offers, for
/// the all-categories search that spans all of them.
pub fn install_sources_for_all_categories(
    mixer: &mut AIContextMenuMixer,
    categories: &[AIContextMenuCategory],
    source_context: &AtContextMenuSourceContext,
    ctx: &mut ModelContext<AIContextMenuMixer>,
) {
    for category in categories {
        install_sources_for_category(mixer, *category, source_context, ctx);
    }
}

#[cfg(not(target_family = "wasm"))]
fn local_search_options() -> AddAsyncSourceOptions {
    AddAsyncSourceOptions {
        debounce_interval: Some(LOCAL_SEARCH_DEBOUNCE),
        run_in_zero_state: true,
        run_when_unfiltered: true,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AIContextMenuSearchableAction {
    InsertFilePath {
        /// This is the file path relative to the root of the current git
        /// repository. If this changes, this could break how we resolve
        /// the file path outside of AI mode, so just note the downstream
        /// dependencies.
        file_path: String,
    },
    InsertText {
        /// Text to insert into the input buffer.
        text: String,
    },
    InsertDriveObject {
        /// The type of the drive object (Workflow, Notebook, etc.)
        object_type: ObjectType,
        /// The UID of the drive object to insert as <object_type:{uid}>
        object_uid: String,
    },
    InsertPlan {
        /// The UID of the AI document to insert as <plan:{uid}>
        ai_document_uid: String,
    },
    InsertDiffSet {
        /// The diff mode indicating what base to compare against
        diff_mode: DiffMode,
    },
    InsertConversation {
        /// The conversation identifier to insert as <convo:{id}>.
        conversation_id: String,
    },
    InsertSkill {
        /// The skill name to insert as /{name} into the buffer.
        name: String,
    },
}
