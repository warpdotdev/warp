//! Support for pane contents that are shareable, like sessions and Warp Drive objects.
//!
//! This is tightly coupled to the pane header so that different overlays (context menus, the
//! sharing dialog, and so on) are correctly displayed.

use warp_core::{features::FeatureFlag, ui::appearance::Appearance};
use warpui::{
    elements::{MouseStateHandle, ParentElement},
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Element, ViewContext, ViewHandle,
};

use warp_core::ui::theme::Fill;
use warpui::elements::ConstrainedBox;

use crate::{
    drive::sharing::{
        dialog::{SharingDialog, SharingDialogEvent},
        ContentEditability, ShareableObject,
    },
    pane_group::BackingView,
    server::telemetry::SharingDialogSource,
    ui_components::buttons::{icon_button, icon_button_with_color},
    ui_components::icons::Icon,
};

use super::{Event, OpenOverlay, PaneHeader, PaneHeaderAction};

const UNSHARABLE_CONVERSATION_TOOLTIP: &str =
    "This conversation cannot be shared because it is not \
    stored in the cloud.\nTo sync to cloud and share, enable the setting under Settings > Privacy, \
    and then make another request.";

/// Pane header component for sharing the pane contents.
pub struct SharedPaneContent {
    sharing_dialog: ViewHandle<SharingDialog>,

    /// Mouse state handle for the primary sharing action.
    /// * If the object is view-only, this is a "copy link" button
    /// * Otherwise, this is a "share" button
    primary_button_handle: MouseStateHandle,

    /// Mouse state for the secondary view-only indicator.
    view_only_icon_handle: MouseStateHandle,
}

impl SharedPaneContent {
    pub fn new<P: BackingView>(ctx: &mut ViewContext<PaneHeader<P>>) -> Self {
        let sharing_dialog = ctx.add_typed_action_view(|ctx| SharingDialog::new(None, ctx));
        ctx.subscribe_to_view(&sharing_dialog, move |me, _, event, ctx| {
            me.handle_sharing_dialog_event(event, ctx);
        });
        Self {
            sharing_dialog,
            primary_button_handle: Default::default(),
            view_only_icon_handle: Default::default(),
        }
    }
}

impl<P: BackingView> PaneHeader<P> {
    pub fn set_shareable_object(
        &mut self,
        shareable_object: Option<ShareableObject>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.sharing_dialog().update(ctx, |dialog, ctx| {
            dialog.set_target(shareable_object, ctx);
        })
    }

    pub fn sharing_dialog(&self) -> &ViewHandle<SharingDialog> {
        &self.shared_content.sharing_dialog
    }

    pub fn has_shareable_object<C: warpui::ViewAsRef>(&self, ctx: &C) -> bool {
        self.sharing_dialog().as_ref(ctx).has_target()
    }

    pub fn has_shareable_shared_session<C: warpui::ViewAsRef>(&self, ctx: &C) -> bool {
        self.sharing_dialog()
            .as_ref(ctx)
            .has_shared_session_target()
    }

    pub fn is_sharing_dialog_enabled<C: warpui::ViewAsRef>(&self, ctx: &C) -> bool {
        let sharing_enabled = self.has_shareable_object(ctx);
        if self.has_shareable_shared_session(ctx) {
            sharing_enabled && FeatureFlag::SessionSharingAcls.is_enabled()
        } else {
            sharing_enabled
        }
    }

    /// Share the panes' contents.
    ///
    /// If the user can share the pane contents, this will bring up a sharing dialog. Otherwise, it copies
    /// the backing object's URL.
    pub fn share_pane_contents(
        &mut self,
        source: SharingDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.is_sharing_dialog_enabled(ctx) {
            return;
        }

        if !self
            .sharing_dialog()
            .as_ref(ctx)
            .editability(ctx)
            .can_edit()
        {
            self.sharing_dialog()
                .update(ctx, |dialog, ctx| dialog.copy_link(ctx));
            return;
        }

        let dialog_opened = match self.open_overlay {
            OpenOverlay::OverflowMenu => {
                self.open_overlay = OpenOverlay::SharingDialog;
                ctx.emit(Event::PaneHeaderOverflowMenuToggled(false));
                ctx.focus(&self.shared_content.sharing_dialog);
                true
            }
            OpenOverlay::SharingDialog => {
                self.close_overlay(ctx);
                false
            }
            OpenOverlay::None => {
                self.open_overlay = OpenOverlay::SharingDialog;
                ctx.focus(&self.shared_content.sharing_dialog);
                true
            }
        };

        if dialog_opened {
            self.sharing_dialog()
                .update(ctx, |dialog, ctx| dialog.report_open(source, ctx));
        }

        ctx.notify();
    }

