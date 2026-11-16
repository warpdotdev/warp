mod matching_brackets;
pub use matching_brackets::vim_find_matching_bracket;

mod paragraph_iterator;
pub use paragraph_iterator::{find_next_paragraph_end, find_previous_paragraph_start};
pub mod register;

mod text_objects;
pub use text_objects::*;

mod word_iterator;
pub use word_iterator::vim_word_iterator_from_offset;

mod find_char;
pub use find_char::vim_find_char_on_line;

pub mod vim;
