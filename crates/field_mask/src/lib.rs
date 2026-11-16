use itertools::Itertools as _;
use prost_reflect::{DynamicMessage, MessageDescriptor, ReflectMessage, Value};
use prost_types::FieldMask;

/// Errors that can occur when applying a field mask operation.
#[derive(Debug, thiserror::Error)]
pub enum FieldMaskError {
    #[error("Failed to decode dynamic message: {0:#}")]
    Decode(#[from] prost::DecodeError),
    #[error("Expected message field for nested path: {0}")]
    InvalidPath(String),
    #[error("Append is unsupported for field: {0}")]
    UnsupportedAppend(String),
    #[error("Failed to set field: {0:#}")]
    SetField(#[from] prost_reflect::SetFieldError),
}

pub type Result<T> = std::result::Result<T, FieldMaskError>;

#[derive(Debug, Copy, Clone)]
enum OperationType {
    Update,
    Append,
}

/// A field mask operation that selectively copies fields from a `source` message
/// into a `destination` message, producing a new merged result.
pub struct FieldMaskOperation<'a, T: prost::Message + Default> {
    message_descriptor: &'static MessageDescriptor,
    mask: FieldMask,
    destination: &'a T,
    source: &'a T,
    op: OperationType,
}

impl<'a, T: prost::Message + Default> FieldMaskOperation<'a, T> {
    /// Creates an update operation that replaces fields in `destination` with
    /// the corresponding values from `source` for each path in the mask.
    pub fn update(
        message_descriptor: &'static MessageDescriptor,
        destination: &'a T,
        source: &'a T,
        mask: FieldMask,
    ) -> Self {
        Self {
            message_descriptor,
            mask,
            destination,
            source,
            op: OperationType::Update,
        }
    }

    /// Creates an append operation that concatenates string fields from `source`
    /// onto the corresponding fields in `destination` for each path in the mask.
    pub fn append(
        message_descriptor: &'static MessageDescriptor,
        destination: &'a T,
        source: &'a T,
        mask: FieldMask,
    ) -> Self {
        Self {
            message_descriptor,
            mask,
            destination,
            source,
            op: OperationType::Append,
        }
    }

    /// Applies the operation, returning a new message with the masked fields merged.
    pub fn apply(self) -> Result<T> {
        let mut dyn_target = DynamicMessage::new(self.message_descriptor.clone());
        dyn_target.transcode_from(self.destination)?;

        let mut dyn_patch = DynamicMessage::new(self.message_descriptor.clone());
        dyn_patch.transcode_from(self.source)?;

        for path in self.mask.paths {
            apply_path(
                &mut dyn_target,
                &dyn_patch,
                &path.split('.').collect_vec(),
                self.op,
            )?;
        }

        dyn_target.transcode_to::<T>().map_err(FieldMaskError::from)
    }
}

fn apply_path(
    target: &mut DynamicMessage,
    patch: &DynamicMessage,
    path_segments: &[&str],
    operation: OperationType,
) -> std::result::Result<(), FieldMaskError> {
    let Some(field_name) = path_segments.first() else {
        return Ok(());
    };
    let field_desc = match target.descriptor().get_field_by_name(field_name) {
        Some(f) => f,
        None => {
            // Applying a field mask on unknown fields are a no-op.
            //
            // This implies the client's API version is outdated with respect
            // to the server response. Adding fields is backwards-compatible
            // in protobuf, where expected behavior is to no-op.
            return Ok(());
        }
    };
    if path_segments.len() == 1 {
        let updated_field_value = match operation {
            OperationType::Update => patch.get_field(&field_desc).into_owned(),
            OperationType::Append => {
                match (
                    target.get_field(&field_desc).as_ref(),
                    patch.get_field(&field_desc).as_ref(),
                ) {
                    (Value::String(value), Value::String(patch_value)) => {
                        Value::String(format!("{value}{patch_value}"))
                    }
                    _ => {
                        return Err(FieldMaskError::UnsupportedAppend(
                            field_desc.full_name().to_owned(),
                        ));
                    }
                }
            }
        };
        Ok(target.try_set_field(&field_desc, updated_field_value)?)
    } else {
        // Handle nested paths
        let patch_field = patch.get_field(&field_desc);
        match (target.get_field_mut(&field_desc), patch_field.as_ref()) {
            (Value::List(target_list), Value::List(patch_list)) => {
                // For repeated fields, apply the patch on every element of the list
                // Both lists must have the same length
                if target_list.len() != patch_list.len() {
                    return Err(FieldMaskError::InvalidPath(format!(
                        "Field {} lists have different lengths: target has {}, patch has {}",
                        field_name,
                        target_list.len(),
                        patch_list.len()
                    )));
                }

                // Apply the patch to each corresponding pair of elements
                for (target_elem, patch_elem) in target_list.iter_mut().zip(patch_list.iter()) {
                    match (target_elem, patch_elem) {
                        (Value::Message(target_msg), Value::Message(patch_msg)) => {
                            apply_path(target_msg, patch_msg, &path_segments[1..], operation)?;
                        }
                        _ => {
                            return Err(FieldMaskError::InvalidPath(format!(
                                "Field {field_name} list elements are not messages"
                            )));
                        }
                    }
                }
                Ok(())
            }
            (Value::Message(target_msg), Value::Message(patch_msg)) => {
                apply_path(target_msg, patch_msg, &path_segments[1..], operation)
            }
            _ => Err(FieldMaskError::InvalidPath(path_segments.iter().join("."))),
        }
    }
}
