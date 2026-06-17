use std::any::Any;

use anyhow::Result;
use cloud_objects::cloud_object::{
    GenericServerObject, GenericStringModel, Serializer, ServerMetadata,
};
use cloud_objects::ids::{GenericStringObjectId, ObjectUid, ServerId, SyncId};
use warp_graphql::object::CloudObjectWithDescendants;

use crate::{
    AIExecutionProfile, AIFact, AmbientAgentEnvironment, CloudFolderModel, CloudNotebookModel,
    CloudWorkflowModel, EnvVarCollection, JsonSerializer, MCPServer, Preference,
    ScheduledAmbientAgent, ServerAIExecutionProfile, ServerAIFact, ServerAmbientAgentEnvironment,
    ServerCloudAgentConfig, ServerEnvVarCollection, ServerFolder, ServerMCPServer, ServerNotebook,
    ServerPreference, ServerScheduledAmbientAgent, ServerTemplatableMCPServer, ServerWorkflow,
    ServerWorkflowEnum, TemplatableMCPServer, WorkflowEnum,
};

/// A cloud object from the server.
#[derive(Clone, Debug)]
pub enum ServerCloudObject {
    Notebook(ServerNotebook),
    Workflow(Box<ServerWorkflow>),
    Folder(ServerFolder),
    Preference(ServerPreference),
    EnvVarCollection(ServerEnvVarCollection),
    WorkflowEnum(ServerWorkflowEnum),
    AIFact(ServerAIFact),
    MCPServer(ServerMCPServer),
    AIExecutionProfile(ServerAIExecutionProfile),
    TemplatableMCPServer(ServerTemplatableMCPServer),
    AmbientAgentEnvironment(ServerAmbientAgentEnvironment),
    ScheduledAmbientAgent(ServerScheduledAmbientAgent),
    CloudAgentConfig(ServerCloudAgentConfig),
}

impl ServerCloudObject {
    pub fn metadata(&self) -> &ServerMetadata {
        match self {
            ServerCloudObject::Notebook(notebook) => &notebook.metadata,
            ServerCloudObject::Workflow(workflow) => &workflow.metadata,
            ServerCloudObject::Folder(folder) => &folder.metadata,
            ServerCloudObject::Preference(preferences) => &preferences.metadata,
            ServerCloudObject::EnvVarCollection(env_var_collection) => &env_var_collection.metadata,
            ServerCloudObject::WorkflowEnum(workflow_enum) => &workflow_enum.metadata,
            ServerCloudObject::AIFact(aifact) => &aifact.metadata,
            ServerCloudObject::MCPServer(mcp_server) => &mcp_server.metadata,
            ServerCloudObject::TemplatableMCPServer(templatable_mcp_server) => {
                &templatable_mcp_server.metadata
            }
            ServerCloudObject::AIExecutionProfile(ai_execution_profile) => {
                &ai_execution_profile.metadata
            }
            ServerCloudObject::AmbientAgentEnvironment(ambient_agent_environment) => {
                &ambient_agent_environment.metadata
            }
            ServerCloudObject::ScheduledAmbientAgent(scheduled_ambient_agent) => {
                &scheduled_ambient_agent.metadata
            }
            ServerCloudObject::CloudAgentConfig(cloud_agent_config) => &cloud_agent_config.metadata,
        }
    }

    pub fn uid(&self) -> ObjectUid {
        match self {
            ServerCloudObject::Notebook(notebook) => notebook.id.uid(),
            ServerCloudObject::Workflow(workflow) => workflow.id.uid(),
            ServerCloudObject::Folder(folder) => folder.id.uid(),
            ServerCloudObject::Preference(preferences) => preferences.id.uid(),
            ServerCloudObject::EnvVarCollection(env_var_collection) => env_var_collection.id.uid(),
            ServerCloudObject::WorkflowEnum(workflow_enum) => workflow_enum.id.uid(),
            ServerCloudObject::AIFact(aifact) => aifact.id.uid(),
            ServerCloudObject::MCPServer(mcp_server) => mcp_server.id.uid(),
            ServerCloudObject::AIExecutionProfile(ai_execution_profile) => {
                ai_execution_profile.id.uid()
            }
            ServerCloudObject::TemplatableMCPServer(templatable_mcp_server) => {
                templatable_mcp_server.id.uid()
            }
            ServerCloudObject::AmbientAgentEnvironment(ambient_agent_environment) => {
                ambient_agent_environment.id.uid()
            }
            ServerCloudObject::ScheduledAmbientAgent(scheduled_ambient_agent) => {
                scheduled_ambient_agent.id.uid()
            }
            ServerCloudObject::CloudAgentConfig(cloud_agent_config) => cloud_agent_config.id.uid(),
        }
    }
}

