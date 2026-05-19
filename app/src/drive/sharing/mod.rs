use std::borrow::Cow;

use chrono::{DateTime, Local};
use session_sharing_protocol::common::SessionId;
use warp_core::{channel::ChannelState, ui::appearance::Appearance};
use warpui::{
    color::ColorU,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, SingletonEntity, WeakViewHandle,
};

use crate::{
    ai::{agent::conversation::AIConversationId, blocklist::BlocklistAIHistoryModel},
    cloud_object::model::persistence::CloudModel,
    server::{ids::ServerId, server_api::object::GuestIdentifier},
    terminal::{shared_session::join_link, TerminalView},
    ui_components::{
        avatar::{Avatar, AvatarContent},
        icons::Icon,
    },
    workspaces::{user_profiles::UserProfiles, user_workspaces::UserWorkspaces},
};

pub mod dialog;
mod style;

// Re-export types from cloud_objects.
pub use cloud_objects::drive::sharing::{
    LinkSharingSubjectType, SharingAccessLevel, Subject, TeamKind, UserKind,
};

/// Identifier for an object that's shareable via the Warp Drive ACL model. Not all sharing in Warp
/// is _currently_ tied into this model (e.g. block sharing).
#[derive(Debug, Clone)]
pub enum ShareableObject {
    /// A shareable Warp Drive object.
    WarpDriveObject(ServerId),
    /// A shared terminal session. Shared sessions are identified by the participating terminal
    /// pane.
    Session {
        handle: WeakViewHandle<TerminalView>,
        session_id: SessionId,
        started_at: DateTime<Local>,
    },
    /// An AI conversation.
    AIConversation(AIConversationId),
}

impl ShareableObject {
    /// The canonical link to this object.
    pub fn link(&self, app: &AppContext) -> Option<String> {
        match self {
            ShareableObject::WarpDriveObject(id) => CloudModel::as_ref(app)
                .get_by_uid(&id.uid())
                .and_then(|object| object.object_link()),
            ShareableObject::Session { session_id, .. } => Some(join_link(session_id)),
            ShareableObject::AIConversation(id) => {
                // Use the unified helper that checks both loaded conversation and historical metadata
                BlocklistAIHistoryModel::as_ref(app)
                    .get_server_conversation_metadata(id)
                    .map(|m| {
                        format!(
                            "{}/conversation/{}",
                            ChannelState::server_root_url(),
                            m.server_conversation_token.as_str()
                        )
                    })
            }
        }
    }
}

/// Whether not a shared object's contents are editable by the current user.
///
/// This is not purely a function of their access level since anonymous users are not allowed to
/// edit (due to the lack of attribution).
#[derive(Debug, Clone, Copy)]
pub enum ContentEditability {
    ReadOnly,
    RequiresLogin,
    Editable,
}

impl ContentEditability {
    pub fn can_edit(self) -> bool {
        matches!(self, ContentEditability::Editable)
    }
}

/// Extension trait for Subject with methods that require AppContext.
pub trait SubjectExt {
    /// The name of this subject.
    fn name(&self, app: &AppContext) -> Option<Cow<'static, str>>;
    /// Detail text to display under this subject's name.
    fn detail(&self, app: &AppContext) -> Option<String>;
    /// Avatar component to show for this subject.
    fn avatar(&self, appearance: &Appearance, app: &AppContext) -> Avatar;
    /// Gets the email address for this subject, if it has one.
    fn email<'a>(&'a self, app: &'a AppContext) -> Option<&'a str>;
    /// Checks if this subject refers to the same user as an email address.
    fn matches_email(&self, email: &str, app: &AppContext) -> bool;
    /// Converts this subject to a [`GuestIdentifier`] for guest removal.
    /// Returns `Some` for team or user subjects (that have an email), `None` otherwise.
    fn to_guest_identifier(&self, app: &AppContext) -> Option<GuestIdentifier>;
}

