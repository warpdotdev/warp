use warpui::integration::TestStep;

use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::tab::assert_tab_title;

/// Checks whether the current tab has an expected title.
/// #Panics if any of the assertions fail (including if the tab title doesn't match
/// `expected_tab_title`)
pub fn tab_title_step(assertion_name: &str, expected_tab_title: String) -> TestStep {
    new_step_with_default_assertions(assertion_name)
        .add_assertion(assert_tab_title(0, expected_tab_title))
}
