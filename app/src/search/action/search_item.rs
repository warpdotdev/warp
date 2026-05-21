use std::sync::Arc;

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use pathfinder_color::ColorU;
use warpui::elements::{
    Align, ConstrainedBox, Container, Flex, Highlight, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::{DescriptionContext, Keystroke};
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::drive::DriveObjectType;
use crate::features::FeatureFlag;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::{
    colors, render_search_item_icon, render_search_item_icon_placeholder,
};
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon;
use crate::util::bindings::{BindingGroup, CommandBinding};

/// A matched binding from a search query.
#[derive(Debug)]
pub struct MatchedBinding {
    fuzzy_match_result: FuzzyMatchResult,
    binding: Arc<CommandBinding>,
}

impl MatchedBinding {
    pub fn new(fuzzy_match_result: FuzzyMatchResult, binding: Arc<CommandBinding>) -> Self {
        Self {
            fuzzy_match_result,
            binding,
        }
    }

    /// Creates a new placeholder [`MatchedBinding`] using `name` as the [`CommandBinding`] name.
    pub fn placeholder(name: String) -> Self {
        Self::new(
            FuzzyMatchResult::no_match(),
            Arc::new(CommandBinding::placeholder(name)),
        )
    }

    pub fn render(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label = self.render_label(highlight_state, appearance);
        let mut binding = Flex::row();

        binding.add_child(Shrinkable::new(1., Align::new(label).left().finish()).finish());

        if let Some(trigger) = self.binding.trigger.clone() {
            let shortcut = appearance.ui_builder().keyboard_shortcut(&trigger).build();
            binding.add_child(
                Container::new(shortcut.finish())
                    .with_margin_right(styles::KEYBINDING_MARGIN_RIGHT)
                    .finish(),
            );
        }
        ConstrainedBox::new(binding.finish())
            .with_height(styles::SEARCH_ITEM_HEIGHT)
            .finish()
    }

    fn render_label(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Text::new_inline(
            self.binding
                .description
                .in_context(DescriptionContext::Default)
                .to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(item_highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .with_single_highlight(
            Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(
                    item_highlight_state.main_text_fill(appearance).into_solid(),
                ),
            self.fuzzy_match_result.matched_indices.clone(),
        )
        .finish()
    }
}

impl SearchItem for MatchedBinding {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self.binding.group {
            None => render_search_item_icon_placeholder(appearance),
            Some(group) => render_search_item_icon(
                appearance,
                group.icon(),
                group.icon_color(appearance),
                highlight_state,
            ),
        }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render(highlight_state, appearance)
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        // Bindings do not support details panels.
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.fuzzy_match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::AcceptBinding {
            binding: self.binding.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        let trigger = self.binding.trigger.as_ref();

        format!(
            "Selected {}, {}.",
            &self
                .binding
                .description
                .in_context(DescriptionContext::Default),
            trigger.map(Keystroke::normalized).unwrap_or_default()
        )
    }

    fn accessibility_help_message(&self) -> Option<String> {
        self.binding
            .trigger
            .as_ref()
            .map_or("Press enter to confirm.".into(), |trigger| {
                format!(
                    "Press enter to confirm. Use {} binding to run this action in the future.",
                    trigger.normalized()
                )
            })
            .into()
    }
}

/// Trait to compute an icon for a search item.
trait SearchItemIcon {
    fn icon(&self) -> Icon;

    fn icon_color(&self, appearance: &Appearance) -> ColorU;
}

impl SearchItemIcon for BindingGroup {
    fn icon(&self) -> Icon {
        match self {
            Self::Settings => Icon::Gear,
            Self::WarpAi => {
                if !FeatureFlag::AgentMode.is_enabled() {
                    Icon::AiAssistant
                } else {
                    Icon::Oz
                }
            }
            Self::Close => Icon::X,
            Self::Navigation => Icon::Navigation,
            Self::Workflow => Icon::Workflow,
            Self::Notebooks => Icon::Notebook,
            Self::Folders => Icon::Folder,
            Self::KeyboardShortcuts => Icon::Keyboard,
            Self::AutoUpdate => Icon::AutoUpdate,
            Self::Notifications => Icon::Bell,
            Self::EnvVarCollection => Icon::EnvVarCollection,
            Self::Terminal => Icon::Terminal,
        }
    }

    fn icon_color(&self, appearance: &Appearance) -> ColorU {
        match self {
            Self::Settings
            | Self::Navigation
            | Self::Close
            | Self::KeyboardShortcuts
            | Self::AutoUpdate
            | Self::Folders
            | Self::Terminal
            | Self::Notifications => appearance.theme().foreground().into_solid(),
            Self::WarpAi if !FeatureFlag::AgentMode.is_enabled() => {
                ColorU::from_u32(colors::WARP_AI)
            }
            Self::WarpAi => appearance.theme().foreground().into_solid(),
            Self::Workflow => warp_drive_icon_color(appearance, DriveObjectType::Workflow),
            Self::Notebooks => warp_drive_icon_color(
                appearance,
                DriveObjectType::Notebook {
                    is_ai_document: false,
                },
            ),
            Self::EnvVarCollection => {
                warp_drive_icon_color(appearance, DriveObjectType::EnvVarCollection)
            }
        }
    }
}

pub(crate) mod styles {
    /// Total height of the search item.
    pub const SEARCH_ITEM_HEIGHT: f32 = 40.;

    /// Margin between the right-side of the element and the end of the keybinding.
    pub const KEYBINDING_MARGIN_RIGHT: f32 = 14.;
}
