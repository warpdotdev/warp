use crate::appearance::Appearance;
use crate::i18n::{self, I18nKey};
use crate::ui_components::icons::Icon;
use pathfinder_geometry::vector::vec2f;
use warpui::{
    elements::{Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle},
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext,
};

use super::{
    settings_page::{MatchData, NAV_ITEM_LEFT_MARGIN},
    SettingsSection,
};

/// The font size for subpage items inside an umbrella.
const SUBPAGE_FONT_SIZE: f32 = 10.;

/// Left margin for subpage items inside an umbrella (top-level margin + indent).
const SUBPAGE_LEFT_MARGIN: f32 = NAV_ITEM_LEFT_MARGIN + 12.;

/// A collapsible group of settings subpages in the sidebar.
pub struct SettingsUmbrella {
    pub label_key: I18nKey,
    pub subpages: Vec<SettingsSection>,
    pub expanded: bool,
    /// Saved expanded state from before search began, restored when search is cleared.
    pub pre_search_expanded: Option<bool>,
    pub button_state_handle: MouseStateHandle,
    pub subpage_button_states: Vec<MouseStateHandle>,
}

impl SettingsUmbrella {
    pub fn new(label_key: I18nKey, subpages: Vec<SettingsSection>) -> Self {
        let subpage_count = subpages.len();
        Self {
            label_key,
            subpages,
            expanded: false,
            pre_search_expanded: None,
            button_state_handle: MouseStateHandle::default(),
            subpage_button_states: (0..subpage_count)
                .map(|_| MouseStateHandle::default())
                .collect(),
        }
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Returns true if `section` is one of this umbrella's subpages.
    pub fn contains(&self, section: SettingsSection) -> bool {
        self.subpages.contains(&section)
    }

    /// Render the umbrella header row (label + chevron).
    /// Returns a `Hoverable` so the entire row shares a single hover/click
    /// target — i.e. the hover styling and pointing-hand cursor apply to the
    /// whole clickable area rather than just the text.
    pub fn render_umbrella_row(&self, app: &AppContext, appearance: &Appearance) -> Hoverable {
        let chevron_icon = if self.expanded {
            Icon::ChevronUp
        } else {
            Icon::ChevronDown
        };

        // Initial chevron color is overridden by the button's font_color when
        // rendered, so this just seeds a sensible default.
        let text_color = appearance.theme().nonactive_ui_text_color();

        // Use a single full-width text button with a text+icon label so the
        // text label aligns with other top-level settings items and the
        // chevron sits flush-right — while the whole button area receives the
        // hover styling and pointing-hand cursor.
        appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.button_state_handle.clone())
            .with_text_and_icon_label(TextAndIcon::new(
                TextAndIconAlignment::TextFirst,
                i18n::tr(app, self.label_key).to_string(),
                chevron_icon.to_warpui_icon(text_color),
                MainAxisSize::Max,
                MainAxisAlignment::SpaceBetween,
                vec2f(16., 16.),
            ))
            .with_style(
                UiComponentStyles::default()
                    .set_border_width(0.)
                    .set_margin(Coords::default().left(NAV_ITEM_LEFT_MARGIN))
                    .set_padding(Coords::uniform(8.)),
            )
            .build()
    }

    /// Render a single subpage button within this umbrella.
    pub fn render_subpage_button(
        &self,
        index: usize,
        app: &AppContext,
        appearance: &Appearance,
        match_data: MatchData,
        is_active: bool,
    ) -> Option<Hoverable> {
        let section = self.subpages.get(index)?;
        let mouse_state = self.subpage_button_states.get(index)?.clone();

        let label = section.localized_label(app).to_owned() + &match_data.to_string();

        let hoverable = appearance
            .ui_builder()
            .button(
                if is_active {
                    ButtonVariant::Accent
                } else {
                    ButtonVariant::Text
                },
                mouse_state,
            )
            .with_text_label(label)
            .with_style(
                UiComponentStyles::default()
                    .set_border_width(0.)
                    .set_margin(Coords::default().left(SUBPAGE_LEFT_MARGIN))
                    .set_padding(Coords::uniform(8.))
                    .set_font_size(SUBPAGE_FONT_SIZE),
            )
            .build();

        Some(hoverable)
    }
}

/// A sidebar navigation item: either a direct page link or a collapsible umbrella.
pub enum SettingsNavItem {
    /// A top-level page that is rendered directly in the sidebar.
    Page(SettingsSection),
    /// A collapsible group header whose children are subpage sections.
    Umbrella(SettingsUmbrella),
}
