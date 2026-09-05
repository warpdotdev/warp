use string_offset::CharOffset;

use crate::handler::{
    CaseTransform, VimBufferOps, VimCaret, VimSnapshot, YankedText, apply_mode_change,
    apply_operator, apply_visual_operator, apply_visual_paste, jump_to_matching_bracket, move_char,
    operand_motion_type, visual_highlight_ranges,
};
use crate::vim::{
    CharacterMotion, Direction, InsertPosition, ModeTransition, MotionType, TextObjectInclusion,
    TextObjectType, VimMode, VimOperand, VimOperator, VimTextObject, WordBound, WordMotion,
    WordType,
};

struct FakeBuffer {
    text: String,
    carets: Vec<VimCaret>,
    supported_operators: Option<Vec<VimOperator>>,
    comments_toggled: bool,
    indented: Option<bool>,
    text_objects: bool,
}

impl Default for FakeBuffer {
    fn default() -> Self {
        Self {
            text: "abc".into(),
            carets: vec![VimCaret {
                head: CharOffset::from(1),
                tail: CharOffset::from(4),
                goal_column: None,
            }],
            supported_operators: None,
            comments_toggled: false,
            indented: None,
            text_objects: false,
        }
    }
}

impl FakeBuffer {
    fn supporting(operators: Vec<VimOperator>) -> Self {
        Self {
            supported_operators: Some(operators),
            ..Self::default()
        }
    }
}

impl VimBufferOps for FakeBuffer {
    type Ctx<'a> = ();

    fn snapshot(&self, _ctx: &Self::Ctx<'_>) -> VimSnapshot {
        VimSnapshot::from_plain_text(&self.text, self.carets.clone())
    }

    fn set_selections(&mut self, carets: &[VimCaret], _ctx: &mut Self::Ctx<'_>) {
        self.carets = carets.to_vec();
    }

    fn replace_ranges(
        &mut self,
        edits: &[(CharOffset, CharOffset, String)],
        _ctx: &mut Self::Ctx<'_>,
    ) {
        let mut chars: Vec<char> = self.text.chars().collect();
        let mut sorted = edits.to_vec();
        sorted.sort_by_key(|(start, _, _)| start.as_usize());
        sorted.reverse();
        for (start, end, replacement) in sorted {
            let s = start.as_usize().saturating_sub(1).min(chars.len());
            let e = end.as_usize().saturating_sub(1).min(chars.len()).max(s);
            let repl: Vec<char> = replacement.chars().collect();
            chars.splice(s..e, repl);
        }
        self.text = chars.into_iter().collect();
        if let Some(caret) = self.carets.first_mut() {
            caret.head = CharOffset::from(1);
            caret.tail = caret.head;
        }
    }

    fn toggle_comments(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.comments_toggled = true;
    }

    fn indent(&mut self, dedent: bool, _ctx: &mut Self::Ctx<'_>) {
        self.indented = Some(dedent);
    }

    fn supports_operator(&self, operator: &VimOperator) -> bool {
        self.supported_operators
            .as_ref()
            .is_none_or(|ops| ops.contains(operator))
    }

    fn supports_text_objects(&self) -> bool {
        self.text_objects
    }
}

fn word_operand() -> VimOperand {
    VimOperand::Motion {
        motion_type: MotionType::Charwise,
        motion: crate::vim::VimMotion::Word(WordMotion::new(
            Direction::Forward,
            WordBound::Start,
            WordType::Default,
        )),
    }
}

#[test]
fn operand_motion_type_treats_paragraph_objects_as_linewise() {
    let paragraph = VimOperand::TextObject(VimTextObject {
        inclusion: TextObjectInclusion::Inner,
        object_type: TextObjectType::Paragraph,
    });
    let word = VimOperand::TextObject(VimTextObject {
        inclusion: TextObjectInclusion::Inner,
        object_type: TextObjectType::Word(WordType::Default),
    });

    assert_eq!(operand_motion_type(&paragraph), MotionType::Linewise);
    assert_eq!(operand_motion_type(&word), MotionType::Charwise);
    assert_eq!(operand_motion_type(&VimOperand::Line), MotionType::Linewise);
}

