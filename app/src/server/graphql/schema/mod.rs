pub mod util;

use anyhow::{bail, Result};
use warp_graphql::generic_string_object::GenericStringObjectFormat;
use warp_graphql::mutations::update_generic_string_object::{
    GenericStringObjectUpdate, UpdateGenericStringObjectResult,
};
use warp_graphql::object::ObjectUpdateSuccess;

use crate::cloud_object::{
    RevisionAndLastEditor, ServerAIExecutionProfile, ServerAIFact, ServerAmbientAgentEnvironment,
    ServerEnvVarCollection, ServerFolder, ServerMCPServer, ServerObject, ServerPreference,
    ServerScheduledAmbientAgent, ServerTemplatableMCPServer, ServerWorkflowEnum, TryFromGql,
    UpdateCloudObjectResult,
};
use crate::server::graphql::get_user_facing_error_message;

fn boxed_rejected_generic_string_object<T>(
    object: warp_graphql::generic_string_object::GenericStringObject,
) -> Result<Box<dyn ServerObject>>
where
    T: TryFromGql<GqlType = warp_graphql::generic_string_object::GenericStringObject>
        + ServerObject
        + 'static,
{
    Ok(Box::new(T::try_from_gql(object)?))
}

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
                    let format = rejected.conflicting_generic_string_object.format;
                    let boxed: Box<dyn ServerObject> = match format {
                        GenericStringObjectFormat::JsonEnvVarCollection => {
                            boxed_rejected_generic_string_object::<ServerEnvVarCollection>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonPreference => {
                            boxed_rejected_generic_string_object::<ServerPreference>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonWorkflowEnum => {
                            boxed_rejected_generic_string_object::<ServerWorkflowEnum>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonAIFact => {
                            boxed_rejected_generic_string_object::<ServerAIFact>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonAIExecutionProfile => {
                            boxed_rejected_generic_string_object::<ServerAIExecutionProfile>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonMCPServer => {
                            boxed_rejected_generic_string_object::<ServerMCPServer>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonTemplatableMCPServer => {
                            boxed_rejected_generic_string_object::<ServerTemplatableMCPServer>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonCloudEnvironment => {
                            boxed_rejected_generic_string_object::<ServerAmbientAgentEnvironment>(
                                rejected.conflicting_generic_string_object,
                            )?
                        }
                        GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                            boxed_rejected_generic_string_object::<ServerScheduledAmbientAgent>(
                                rejected.conflicting_generic_string_object,
                            )?
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
