use std::fmt::Display;

use warpui::elements::MouseStateHandle;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::Element;

use super::teams_page::TeamsPageAction;
use crate::cloud_object::model::persistence::CloudModel;
use crate::workspaces::team::Team;
use crate::Appearance;

/// The Tabs trait provides common functionality for an enum to be used as a tabs menu UI component.
/// It requires the trait-user to implement action_on_click() and label().
pub trait Tabs: PartialEq + Display + Copy {
    #[allow(dead_code)]
    fn button_variant(&self, selected_view_option: &Self) -> ButtonVariant {
        if self == selected_view_option {
            ButtonVariant::Basic
        } else {
            ButtonVariant::Outlined
        }
    }

    fn tab_name(&self) -> String {
        self.to_string()
    }

    #[allow(dead_code)]
    fn render_tab(
        &self,
        team: &Team,
        cloud_model: &CloudModel,
        selected_view_option: &Self,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let action = self.action_on_click(*self);

        appearance
            .ui_builder()
            .button(
                self.button_variant(selected_view_option),
                mouse_state_handle,
            )
            .with_text_label(self.label(team, cloud_model))
            .with_style(UiComponentStyles::default().set_border_width(0.))
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    // The trait-inheriter must define their own action and their own labels.
    #[allow(dead_code)]
    fn action_on_click(&self, selection: Self) -> TeamsPageAction;
    #[allow(dead_code)]
    fn label(&self, team: &Team, cloud_model: &CloudModel) -> String;
}
