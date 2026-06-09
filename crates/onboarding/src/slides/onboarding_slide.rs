use warpui_core::{View, ViewContext};

pub trait OnboardingSlide: View {
    fn on_up(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_down(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_left(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_right(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_tab(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_enter(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_cmd_or_ctrl_enter(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn on_escape(&mut self, _ctx: &mut ViewContext<Self>) {}
}
