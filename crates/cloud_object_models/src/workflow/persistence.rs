use cloud_object_persistence::{
    CloudObjectReadContext, id_from_metadata, to_cloud_object_metadata, upsert_cloud_object,
};
use cloud_objects::cloud_object::ObjectType;
use diesel::{
    Connection, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection, result::Error,
};
use persistence::{
    model::{NewWorkflow, Workflow as PersistedWorkflow},
    schema,
};

use super::{CloudWorkflow, CloudWorkflowModel, WorkflowId};

pub fn upsert_workflows(
    conn: &mut SqliteConnection,
    cloud_workflows: Vec<CloudWorkflow>,
) -> Result<(), Error> {
    use schema::workflows::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for cloud_workflow in cloud_workflows {
            let workflow_id = cloud_workflow.id;
            if let Ok(serialized_workflow) = serde_json::to_string(&cloud_workflow.model().data) {
                // todo: wrap in an arc to avoid unnecessary cloning.
                let serialized_workflow_clone = serialized_workflow.clone();
                upsert_cloud_object(
                    conn,
                    ObjectType::Workflow,
                    workflow_id,
                    cloud_workflow.metadata,
                    cloud_workflow.permissions,
                    Box::new(move |conn| {
                        let workflow = NewWorkflow {
                            data: serialized_workflow.clone(),
                        };
                        diesel::insert_into(schema::workflows::dsl::workflows)
                            .values(workflow)
                            .execute(conn)?;
                        let workflow_id: i32 = schema::workflows::dsl::workflows
                            .select(schema::workflows::columns::id)
                            .order(schema::workflows::columns::id.desc())
                            .first(conn)?;
                        Ok(workflow_id)
                    }),
                    Box::new(move |conn, workflow_id| {
                        diesel::update(
                            workflows.filter(schema::workflows::dsl::id.eq(workflow_id)),
                        )
                        .set((data.eq(serialized_workflow_clone),))
                        .execute(conn)?;
                        Ok(())
                    }),
                )?
            }
        }
        Ok(())
    })
}

pub fn read_workflows(
    conn: &mut SqliteConnection,
    read_context: &CloudObjectReadContext,
) -> Result<Vec<CloudWorkflow>, Error> {
    Ok(schema::workflows::dsl::workflows
        .load::<PersistedWorkflow>(conn)?
        .into_iter()
        .filter_map(|workflow| {
            let metadata = read_context.metadata_for_object(workflow.id, ObjectType::Workflow)?;
            let workflow_content = serde_json::from_str(workflow.data.as_str()).ok()?;
            let workflow_id = id_from_metadata::<WorkflowId>(metadata)?;
            let cloud_object_permissions = read_context.permissions_for_metadata(metadata)?;
            Some(CloudWorkflow::new(
                workflow_id,
                CloudWorkflowModel::new(workflow_content),
                to_cloud_object_metadata(metadata),
                cloud_object_permissions,
            ))
        })
        .collect())
}

pub fn delete_workflow(conn: &mut SqliteConnection, workflow_id: i32) -> Result<(), Error> {
    use schema::workflows::dsl::*;
    diesel::delete(workflows.filter(id.eq(workflow_id))).execute(conn)?;
    Ok(())
}
