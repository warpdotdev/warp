use std::default::Default;
use std::sync::Arc;

use parking_lot::FairMutex;
use style::{DENIED_MODAL_WIDTH, MODAL_HEIGHT, MODAL_WIDTH};
use warp_core::ui::appearance::Appearance;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::ui_components::components::UiComponentStyles;
use warpui::{
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::modal::{Modal, ModalEvent};
use crate::pane_group::TerminalPaneId;
use crate::terminal::TerminalModel;
use crate::ui_components::icons::Icon;

mod body;
mod denied_body;
mod style;

use body::Body;
use denied_body::{DeniedBody, DeniedBodyEvent};

use self::body::BodyEvent;
use super::{SharedSessionActionSource, SharedSessionScrollbackType};

const MODAL_HEADER: &str = "Share session";
const SESSION_LIMIT_REACHED_HEADER: &str = "Shared session limit reached";

pub struct ShareSessionModal {
    modal: ViewHandle<Modal<Body>>,
    denied_modal: ViewHandle<Modal<DeniedBody>>,
    is_denied_modal_open: bool,
    terminal_pane_id: Option<TerminalPaneId>,
    /// Where we opened the modal from.
    open_source: SharedSessionActionSource,
}

#[derive(Debug)]
pub enum ShareSessionModalAction {
    Cancel,
}

pub enum ShareSessionModalEvent {
    Close,
    StartSharing {
        terminal_pane_id: TerminalPaneId,
        scrollback_type: SharedSessionScrollbackType,
        source: SharedSessionActionSource,
    },
    Upgrade,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ShareSessionModalAction::Cancel,
        id!("ShareSessionModal"),
    )]);
}

impl ShareSessionModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let body = ctx.add_typed_action_view(Body::new);
        ctx.subscribe_to_view(&body, move |me, _, event, ctx| {
            me.handle_body_event(event, ctx);
        });

        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some(MODAL_HEADER.to_string()), body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(MODAL_WIDTH),
                    height: Some(MODAL_HEIGHT),
                    ..Default::default()
                })
                .with_header_style(style::modal_header_styles())
                .with_body_style(style::modal_body_styles())
                .with_background_opacity(100)
                .with_dismiss_on_click()
                .close_modal_button_disabled()
        });

        let denied_body = ctx.add_typed_action_view(DeniedBody::new);
        ctx.subscribe_to_view(&denied_body, move |me, _, event, ctx| {
            me.handle_denied_body_event(event, ctx)
        });
        let denied_modal = ctx.add_typed_action_view(|ctx| {
            let mut denied_modal = Modal::new(
                Some(SESSION_LIMIT_REACHED_HEADER.to_string()),
                denied_body,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(DENIED_MODAL_WIDTH),
                ..Default::default()
            })
            .with_header_style(style::modal_header_styles())
            .with_body_style(style::modal_body_styles())
            .with_background_opacity(100)
            .with_dismiss_on_click();

            let appearance = Appearance::as_ref(ctx);
            denied_modal.set_header_icon(Some(Icon::Share));
            denied_modal.set_header_icon_color(Some(appearance.theme().accent()));
            denied_modal
        });
        ctx.subscribe_to_view(&denied_modal, |me, _, event, ctx| match event {
            ModalEvent::Close => me.close(ctx),
        });

        Self {
            modal,
            denied_modal,
            is_denied_modal_open: false,
            terminal_pane_id: None,
            // This should get overwritten when the modal is actually opened.
            open_source: SharedSessionActionSource::Tab,
        }
    }

    /// Closes the share session modal. If `focus_pane` is `true` we will return focus to the most
    /// recently focused pane.
    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if self.terminal_pane_id.is_none() {
            log::warn!("Tried to close share modal when no terminal pane ID was present");
            return;
        };
        self.is_denied_modal_open = false;
        ctx.emit(ShareSessionModalEvent::Close);
        ctx.notify();
    }

    fn start_sharing(
        &mut self,
        scrollback_type: SharedSessionScrollbackType,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_pane_id) = self.terminal_pane_id else {
            return;
        };
        ctx.emit(ShareSessionModalEvent::StartSharing {
            terminal_pane_id,
            scrollback_type,
            source: self.open_source,
        });
    }

    pub fn open(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        open_source: SharedSessionActionSource,
        model: Arc<FairMutex<TerminalModel>>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.terminal_pane_id = Some(terminal_pane_id);
        self.open_source = open_source;
        self.modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |modal, ctx| {
                modal.open(open_source, model, terminal_view_id, ctx);
            });
        });
        ctx.notify();
    }

    pub fn open_denied(&mut self, terminal_pane_id: TerminalPaneId, ctx: &mut ViewContext<Self>) {
        self.terminal_pane_id = Some(terminal_pane_id);
        self.open_source = SharedSessionActionSource::NonUser;
        self.is_denied_modal_open = true;
        ctx.notify();
    }

    fn handle_body_event(&mut self, event: &BodyEvent, ctx: &mut ViewContext<Self>) {
        match event {
            BodyEvent::Close => self.close(ctx),
            BodyEvent::StartSharing { scrollback_type } => {
                self.start_sharing(*scrollback_type, ctx)
            }
        }
    }

    fn handle_denied_body_event(&mut self, event: &DeniedBodyEvent, ctx: &mut ViewContext<Self>) {
        match event {
            DeniedBodyEvent::Upgrade => {
                self.close(ctx);
                ctx.emit(ShareSessionModalEvent::Upgrade)
            }
        }
    }

    #[cfg(test)]
    pub fn terminal_pane_id(&self) -> Option<TerminalPaneId> {
        self.terminal_pane_id
    }
}

impl Entity for ShareSessionModal {
    type Event = ShareSessionModalEvent;
}

impl View for ShareSessionModal {
    fn ui_name() -> &'static str {
        "ShareSessionModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if self.is_denied_modal_open {
            ChildView::new(&self.denied_modal).finish()
        } else {
            ChildView::new(&self.modal).finish()
        }
    }
}

impl TypedActionView for ShareSessionModal {
    type Action = ShareSessionModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ShareSessionModalAction::Cancel => self.close(ctx),
        }
    }
}