    pub fn open_pane_sharing_dialog(
        &mut self,
        source: SharingDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.is_sharing_dialog_enabled(ctx) {
            return;
        }

        if !self
            .sharing_dialog()
            .as_ref(ctx)
            .editability(ctx)
            .can_edit()
        {
            self.sharing_dialog()
                .update(ctx, |dialog, ctx| dialog.copy_link(ctx));
            return;
        }

        let dialog_opened = match self.open_overlay {
            OpenOverlay::OverflowMenu => {
                self.open_overlay = OpenOverlay::SharingDialog;
                ctx.emit(Event::PaneHeaderOverflowMenuToggled(false));
                ctx.focus(&self.shared_content.sharing_dialog);
                true
            }
            OpenOverlay::SharingDialog => {
                ctx.focus(&self.shared_content.sharing_dialog);
                false
            }
            OpenOverlay::None => {
                self.open_overlay = OpenOverlay::SharingDialog;
                ctx.focus(&self.shared_content.sharing_dialog);
                true
            }
        };

        if dialog_opened {
            self.sharing_dialog()
                .update(ctx, |dialog, ctx| dialog.report_open(source, ctx));
        }

        ctx.notify();
    }
    fn handle_sharing_dialog_event(
        &mut self,
        event: &SharingDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SharingDialogEvent::Close => {
                self.close_overlay(ctx);
            }
        }
    }

    /// Render controls for sharing the pane contents. The controls shown depend on the current
    /// user's access level on the contents.
    pub fn render_sharing_controls(
        &self,
        element: &mut impl ParentElement,
        appearance: &Appearance,
        icon_color_override: Option<Fill>,
        button_size_override: Option<f32>,
        app: &AppContext,
    ) {
        if !self.is_sharing_dialog_enabled(app) {
            return;
        }

        let is_unsharable_conversation = self
            .sharing_dialog()
            .as_ref(app)
            .is_unsharable_conversation(app);
        let editability = self.sharing_dialog().as_ref(app).editability(app);

        let (primary_button_icon, primary_button_active, primary_tooltip_text) =
            if is_unsharable_conversation {
                (
                    Icon::Share,
                    false,
                    UNSHARABLE_CONVERSATION_TOOLTIP.to_string(),
                )
            } else if editability.can_edit() {
                (
                    Icon::Share,
                    self.open_overlay == OpenOverlay::SharingDialog,
                    "Share".to_string(),
                )
            } else {
                (Icon::Link, false, "Copy link".to_string())
            };

        let ui_builder = appearance.ui_builder().clone();
        let theme = appearance.theme();

        // When disabled, use the disabled text color for the icon
        let icon_color = if is_unsharable_conversation {
            Fill::Solid(theme.disabled_text_color(theme.background()).into())
        } else {
            icon_color_override
                .unwrap_or_else(|| Fill::Solid(theme.main_text_color(theme.background()).into()))
        };

        let button_builder = icon_button_with_color(
            appearance,
            primary_button_icon,
            primary_button_active,
            self.shared_content.primary_button_handle.clone(),
            icon_color,
        )
        .with_tooltip(move || {
            ConstrainedBox::new(ui_builder.tool_tip(primary_tooltip_text).build().finish())
                .with_max_width(400.)
                .finish()
        });

        let mut primary_button = button_builder.build();
        if !is_unsharable_conversation {
            primary_button = primary_button.on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<P::PaneHeaderOverflowMenuAction, P::CustomAction>::ShareContents,
                )
            });
        }
        let primary_button = primary_button
            .with_cursor(if is_unsharable_conversation {
                Cursor::Arrow
            } else {
                Cursor::PointingHand
            })
            .finish();

        let primary_button = if let Some(size) = button_size_override {
            ConstrainedBox::new(primary_button)
                .with_width(size)
                .with_height(size)
                .finish()
        } else {
            primary_button
        };
        element.add_child(primary_button);

        if !editability.can_edit() {
            let mut tooltip_text = String::from("Read-only");
            if matches!(editability, ContentEditability::RequiresLogin) {
                tooltip_text.push_str(". Sign in to edit");
            }

            let ui_builder = appearance.ui_builder().clone();
            let view_only_button = if let Some(icon_color) = icon_color_override {
                icon_button_with_color(
                    appearance,
                    Icon::Eye,
                    false,
                    self.shared_content.view_only_icon_handle.clone(),
                    icon_color,
                )
            } else {
                icon_button(
                    appearance,
                    Icon::Eye,
                    false,
                    self.shared_content.view_only_icon_handle.clone(),
                )
            }
            .with_tooltip(move || ui_builder.tool_tip(tooltip_text).build().finish())
            .build()
            .with_cursor(Cursor::PointingHand)
            .finish();

            let view_only_button = if let Some(size) = button_size_override {
                ConstrainedBox::new(view_only_button)
                    .with_width(size)
                    .with_height(size)
                    .finish()
            } else {
                view_only_button
            };
            element.add_child(view_only_button);
        }
    }
}
