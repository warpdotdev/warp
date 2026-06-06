use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use warp_js::{JsFunctionId, SerializedJsValue, TypedJsFunctionRef};

#[derive(thiserror::Error, Debug)]
pub enum JsExecutionError {
    #[error("Could not execute JS due to serialization error: {0:?}")]
    Serialization(bincode::Error),
    #[error("Could not execute JS due to deserialization error: {0:?}")]
    Deserialization(bincode::Error),
    #[error("Internal error occurred: {0}")]
    Internal(String),
}

/// Trait to be implemented by callers using V2 command signatures. V2 command signatures are
/// defined in JavaScript and may contain JS functions, which are internally represented with
/// `TypedJsFunctionRef`s.
#[async_trait]
pub trait JsExecutionContext: Send + Sync {
    async fn call_js_function(
        &self,
        input: SerializedJsValue,
        function_id: JsFunctionId,
    ) -> Result<SerializedJsValue, JsExecutionError>;
}

/// Helper function for making typed JS function calls.
pub(crate) async fn call_js_function<I, O>(
    input: &I,
    js_function_ref: &TypedJsFunctionRef<I, O>,
    js_ctx: &dyn JsExecutionContext,
) -> Result<O, JsExecutionError>
where
    I: Serialize,
    O: DeserializeOwned,
{
    let serialized_input =
        SerializedJsValue::from_value(input).map_err(JsExecutionError::Serialization)?;
    let serialized_output = js_ctx
        .call_js_function(serialized_input, js_function_ref.id)
        .await?;
    let output: O = serialized_output
        .to_value()
        .map_err(JsExecutionError::Deserialization)?;
    Ok(output)
}
