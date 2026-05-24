//! Instance selection helpers for local-control clients.
use crate::discovery::{InstanceId, InstanceRecord};
use crate::protocol::{ControlError, ErrorCode};

/// CLI-level selector for choosing one discovered Warp instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceSelector {
    Active,
    Id(InstanceId),
    Pid(u32),
}

pub fn select_instance(
    records: &[InstanceRecord],
    selector: &InstanceSelector,
) -> Result<InstanceRecord, ControlError> {
    match selector {
        InstanceSelector::Active => select_active(records),
        InstanceSelector::Id(instance_id) => records
            .iter()
            .find(|record| &record.instance_id == instance_id)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::NoInstance,
                    format!("no Warp instance with id {}", instance_id.0),
                )
            }),
        InstanceSelector::Pid(pid) => records
            .iter()
            .find(|record| record.pid == *pid)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::NoInstance,
                    format!("no Warp instance with pid {pid}"),
                )
            }),
    }
}

fn select_active(records: &[InstanceRecord]) -> Result<InstanceRecord, ControlError> {
    match records {
        [] => Err(ControlError::new(
            ErrorCode::NoInstance,
            "no local Warp control instances were discovered",
        )),
        [record] => Ok(record.clone()),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousInstance,
            "multiple local Warp control instances were discovered; pass --instance",
        )),
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
