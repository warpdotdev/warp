use std::{collections::HashMap, path::Path, sync::Arc};

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use chrono::Local;
use lazy_static::lazy_static;
use regex::Regex;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

#[cfg(not(target_family = "wasm"))]
use crate::remote_server::codebase_index_model::RemoteCodebaseIndexModel;
use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, AIAgentAttachment, AIAgentContext,
            DocumentContentAttachmentSource, DriveObjectPayload,
        },
        block_context::BlockContext,
        blocklist::{BlocklistAIContextModel, SessionContext},
        document::ai_document_model::{AIDocumentId, AIDocumentModel},
        facts::CloudAIFactModel,
        skills::list_skills_if_changed,
    },
    cloud_object::{
        model::{
            generic_string_model::{CloudStringObject, GenericStringObjectId},
            persistence::CloudModel,
        },
        GenericCloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType,
    },
    terminal::{
        model::{block::BlockId, session::active_session::ActiveSession},
        TerminalView,
    },
};
use warp_graphql::generic_string_object::GenericStringObjectFormat as GraphQLFormat;

lazy_static! {
    // Regex to match <block:[block_id]> patterns
    pub static ref BLOCK_CONTEXT_ATTACHMENT_REGEX: Regex = Regex::new(r"<block:([^>]+)>")
        .expect("Block context attachment regex should be parsed");
    // Regex to match warp drive objects inserted via at-context. Ex: <notebook:[workflow_id]>
    pub static ref DRIVE_OBJECT_ATTACHMENT_REGEX: Regex = Regex::new(r"<(workflow|notebook|plan|rule):([^>]+)>")
        .expect("Drive object attachment regex should be parsed");
    // Regex to match <change:filename:line_start-line_end> patterns
    pub static ref DIFF_HUNK_ATTACHMENT_REGEX: Regex = Regex::new(r"<change:([^>]+)>")
        .expect("Diff hunk attachment regex should be parsed");
}

// Returns the context to be attached to the AIAgentInput sent in a request.
// If `is_user_query` is true, includes selected blocks, text, and images from the context model.
// Always includes base context like current time, execution environment, and codebase info.
pub(super) fn input_context_for_request(
    is_user_query: bool,
    context_model: &BlocklistAIContextModel,
    active_session: &ActiveSession,
    conversation_id: Option<AIConversationId>,
    additional_context: Vec<AIAgentContext>,
    app: &AppContext,
) -> Arc<[AIAgentContext]> {
    let mut context = context_model.pending_context(app, is_user_query);

    context.push(AIAgentContext::CurrentTime {
        current_time: Local::now(),
    });

    if let Some(env) = active_session.ai_execution_environment(app) {
        context.push(AIAgentContext::ExecutionEnvironment(env));
    }

    if FeatureFlag::FullSourceCodeEmbedding.is_enabled()
        && FeatureFlag::CrossRepoContext.is_enabled()
    {
        let session_context = SessionContext::from_session(active_session, app);
        if session_context.is_remote() {
            add_remote_codebase_context(&mut context, app);
        } else {
            add_local_codebase_context(&mut context, app);
        }
    }

    if FeatureFlag::ListSkills.is_enabled() {
        let skills = list_skills_if_changed(
            active_session.current_working_directory().map(Path::new),
            conversation_id,
            app,
        );

        if let Some(skills) = skills {
            context.push(AIAgentContext::Skills { skills });
        }
    }

    context.extend(additional_context);

    context.into()
}

