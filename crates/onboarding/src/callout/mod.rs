mod model;
mod view;

pub use model::{FinalState, OnboardingQuery};
pub use view::{OnboardingCalloutView, OnboardingCalloutViewEvent, OnboardingKeybindings};

pub fn init(app: &mut black_ui::AppContext) {
    view::init(app);
}