impl<K, M> From<&GenericServerObject<K, M>> for ServerCloudObject
where
    K: 'static,
    M: 'static,
{
    fn from(value: &GenericServerObject<K, M>) -> Self {
        let value = value as &dyn Any;
        if let Some(server_notebook) = value.downcast_ref::<ServerNotebook>() {
            ServerCloudObject::Notebook(server_notebook.clone())
        } else if let Some(server_workflow) = value.downcast_ref::<ServerWorkflow>() {
            ServerCloudObject::Workflow(Box::new(server_workflow.clone()))
        } else if let Some(server_folder) = value.downcast_ref::<ServerFolder>() {
            ServerCloudObject::Folder(server_folder.clone())
        } else if let Some(server_preferences) = value.downcast_ref::<ServerPreference>() {
            ServerCloudObject::Preference(server_preferences.clone())
        } else if let Some(server_env_var_collection) =
            value.downcast_ref::<ServerEnvVarCollection>()
        {
            ServerCloudObject::EnvVarCollection(server_env_var_collection.clone())
        } else if let Some(server_workflow_enum) = value.downcast_ref::<ServerWorkflowEnum>() {
            ServerCloudObject::WorkflowEnum(server_workflow_enum.clone())
        } else if let Some(server_aifact) = value.downcast_ref::<ServerAIFact>() {
            ServerCloudObject::AIFact(server_aifact.clone())
        } else if let Some(server_mcp_server) = value.downcast_ref::<ServerMCPServer>() {
            ServerCloudObject::MCPServer(server_mcp_server.clone())
        } else if let Some(server_ai_execution_profile) =
            value.downcast_ref::<ServerAIExecutionProfile>()
        {
            ServerCloudObject::AIExecutionProfile(server_ai_execution_profile.clone())
        } else if let Some(server_templatable_mcp_server) =
            value.downcast_ref::<ServerTemplatableMCPServer>()
        {
            ServerCloudObject::TemplatableMCPServer(server_templatable_mcp_server.clone())
        } else if let Some(server_ambient_agent_environment) =
            value.downcast_ref::<ServerAmbientAgentEnvironment>()
        {
            ServerCloudObject::AmbientAgentEnvironment(server_ambient_agent_environment.clone())
        } else if let Some(server_scheduled_ambient_agent) =
            value.downcast_ref::<ServerScheduledAmbientAgent>()
        {
            ServerCloudObject::ScheduledAmbientAgent(server_scheduled_ambient_agent.clone())
        } else if let Some(server_cloud_agent_config) =
            value.downcast_ref::<ServerCloudAgentConfig>()
        {
            ServerCloudObject::CloudAgentConfig(server_cloud_agent_config.clone())
        } else {
            panic!("Unknown server object type");
        }
    }
}

/// Tries to convert a GraphQL object payload into a local server object.
pub trait TryFromGql: Sized {
    type GqlType;

    fn try_from_gql(value: Self::GqlType) -> Result<Self>;
}

