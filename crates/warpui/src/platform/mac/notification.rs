use anyhow::{Result, anyhow};
use chrono::DateTime;
use cocoa::base::id;
use objc2_foundation::NSUInteger;
use warpui_core::notification::{
    NotificationResponse, NotificationSendError, RequestPermissionsOutcome,
};

use super::utils::nsstring_as_str;

/// Build a Notification Response from a native notification event
///
/// # Safety
///
/// The `data` parameter must be a valid pointer to Objective-C string data
pub unsafe fn response_from_native(
    seconds_from_epoch: i32,
    data: id,
) -> Result<NotificationResponse> {
    unsafe {
        let data = nsstring_as_str(data)?;

        // Only set the data if it's not an empty string.
        let data = (!data.is_empty()).then_some(data);

        let timestamp = DateTime::from_timestamp(seconds_from_epoch as i64, 0)
            .ok_or_else(|| anyhow!("failed to convert time"))?;
        Ok(NotificationResponse::new(
            timestamp.naive_utc(),
            data.map(Into::into),
        ))
    }
}

/// Build a Notification send error from a native notification event
///
/// # Safety
///
/// The `error_message` parameter must be a valid pointer to Objective-C string data
pub unsafe fn send_error_from_native(
    error_type: NSUInteger,
    error_message: id,
) -> Result<NotificationSendError> {
    unsafe {
        let error_message = nsstring_as_str(error_message)?.to_owned();

        Ok(match error_type {
            0 => NotificationSendError::PermissionsDenied,
            1 => NotificationSendError::Other { error_message },
            _ => NotificationSendError::Other { error_message },
        })
    }
}

/// Build a Notification request permissions outcome from a native notification event
///
/// # Safety
///
/// The `outcome_message` parameter must be a valid pointer to Objective-C string data
pub unsafe fn request_permissions_outcome_from_native(
    outcome_type: NSUInteger,
    outcome_message: id,
) -> Result<RequestPermissionsOutcome> {
    unsafe {
        let outcome_message = nsstring_as_str(outcome_message)?.to_owned();

        Ok(match outcome_type {
            0 => RequestPermissionsOutcome::Accepted,
            1 => RequestPermissionsOutcome::PermissionsDenied,
            2 => RequestPermissionsOutcome::OtherError {
                error_message: outcome_message,
            },
            _ => RequestPermissionsOutcome::OtherError {
                error_message: outcome_message,
            },
        })
    }
}
