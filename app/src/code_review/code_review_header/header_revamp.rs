//! Header layout used when the `GitOperationsInCodeReview` feature flag is
//! enabled. This replaces the legacy header (which lives in the parent module)
//! with a simplified layout: the diff-mode dropdown on the left, and file-nav /
//! overflow / maximize buttons on the right.
//!
//! Separated into its own module so the two codepaths are easy to distinguish.

use crate::code_review::code_review_view::{
    CodeReviewAction, CodeReviewHeaderFields, PrimaryGitActionMode,
};
use crate::code_review::diff_selector::DiffSelector;
use crate::menu::Menu;
use crate::view_components::action_button::ActionButton;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CrossAxisAlignment, Flex,
    MainAxisAlignment, MainAxisSize, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Shrinkable, Stack,
};
use warpui::{Element, ViewHandle};

use crate::appearance::Appearance;

use super::CodeReviewHeader;

impl CodeReviewHeader {
    /// Entry-point for the new header layout (feature-flagged behind
    /// `GitOperationsInCodeReview`). Renders a single row: diff-mode dropdown
    /// on the left, action buttons on the right.
    pub fn render_new(
        &self,
        appearance: &Appearance,
        code_review_header_fields: &CodeReviewHeaderFields,
    ) -> Box<dyn Element> {
        let mut right_section = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(git_button) = Self::render_git_operations_button(code_review_header_fields) {
            right_section.add_child(git_button);
        }

        if let Some(nav_button) = &code_review_header_fields.file_nav_button {
            right_section.add_child(Self::render_file_nav_button(nav_button));
        }

        if code_review_header_fields.has_header_menu_items {
            right_section.add_child(self.render_new_header_dropdown_button(
                &code_review_header_fields.header_dropdown_button,
                &code_review_header_fields.header_menu,
                code_review_header_fields.header_menu_open,
            ));
        }

        if code_review_header_fields.is_in_split_pane {
            right_section = right_section.with_child(self.render_maximize_pane_button(
                &code_review_header_fields.maximize_button,
                appearance,
            ));
        }

        let row = Clipped::new(
            Shrinkable::new(
                1.,
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Self::render_diff_mode_dropdown(
                        &code_review_header_fields.diff_selector,
                    ))
                    .with_child(right_section.finish())
                    .finish(),
            )
            .finish(),
        )
        .finish();

        Container::new(row).with_margin_bottom(8.).finish()
    }

    /// Renders the diff target selector in the left section of the header.
    fn render_diff_mode_dropdown(diff_selector: &ViewHandle<DiffSelector>) -> Box<dyn Element> {
        Container::new(ChildView::new(diff_selector).finish())
            .with_margin_right(8.)
            .finish()
    }

    fn render_file_nav_button(button: &ViewHandle<ActionButton>) -> Box<dyn Element> {
        ConstrainedBox::new(ChildView::new(button).finish())
            .with_height(warp_core::ui::icons::ICON_DIMENSIONS)
            .with_width(warp_core::ui::icons::ICON_DIMENSIONS)
            .finish()
    }

    fn render_git_operations_button(
        code_review_header_fields: &CodeReviewHeaderFields,
    ) -> Option<Box<dyn Element>> {
        if !FeatureFlag::GitOperationsInCodeReview.is_enabled() {
            return None;
        }

        let mut row = Flex::row().with_child(
            ChildView::new(&code_review_header_fields.git_primary_action_button).finish(),
        );

        if matches!(
            code_review_header_fields.primary_git_action_mode,
            PrimaryGitActionMode::Commit | PrimaryGitActionMode::Push
        ) {
            row.add_child(
                ChildView::new(&code_review_header_fields.git_operations_chevron).finish(),
            );
        }

        let mut stack = Stack::new().with_child(row.finish());
        if code_review_header_fields.git_operations_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&code_review_header_fields.git_operations_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        Some(
            Container::new(stack.finish())
                .with_margin_right(4.)
                .finish(),
        )
    }

    /// Like `render_header_dropdown_button` but without `margin_left(4.)`,
    /// matching the tighter spacing of the new header layout.
    fn render_new_header_dropdown_button(
        &self,
        header_dropdown_button: &ViewHandle<ActionButton>,
        header_menu: &ViewHandle<Menu<CodeReviewAction>>,
        header_menu_open: bool,
    ) -> Box<dyn Element> {
        let button_container = Container::new(
            ConstrainedBox::new(ChildView::new(header_dropdown_button).finish())
                .with_height(warp_core::ui::icons::ICON_DIMENSIONS)
                .with_width(warp_core::ui::icons::ICON_DIMENSIONS)
                .finish(),
        )
        .finish();

        let mut stack = Stack::new().with_child(button_container);

        if header_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(header_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        stack.finish()
    }
}
