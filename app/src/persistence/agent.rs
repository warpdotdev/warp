use diesel::associations::HasTable;
use diesel::connection::DefaultLoadingMode;
use diesel::{prelude::*, result::Error, SqliteConnection};
use prost::Message;
use std::collections::{HashMap, HashSet};
use warp_multi_agent_api as api;

use super::model::{AgentConversation, AgentConversationData};
use crate::persistence::model::AgentConversationRecord;
use crate::persistence::schema::{self, agent_conversations, agent_tasks};

// Persisted agent tasks can include large tool results (for example, file contents from
// read/edit tools). Decoding and then eagerly converting those payloads during startup can
// clone them several times and push app launch memory into multi-GB territory. Keep local
// restore bounded; oversized conversations can still be loaded from the cloud when needed.
const MAX_RESTORED_AGENT_TASK_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESTORED_AGENT_CONVERSATION_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = agent_conversations)]
struct NewAgentConversation {
    conversation_id: String,
    conversation_data: String,
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = agent_tasks)]
struct NewAgentTask {
    conversation_id: String,
    task_id: String,
    task: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum UpsertConversationError {
    #[error("Failed to serialize conversation data: {0:?}")]
    Serialization(#[from] serde_json::Error),
    #[error("Failed to upsert conversation to sqlite: {0:?}")]
    DB(#[from] diesel::result::Error),
}

pub(super) fn upsert_agent_conversation<'a>(
    conn: &mut SqliteConnection,
    conversation_id_param: &str,
    tasks: impl IntoIterator<Item = &'a api::Task>,
    conversation_data_param: AgentConversationData,
) -> Result<(), UpsertConversationError> {
    use diesel::ExpressionMethods;
    use diesel::QueryDsl;
    use schema::agent_conversations::dsl::*;
    use schema::agent_tasks::dsl as tasks_dsl;
    const MAX_PERSISTED_CONVERSATION_COUNT: i64 = 100;

    let serialized_conversation_data = serde_json::to_string(&conversation_data_param)?;

    conn.transaction::<_, Error, _>(|conn| {
        // Upsert the conversation level metadata
        let new_conversation = NewAgentConversation {
            conversation_id: conversation_id_param.to_owned(),
            conversation_data: serialized_conversation_data,
        };

        diesel::insert_into(agent_conversations::table())
            .values(&new_conversation)
            .on_conflict(conversation_id)
            .do_update()
            .set(&new_conversation)
            .execute(conn)?;

        // Upsert each task
        for task in tasks {
            let task_binary = task.encode_to_vec();
            let new_task = NewAgentTask {
                conversation_id: conversation_id_param.to_owned(),
                task_id: task.id.clone(),
                task: task_binary,
            };

            if let Err(e) = diesel::insert_into(agent_tasks::table)
                .values(&new_task)
                .on_conflict(tasks_dsl::task_id)
                .do_update()
                .set(&new_task)
                .execute(conn)
            {
                log::warn!("Failed to upsert task {e:?}");
                return Err(e);
            }
        }

        // Prune old conversations if we exceed MAX_PERSISTED_CONVERSATION_COUNT conversations
        let conversation_count: i64 = agent_conversations::table().count().get_result(conn)?;
        if conversation_count > MAX_PERSISTED_CONVERSATION_COUNT {
            // Remove the oldest conversations, keeping only the most recent MAX_PERSISTED_CONVERSATION_COUNT
            let conversations_to_remove: Vec<String> = agent_conversations::table()
                .order(last_modified_at.asc())
                .limit(conversation_count - MAX_PERSISTED_CONVERSATION_COUNT)
                .select(conversation_id)
                .load(conn)?;

            delete_agent_conversations(conn, conversations_to_remove)?;
        }

        Ok(())
    })?;

    Ok(())
}

pub(super) fn read_agent_conversations(
    conn: &mut SqliteConnection,
) -> Result<Vec<AgentConversation>, diesel::result::Error> {
    use schema::agent_conversations::dsl::*;

    let mut conversations_by_id = HashMap::<String, AgentConversation>::from_iter(
        agent_conversations
            .select(AgentConversationRecord::as_select())
            .load(conn)?
            .into_iter()
            .map(|conversation| {
                (
                    conversation.conversation_id.clone(),
                    AgentConversation {
                        conversation,
                        tasks: vec![],
                    },
                )
            }),
    );

    let task_records = agent_tasks::table
        .select((agent_tasks::conversation_id, agent_tasks::task))
        .load_iter::<(String, Vec<u8>), DefaultLoadingMode>(conn)?;

    let mut invalid_conversation_ids = HashSet::new();
    let mut restored_bytes_by_conversation = HashMap::<String, usize>::new();
    for task_record in task_records {
        let (task_conversation_id, task_bytes) = task_record?;
        if let Some(conversation) = conversations_by_id.get_mut(&task_conversation_id) {
            let task_bytes_len = task_bytes.len();
            let restored_bytes = restored_bytes_by_conversation
                .entry(task_conversation_id.clone())
                .or_default();
            *restored_bytes += task_bytes_len;

            if task_bytes_len > MAX_RESTORED_AGENT_TASK_BYTES
                || *restored_bytes > MAX_RESTORED_AGENT_CONVERSATION_BYTES
            {
                log::warn!(
                    "Skipping persisted agent conversation {} because task restoration would decode {} bytes for this task and {} bytes total",
                    conversation.conversation.conversation_id,
                    task_bytes_len,
                    *restored_bytes
                );
                invalid_conversation_ids.insert(conversation.conversation.conversation_id.clone());
                continue;
            }

            match api::Task::decode(&task_bytes[..]) {
                Ok(api_task) => {
                    conversation.tasks.push(api_task);
                }
                Err(e) => {
                    log::error!("Failed to decode task protobuf: {e}");

                    invalid_conversation_ids
                        .insert(conversation.conversation.conversation_id.clone());
                }
            }
        }
    }

    conversations_by_id.retain(|c_id, _| !invalid_conversation_ids.contains(c_id));

    Ok(conversations_by_id.into_values().collect())
}

/// Read a single agent conversation by its ID, including decoded tasks.
pub(crate) fn read_agent_conversation_by_id(
    conn: &mut SqliteConnection,
    conversation_id_str: &str,
) -> Result<Option<AgentConversation>, diesel::result::Error> {
    use schema::agent_conversations::dsl as convo_dsl;
    use schema::agent_tasks::dsl as tasks_dsl;

    let maybe_record: Option<AgentConversationRecord> = convo_dsl::agent_conversations
        .filter(convo_dsl::conversation_id.eq(conversation_id_str.to_owned()))
        .select(AgentConversationRecord::as_select())
        .first(conn)
        .optional()?;

    let Some(conversation_record) = maybe_record else {
        return Ok(None);
    };

    let task_records = schema::agent_tasks::table
        .filter(tasks_dsl::conversation_id.eq(conversation_id_str))
        .select((tasks_dsl::conversation_id, tasks_dsl::task))
        .load_iter::<(String, Vec<u8>), DefaultLoadingMode>(conn)?;

    let mut decoded_tasks = Vec::new();
    let mut restored_bytes = 0;
    for task_record in task_records {
        let (_, task_bytes) = task_record?;
        let task_bytes_len = task_bytes.len();
        restored_bytes += task_bytes_len;

        if task_bytes_len > MAX_RESTORED_AGENT_TASK_BYTES
            || restored_bytes > MAX_RESTORED_AGENT_CONVERSATION_BYTES
        {
            log::warn!(
                "Skipping persisted agent conversation {conversation_id_str} because task restoration would decode {task_bytes_len} bytes for this task and {restored_bytes} bytes total"
            );
            return Ok(None);
        }

        match api::Task::decode(&task_bytes[..]) {
            Ok(task) => decoded_tasks.push(task),
            Err(e) => {
                log::error!("Failed to decode task protobuf: {e}");
            }
        }
    }

    Ok(Some(AgentConversation {
        conversation: conversation_record,
        tasks: decoded_tasks,
    }))
}

pub(super) fn delete_agent_conversations(
    conn: &mut SqliteConnection,
    conversation_ids: Vec<String>,
) -> Result<(), diesel::result::Error> {
    use diesel::ExpressionMethods;
    use diesel::QueryDsl;
    use schema::agent_conversations::dsl::*;
    use schema::agent_tasks::dsl as tasks_dsl;

    conn.transaction::<_, Error, _>(|conn| {
        // Delete tasks for these conversations first (due to foreign key constraint)
        diesel::delete(
            agent_tasks::table.filter(tasks_dsl::conversation_id.eq_any(&conversation_ids)),
        )
        .execute(conn)?;

        // Delete the conversations themselves
        diesel::delete(
            agent_conversations::table().filter(conversation_id.eq_any(&conversation_ids)),
        )
        .execute(conn)?;

        Ok(())
    })?;

    Ok(())
}
