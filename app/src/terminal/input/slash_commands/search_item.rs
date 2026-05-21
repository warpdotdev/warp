use ordered_float::OrderedFloat;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{Expanded, Highlight, Icon, ParentElement, Shrinkable};
use warpui::fonts::{Properties, Weight};
use warpui::prelude::{ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, Text};
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::blocklist::agent_view::shortcuts::render_keystroke_with_color_overrides;
use crate::search::item::SearchItemDetail;
use crate::search::slash_command_menu::static_commands::commands::COMMAND_REGISTRY;
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::util::bindings::keybinding_name_to_keystroke;

use super::{AcceptSlashCommandOrSavedPrompt, InlineItem};

fn inline_width_for_name_column(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);

    let longest_slash_command_len = COMMAND_REGISTRY
        .all_commands()
        .max_by_key(|command| command.name.len())
        .expect("static commands is non-empty")
        .name
        .len();

    app.font_cache().em_width(
        appearance.monospace_font_family(),
        inline_styles::font_size(appearance),
    ) * (longest_slash_command_len as f32 + 3.)
        + 72.
}

impl SearchItem for InlineItem {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color = inline_styles::icon_color(appearance);
        let icon_size = inline_styles::font_size(appearance);

        Container::new(
            ConstrainedBox::new(Icon::new(self.icon_path, icon_color.into_solid()).finish())
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(inline_styles::ICON_MARGIN)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);
        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_color = inline_styles::secondary_text_color(theme, background_color.into());

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut name_text = Text::new_inline(self.name.clone(), self.font_family, font_size)
            .with_color(primary_text_color.into());

        if let Some(name_match) = &self.name_match_result {
            if !name_match.matched_indices.is_empty() {
                name_text = name_text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    name_match.matched_indices.clone(),
                );
            }
        }

        let keystroke = if highlight_state.is_selected()
            && matches!(
                &self.action,
                AcceptSlashCommandOrSavedPrompt::SlashCommand { .. }
            ) {
            keybinding_name_to_keystroke(&self.name, app)
        } else {
            None
        };

        let name_element = if let Some(keystroke) = keystroke {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(name_text.finish())
                .with_child(
                    Text::new(
                        " or ",
                        appearance.ui_font_family(),
                        inline_styles::font_size(appearance),
                    )
                    .with_color(secondary_color.into_solid())
                    .finish(),
                )
                .with_child(
                    Container::new(render_keystroke_with_color_overrides(
                        &keystroke,
                        None,
                        Some(theme.surface_overlay_3().into_solid()),
                        app,
                    ))
                    .with_margin_left(4.)
                    .finish(),
                );
            if !self.compact_layout {
                row = row.with_child(Shrinkable::new(1., Empty::new().finish()).finish());
            }
            row.finish()
        } else {
            name_text.finish()
        };

        row.add_child(if self.description.is_some() {
            if self.compact_layout {
                Container::new(name_element).with_margin_right(8.).finish()
            } else {
                ConstrainedBox::new(name_element)
                    .with_width(inline_width_for_name_column(app))
                    .finish()
            }
        } else {
            name_element
        });

        if let Some(description) = self.description.clone() {
            let mut description_text =
                Text::new_inline(description, appearance.ui_font_family(), font_size)
                    .with_color(secondary_color.into());

            // Add bold highlighting for matching characters in the description
            if let Some(description_match) = &self.description_match_result {
                if !description_match.matched_indices.is_empty() {
                    description_text = description_text.with_single_highlight(
                        Highlight::new()
                            .with_properties(Properties::default().weight(Weight::Bold)),
                        description_match.matched_indices.clone(),
                    );
                }
            }

            row.add_child(Expanded::new(1., description_text.finish()).finish());
        }

        row.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        self.action.clone()
    }

    fn execute_result(&self) -> Self::Action {
        self.action.clone()
    }

    fn accessibility_label(&self) -> String {
        format!("{:?}", self.action)
    }

    fn detail_data(&self) -> Option<SearchItemDetail> {
        Some(SearchItemDetail {
            title: self.name.clone(),
            description: self.description.clone(),
            title_font_family: self.font_family,
        })
    }
}
