use serde::de::DeserializeOwned;
use serde::Serialize;
use warp_server_client::cloud_object::GenericStringObjectFormat;

use super::generic_string_model::{Serializer, StringModel};
use crate::cloud_object::JsonObjectType;
use crate::server::sync_queue::SerializedModel;

/// A `JsonModel` is a string model that can be serialized to and deserialized from JSON.
pub trait JsonModel: StringModel + Serialize + DeserializeOwned + 'static {
    /// Returns the JsonObjectType for this model.
    fn json_object_type() -> JsonObjectType;
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct JsonSerializer;

impl<M: JsonModel> Serializer<M> for JsonSerializer {
    fn model_format() -> GenericStringObjectFormat {
        M::model_format()
    }
    fn serialize(model: &M) -> SerializedModel {
        SerializedModel::new(serde_json::to_string(model).expect("model should serialize"))
    }

    fn deserialize_owned(serialized: &str) -> anyhow::Result<M>
    where
        Self: Sized,
    {
        Ok(serde_json::from_str(serialized)?)
    }
}
