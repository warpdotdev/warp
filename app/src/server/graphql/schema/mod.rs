pub mod util;

use crate::{
    ai::cloud_environments::CloudAmbientAgentEnvironmentModel,
    ai::{
        ambient_agents::scheduled::CloudScheduledAmbientAgentModel,
        execution_profiles::CloudAIExecutionProfileModel,
        facts::CloudAIFactModel,
        mcp::{templatable::CloudTemplatableMCPServerModel, CloudMCPServerModel},
    },
    cloud_object::{
        model::generic_string_model::GenericStringObjectId, GenericServerObject,
        RevisionAndLastEditor, ServerFolder, ServerObject, UpdateCloudObjectResult,
    },
    env_vars::CloudEnvVarCollectionModel,
    server::{graphql::get_user_facing_error_message, ids::ServerId},
    settings::cloud_preferences::CloudPreferenceModel,
    workflows::workflow_enum::CloudWorkflowEnumModel,
};

use anyhow::{bail, Result};
use warp_graphql::{
    generic_string_object::GenericStringObjectFormat,
    mutations::update_generic_string_object::{
        GenericStringObjectUpdate, UpdateGenericStringObjectResult,
    },
    object::ObjectUpdateSuccess,
};

pub fn update_generic_string_object_result_to_update_result(
    value: UpdateGenericStringObjectResult,
) -> Result<UpdateCloudObjectResult<Box<dyn ServerObject>>> {
    match value {
        UpdateGenericStringObjectResult::UpdateGenericStringObjectOutput(output) => {
            match output.update {
                GenericStringObjectUpdate::ObjectUpdateSuccess(success) => {
                    Ok(UpdateCloudObjectResult::Success {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: success.revision_ts.into(),
                            last_editor_uid: Some(success.last_editor_uid.into_inner()),
                        },
                    })
                }
                GenericStringObjectUpdate::GenericStringObjectUpdateRejected(rejected) => {
                    let boxed: Box<dyn ServerObject> = match rejected
                        .conflicting_generic_string_object
                        .format
                    {
                        GenericStringObjectFormat::JsonEnvVarCollection => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudEnvVarCollectionModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonPreference => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudPreferenceModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonWorkflowEnum => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudWorkflowEnumModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonAIFact => {
                            let gso = GenericServerObject::<
                                    GenericStringObjectId,
                                    CloudAIFactModel,
                                >::try_from_graphql_fields(
                                    ServerId::from_string_lossy(
                                        rejected
                                            .conflicting_generic_string_object
                                            .metadata
                                            .uid
                                            .inner(),
                                    ),
                                    Some(
                                        rejected.conflicting_generic_string_object.serialized_model,
                                    ),
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .try_into()?,
                                    rejected
                                        .conflicting_generic_string_object
                                        .permissions
                                        .try_into()?,
                                )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonAIExecutionProfile => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudAIExecutionProfileModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonMCPServer => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudMCPServerModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonTemplatableMCPServer => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudTemplatableMCPServerModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonCloudEnvironment => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudAmbientAgentEnvironmentModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                        GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                            let gso = GenericServerObject::<
                                GenericStringObjectId,
                                CloudScheduledAmbientAgentModel,
                            >::try_from_graphql_fields(
                                ServerId::from_string_lossy(
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .uid
                                        .inner(),
                                ),
                                Some(rejected.conflicting_generic_string_object.serialized_model),
                                rejected
                                    .conflicting_generic_string_object
                                    .metadata
                                    .try_into()?,
                                rejected
                                    .conflicting_generic_string_object
                                    .permissions
                                    .try_into()?,
                            )?;
                            let boxed: Box<dyn ServerObject> = Box::new(gso);
                            boxed
                        }
                    };
                    Ok(UpdateCloudObjectResult::Rejected { object: boxed })
                }
                GenericStringObjectUpdate::Unknown => {
                    bail!("update generic string object response has unknown variant")
                }
            }
        }
        UpdateGenericStringObjectResult::UserFacingError(e) => {
            bail!(get_user_facing_error_message(e))
        }
        UpdateGenericStringObjectResult::Unknown => {
            bail!("update generic string object response has unknown variant")
        }
    }
}

pub fn object_update_success_to_update_result(
    value: ObjectUpdateSuccess,
) -> Result<UpdateCloudObjectResult<ServerFolder>> {
    Ok(UpdateCloudObjectResult::Success {
        revision_and_editor: RevisionAndLastEditor {
            revision: value.revision_ts.into(),
            last_editor_uid: Some(value.last_editor_uid.into_inner()),
        },
    })
}
