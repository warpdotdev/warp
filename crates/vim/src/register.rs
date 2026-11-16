/// In Vim, see ":help quote_".
/// The black hole register can be written to but it always reads as empty.
pub const BLACK_HOLE_REGISTER: char = '_';

/// The registers we currently support.
pub fn valid_register_name(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '+' | '*' | '"')
}