fn add_local_codebase_context(context: &mut Vec<AIAgentContext>, app: &AppContext) {
    for (codebase_path, status) in
        CodebaseIndexManager::as_ref(app).get_codebase_index_statuses(app)
    {
        // TODO(daniel): We should figure out a mechanism for handling stale codebases.
        if status.has_synced_version() {
            // For now, we pass the name of the directory as the name of the
            // codebase.
            let codebase_name = codebase_path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();

            context.push(AIAgentContext::Codebase {
                name: codebase_name.into(),
                path: codebase_path.to_string_lossy().into(),
            })
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn add_remote_codebase_context(context: &mut Vec<AIAgentContext>, app: &AppContext) {
    for codebase in RemoteCodebaseIndexModel::as_ref(app).codebases_for_agent_context() {
        context.push(AIAgentContext::Codebase {
            name: codebase.name,
            path: codebase.path,
        });
    }
}

#[cfg(target_family = "wasm")]
fn add_remote_codebase_context(_context: &mut Vec<AIAgentContext>, _app: &AppContext) {}

/// Parses context reference strings like <block:123> from the user query and returns
/// a map of reference strings to AIAgentAttachment objects.
///
/// This searches across ALL TerminalModels, not just the active session, to find
/// the requested blocks.
pub(super) fn parse_context_attachments(
    query: &str,
    context_model: &BlocklistAIContextModel,
    ctx: &AppContext,
) -> HashMap<String, AIAgentAttachment> {
    let mut referenced_attachments = HashMap::new();

    // Parse block attachments
    for capture in BLOCK_CONTEXT_ATTACHMENT_REGEX.captures_iter(query) {
        if let (Some(full_match), Some(block_id_match)) = (capture.get(0), capture.get(1)) {
            let reference_string = full_match.as_str().to_string();
            let block_id_str = block_id_match.as_str();

            let block_id = BlockId::from(block_id_str.to_string());

            // Search across ALL TerminalModels to find the block
            if let Some(attachment) = find_block_attachment_in_all_terminals(&block_id, ctx) {
                referenced_attachments.insert(reference_string, attachment);
            }
        }
    }

    // Parse drive object attachments (notebooks, workflows, etc)
    for capture in DRIVE_OBJECT_ATTACHMENT_REGEX.captures_iter(query) {
        if let (Some(full_match), Some(object_type_match), Some(object_id_match)) =
            (capture.get(0), capture.get(1), capture.get(2))
        {
            let reference_string = full_match.as_str().to_string();
            let object_type_str = object_type_match.as_str();
            let id_str = object_id_match.as_str();

            if object_type_str == "plan" {
                // For plans, id_str is ai_document_id
                let ai_doc_id = match AIDocumentId::try_from(id_str) {
                    Ok(id) => id,
                    Err(_) => {
                        log::warn!("Invalid ai_document_id in plan reference: {id_str}");
                        continue;
                    }
                };

                // Prefer live editor content from AIDocumentModel (picks up unsaved user edits).
                // Fall back to the synced CloudModel notebook if the document isn't loaded in
                // the current session.
                let content = AIDocumentModel::as_ref(ctx)
                    .get_document_content(&ai_doc_id, ctx)
                    .or_else(|| {
                        CloudModel::as_ref(ctx)
                            .get_all_active_notebooks()
                            .find(|nb| nb.model().ai_document_id.as_ref() == Some(&ai_doc_id))
                            .map(|nb| nb.model().data.clone())
                    });

                if let Some(content) = content {
                    let attachment = AIAgentAttachment::DocumentContent {
                        document_id: id_str.to_string(),
                        content,
                        source: DocumentContentAttachmentSource::UserAttached,
                        line_range: None,
                    };
                    referenced_attachments.insert(reference_string, attachment);
                } else {
                    log::warn!("Plan not found for ai_document_id: {ai_doc_id}");
                }
            } else {
                let object_type = match object_type_str {
                    "workflow" => ObjectType::Workflow,
                    "notebook" => ObjectType::Notebook,
                    "rule" => ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::AIFact,
                    )),
                    _ => continue, // Skip unknown object types
                };

                // Try to get the object data from CloudModel
                let payload = get_object_attachment_payload(id_str, object_type, ctx);

                // Create a DriveObject attachment with the object UID and payload
                let attachment = AIAgentAttachment::DriveObject {
                    uid: id_str.to_string(),
                    payload,
                };
                referenced_attachments.insert(reference_string, attachment);
            }
        }
    }

    // Parse diff hunk attachments
    for capture in DIFF_HUNK_ATTACHMENT_REGEX.captures_iter(query) {
        if let (Some(full_match), Some(diff_hunk_match)) = (capture.get(0), capture.get(1)) {
            let reference_string = full_match.as_str().to_string();
            let diff_hunk_key = diff_hunk_match.as_str();

            // Check if we have a stored diff hunk attachment for this key
            if let Some(attachment) = context_model.get_diff_hunk_attachment(diff_hunk_key) {
                referenced_attachments.insert(reference_string, attachment.clone());
            }
        }
    }

    // Add pending file attachments as FilePathReference.
    // Duplicate basenames get a (1), (2), ... suffix to avoid collisions,
    // matching the pattern in build_file_attachment_map.
    for file in context_model.pending_files().iter() {
        let attachment = AIAgentAttachment::FilePathReference {
            file_id: uuid::Uuid::new_v4().to_string(),
            file_name: file.file_name.clone(),
            file_path: file.file_path.to_string_lossy().to_string(),
        };
        let mut key = file.file_name.clone();
        if referenced_attachments.contains_key(&key) {
            let mut suffix = 1;
            loop {
                key = format!("{} ({suffix})", file.file_name);
                if !referenced_attachments.contains_key(&key) {
                    break;
                }
                suffix += 1;
            }
        }
        referenced_attachments.insert(key, attachment);
    }

    // Add pending AI document as attachment if present
    if let Some(document_id) = context_model.pending_document_id() {
        if let Some(content) = AIDocumentModel::as_ref(ctx).get_document_content(&document_id, ctx)
        {
            let document_id_str = document_id.to_string();
            let attachment = AIAgentAttachment::DocumentContent {
                document_id: document_id_str.clone(),
                content,
                source: DocumentContentAttachmentSource::PlanEdited,
                line_range: None,
            };
            // Use the document ID as the reference key
            referenced_attachments.insert(document_id_str, attachment);
        }
    }

    referenced_attachments
}

/// Searches for a block across all terminal models in the application.
/// Returns an AIAgentAttachment if the block is found.
fn find_block_attachment_in_all_terminals(
    block_id: &BlockId,
    ctx: &AppContext,
) -> Option<AIAgentAttachment> {
    // Iterate over all window IDs to search across all terminal views
    for window_id in ctx.window_ids() {
        // Try to get all terminal views for this window
        if let Some(terminal_views) = ctx.views_of_type::<TerminalView>(window_id) {
            for terminal_view_handle in terminal_views {
                let terminal_view = terminal_view_handle.as_ref(ctx);
                let terminal_model = terminal_view.model.lock();
                let block_list = terminal_model.block_list();

                if let Some(block) = block_list.block_with_id(block_id) {
                    // Create an AIAgentAttachment for the block
                    return Some(AIAgentAttachment::Block(BlockContext {
                        id: block.id().clone(),
                        index: block.index(),
                        command: block.command_to_string(),
                        output: block.output_to_string(),
                        exit_code: block.exit_code(),
                        is_auto_attached: false,
                        started_ts: block.start_ts().cloned(),
                        finished_ts: block.completed_ts().cloned(),
                        pwd: None,
                        shell: None,
                        username: None,
                        hostname: None,
                        git_branch: None,
                        os: None,
                        session_id: None,
                    }));
                }
            }
        }
    }

    None
}

/// Gets the object payload from CloudModel for the given UID and object type.
/// Returns None if the object is not found.
fn get_object_attachment_payload(
    uid: &str,
    object_type: ObjectType,
    ctx: &AppContext,
) -> Option<DriveObjectPayload> {
    match object_type {
        ObjectType::Workflow => CloudModel::as_ref(ctx)
            .get_workflow_by_uid(uid)
            .map(|workflow| {
                let workflow_data = &workflow.model().data;
                DriveObjectPayload::Workflow {
                    name: workflow_data.name().to_string(),
                    description: workflow_data.description().cloned().unwrap_or_default(),
                    command: workflow_data.content().to_string(),
                }
            }),
        ObjectType::Notebook => CloudModel::as_ref(ctx)
            .get_notebook_by_uid(uid)
            .map(|notebook| DriveObjectPayload::Notebook {
                title: notebook.model().title.clone(),
                content: notebook.model().data.clone(),
            }),
        ObjectType::GenericStringObject(_) => {
            // For generic string objects, we only support AI facts (rules) for now
            CloudModel::as_ref(ctx)
                .get_by_uid(&uid.to_string())
                .and_then(|object| {
                    if let Some(ai_fact) = object.as_any().downcast_ref::<GenericCloudObject<GenericStringObjectId, CloudAIFactModel>>() {
                        let string_object = ai_fact as &dyn CloudStringObject;
                        // Convert the format to GraphQL format since that's what the server expects
                        let graphql_format: GraphQLFormat =
                            string_object.generic_string_object_format().into();
                        Some(DriveObjectPayload::GenericStringObject {
                            payload: string_object.serialized().model_as_str().to_string(),
                            object_type: graphql_format.to_string(),
                        })
                    } else {
                        None
                    }
                })
        }
        _ => None, // Other object types not supported for drive object attachments
    }
}
