use crate::menu::{Menu, MenuItem, MenuItemFields};
use crate::pane_group::PaneHeaderAction;
use crate::pane_group::PaneHeaderCustomAction;

use crate::terminal::view::{TerminalAction, TerminalView};
use crate::ui_components::icons::Icon;

use session_sharing_protocol::common::{Role, WindowSize};
use warpui::{elements::MouseStateHandle, AppContext, ViewContext, ViewHandle};

use super::adapter::Participant;

pub struct Viewer {
    pub sharer: Option<Participant>,
    pub is_reconnecting: bool,
    pub is_role_change_menu_open: bool,

    /// The menu in the pane header for requesting a different roles.
    pub role_change_menu: ViewHandle<Menu<PaneHeaderAction<TerminalAction, TerminalAction>>>,

    pub role_change_menu_button: MouseStateHandle,
    /// The handle for the "Request edit access" button for viewers.
    pub input_request_edit_access_button_handle: MouseStateHandle,
    /// The viewer has a pending role request.
    pub pending_role_request: bool,

    pub sharer_size: Option<WindowSize>,

    /// The last natural size (rows, cols) that was reported to the sharer.
    /// Used to deduplicate ReportViewerTerminalSize events.
    pub last_reported_natural_size: Option<(usize, usize)>,
}

impl Viewer {
    pub fn new(ctx: &mut ViewContext<TerminalView>) -> Self {
        let role_change_menu = ctx.add_typed_action_view(|_| Menu::new().with_width(220.));
        ctx.subscribe_to_view(&role_change_menu, |me, _, event, ctx| {
            me.handle_viewer_role_change_menu_event(event, ctx);
        });

        Self {
            is_reconnecting: false,
            is_role_change_menu_open: false,
            role_change_menu,
            sharer: None,
            role_change_menu_button: Default::default(),
            input_request_edit_access_button_handle: Default::default(),
            pending_role_request: false,
            sharer_size: None,
            last_reported_natural_size: None,
        }
    }

    pub fn set_is_reconnecting(&mut self, is_reconnectiong: bool) {
        self.is_reconnecting = is_reconnectiong;
    }

    pub fn role_change_menu_items(
        current_role: Role,
        is_reconnecting: bool,
        app: &AppContext,
    ) -> Vec<MenuItem<PaneHeaderAction<TerminalAction, TerminalAction>>> {
        let mut items = Vec::new();
        match current_role {
            Role::Reader => items.extend([
                // TODO: this should still dispatch an action that eventually no-ops
                MenuItemFields::new(crate::i18n::tr_static(app, "View"))
                    .with_icon(Icon::Check)
                    .with_disabled(is_reconnecting)
                    .into_item(),
                MenuItemFields::new(crate::i18n::tr_static(app, "Edit"))
                    .with_indent()
                    .with_disabled(is_reconnecting)
                    .with_on_select_action(
                        PaneHeaderCustomAction::<TerminalAction, TerminalAction>(
                            TerminalAction::RequestSharedSessionRole(Role::Executor),
                        ),
                    )
                    .into_item(),
            ]),
            Role::Executor | Role::Full => items.extend([
                MenuItemFields::new(crate::i18n::tr_static(app, "View"))
                    .with_indent()
                    .with_disabled(true)
                    .into_item(),
                // TODO: this should still dispatch an action that eventually no-ops
                MenuItemFields::new(crate::i18n::tr_static(app, "Edit"))
                    .with_icon(Icon::Check)
                    .with_disabled(is_reconnecting)
                    .into_item(),
            ]),
        }
        items
    }

    pub fn open_role_change_menu(
        &mut self,
        current_role: Role,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        self.is_role_change_menu_open = true;
        let items = Self::role_change_menu_items(current_role, self.is_reconnecting, ctx);
        self.role_change_menu.update(ctx, |menu, ctx| {
            menu.set_items(items, ctx);
        });
    }

    pub fn close_role_change_menu(&mut self) {
        self.is_role_change_menu_open = false;
    }
}
