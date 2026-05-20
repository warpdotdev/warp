use super::object::{Container, Space};
use crate::scalars::Time;
use crate::schema;
use std::fmt;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ObjectPermissions {
    pub guests: Vec<ObjectGuest>,
    pub last_updated_ts: Time,
    pub anyone_link_sharing: Option<LinkSharing>,
    pub space: Space,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ObjectGuest {
    pub access_level: AccessLevel,
    pub source: Option<Container>,
    pub subject: GuestSubject,
}

#[derive(cynic::InputObject, Debug)]
pub struct Owner {
    pub uid: Option<cynic::Id>,
    #[cynic(rename = "type")]
    pub type_: OwnerType,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum OwnerType {
    #[cynic(rename = "Team")]
    Team,
    #[cynic(rename = "User")]
    User,
}

impl fmt::Display for OwnerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let owner_type = match self {
            OwnerType::Team => "Team",
            OwnerType::User => "Personal",
        };
        write!(f, "{owner_type}")
    }
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UserGuest {
    pub firebase_uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PendingUserGuest {
    pub email: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TeamGuest {
    pub uid: cynic::Id,
}

#[derive(cynic::InlineFragments, Debug, Clone)]
pub enum GuestSubject {
    UserGuest(UserGuest),
    PendingUserGuest(PendingUserGuest),
    TeamGuest(TeamGuest),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum AccessLevel {
    #[cynic(rename = "Editor")]
    Editor,
    #[cynic(rename = "Full")]
    Full,
    #[cynic(rename = "Viewer")]
    Viewer,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LinkSharing {
    pub access_level: AccessLevel,
    pub source: Option<Container>,
}
