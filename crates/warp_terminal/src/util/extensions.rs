pub trait TrimStringExt {
    fn trim_trailing_newline(&mut self);
}

impl TrimStringExt for String {
    fn trim_trailing_newline(&mut self) {
        if self.ends_with('\n') {
            self.pop();
        }
        if self.ends_with('\r') {
            self.pop();
        }
    }
}
