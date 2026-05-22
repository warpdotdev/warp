use warpui::elements::{CrossAxisAlignment, Flex, ParentElement};
use warpui::prelude::ChildView;
use warpui::{AppContext, Element};

use crate::ai::blocklist::block::ImportedCommentGroup;

pub(crate) fn render_imported_comments(
    group: &ImportedCommentGroup,
    app: &AppContext,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.);

    for (card, state) in group.cards.iter().zip(group.element_states.iter()) {
        let open_in_code_review = ChildView::new(&state.open_in_code_review_button).finish();
        let chevron = ChildView::new(&state.chevron_button).finish();

        let mut header_trailing = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);

        if let Some(open_in_github) = &state.open_in_github_button {
            header_trailing.add_child(ChildView::new(open_in_github).finish());
        }

        let header_trailing = header_trailing
            .with_child(open_in_code_review)
            .with_child(chevron)
            .finish();

        column.add_child(card.render(
            None,
            Some(header_trailing),
            None,
            Some(&state.header_click_handler),
            app,
        ));
    }

    column.finish()
}
