use cloud_object_persistence::{
    CloudObjectReadContext, id_from_metadata, read_generic_string_object_rows,
    to_cloud_object_metadata,
};
use cloud_objects::cloud_object::{
    GENERIC_STRING_OBJECT_PREFIX, GenericStringObjectFormat, JSON_OBJECT_PREFIX, JsonObjectType,
    ObjectType,
};
use cloud_objects::ids::GenericStringObjectId;
use diesel::SqliteConnection;
use diesel::result::Error;

use crate::{
    CloudAIExecutionProfile, CloudAIExecutionProfileModel, CloudAIFact, CloudAIFactModel,
    CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel, CloudEnvVarCollection,
    CloudEnvVarCollectionModel, CloudMCPServer, CloudMCPServerModel, CloudPreference,
    CloudPreferenceModel, CloudScheduledAmbientAgent, CloudScheduledAmbientAgentModel,
    CloudTemplatableMCPServer, CloudTemplatableMCPServerModel, CloudWorkflowEnum,
    CloudWorkflowEnumModel,
};

pub enum PersistedGenericStringObject {
    Preference(CloudPreference),
    EnvVarCollection(CloudEnvVarCollection),
    WorkflowEnum(CloudWorkflowEnum),
    AIFact(CloudAIFact),
    MCPServer(CloudMCPServer),
    TemplatableMCPServer(CloudTemplatableMCPServer),
    AIExecutionProfile(CloudAIExecutionProfile),
    CloudEnvironment(CloudAmbientAgentEnvironment),
    ScheduledAmbientAgent(CloudScheduledAmbientAgent),
}

pub fn read_generic_string_objects(
    conn: &mut SqliteConnection,
    read_context: &CloudObjectReadContext,
) -> Result<Vec<PersistedGenericStringObject>, Error> {
    Ok(read_generic_string_object_rows(conn)?
        .into_iter()
        .filter_map(|object| {
            let metadata = read_context.metadata_for_object(
                object.id,
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::Preference,
                )),
            )?;
            let object_id = id_from_metadata::<GenericStringObjectId>(metadata)?;
            let cloud_object_permissions = read_context.permissions_for_metadata(metadata)?;
            let json_object_type: JsonObjectType = metadata
                .object_type
                .strip_prefix(&format!(
                    "{GENERIC_STRING_OBJECT_PREFIX}{JSON_OBJECT_PREFIX}"
                ))?
                .try_into()
                .ok()?;
            match json_object_type {
                JsonObjectType::Preference => {
                    let model = CloudPreferenceModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::Preference(CloudPreference::new(
                            object_id,
                            model,
                            to_cloud_object_metadata(metadata),
                            cloud_object_permissions,
                        ))
                    })
                }
                JsonObjectType::EnvVarCollection => {
                    let model = CloudEnvVarCollectionModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::EnvVarCollection(CloudEnvVarCollection::new(
                            object_id,
                            model,
                            to_cloud_object_metadata(metadata),
                            cloud_object_permissions,
                        ))
                    })
                }
                JsonObjectType::WorkflowEnum => {
                    let model = CloudWorkflowEnumModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::WorkflowEnum(CloudWorkflowEnum::new(
                            object_id,
                            model,
                            to_cloud_object_metadata(metadata),
                            cloud_object_permissions,
                        ))
                    })
                }
                JsonObjectType::AIFact => {
                    let model = CloudAIFactModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::AIFact(CloudAIFact::new(
                            object_id,
                            model,
                            to_cloud_object_metadata(metadata),
                            cloud_object_permissions,
                        ))
                    })
                }
                JsonObjectType::MCPServer => {
                    let model = CloudMCPServerModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::MCPServer(CloudMCPServer::new(
                            object_id,
                            model,
                            to_cloud_object_metadata(metadata),
                            cloud_object_permissions,
                        ))
                    })
                }
                JsonObjectType::TemplatableMCPServer => {
                    let model = CloudTemplatableMCPServerModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::TemplatableMCPServer(
                            CloudTemplatableMCPServer::new(
                                object_id,
                                model,
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ),
                        )
                    })
                }
                JsonObjectType::AIExecutionProfile => {
                    let model = CloudAIExecutionProfileModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::AIExecutionProfile(
                            CloudAIExecutionProfile::new(
                                object_id,
                                model,
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ),
                        )
                    })
                }
                JsonObjectType::CloudEnvironment => {
                    let model = CloudAmbientAgentEnvironmentModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::CloudEnvironment(
                            CloudAmbientAgentEnvironment::new(
                                object_id,
                                model,
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ),
                        )
                    })
                }
                JsonObjectType::ScheduledAmbientAgent => {
                    let model = CloudScheduledAmbientAgentModel::deserialize_owned(&object.data);
                    model.ok().map(|model| {
                        PersistedGenericStringObject::ScheduledAmbientAgent(
                            CloudScheduledAmbientAgent::new(
                                object_id,
                                model,
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ),
                        )
                    })
                }
                // TODO: Implement CloudAgentConfig model when full sync support is added
                JsonObjectType::CloudAgentConfig => None,
            }
        })
        .collect())
}
