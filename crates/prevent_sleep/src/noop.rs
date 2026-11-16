pub struct Guard;

pub fn prevent_sleep(_reason: &'static str) -> Guard {
    Guard
}
