use ai::document::AIDocumentId;
use cloud_object_persistence::{
    CloudObjectReadContext, id_from_metadata, to_cloud_object_metadata, upsert_cloud_object,
};
use cloud_objects::cloud_object::ObjectType;
use diesel::{
    Connection, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection, result::Error,
};
use persistence::{
    model::{NewNotebook, Notebook},
    schema,
};

use super::{CloudNotebook, CloudNotebookModel, NotebookId};

pub fn upsert_notebooks(
    conn: &mut SqliteConnection,
    cloud_notebooks: Vec<CloudNotebook>,
) -> Result<(), Error> {
    use schema::notebooks::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for cloud_notebook in cloud_notebooks {
            let notebook_clone = cloud_notebook.clone();
            let title_clone = cloud_notebook.model().title.clone();
            let data_clone = cloud_notebook.model().data.clone();
            let ai_document_id_clone = cloud_notebook
                .model()
                .ai_document_id
                .as_ref()
                .map(|doc_id| doc_id.to_string());
            upsert_cloud_object(
                conn,
                ObjectType::Notebook,
                cloud_notebook.id,
                cloud_notebook.metadata,
                cloud_notebook.permissions,
                Box::new(move |conn| {
                    let new_notebook = NewNotebook {
                        title: Some(title_clone),
                        data: Some(data_clone),
                        ai_document_id: ai_document_id_clone,
                    };
                    diesel::insert_into(schema::notebooks::dsl::notebooks)
                        .values(new_notebook)
                        .execute(conn)?;
                    let notebook_id: i32 = schema::notebooks::dsl::notebooks
                        .select(schema::notebooks::columns::id)
                        .order(schema::notebooks::columns::id.desc())
                        .first(conn)?;
                    Ok(notebook_id)
                }),
                Box::new(move |conn, notebook_id| {
                    diesel::update(notebooks.filter(schema::notebooks::dsl::id.eq(notebook_id)))
                        .set((
                            title.eq(notebook_clone.model().title.clone()),
                            data.eq(notebook_clone.model().data.clone()),
                            ai_document_id.eq(notebook_clone
                                .model()
                                .ai_document_id
                                .as_ref()
                                .map(|doc_id| doc_id.to_string())),
                        ))
                        .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

pub fn read_notebooks(
    conn: &mut SqliteConnection,
    read_context: &CloudObjectReadContext,
) -> Result<Vec<CloudNotebook>, Error> {
    Ok(schema::notebooks::dsl::notebooks
        .load::<Notebook>(conn)?
        .into_iter()
        .filter_map(|notebook| {
            let metadata = read_context.metadata_for_object(notebook.id, ObjectType::Notebook)?;
            let notebook_id = id_from_metadata::<NotebookId>(metadata)?;
            let cloud_object_permissions = read_context.permissions_for_metadata(metadata)?;
            let ai_document_id = notebook
                .ai_document_id
                .as_ref()
                .and_then(|doc_id_str| AIDocumentId::try_from(doc_id_str.as_str()).ok());
            Some(CloudNotebook::new(
                notebook_id,
                CloudNotebookModel {
                    title: notebook.title.unwrap_or_default(),
                    data: notebook.data.unwrap_or_default(),
                    ai_document_id,
                    conversation_id: None,
                },
                to_cloud_object_metadata(metadata),
                cloud_object_permissions,
            ))
        })
        .collect())
}

pub fn delete_notebook(conn: &mut SqliteConnection, notebook_id: i32) -> Result<(), Error> {
    use schema::notebooks::dsl::*;
    diesel::delete(notebooks.filter(id.eq(notebook_id))).execute(conn)?;
    Ok(())
}
