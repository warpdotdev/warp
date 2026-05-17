use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
pub use cloud_object_client::{
    ObjectAction, ObjectActionHistory, ObjectActionSubtype, ObjectActionType,
};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::model::PersistedObjectAction;
use crate::server::ids::{parse_sqlite_id_to_uid, HashedSqliteId, ObjectUid};

pub enum ObjectActionsEvent {}

pub fn object_action_from_persisted(other: PersistedObjectAction) -> Result<ObjectAction, ()> {
    // Each persisted object action is either a single action or a bundled action.
    // If there's any inconsistencies from the SQL row, we return an error.
    let action_subtype = if let Some(count) = other.count {
        let oldest_timestamp = other
            .oldest_timestamp
            .as_ref()
            .map(|time| time.and_utc())
            .ok_or(())?;
        let latest_timestamp = other
            .latest_timestamp
            .as_ref()
            .map(|time| time.and_utc())
            .ok_or(())?;

        // When the db row is a bundled action, the processed_at_timestamp field refers
        // to the latest processed_at_timestamp in the bundle. Because bundled actions come
        // from the server, this is a value, not an option.
        let latest_processed_at_timestamp = other
            .processed_at_timestamp
            .as_ref()
            .map(|time| time.and_utc())
            .ok_or(())?;
        ObjectActionSubtype::BundledActions {
            count,
            oldest_timestamp,
            latest_timestamp,
            latest_processed_at_timestamp,
        }
    } else {
        let timestamp = other
            .timestamp
            .as_ref()
            .map(|time| time.and_utc())
            .ok_or(())?;
        let pending = other.pending.ok_or(())?;

        // The processed_at_timestamp is still None when the action hasn't been synced.
        let processed_at_timestamp = other
            .processed_at_timestamp
            .as_ref()
            .map(|time| time.and_utc());
        ObjectActionSubtype::SingleAction {
            timestamp,
            data: other.data,
            pending,
            processed_at_timestamp,
        }
    };

    // The object_sync_id stored in SQLite is the hashed id that's used to index into the ObjectActions
    // model.
    let hashed_object_id = other.hashed_object_id;
    let action_type = match other.action.as_str() {
        s if s == ObjectActionType::Execute.to_string() => ObjectActionType::Execute,
        _ => return Err(()),
    };

    // NOTE: This is needed since we only store the sqlite hash, but we need the uid (the second part of the hash)
    // to index into CloudModel and store the object actions in memory.
    let uid = parse_sqlite_id_to_uid(hashed_object_id.clone())?;

    Ok(ObjectAction {
        uid: uid.to_string(),
        hashed_sqlite_id: hashed_object_id,
        action_type,
        action_subtype,
    })
}

/// A singleton model representing the actions that have occurred on a per-object basis. These
/// represent actions taken by the user or by teammates. The actions have a pending status that is
/// true when the server doesn't know about it and is false anytime after the action is successfully
/// synced.
pub struct ObjectActions {
    #[allow(dead_code)]
    object_actions_by_id: HashMap<ObjectUid, Vec<ObjectAction>>,
}

impl ObjectActions {
    /// Accepts a vector of object actions read out of SQLite.
    pub fn new(persisted_actions: Vec<ObjectAction>) -> Self {
        // Partitions the actions by object id and plops them into the map.
        let object_actions_by_id = persisted_actions.into_iter().fold(
            HashMap::new(),
            |mut map: HashMap<ObjectUid, Vec<ObjectAction>>, object_action| {
                map.entry(object_action.uid.clone())
                    .or_default()
                    .push(object_action);
                map
            },
        );

        Self {
            object_actions_by_id,
        }
    }

