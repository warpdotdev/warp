use session_sharing_protocol::common::{ParticipantId, Role, RoleRequestId};
use warpui::elements::Empty;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{AppContext, Element, Entity, View, ViewContext, ViewHandle};

use crate::modal::Modal;
use crate::pane_group::TerminalPaneId;
use crate::terminal::shared_session::render_util::ParticipantAvatarParams;

mod sharer_grant_body;
mod sharer_response_body;
mod viewer_request_body;
use sharer_grant_body::{SharerGrantBody, SharerGrantBodyEvent};
use sharer_response_body::{SharerResponseBody, SharerResponseBodyEvent};
use viewer_request_body::{ViewerRequestBody, ViewerRequestBodyEvent};

pub const MODAL_WIDTH: f32 = 400.;
pub const MODAL_PADDING: f32 = 24.;
pub const BODY_PADDING: f32 = 8.;
pub const HEADER_FONT_SIZE: f32 = 16.;
pub const TEXT_FONT_SIZE: f32 = 14.;

#[derive(Debug, Clone)]
pub enum RoleChangeOpenSource {
    ViewerRequest {
        role: Role,
    },
    SharerResponse {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
    },
    SharerGrant {
        participant_id: ParticipantId,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum RoleChangeCloseSource {
    ViewerRequest,
    SharerResponse,
    SharerGrant,
}

#[derive(Debug, Clone)]
pub enum RoleChangeModalEvent {
    // Viewer request event
    CancelRequest {
        terminal_pane_id: TerminalPaneId,
        role_request_id: RoleRequestId,
    },
    // Sharer response events
    ApproveRequest {
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
    },
    DenyRequest {
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
    },
    Close {
        source: RoleChangeCloseSource,
    },
    // Sharer grant events
    CancelGrant,
    GrantRole {
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        dont_show_again: bool,
    },
}

pub struct RoleChangeModal {
    terminal_pane_id: Option<TerminalPaneId>,
    role_request_id: Option<RoleRequestId>,
    participant_id: Option<ParticipantId>,

    is_viewer_request_modal_open: bool,
    viewer_request_modal: ViewHandle<Modal<ViewerRequestBody>>,

    is_sharer_response_modal_open: bool,
    sharer_response_modal: ViewHandle<Modal<SharerResponseBody>>,

    is_sharer_grant_modal_open: bool,
    sharer_grant_modal: ViewHandle<Modal<SharerGrantBody>>,
}

impl Entity for RoleChangeModal {
    type Event = RoleChangeModalEvent;
}

impl RoleChangeModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let modal_style = UiComponentStyles {
            width: Some(MODAL_WIDTH),
            ..Default::default()
        };
        let body_style = UiComponentStyles {
            padding: Some(Coords {
                top: MODAL_PADDING,
                bottom: MODAL_PADDING,
                left: MODAL_PADDING,
                right: MODAL_PADDING,
            }),
            ..Default::default()
        };

        let viewer_request_body = ctx.add_typed_action_view(|_| ViewerRequestBody::new());
        ctx.subscribe_to_view(&viewer_request_body, |me, _, event, ctx| {
            me.handle_viewer_event(event, ctx);
        });

        let viewer_request_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, viewer_request_body, ctx)
                .with_modal_style(modal_style)
                .with_body_style(body_style)
                .with_background_opacity(100)
                .close_modal_button_disabled()
        });

        let sharer_response_body = ctx.add_typed_action_view(|_| SharerResponseBody::new());
        ctx.subscribe_to_view(&sharer_response_body, |me, _, event, ctx| {
            me.handle_sharer_response_event(event, ctx);
        });

        let sharer_response_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, sharer_response_body, ctx)
                .with_modal_style(modal_style)
                .with_body_style(body_style)
                .with_background_opacity(100)
                .close_modal_button_disabled()
        });

        let sharer_grant_body = ctx.add_typed_action_view(|_| SharerGrantBody::new());
        ctx.subscribe_to_view(&sharer_grant_body, |me, _, event, ctx| {
            me.handle_sharer_grant_event(event, ctx);
        });

        let sharer_grant_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, sharer_grant_body, ctx)
                .with_modal_style(modal_style)
                .with_body_style(body_style)
                .with_background_opacity(100)
                .close_modal_button_disabled()
        });

        Self {
            terminal_pane_id: None,
            role_request_id: None,
            participant_id: None,
            is_viewer_request_modal_open: false,
            viewer_request_modal,
            is_sharer_response_modal_open: false,
            sharer_response_modal,
            is_sharer_grant_modal_open: false,
            sharer_grant_modal,
        }
    }

    pub fn set_role_request_id(&mut self, role_request_id: RoleRequestId) {
        self.role_request_id = Some(role_request_id);
    }

    /// Opens viewer's role request modal which awaits a sharer's response.
    /// Viewer can cancel their request through this modal.
    pub fn open_for_viewer_request(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        display_name: String,
        role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        self.terminal_pane_id = Some(terminal_pane_id);
        self.is_viewer_request_modal_open = true;

        self.viewer_request_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |modal, ctx| {
                modal.open(display_name, role, ctx);
            });
        });
        ctx.notify();
    }

    /// Opens sharer's role response modal.
    /// Sharer can approve/deny role requests through this modal.
    #[allow(clippy::too_many_arguments)]
    pub fn open_for_sharer_response(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        firebase_uid: String,
        role_request_id: RoleRequestId,
        params: ParticipantAvatarParams,
        role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        self.terminal_pane_id = Some(terminal_pane_id);
        self.role_request_id = Some(role_request_id.clone());
        self.is_sharer_response_modal_open = true;

        self.sharer_response_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |modal, ctx| {
                modal.add_role_request(
                    participant_id,
                    firebase_uid,
                    role_request_id.clone(),
                    role,
                    params,
                    ctx,
                );
            });
        });
        ctx.notify();
    }

    /// Opens sharer's role grant confirmation modal.
    /// Sharer can cancel/continue the role grant through this modal.
    pub fn open_for_sharer_grant(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.terminal_pane_id = Some(terminal_pane_id);
        self.participant_id = Some(participant_id);
        self.is_sharer_grant_modal_open = true;
        ctx.notify();
    }

    pub fn all_child_modals_are_closed(&self) -> bool {
        !self.is_viewer_request_modal_open
            && !self.is_sharer_response_modal_open
            && !self.is_sharer_grant_modal_open
    }

    /// Closes viewer's role request modal.
    pub fn close_for_viewer_request(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_viewer_request_modal_open = false;
        ctx.notify();
    }

    /// Closes sharer's role response modal.
    pub fn close_for_sharer_response(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_sharer_response_modal_open = false;
        ctx.notify();
    }

    /// Closes sharer's role grant modal.
    /// Should only be closed when there are no pending role requests.
    pub fn close_for_sharer_grant(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_sharer_grant_modal_open = false;
        ctx.notify();
    }

    /// Cancels the role request identified by the role request id.
    /// Can only cancel our own role request as a viewer.
    pub fn cancel_request(&mut self, ctx: &mut ViewContext<Self>) {
        // Ensure the right modal is open before cancelling
        if !self.is_viewer_request_modal_open {
            return;
        }

        let Some(terminal_pane_id) = self.terminal_pane_id else {
            log::warn!("Tried to close role request modal when no terminal pane ID was present");
            return;
        };

        if let Some(role_request_id) = self.role_request_id.as_ref() {
            ctx.emit(RoleChangeModalEvent::CancelRequest {
                terminal_pane_id,
                role_request_id: role_request_id.clone(),
            })
        } else {
            log::warn!("Tried to cancel role request when no role request ID was present");
            // If no role request ID is present, we should still close the modal.
            ctx.emit(RoleChangeModalEvent::Close {
                source: RoleChangeCloseSource::ViewerRequest,
            })
        };
    }

    /// Removes a role request given the id, and is called when a sharer approves/denies one.
    /// Only the sharer can remove the role requests for their shared session.
    pub fn remove_role_request(
        &mut self,
        role_request_id: RoleRequestId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ensure the right modal is open before removing
        if !self.is_sharer_response_modal_open {
            return;
        }

        self.sharer_response_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |modal, ctx| {
                modal.remove_role_request(role_request_id, ctx);
            });
            ctx.notify();
        });
    }

    fn handle_viewer_event(&mut self, event: &ViewerRequestBodyEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ViewerRequestBodyEvent::Cancel => self.cancel_request(ctx),
        }
    }

    fn handle_sharer_grant_event(
        &mut self,
        event: &SharerGrantBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SharerGrantBodyEvent::Cancel => {
                if self.terminal_pane_id.is_none() {
                    log::warn!("Tried to cancel role grant when no terminal pane ID was present");
                    return;
                };
                ctx.emit(RoleChangeModalEvent::CancelGrant)
            }
            SharerGrantBodyEvent::GrantRole { dont_show_again } => {
                let Some(terminal_pane_id) = self.terminal_pane_id else {
                    log::warn!("Tried to grant role when no terminal pane ID was present");
                    return;
                };
                let Some(participant_id) = self.participant_id.clone() else {
                    log::warn!("Tried to grant role without participant ID");
                    return;
                };
                ctx.emit(RoleChangeModalEvent::GrantRole {
                    terminal_pane_id,
                    participant_id,
                    dont_show_again: *dont_show_again,
                })
            }
        }
    }

    fn handle_sharer_response_event(
        &mut self,
        event: &SharerResponseBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SharerResponseBodyEvent::Approve {
                participant_id,
                role_request_id,
                role,
            } => {
                let Some(terminal_pane_id) = self.terminal_pane_id else {
                    log::warn!(
                        "Tried to close role request modal when no terminal pane ID was present"
                    );
                    return;
                };

                ctx.emit(RoleChangeModalEvent::ApproveRequest {
                    terminal_pane_id,
                    participant_id: participant_id.clone(),
                    role_request_id: role_request_id.clone(),
                    role: *role,
                });
            }
            SharerResponseBodyEvent::Deny {
                participant_id,
                role_request_id,
            } => {
                let Some(terminal_pane_id) = self.terminal_pane_id else {
                    log::warn!(
                        "Tried to close role request modal when no terminal pane ID was present"
                    );
                    return;
                };

                ctx.emit(RoleChangeModalEvent::DenyRequest {
                    terminal_pane_id,
                    participant_id: participant_id.clone(),
                    role_request_id: role_request_id.clone(),
                });
            }
            SharerResponseBodyEvent::Close => {
                if self.terminal_pane_id.is_none() {
                    log::warn!(
                        "Tried to close role request modal when no terminal pane ID was present"
                    );
                    return;
                };

                ctx.emit(RoleChangeModalEvent::Close {
                    source: RoleChangeCloseSource::SharerResponse,
                });
            }
        }
    }
}

impl View for RoleChangeModal {
    fn ui_name() -> &'static str {
        "SharedSessionRoleChangeModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if self.is_sharer_grant_modal_open {
            ChildView::new(&self.sharer_grant_modal).finish()
        } else if self.is_sharer_response_modal_open {
            ChildView::new(&self.sharer_response_modal).finish()
        } else if self.is_viewer_request_modal_open {
            ChildView::new(&self.viewer_request_modal).finish()
        } else {
            Empty::new().finish()
        }
    }
}
