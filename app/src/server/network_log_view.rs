//! A read-only pane that shows a one-shot snapshot of the in-memory
//! [`NetworkLogModel`].
//!
//! The pane is opened via [`Workspace::open_network_log_pane`]. It seeds a
//! `CodeEditorView` with the snapshot text at open time and does not live-
//! update as new requests arrive. Re-triggering the open action while the
//! pane is already open reloads the snapshot via [`Self::reload_snapshot`]
//! so the user can pick up items captured since the pane was opened. The
//! pane header also exposes a refresh icon that reloads the snapshot in
//! place.
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_server_client::network_logging::NetworkLogModel;
use warp_util::path::LineAndColumnArg;
use warpui::elements::{ChildView, MouseStateHandle};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::code::editor::scroll::{ScrollPosition, ScrollTrigger};
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::editor::InteractionState;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent, PaneHeaderAction};
use crate::ui_components::buttons::icon_button_with_color;
use crate::ui_components::{blended_colors, icons};

/// Header text for the network log pane.
pub const NETWORK_LOG_HEADER_TEXT: &str = "Network log";

/// Tooltip shown on hover over the refresh button in the pane header.
const REFRESH_TOOLTIP: &str = "Refresh";

/// Event emitted by the [`NetworkLogView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkLogViewEvent {
    Pane(PaneEvent),
}

/// Actions supported by the pane header's overflow menu (currently none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkLogViewAction {}

/// Custom actions dispatched by elements that the [`NetworkLogView`] renders
/// inside its pane header (e.g. the refresh button).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkLogViewCustomAction {
    Refresh,
}

/// A pane view backed by a read-only [`CodeEditorView`] displaying a snapshot
/// of the current in-memory network log.
pub struct NetworkLogView {
    editor: ViewHandle<CodeEditorView>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    refresh_button_mouse_state: MouseStateHandle,
}

impl NetworkLogView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration =
            ctx.add_model(|_ctx| PaneConfiguration::new(NETWORK_LOG_HEADER_TEXT));

        // Capture a one-shot snapshot of the model. We intentionally do not
        // subscribe to the model: new items that arrive after the pane is
        // opened are not reflected until the pane is explicitly reopened
        // (see `reload_snapshot`).
        let snapshot = NetworkLogModel::as_ref(ctx).snapshot_text();

        let editor = ctx.add_typed_action_view(|ctx| {
            let mut view = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight),
                ctx,
            );
            Self::apply_snapshot_to_editor(&mut view, &snapshot, ctx);
            // Read-only pane: disallow editing but keep selection/copy/find
            // available.
            view.set_interaction_state(InteractionState::Selectable, ctx);
            view
        });

        Self {
            editor,
            pane_configuration,
            focus_handle: None,
            refresh_button_mouse_state: MouseStateHandle::default(),
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
    }

    /// Re-seed the editor with a fresh snapshot from [`NetworkLogModel`] and
    /// scroll back to the top. Called when the user re-triggers the
    /// open-network-log-pane action while the pane is already open, or when
    /// the user clicks the refresh icon in the pane header, so they can see
    /// items captured since the pane was opened.
    pub fn reload_snapshot(&self, ctx: &mut ViewContext<Self>) {
        let snapshot = NetworkLogModel::as_ref(ctx).snapshot_text();
        self.editor.update(ctx, |view, ctx| {
            Self::apply_snapshot_to_editor(view, &snapshot, ctx);
        });
    }

    /// Resets the editor buffer with the given snapshot text and queues a
    /// pending scroll-to-top once layout completes. `reset` places the
    /// cursor at the end of the buffer by default, which would scroll the
    /// viewport to the bottom when the pane renders.
    fn apply_snapshot_to_editor(
        view: &mut CodeEditorView,
        snapshot: &str,
        ctx: &mut ViewContext<CodeEditorView>,
    ) {
        let state = InitialBufferState::plain_text(snapshot);
        view.reset(state, ctx);
        let version = view.buffer_version(ctx);
        view.set_pending_scroll(ScrollTrigger::new(
            ScrollPosition::LineAndColumn(LineAndColumnArg {
                line_num: 1,
                column_num: Some(0),
            }),
            version,
        ));
    }

    /// Renders the refresh icon button for the pane header. Clicking the
    /// button dispatches [`NetworkLogViewCustomAction::Refresh`], which the
    /// pane header forwards back to [`Self::handle_custom_action`].
    fn render_refresh_button(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder().clone();

        icon_button_with_color(
            appearance,
            icons::Icon::Refresh,
            false, /* active */
            self.refresh_button_mouse_state.clone(),
            blended_colors::text_sub(theme, theme.background()).into(),
        )
        .with_tooltip(move || {
            ui_builder
                .tool_tip(REFRESH_TOOLTIP.to_string())
                .build()
                .finish()
        })
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action::<PaneHeaderAction<
                NetworkLogViewAction,
                NetworkLogViewCustomAction,
            >>(PaneHeaderAction::CustomAction(
                NetworkLogViewCustomAction::Refresh,
            ));
        })
        .finish()
    }
}

impl Entity for NetworkLogView {
    type Event = NetworkLogViewEvent;
}

impl View for NetworkLogView {
    fn ui_name() -> &'static str {
        "NetworkLogView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.editor).finish()
    }
}

impl TypedActionView for NetworkLogView {
    type Action = NetworkLogViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        // NetworkLogViewAction is currently uninhabited.
    }
}

impl BackingView for NetworkLogView {
    type PaneHeaderOverflowMenuAction = NetworkLogViewAction;
    type CustomAction = NetworkLogViewCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items are registered.
    }

    fn handle_custom_action(
        &mut self,
        custom_action: &Self::CustomAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match custom_action {
            NetworkLogViewCustomAction::Refresh => self.reload_snapshot(ctx),
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(NetworkLogViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: NETWORK_LOG_HEADER_TEXT.to_string(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: Some(self.render_refresh_button(app)),
            // Keep the close button always visible so hovering the header
            // doesn't cause the refresh button to shift horizontally as the
            // close button appears.
            options: StandardHeaderOptions {
                always_show_icons: true,
                ..StandardHeaderOptions::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