    /// Insert a single action into the model. Returns the created action.
    pub fn insert_action(
        &mut self,
        uid: ObjectUid,
        hashed_sqlite_id: HashedSqliteId,
        action_type: ObjectActionType,
        data: Option<String>,
        timestamp: DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) -> ObjectAction {
        // Create an action with pending=true.
        let action = ObjectAction {
            action_type,
            uid: uid.clone(),
            hashed_sqlite_id,
            action_subtype: ObjectActionSubtype::SingleAction {
                timestamp,
                data,
                pending: true,
                processed_at_timestamp: None,
            },
        };

        // Insert the action into the model.
        self.object_actions_by_id
            .entry(uid)
            .or_default()
            .push(action.clone());

        ctx.notify();

        action
    }

    /// Remove the action from the model with the corresponding object_id, timestamp, and pending=true.
    pub fn remove_pending_action(
        &mut self,
        uid: &ObjectUid,
        timestamp_of_action: &DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(actions) = self.object_actions_by_id.get_mut(uid) {
            // Remove the action that has a matching timestamp and pending=true
            if let Some(index) = actions.iter().position(|a| {
                matches!(
                    &a.action_subtype,
                    ObjectActionSubtype::SingleAction {
                        timestamp,
                        pending: true,
                        ..
                    } if timestamp == timestamp_of_action
                )
            }) {
                actions.remove(index);
            } else {
                log::warn!(
                    "Could not find the pending action to remove from the ObjectActions model"
                )
            }
        } else {
            log::warn!("Could not find the object id in the ObjectActions model")
        }
        ctx.notify();
    }

    /// Get the processed_at_timestamp of the most recent server-synced action we have for a given object. This determines
    /// whether or not we should accept some update from the server.
    pub fn get_latest_processed_at_ts(&self, uid: &ObjectUid) -> Option<DateTime<Utc>> {
        if let Some(actions) = self.object_actions_by_id.get(uid) {
            actions
                .iter()
                .filter_map(|a| match a.action_subtype {
                    ObjectActionSubtype::SingleAction {
                        processed_at_timestamp,
                        pending: false,
                        ..
                    } => processed_at_timestamp,
                    ObjectActionSubtype::BundledActions {
                        latest_processed_at_timestamp,
                        ..
                    } => Some(latest_processed_at_timestamp),
                    _ => None,
                })
                .max()
        } else {
            None
        }
    }

