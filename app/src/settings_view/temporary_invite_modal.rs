use chrono::{DateTime, Utc};
use warpui::clipboard::ClipboardContent;
use warpui::elements::{Flex, MouseStateHandle, ParentElement, Text};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::auth::{AuthStateProvider, UserUid};
use crate::editor::{EditorView, InteractionState, SingleLineEditorOptions};
use crate::server::ids::ServerId;
use crate::server::server_api::team::TemporaryInviteCode;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::WorkspaceUid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct InviteTarget {
    pub workspace_uid: WorkspaceUid,
    pub team_uid: ServerId,
    pub actor_uid: UserUid,
}

#[derive(Debug)]
pub(super) enum TemporaryInviteAction {
    Copy,
}

pub(super) struct TemporaryInviteModal {
    target: Option<InviteTarget>,
    destination: String,
    domains: String,
    pending: bool,
    epoch: u64,
    result: Option<TemporaryInviteCode>,
    message: Option<String>,
    link_editor: ViewHandle<EditorView>,
    copy_mouse_state: MouseStateHandle,
}

impl TemporaryInviteModal {
    pub(super) fn new(ctx: &mut ViewContext<Self>) -> Self {
        let link_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    select_all_on_focus: true,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_interaction_state(InteractionState::Selectable, ctx);
            editor
        });
        Self {
            target: None,
            destination: String::new(),
            domains: String::new(),
            pending: false,
            epoch: 0,
            result: None,
            message: None,
            link_editor,
            copy_mouse_state: Default::default(),
        }
    }

    pub(super) fn open(
        &mut self,
        target: InviteTarget,
        destination: String,
        domains: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.clear(ctx);
        self.target = Some(target);
        self.destination = destination;
        self.domains = domains;
        ctx.notify();
    }

    fn finish_generation(
        &mut self,
        epoch: u64,
        target: InviteTarget,
        result: anyhow::Result<TemporaryInviteCode>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.epoch != epoch || self.target != Some(target) {
            return;
        }
        let auth = AuthStateProvider::as_ref(ctx).get();
        if auth.user_id() != Some(target.actor_uid)
            || auth.is_anonymous_or_logged_out()
            || auth.is_service_account()
        {
            self.clear(ctx);
            return;
        }
        self.pending = false;
        match result {
            Ok(result) => {
                self.link_editor.update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Editable, ctx);
                    editor.set_buffer_text(&result.invite_link, ctx);
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
                self.result = Some(result);
                self.copy(ctx);
            }
            Err(_) => {
                self.message = Some("Could not create an invite link. Retry when ready; a new request creates a separate link.".into());
            }
        }
        ctx.notify();
    }

    pub(super) fn target(&self) -> Option<InviteTarget> {
        self.target
    }

    pub(super) fn clear(&mut self, ctx: &mut ViewContext<Self>) {
        self.epoch += 1;
        self.target = None;
        self.pending = false;
        self.result = None;
        self.message = None;
        self.destination.clear();
        self.domains.clear();
        self.link_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.notify();
    }

    fn copy(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(result) = self.result.as_ref() else {
            return;
        };
        if invite_expired(result.expires_at, Utc::now()) {
            self.message =
                Some("This link has expired. Close this dialog and create a new link.".into());
            self.link_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx)
            });
            ctx.notify();
            return;
        }
        ctx.clipboard()
            .write(ClipboardContent::plain_text(result.invite_link.clone()));
        self.message = Some(if ctx.clipboard().read().plain_text == result.invite_link {
            "Link copied to clipboard.".into()
        } else {
            "Could not confirm clipboard access. Select and copy the link below, or retry Copy."
                .into()
        });
        ctx.notify();
    }
}

fn invite_expired(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now >= expires_at
}

impl Entity for TemporaryInviteModal {
    type Event = ();
}

impl TypedActionView for TemporaryInviteModal {
    type Action = TemporaryInviteAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TemporaryInviteAction::Copy => {
                if self.pending {
                    return;
                }
                let Some(target) = self.target else { return };
                let auth = AuthStateProvider::as_ref(ctx).get();
                if auth.user_id() != Some(target.actor_uid)
                    || auth.is_anonymous_or_logged_out()
                    || auth.is_service_account()
                {
                    self.clear(ctx);
                    return;
                }
                if self.result.is_some() {
                    self.copy(ctx);
                    return;
                }
                self.pending = true;
                self.message = None;
                let epoch = self.epoch;
                let future = UserWorkspaces::as_ref(ctx)
                    .generate_temporary_invite_code(target.workspace_uid, Some(target.team_uid));
                let _ = ctx.spawn(future, move |me, result, ctx| {
                    me.finish_generation(epoch, target, result, ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl View for TemporaryInviteModal {
    fn ui_name() -> &'static str {
        "TemporaryInviteModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let text = |value: String| {
            Text::new(
                value,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish()
        };
        let mut content = Flex::column().with_spacing(12.)
            .with_child(text(format!("Invite to {}", self.destination)))
            .with_child(text(self.domains.clone()))
            .with_child(text(match &self.result {
                Some(result) => format!("Expires {}", result.expires_at.format("%b %d, %Y at %H:%M UTC")),
                None => "This link expires in 3 days. Anyone who meets your workspace restrictions can use it until then.".into(),
            }));
        if let Some(message) = &self.message {
            content.add_child(text(message.clone()));
        }
        if self.result.is_some() {
            content.add_child(
                TextInput::new(self.link_editor.clone(), UiComponentStyles::default())
                    .build()
                    .finish(),
            );
        }
        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.copy_mouse_state.clone())
            .with_centered_text_label(
                if self.pending {
                    "Creating…"
                } else if self.result.is_some() {
                    "Copy link again"
                } else {
                    "Copy invite link"
                }
                .into(),
            );
        if self.pending {
            button = button.disabled();
        }
        content.add_child(
            button
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(TemporaryInviteAction::Copy);
                })
                .finish(),
        );
        content.finish()
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "temporary_invite_modal_tests.rs"]
mod tests;
