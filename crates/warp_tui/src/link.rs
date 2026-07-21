use warpui_core::AppContext;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{Modifier, TuiElement, TuiEventContext, TuiHoverable, TuiText};

use crate::tui_builder::TuiUiBuilder;

/// Reusable link presentation with persistent hover state.
#[derive(Clone, Default)]
pub(crate) struct TuiLink {
    hover_state: MouseStateHandle,
}

impl TuiLink {
    /// Renders caller-provided link text and invokes `on_open` on click.
    pub(crate) fn render(
        &self,
        label: impl Into<String>,
        app: &AppContext,
        on_open: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let style = builder.muted_text_style();
        let is_hovered = self
            .hover_state
            .lock()
            .is_ok_and(|state| state.is_hovered());
        let style = if is_hovered {
            style
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            style.add_modifier(Modifier::UNDERLINED)
        };
        TuiHoverable::new(
            self.hover_state.clone(),
            TuiText::new(label.into()).with_style(style).finish(),
        )
        .on_click(on_open)
        .finish()
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