impl<T, S> TryFromGql for GenericServerObject<GenericStringObjectId, GenericStringModel<T, S>>
where
    T: std::fmt::Debug + Clone + Send + Sync + 'static,
    S: Serializer<T>,
{
    type GqlType = warp_graphql::generic_string_object::GenericStringObject;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let model = GenericStringModel::<T, S>::deserialize_owned(&value.serialized_model)?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            model,
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerFolder {
    type GqlType = warp_graphql::folder::Folder;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudFolderModel::new(&value.name, value.is_warp_pack),
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerNotebook {
    type GqlType = warp_graphql::notebook::Notebook;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let ai_document_id = value
            .ai_document_id
            .map(|id| ai::document::AIDocumentId::try_from(&id[..]))
            .transpose()?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudNotebookModel {
                title: value.title,
                data: value.data,
                ai_document_id,
                conversation_id: None,
            },
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerWorkflow {
    type GqlType = warp_graphql::workflow::Workflow;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let workflow = serde_json::from_str(value.data.as_str())?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudWorkflowModel { data: workflow },
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFrom<warp_graphql::object::CloudObject> for ServerCloudObject {
    type Error = anyhow::Error;

    fn try_from(value: warp_graphql::object::CloudObject) -> Result<Self, Self::Error> {
        match value {
            warp_graphql::object::CloudObject::AIConversation(_) => Err(anyhow::anyhow!(
                "AIConversation is not a supported object type for this operation"
            )),
            warp_graphql::object::CloudObject::Folder(folder) => Ok(ServerCloudObject::Folder(
                ServerFolder::try_from_gql(folder)?,
            )),
            warp_graphql::object::CloudObject::GenericStringObject(gso) => {
                server_gso_to_cloud_object(gso)
            }
            warp_graphql::object::CloudObject::Notebook(notebook) => Ok(
                ServerCloudObject::Notebook(ServerNotebook::try_from_gql(notebook)?),
            ),
            warp_graphql::object::CloudObject::Workflow(workflow) => Ok(
                ServerCloudObject::Workflow(Box::new(ServerWorkflow::try_from_gql(workflow)?)),
            ),
            warp_graphql::object::CloudObject::Unknown => {
                Err(anyhow::anyhow!("Unable to convert cloud object type"))
            }
        }
    }
}

impl TryFrom<CloudObjectWithDescendants> for ServerCloudObject {
    type Error = anyhow::Error;

    fn try_from(value: CloudObjectWithDescendants) -> Result<Self, Self::Error> {
        match value {
            CloudObjectWithDescendants::AIConversation(_) => Err(anyhow::anyhow!(
                "AIConversation is not a supported object type for this operation"
            )),
            CloudObjectWithDescendants::FolderWithDescendants(fwd) => Ok(
                ServerCloudObject::Folder(ServerFolder::try_from_gql(fwd.folder)?),
            ),
            CloudObjectWithDescendants::GenericStringObject(gso) => server_gso_to_cloud_object(gso),
            CloudObjectWithDescendants::Notebook(notebook) => Ok(ServerCloudObject::Notebook(
                ServerNotebook::try_from_gql(notebook)?,
            )),
            CloudObjectWithDescendants::Workflow(workflow) => Ok(ServerCloudObject::Workflow(
                Box::new(ServerWorkflow::try_from_gql(workflow)?),
            )),
            CloudObjectWithDescendants::Unknown => Err(anyhow::anyhow!(
                "Unable to convert cloud object with descendants type"
            )),
        }
    }
}

fn server_gso_to_cloud_object(
    gso: warp_graphql::generic_string_object::GenericStringObject,
) -> Result<ServerCloudObject> {
    match gso.format {
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonEnvVarCollection => {
            Ok(ServerCloudObject::EnvVarCollection(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<EnvVarCollection, JsonSerializer>>::try_from_gql(gso)?,
            ))
        }
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonPreference => Ok(
            ServerCloudObject::Preference(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<Preference, JsonSerializer>>::try_from_gql(gso)?,
            ),
        ),
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonWorkflowEnum => Ok(
            ServerCloudObject::WorkflowEnum(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<WorkflowEnum, JsonSerializer>>::try_from_gql(gso)?,
            ),
        ),
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIFact => Ok(
            ServerCloudObject::AIFact(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<AIFact, JsonSerializer>>::try_from_gql(gso)?,
            ),
        ),
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonMCPServer => Ok(
            ServerCloudObject::MCPServer(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<MCPServer, JsonSerializer>>::try_from_gql(gso)?,
            ),
        ),
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIExecutionProfile => {
            Ok(ServerCloudObject::AIExecutionProfile(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<AIExecutionProfile, JsonSerializer>>::try_from_gql(gso)?,
            ))
        }
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonTemplatableMCPServer => {
            Ok(ServerCloudObject::TemplatableMCPServer(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<TemplatableMCPServer, JsonSerializer>>::try_from_gql(gso)?,
            ))
        }
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonCloudEnvironment => {
            Ok(ServerCloudObject::AmbientAgentEnvironment(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<AmbientAgentEnvironment, JsonSerializer>>::try_from_gql(gso)?,
            ))
        }
        warp_graphql::generic_string_object::GenericStringObjectFormat::JsonScheduledAmbientAgent => {
            Ok(ServerCloudObject::ScheduledAmbientAgent(
                GenericServerObject::<GenericStringObjectId, GenericStringModel<ScheduledAmbientAgent, JsonSerializer>>::try_from_gql(gso)?,
            ))
        }
    }
}
