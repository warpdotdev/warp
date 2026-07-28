//! Shared presentation and lifecycle types for stateless read-only menus.

use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};

use crate::tui_builder::TuiUiBuilder;

/// The read-only menu currently projected above the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiReadOnlyMenuKind {
    Shortcuts,
    Status,
}

/// One titled section in a [`TuiReadOnlyMenu`].
pub(crate) struct TuiReadOnlyMenuSection {
    title: &'static str,
    rows: Vec<Box<dyn TuiElement>>,
}

impl TuiReadOnlyMenuSection {
    pub(crate) fn new(title: &'static str, rows: Vec<Box<dyn TuiElement>>) -> Self {
        Self { title, rows }
    }
}

/// Shared stateless component used by the shortcuts and status menus.
pub(crate) struct TuiReadOnlyMenu {
    sections: Vec<TuiReadOnlyMenuSection>,
}

impl TuiReadOnlyMenu {
    pub(crate) fn new(sections: Vec<TuiReadOnlyMenuSection>) -> Self {
        Self { sections }
    }

    pub(crate) fn render(self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let mut panel = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (index, section) in self.sections.into_iter().enumerate() {
            if index > 0 {
                panel.add_child(TuiText::new(" ").finish());
            }
            panel.add_child(
                TuiText::new(section.title)
                    .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                    .truncate()
                    .finish(),
            );
            panel.add_children(section.rows);
        }

        TuiContainer::new(panel.finish())
            .with_padding_x(1)
            .with_background(builder.shortcuts_background())
            .finish()
    }
}
