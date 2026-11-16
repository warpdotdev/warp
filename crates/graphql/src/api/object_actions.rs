use super::object::ObjectType;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum ActionType {
    Executed,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ObjectActionHistory {
    pub actions: Option<Vec<ActionRecord>>,
    pub latest_processed_at_timestamp: Option<Time>,
    pub latest_timestamp: Option<Time>,
    pub object_type: ObjectType,
    pub uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct BundledActions {
    pub action_type: ActionType,
    pub count: i32,
    pub latest_processed_at_timestamp: Time,
    pub latest_timestamp: Time,
    pub oldest_timestamp: Time,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SingleAction {
    pub action_type: ActionType,
    pub processed_at_timestamp: Time,
    pub timestamp: Time,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ActionRecord {
    BundledActions(BundledActions),
    SingleAction(SingleAction),
    #[cynic(fallback)]
    Unknown,
}
