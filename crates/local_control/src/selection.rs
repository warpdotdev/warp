use crate::discovery::{InstanceId, InstanceRecord};
use crate::protocol::{ControlError, ErrorCode};

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
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::discovery::ControlEndpoint;
    use crate::protocol::{ActionKind, PROTOCOL_VERSION};

    fn record(id: &str, pid: u32) -> InstanceRecord {
        InstanceRecord {
            protocol_version: PROTOCOL_VERSION,
            instance_id: InstanceId(id.to_owned()),
            pid,
            channel: "local".to_owned(),
            app_id: "dev.warp.WarpLocal".to_owned(),
            app_version: None,
            started_at: Utc::now(),
            executable_path: None,
            endpoint: ControlEndpoint::localhost(4000),
            credential_broker: crate::discovery::CredentialBrokerReference {
                endpoint: ControlEndpoint::localhost(4000),
            },
            actions: vec![ActionKind::TabCreate.metadata()],
        }
    }

    #[test]
    fn selects_instance_by_id() {
        let records = vec![record("one", 1), record("two", 2)];
        let selected = select_instance(&records, &InstanceSelector::Id(InstanceId("two".into())))
            .expect("selected");
        assert_eq!(selected.pid, 2);
    }

    #[test]
    fn active_selector_rejects_ambiguity() {
        let records = vec![record("one", 1), record("two", 2)];
        let err = select_instance(&records, &InstanceSelector::Active).expect_err("ambiguous");
        assert_eq!(err.code, ErrorCode::AmbiguousInstance);
    }
}