#[test]
fn delete_yanks_then_deletes_selection() {
    let mut buffer = FakeBuffer {
        text: "abc def".into(),
        carets: vec![VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::Delete,
        1,
        &word_operand(),
        "",
        &mut (),
    );
    assert!(yanked.is_some());
    assert_ne!(buffer.text, "abc");
}

#[test]
fn empty_linewise_delete_yanks_newline_without_deleting_empty_selection() {
    let mut buffer = FakeBuffer {
        text: String::new(),
        carets: vec![VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::Delete,
        1,
        &VimOperand::Line,
        "",
        &mut (),
    );
    assert_eq!(
        yanked,
        Some(YankedText {
            text: "\n".into(),
            motion_type: MotionType::Linewise,
        })
    );
}

#[test]
fn unsupported_operators_are_no_ops() {
    let mut buffer = FakeBuffer::supporting(vec![VimOperator::Delete, VimOperator::Yank]);
    let before = buffer.text.clone();
    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::ToggleComment,
        1,
        &VimOperand::Line,
        "",
        &mut (),
    );
    assert!(yanked.is_none());
    assert!(!buffer.comments_toggled);
    assert_eq!(buffer.text, before);
}

#[test]
fn unsupported_visual_operators_do_not_expand_or_mutate() {
    let mut buffer = FakeBuffer::supporting(vec![VimOperator::Delete, VimOperator::Yank]);
    let before = buffer.text.clone();
    let carets = buffer.carets.clone();
    let yanked = apply_visual_operator(
        &mut buffer,
        &VimOperator::ToggleComment,
        MotionType::Charwise,
        &mut (),
    );
    assert!(yanked.is_none());
    assert!(!buffer.comments_toggled);
    assert_eq!(buffer.text, before);
    assert_eq!(buffer.carets, carets);
}

#[test]
fn visual_paste_replaces_selection_and_returns_replaced_text() {
    let mut buffer = FakeBuffer::default();
    let yanked = apply_visual_paste(
        &mut buffer,
        MotionType::Charwise,
        "new",
        MotionType::Charwise,
        &mut (),
    );
    assert_eq!(buffer.text, "new");
    assert_eq!(
        yanked,
        Some(YankedText {
            text: "abc".into(),
            motion_type: MotionType::Charwise,
        })
    );
}

#[test]
fn mode_change_applies_insert_exit_and_visual_tails() {
    let mut buffer = FakeBuffer {
        text: "ab".into(),
        carets: vec![VimCaret {
            head: CharOffset::from(2),
            tail: CharOffset::from(2),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    apply_mode_change(
        &mut buffer,
        &VimMode::Insert,
        &VimMode::Normal.into(),
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(1));

    let mut buffer = FakeBuffer::default();
    apply_mode_change(
        &mut buffer,
        &VimMode::Normal,
        &ModeTransition {
            mode: VimMode::Insert,
            position: InsertPosition::LineBelow,
        },
        &mut (),
    );
    assert!(buffer.text.contains('\n'));

    let mut buffer = FakeBuffer::default();
    apply_mode_change(
        &mut buffer,
        &VimMode::Normal,
        &ModeTransition {
            mode: VimMode::Visual(MotionType::Charwise),
            position: InsertPosition::AtCursor,
        },
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, buffer.carets[0].tail);
}

#[test]
fn case_transform_toggle_swaps_case() {
    assert_eq!(CaseTransform::Toggle.apply_to("AbC"), "aBc");
}

#[test]
fn line_end_insert_appends_after_last_character() {
    let mut buffer = FakeBuffer {
        text: "hello".into(),
        carets: vec![VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    apply_mode_change(
        &mut buffer,
        &VimMode::Normal,
        &ModeTransition {
            mode: VimMode::Insert,
            position: InsertPosition::LineEnd,
        },
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(6));
    buffer.insert_text("x", &mut ());
    assert_eq!(buffer.text, "hellox");
}

#[test]
fn visual_move_char_keeps_origin_tail() {
    let mut buffer = FakeBuffer {
        text: "hello".into(),
        carets: vec![VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    move_char(&mut buffer, 2, &CharacterMotion::Right, true, &mut ());
    assert_eq!(buffer.carets[0].tail, CharOffset::from(1));
    assert_eq!(buffer.carets[0].head, CharOffset::from(3));
}

#[test]
fn matching_bracket_jump_lands_on_pair() {
    let mut buffer = FakeBuffer {
        text: "(ab)".into(),
        carets: vec![VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
            goal_column: None,
        }],
        ..FakeBuffer::default()
    };
    jump_to_matching_bracket(&mut buffer, false, &mut ());
    assert_eq!(buffer.carets[0].head, CharOffset::from(4));
}

fn buffer_at(text: &str, head: usize) -> FakeBuffer {
    FakeBuffer {
        text: text.into(),
        carets: vec![VimCaret::new(
            CharOffset::from(head),
            CharOffset::from(head),
        )],
        ..FakeBuffer::default()
    }
}

#[test]
fn wrapping_navigation_skips_newlines() {
    let mut buffer = buffer_at("ab\ncd", 2);
    move_char(
        &mut buffer,
        1,
        &CharacterMotion::WrappingRight,
        false,
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(4));

    move_char(
        &mut buffer,
        1,
        &CharacterMotion::WrappingLeft,
        false,
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(2));
}

#[test]
fn wrapping_operator_counts_newlines() {
    let mut buffer = buffer_at("ab\ncd", 2);
    move_char(
        &mut buffer,
        1,
        &CharacterMotion::WrappingRight,
        true,
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(3));
    assert_eq!(buffer.carets[0].tail, CharOffset::from(2));
}

#[test]
fn wrapping_navigation_lands_on_empty_line() {
    let mut buffer = buffer_at("ab\n\ncd", 2);
    move_char(
        &mut buffer,
        1,
        &CharacterMotion::WrappingRight,
        false,
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(3));
}

#[test]
fn wrapping_navigation_count_skips_one_newline() {
    let mut buffer = buffer_at("ab\ncd", 1);
    move_char(
        &mut buffer,
        2,
        &CharacterMotion::WrappingRight,
        false,
        &mut (),
    );
    assert_eq!(buffer.carets[0].head, CharOffset::from(4));
}

#[test]
fn vertical_motion_restores_goal_column_after_short_line() {
    let mut buffer = buffer_at("xxxx\nab\nxxxx", 4);
    move_char(&mut buffer, 1, &CharacterMotion::Down, false, &mut ());
    assert_eq!(buffer.carets[0].head, CharOffset::from(7));
    assert_eq!(buffer.carets[0].goal_column, Some(3));
    move_char(&mut buffer, 1, &CharacterMotion::Down, false, &mut ());
    assert_eq!(buffer.carets[0].head, CharOffset::from(12));
    assert_eq!(buffer.carets[0].goal_column, Some(3));
}

#[test]
fn line_cap_preserves_goal_column() {
    let mut buffer = buffer_at("xxxx\nab\nxxxx", 8);
    buffer.carets[0].goal_column = Some(3);
    buffer.enforce_cursor_line_cap(&mut ());
    assert_eq!(buffer.carets[0].head, CharOffset::from(7));
    assert_eq!(buffer.carets[0].goal_column, Some(3));
}

#[test]
fn vertical_visual_motion_keeps_tail_and_goal_column() {
    let mut buffer = buffer_at("xxxx\nab\nxxxx", 4);
    move_char(&mut buffer, 1, &CharacterMotion::Down, true, &mut ());
    assert_eq!(buffer.carets[0].tail, CharOffset::from(4));
    move_char(&mut buffer, 1, &CharacterMotion::Down, true, &mut ());
    assert_eq!(buffer.carets[0].tail, CharOffset::from(4));
    assert_eq!(buffer.carets[0].head, CharOffset::from(12));
}

#[test]
fn visual_highlight_includes_block_cursor_character() {
    let mut buffer = buffer_at("abc", 1);
    assert_eq!(
        visual_highlight_ranges(&buffer.snapshot(&()), MotionType::Charwise),
        vec![CharOffset::from(1)..CharOffset::from(2)]
    );

    move_char(&mut buffer, 2, &CharacterMotion::Right, true, &mut ());
    assert_eq!(
        visual_highlight_ranges(&buffer.snapshot(&()), MotionType::Charwise),
        vec![CharOffset::from(1)..CharOffset::from(4)]
    );
}

#[test]
fn visual_highlight_linewise_covers_full_lines() {
    let mut buffer = buffer_at("ab\ncd", 1);
    assert_eq!(
        visual_highlight_ranges(&buffer.snapshot(&()), MotionType::Linewise),
        vec![CharOffset::from(1)..CharOffset::from(3)]
    );

    move_char(&mut buffer, 1, &CharacterMotion::Down, true, &mut ());
    assert_eq!(
        visual_highlight_ranges(&buffer.snapshot(&()), MotionType::Linewise),
        vec![CharOffset::from(1)..CharOffset::from(6)]
    );
}