    /// Takes a list of ObjectActions for a single object from the server and replaces the existing actions
    /// for this object with the new ones. Any pending actions are persisted so we make sure we don't delete actions
    /// that are currently in the process of syncing.
    pub fn overwrite_action_history_for_object(
        &mut self,
        uid: &ObjectUid,
        mut actions: Vec<ObjectAction>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Get the pending actions out of the old set.
        let old_pending_actions: Vec<ObjectAction> = self
            .object_actions_by_id
            .get(uid)
            .map(|actions| {
                actions
                    .iter()
                    .filter(|a| {
                        matches!(
                            a.action_subtype,
                            ObjectActionSubtype::SingleAction { pending: true, .. }
                        )
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        actions.extend(old_pending_actions);

        self.object_actions_by_id
            .insert(uid.to_string(), actions.clone());
        ctx.notify();
    }

    /// Returns a time-boxed summary of the number of times this action type has occurred on this object.
    /// This summary prioritizes smaller units of time where possible, starting from Day and going to Year.
    /// If the action type has occurred on the object in the last day, we return "X actions in the last day".
    /// If not, we increase the time unit from Day to Week to Month. If no actions have occurred in the last month,
    /// we return however many actions have occurred in the last year, possibly 0.
    ///
    /// This function operates by cloning a filtered Iterator<Item=&ObjectAction>, saving some performance overhead
    /// by cloning references instead of objects.
    pub fn get_action_history_summary_for_action_type(
        &self,
        uid: &ObjectUid,
        action_type: ObjectActionType,
    ) -> Option<String> {
        // If the object is not in the model, return 0.
        let all_actions_on_this_object = self.object_actions_by_id.get(uid);
        if all_actions_on_this_object.is_none() {
            return Some("0 runs in the last year".to_string());
        }

        // If the object doesn't have any of these action types recorded, return 0.
        let all_relevant_actions = all_actions_on_this_object?
            .iter()
            .filter(|a| a.action_type == action_type);
        if all_relevant_actions.clone().count() == 0 {
            return Some("0 runs in the last year".to_string());
        }

        // If the action has occurred in the last day, return Day as the time unit.
        let one_day_ago = Utc::now() - Duration::days(1);
        let in_the_last_day = all_relevant_actions.clone().filter(|a| matches!(a.action_subtype, ObjectActionSubtype::SingleAction { timestamp, .. } if timestamp > one_day_ago)).count();
        if in_the_last_day > 0 {
            return Some(format!(
                "{} {} in the last day",
                in_the_last_day,
                if in_the_last_day == 1 {
                    action_type.singular()
                } else {
                    action_type.plural()
                }
            ));
        }

        // If the action has occurred in the last week, return Week as the time unit.
        let one_week_ago = Utc::now() - Duration::days(7);
        let in_the_last_week = all_relevant_actions.clone().filter(|a| matches!(a.action_subtype, ObjectActionSubtype::SingleAction { timestamp, .. } if timestamp > one_week_ago)).count();
        if in_the_last_week > 0 {
            return Some(format!(
                "{} {} in the last week",
                in_the_last_week,
                if in_the_last_week == 1 {
                    action_type.singular()
                } else {
                    action_type.plural()
                }
            ));
        }

        // If the action has occurred in the last month, return Month as the time unit.
        let one_month_ago = Utc::now() - Duration::days(30);
        let in_the_last_month = all_relevant_actions.clone().filter(|a| matches!(a.action_subtype, ObjectActionSubtype::SingleAction { timestamp, .. } if timestamp > one_month_ago)).count();
        if in_the_last_month > 0 {
            return Some(format!(
                "{} {} in the last month",
                in_the_last_month,
                if in_the_last_month == 1 {
                    action_type.singular()
                } else {
                    action_type.plural()
                }
            ));
        }

        // Finally, if all else turned up fruitless, return the yearly count.
        let one_year_ago = Utc::now() - Duration::days(365);
        let in_the_last_year: i32 = all_relevant_actions
            .clone()
            .filter_map(|a| match a.action_subtype {
                ObjectActionSubtype::SingleAction { timestamp, .. } if timestamp > one_year_ago => {
                    Some(1)
                }
                ObjectActionSubtype::BundledActions {
                    count,
                    oldest_timestamp,
                    ..
                } if oldest_timestamp > one_year_ago => Some(count),
                _ => None,
            })
            .sum();

        Some(format!(
            "{} {} in the last year",
            in_the_last_year,
            if in_the_last_year == 1 {
                action_type.singular()
            } else {
                action_type.plural()
            }
        ))
    }

    /// Returns all the actions on the objects specified by the parameter hashed_object_ids.
    /// The return value is a HashMap, which represents a subset of the model, filtered to just the actions
    /// that occurred on the requested objects.
    pub fn get_actions_for_objects(
        &self,
        uids: Vec<&ObjectUid>,
    ) -> HashMap<ObjectUid, Vec<ObjectAction>> {
        uids.iter()
            .map(|&uid| {
                let actions_on_this_object = self
                    .object_actions_by_id
                    .get(uid)
                    .cloned()
                    .unwrap_or_default();
                (uid.clone(), actions_on_this_object)
            })
            .collect()
    }

    pub fn delete_actions_for_object(&mut self, uid: &ObjectUid, ctx: &mut ModelContext<Self>) {
        self.object_actions_by_id.remove(uid);
        ctx.notify()
    }

    #[cfg(test)]
    pub fn count_actions_for_object(&mut self, uid: &ObjectUid) -> usize {
        self.object_actions_by_id.get(uid).map_or(0, |v| v.len())
    }
}

impl Entity for ObjectActions {
    type Event = ObjectActionsEvent;
}

impl SingletonEntity for ObjectActions {}

#[cfg(test)]
#[path = "actions_tests.rs"]
pub mod tests;
