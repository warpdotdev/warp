use super::TuiElement;

pub trait TuiParentElement: Extend<Box<dyn TuiElement>> + Sized {
    fn add_children(&mut self, children: impl IntoIterator<Item = Box<dyn TuiElement>>) {
        self.extend(children);
    }

    fn add_child(&mut self, child: Box<dyn TuiElement>) {
        self.extend(Some(child));
    }

    fn with_children(mut self, children: impl IntoIterator<Item = Box<dyn TuiElement>>) -> Self {
        self.add_children(children);
        self
    }

    fn with_child(self, child: Box<dyn TuiElement>) -> Self {
        self.with_children(Some(child))
    }
}

impl<T> TuiParentElement for T where T: Extend<Box<dyn TuiElement>> {}
