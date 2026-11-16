use std::collections::HashMap;

use itertools::Itertools;
use warpui::{AppContext, SingletonEntity};

use super::{WorkflowIdentity, WorkflowSearchItem};
use crate::completer::SessionContext;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::command_search::settings::CommandSearchSettings;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::user_config::WarpConfig;
use crate::workflows::local_workflows::LocalWorkflows;
#[cfg(feature = "local_fs")]
use crate::workflows::local_workflows::UseCache;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowSource, WorkflowType};

pub struct WorkflowsDataSource {
    /// Contains workflows keyed by WorkflowSource.
    workflows_by_source: HashMap<WorkflowSource, Vec<Workflow>>,
}

impl WorkflowsDataSource {
    /// Creates a new WorkflowsDataSource containing team, project, local, and global workflows.
    pub fn new(session_context: Option<&SessionContext>, app: &mut AppContext) -> Self {
        let mut workflows_by_source: HashMap<WorkflowSource, Vec<Workflow>> = HashMap::new();

        let global_workflows = LocalWorkflows::as_ref(app)
            .global_workflows(session_context.map(|context| context.session.clone()))
            .cloned()
            .collect_vec();

        workflows_by_source.insert(WorkflowSource::Global, global_workflows);

        let user_workflows = WarpConfig::as_ref(app).local_user_workflows().clone();
        workflows_by_source.insert(WorkflowSource::Local, user_workflows);

        #[cfg(feature = "local_fs")]
        if let Some(session_context) = session_context
            && session_context.session.is_local()
        {
            let project_workflows =
                LocalWorkflows::handle(app).update(app, move |local_workflows, _| {
                    if let Ok(working_directory) = std::path::PathBuf::try_from(
                        session_context.current_working_directory.clone(),
                    ) {
                        local_workflows
                            .project_workflows(&working_directory, UseCache::No)
                            .cloned()
                            .collect_vec()
                    } else {
                        log::warn!(
                            "Unable to convert session working directory into OS-native path"
                        );
                        Vec::new()
                    }
                });
            workflows_by_source.insert(WorkflowSource::Project, project_workflows);
        }

        workflows_by_source.insert(
            WorkflowSource::App,
            LocalWorkflows::as_ref(app)
                .app_workflows()
                .map(Clone::clone)
                .collect_vec(),
        );

        Self {
            workflows_by_source,
        }
    }
}

impl SyncDataSource for WorkflowsDataSource {
    type Action = CommandSearchItemAction;

    /// Runs fuzzy matching of the query against all workflows (specifically, against their names, command and descriptions).
    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        // Only show global workflows in command search if there is no toggle in settings OR
        // there is a toggle and it's enabled.
        let command_search_settings = CommandSearchSettings::as_ref(app);
        let include_global_workflows =
            *command_search_settings.show_global_workflows_in_universal_search;
        let query_str = query.text.as_str();
        Ok(self
            .workflows_by_source
            .clone()
            .into_iter()
            .filter(|(source, _)| *source != WorkflowSource::Global || include_global_workflows)
            .flat_map(move |(source, workflows)| {
                workflows.into_iter().filter_map(
                    move |workflow| -> Option<QueryResult<Self::Action>> {
                        FuzzyMatchWorkflowResult::try_match(query_str, &workflow, "").map(
                            |match_result| {
                                WorkflowSearchItem {
                                    identity: WorkflowIdentity::Local(Box::new(
                                        WorkflowType::Local(workflow),
                                    )),
                                    source,
                                    fuzzy_matched_workflow: match_result,
                                }
                                .into()
                            },
                        )
                    },
                )
            })
            .collect_vec())
    }
}
