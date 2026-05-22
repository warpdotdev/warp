use itertools::Itertools as _;
use warpui::elements::ChildView;
use warpui::{
    Element as _, Entity, SingletonEntity as _, TypedActionView, View, ViewAsRef, ViewContext,
    ViewHandle,
};

use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{CloudObject as _, GenericStringObjectFormat, JsonObjectType};
use crate::drive::CloudObjectTypeAndId;
use crate::server::ids::SyncId;
use crate::view_components::{DropdownItem, FilterableDropdown, FilterableDropdownOrientation};

/// A reusable [`View`] for choosing environment variable collections.
pub struct EnvVarSelector {
    dropdown: ViewHandle<FilterableDropdown<EnvVarSelectorAction>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnvVarSelectorAction {
    Select(Option<SyncId>),
}

pub enum EnvVarSelectorEvent {
    SelectionChanged(Option<SyncId>),
    Refreshed,
}

/// The default width for the env var selector dropdown.
const DEFAULT_DROPDOWN_WIDTH: f32 = super::argument_editor::ALIAS_ARGUMENT_EDITOR_WIDTH;

impl EnvVarSelector {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(DEFAULT_DROPDOWN_WIDTH);
            dropdown.set_menu_width(DEFAULT_DROPDOWN_WIDTH, ctx);
            dropdown
        });

        let mut selector = Self { dropdown };
        selector.refresh_dropdown_items(ctx);
        selector
    }

    pub fn set_orientation(
        &mut self,
        orientation: FilterableDropdownOrientation,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dropdown
            .update(ctx, |dropdown, _ctx| dropdown.set_orientation(orientation));
    }

    pub fn set_width(&mut self, width: f32, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_top_bar_max_width(width);
            dropdown.set_menu_width(width, ctx);
        });
    }

    pub fn set_selected_env_vars(&mut self, id: Option<SyncId>, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(EnvVarSelectorAction::Select(id), ctx)
        });
    }

    pub fn has_env_vars<C>(&self, ctx: &C) -> bool
    where
        C: ViewAsRef,
    {
        // We add a `None` item, so there are env vars iff there is more than one item.
        self.dropdown.as_ref(ctx).len() > 1
    }

    fn refresh_dropdown_items(&mut self, ctx: &mut ViewContext<Self>) {
        let mut env_vars = CloudModel::as_ref(ctx)
            .get_all_active_env_var_collections()
            .map(|collection| (collection.display_name(), collection.sync_id()))
            .collect_vec();
        env_vars.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let remove_item = std::iter::once(DropdownItem::new(
            "None",
            EnvVarSelectorAction::Select(None),
        ));

        let env_var_items = env_vars
            .into_iter()
            .map(|(name, id)| DropdownItem::new(name, EnvVarSelectorAction::Select(Some(id))));
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(remove_item.chain(env_var_items).collect(), ctx)
        });
        ctx.emit(EnvVarSelectorEvent::Refreshed);
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectUpdated { type_and_id, .. }
            | CloudModelEvent::ObjectCreated { type_and_id }
            | CloudModelEvent::ObjectUntrashed { type_and_id, .. }
            | CloudModelEvent::ObjectTrashed { type_and_id, .. } => {
                if matches!(
                    type_and_id,
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(
                            JsonObjectType::EnvVarCollection
                        ),
                        ..
                    }
                ) {
                    self.refresh_dropdown_items(ctx);
                }
            }
            _ => (),
        }
    }
}

impl Entity for EnvVarSelector {
    type Event = EnvVarSelectorEvent;
}

impl View for EnvVarSelector {
    fn ui_name() -> &'static str {
        "EnvVarSelector"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        ChildView::new(&self.dropdown).finish()
    }
}

impl TypedActionView for EnvVarSelector {
    type Action = EnvVarSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let EnvVarSelectorAction::Select(id) = action;
        ctx.emit(EnvVarSelectorEvent::SelectionChanged(*id));
    }
}