impl SubjectExt for Subject {
    fn name(&self, app: &AppContext) -> Option<Cow<'static, str>> {
        match self {
            Subject::User(kind) => kind.name(app),
            Subject::PendingUser { email } => email.clone().map(Cow::from),
            Subject::Team(kind) => kind.display_name(app).map(Cow::from),
            Subject::AnyoneWithLink(_) => Some(Cow::from("Anyone with the link")),
        }
    }

    fn detail(&self, app: &AppContext) -> Option<String> {
        if let Subject::User(kind) = self {
            kind.detail(app)
        } else {
            None
        }
    }

    fn avatar(&self, appearance: &Appearance, app: &AppContext) -> Avatar {
        match self {
            Subject::User(kind) => named_subject_avatar(kind.avatar_content(app), appearance),
            Subject::PendingUser { email } => named_subject_avatar(
                AvatarContent::DisplayName(email.clone().unwrap_or_default()),
                appearance,
            ),
            Subject::Team(_) => icon_avatar(Icon::Users, appearance),
            Subject::AnyoneWithLink(subject_type) => {
                let icon = match subject_type {
                    LinkSharingSubjectType::Anyone => Icon::Globe,
                    LinkSharingSubjectType::None => Icon::Lock,
                };
                icon_avatar(icon, appearance)
            }
        }
    }

    fn email<'a>(&'a self, app: &'a AppContext) -> Option<&'a str> {
        match self {
            Subject::User(user_kind) => match user_kind {
                UserKind::Account(user_uid) => UserProfiles::as_ref(app)
                    .profile_for_uid(*user_uid)
                    .map(|profile| profile.email.as_str()),
                UserKind::SharedSessionParticipant(profile_data) => profile_data.email.as_deref(),
            },
            Subject::PendingUser { email } => email.as_deref(),
            Subject::Team(_) => None,
            Subject::AnyoneWithLink(_) => None,
        }
    }

    fn matches_email(&self, email: &str, app: &AppContext) -> bool {
        self.email(app)
            .is_some_and(|subject_email| subject_email == email)
    }

    fn to_guest_identifier(&self, app: &AppContext) -> Option<GuestIdentifier> {
        if let Some(team_uid) = self.team_uid() {
            return Some(GuestIdentifier::TeamUid(team_uid));
        }
        if let Some(email) = self.email(app) {
            return Some(GuestIdentifier::Email(email.to_owned()));
        }
        None
    }
}

/// Extension trait for UserKind with methods that require AppContext.
pub trait UserKindExt {
    /// Gets the display name for this user kind.
    fn name(&self, app: &AppContext) -> Option<Cow<'static, str>>;
    /// Detail text to display under this user's name.
    fn detail(&self, app: &AppContext) -> Option<String>;
    /// Avatar content for this user kind.
    fn avatar_content(&self, app: &AppContext) -> AvatarContent;
}

impl UserKindExt for UserKind {
    fn name(&self, app: &AppContext) -> Option<Cow<'static, str>> {
        match self {
            UserKind::Account(id) => UserProfiles::as_ref(app)
                .displayable_identifier_for_uid(*id)
                .map(Cow::from),
            UserKind::SharedSessionParticipant(participant_info) => {
                Some(participant_info.display_name.clone().into())
            }
        }
    }

    fn detail(&self, app: &AppContext) -> Option<String> {
        match self {
            UserKind::Account(uid) => {
                let profile = UserProfiles::as_ref(app).profile_for_uid(*uid)?;
                // Only show the user's email if we're already showing a display name.
                if profile.display_name.is_some() {
                    Some(profile.email.clone())
                } else {
                    None
                }
            }
            UserKind::SharedSessionParticipant(participant_info) => {
                // Only show the user's email if it's not the display name.
                if participant_info
                    .email
                    .as_ref()
                    .is_some_and(|email| email == &participant_info.display_name)
                {
                    None
                } else {
                    participant_info.email.clone()
                }
            }
        }
    }

    fn avatar_content(&self, app: &AppContext) -> AvatarContent {
        match self {
            UserKind::Account(uid) => match UserProfiles::as_ref(app).profile_for_uid(*uid) {
                Some(profile) => AvatarContent::Image {
                    url: profile.photo_url.clone(),
                    display_name: profile.displayable_identifier(),
                },
                None => AvatarContent::DisplayName(String::new()),
            },
            UserKind::SharedSessionParticipant(participant_info) => {
                match &participant_info.photo_url {
                    Some(url) => AvatarContent::Image {
                        url: url.clone(),
                        display_name: participant_info.display_name.clone(),
                    },
                    None => AvatarContent::DisplayName(participant_info.display_name.clone()),
                }
            }
        }
    }
}

/// Extension trait for TeamKind with methods that require AppContext.
pub trait TeamKindExt {
    /// Gets the display name for this team kind.
    fn display_name(&self, app: &AppContext) -> Option<String>;
}

impl TeamKindExt for TeamKind {
    fn display_name(&self, app: &AppContext) -> Option<String> {
        match self {
            TeamKind::Team { team_uid, .. } => UserWorkspaces::as_ref(app)
                .team_from_uid(*team_uid)
                .map(|team| team.name.clone()),
            TeamKind::SharedSessionTeam { name, .. } => Some(name.clone()),
        }
    }
}

/// Helper to build an [Avatar] that shows a named subject.
fn named_subject_avatar(content: AvatarContent, appearance: &Appearance) -> Avatar {
    Avatar::new(
        content,
        UiComponentStyles {
            // TODO: Apply session-sharing color logic.
            background: Some(ColorU::new(93, 202, 60, 255).into()),
            font_color: Some(ColorU::black()),
            ..Default::default()
        },
    )
    .with_style(style::subject_avatar_styles(appearance))
}

/// Helper to build an [Avatar] that shows a subject icon.
fn icon_avatar(icon: Icon, appearance: &Appearance) -> Avatar {
    Avatar::new(
        AvatarContent::Icon(icon),
        UiComponentStyles {
            font_color: Some(style::acl_secondary_text_color(appearance)),
            ..Default::default()
        },
    )
    .with_style(style::subject_avatar_styles(appearance))
}
