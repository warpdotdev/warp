use super::*;

fn type_chars(fsa: &mut VimFSA, chars: &str) -> Vec<VimEvent> {
    chars
        .chars()
        .filter_map(|c| fsa.typed_character(c))
        .collect()
}

fn enter_normal_mode() -> VimFSA {
    let mut fsa = VimFSA::new();
    fsa.mode = VimMode::Normal;
    fsa
}

fn enter_visual_mode(motion_type: MotionType) -> VimFSA {
    let mut fsa = enter_normal_mode();
    fsa.mode = VimMode::Visual(motion_type);
    fsa
}

fn assert_navigate_jump_to_line(event: &VimEvent, expected_line: u32) {
    match &event.event_type {
        VimEventType::Navigate(VimMotion::JumpToLine(line)) => {
            assert_eq!(*line, expected_line, "expected JumpToLine({expected_line})");
        }
        other => panic!("expected Navigate(JumpToLine({expected_line})), got {other:?}"),
    }
}

fn assert_navigate_jump_to_first_line(event: &VimEvent) {
    match &event.event_type {
        VimEventType::Navigate(VimMotion::JumpToFirstLine) => {}
        other => panic!("expected Navigate(JumpToFirstLine), got {other:?}"),
    }
}

fn assert_navigate_jump_to_last_line(event: &VimEvent) {
    match &event.event_type {
        VimEventType::Navigate(VimMotion::JumpToLastLine) => {}
        other => panic!("expected Navigate(JumpToLastLine), got {other:?}"),
    }
}

fn assert_operation_motion(
    event: &VimEvent,
    expected_operator: VimOperator,
    expected_motion: &VimMotion,
    expected_motion_type: MotionType,
) {
    match &event.event_type {
        VimEventType::Operation {
            operator,
            operand:
                VimOperand::Motion {
                    motion,
                    motion_type,
                },
            register_name: _,
            replacement_text: _,
        } => {
            assert_eq!(*operator, expected_operator, "operator mismatch");
            assert_eq!(*motion_type, expected_motion_type, "motion_type mismatch");
            match (motion, expected_motion) {
                (VimMotion::JumpToLine(actual), VimMotion::JumpToLine(expected)) => {
                    assert_eq!(*actual, *expected, "line number mismatch");
                }
                (VimMotion::JumpToFirstLine, VimMotion::JumpToFirstLine) => {}
                (VimMotion::JumpToLastLine, VimMotion::JumpToLastLine) => {}
                _ => {
                    assert_eq!(
                        std::mem::discriminant(motion),
                        std::mem::discriminant(expected_motion),
                        "motion variant mismatch: expected {expected_motion:?}, got {motion:?}"
                    );
                }
            }
        }
        other => panic!("expected Operation with Motion, got {other:?}"),
    }
}

fn assert_operation_line(event: &VimEvent, expected_operator: VimOperator) {
    match &event.event_type {
        VimEventType::Operation {
            operator,
            operand: VimOperand::Line,
            register_name: _,
            replacement_text: _,
        } => {
            assert_eq!(*operator, expected_operator, "operator mismatch");
        }
        other => panic!("expected Operation with Line operand, got {other:?}"),
    }
}

fn assert_visual_operator(
    event: &VimEvent,
    expected_operator: VimOperator,
    expected_motion_type: MotionType,
) {
    match &event.event_type {
        VimEventType::VisualOperator {
            operator,
            motion_type,
            register_name: _,
        } => {
            assert_eq!(*operator, expected_operator, "operator mismatch");
            assert_eq!(*motion_type, expected_motion_type, "motion_type mismatch");
        }
        other => panic!("expected VisualOperator, got {other:?}"),
    }
}

#[test]
fn test_normal_mode_gg_jumps_to_first_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "gg");
    assert_eq!(events.len(), 1, "gg should produce exactly one event");
    assert_navigate_jump_to_first_line(&events[0]);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_counted_gg_jumps_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "5gg");
    assert_eq!(events.len(), 1, "5gg should produce exactly one event");
    assert_navigate_jump_to_line(&events[0], 5);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_counted_gg_two_digits() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "42gg");
    assert_eq!(events.len(), 1, "42gg should produce exactly one event");
    assert_navigate_jump_to_line(&events[0], 42);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_G_jumps_to_last_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "G");
    assert_eq!(events.len(), 1, "G should produce exactly one event");
    assert_navigate_jump_to_last_line(&events[0]);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_counted_G_jumps_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "3G");
    assert_eq!(events.len(), 1, "3G should produce exactly one event");
    assert_navigate_jump_to_line(&events[0], 3);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_dgg_deletes_to_first_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "dgg");
    assert_eq!(events.len(), 1, "dgg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Delete,
        &VimMotion::JumpToFirstLine,
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_d5gg_deletes_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "d5gg");
    assert_eq!(events.len(), 1, "d5gg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Delete,
        &VimMotion::JumpToLine(5),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_cgg_changes_to_first_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "cgg");
    assert_eq!(events.len(), 1, "cgg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Change,
        &VimMotion::JumpToFirstLine,
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Insert);
}

#[test]
fn test_operator_pending_c3gg_changes_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "c3gg");
    assert_eq!(events.len(), 1, "c3gg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Change,
        &VimMotion::JumpToLine(3),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Insert);
}

#[test]
fn test_operator_pending_ygg_yanks_to_first_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "ygg");
    assert_eq!(events.len(), 1, "ygg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Yank,
        &VimMotion::JumpToFirstLine,
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_y7gg_yanks_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "y7gg");
    assert_eq!(events.len(), 1, "y7gg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Yank,
        &VimMotion::JumpToLine(7),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_dG_deletes_to_last_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "dG");
    assert_eq!(events.len(), 1, "dG should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Delete,
        &VimMotion::JumpToLastLine,
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_d4G_deletes_to_specific_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "d4G");
    assert_eq!(events.len(), 1, "d4G should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Delete,
        &VimMotion::JumpToLine(4),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_visual_linewise_mode_gg_selects_to_first_line() {
    let mut fsa = enter_visual_mode(MotionType::Linewise);
    let events = type_chars(&mut fsa, "gg");
    assert_eq!(
        events.len(),
        1,
        "gg in visual linewise should produce exactly one event"
    );
    assert_navigate_jump_to_first_line(&events[0]);
}

#[test]
fn test_visual_linewise_mode_5gg_selects_to_specific_line() {
    let mut fsa = enter_visual_mode(MotionType::Linewise);
    let events = type_chars(&mut fsa, "5gg");
    assert_eq!(
        events.len(),
        1,
        "5gg in visual linewise should produce exactly one event"
    );
    assert_navigate_jump_to_line(&events[0], 5);
}

#[test]
fn test_visual_charwise_mode_gg_selects_to_first_line() {
    let mut fsa = enter_visual_mode(MotionType::Charwise);
    let events = type_chars(&mut fsa, "gg");
    assert_eq!(
        events.len(),
        1,
        "gg in visual charwise should produce exactly one event"
    );
    assert_navigate_jump_to_first_line(&events[0]);
}

#[test]
fn test_visual_charwise_mode_10gg_selects_to_specific_line() {
    let mut fsa = enter_visual_mode(MotionType::Charwise);
    let events = type_chars(&mut fsa, "10gg");
    assert_eq!(
        events.len(),
        1,
        "10gg in visual charwise should produce exactly one event"
    );
    assert_navigate_jump_to_line(&events[0], 10);
}

#[test]
fn test_visual_linewise_mode_G_selects_to_last_line() {
    let mut fsa = enter_visual_mode(MotionType::Linewise);
    let events = type_chars(&mut fsa, "G");
    assert_eq!(
        events.len(),
        1,
        "G in visual linewise should produce exactly one event"
    );
    assert_navigate_jump_to_last_line(&events[0]);
}

#[test]
fn test_visual_linewise_mode_3G_selects_to_specific_line() {
    let mut fsa = enter_visual_mode(MotionType::Linewise);
    let events = type_chars(&mut fsa, "3G");
    assert_eq!(
        events.len(),
        1,
        "3G in visual linewise should produce exactly one event"
    );
    assert_navigate_jump_to_line(&events[0], 3);
}

#[test]
fn test_normal_mode_1gg_jumps_to_line_1() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "1gg");
    assert_eq!(events.len(), 1, "1gg should produce exactly one event");
    assert_navigate_jump_to_line(&events[0], 1);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_operator_pending_d1gg_deletes_to_line_1() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "d1gg");
    assert_eq!(events.len(), 1, "d1gg should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Delete,
        &VimMotion::JumpToLine(1),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_double_greater_indents_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, ">>");
    assert_eq!(events.len(), 1, ">> should produce exactly one event");
    assert_operation_line(&events[0], VimOperator::Indent);
    assert_eq!(events[0].count, 1, ">> should have count 1");
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_double_less_dedents_line() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "<<");
    assert_eq!(events.len(), 1, "<< should produce exactly one event");
    assert_operation_line(&events[0], VimOperator::Dedent);
    assert_eq!(events[0].count, 1, "<< should have count 1");
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_greater_with_down_motion_is_linewise() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, ">j");
    assert_eq!(events.len(), 1, ">j should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Indent,
        &VimMotion::Character(CharacterMotion::Down),
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_greater_with_word_motion_is_charwise() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, ">w");
    assert_eq!(events.len(), 1, ">w should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Indent,
        &VimMotion::Word(WordMotion::new(
            Direction::Forward,
            WordBound::Start,
            WordType::Default,
        )),
        MotionType::Charwise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_greater_to_last_line_is_linewise() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, ">G");
    assert_eq!(events.len(), 1, ">G should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Indent,
        &VimMotion::JumpToLastLine,
        MotionType::Linewise,
    );
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_counted_double_greater_indents_count_lines() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "2>>");
    assert_eq!(events.len(), 1, "2>> should produce exactly one event");
    assert_operation_line(&events[0], VimOperator::Indent);
    assert_eq!(events[0].count, 2, "2>> should have count 2");
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_counted_greater_with_motion_multiplies_count() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "3>j");
    assert_eq!(events.len(), 1, "3>j should produce exactly one event");
    assert_operation_motion(
        &events[0],
        VimOperator::Indent,
        &VimMotion::Character(CharacterMotion::Down),
        MotionType::Linewise,
    );
    assert_eq!(events[0].count, 3, "3>j should have count 3");
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_visual_linewise_greater_emits_visual_indent_operator() {
    let mut fsa = enter_visual_mode(MotionType::Linewise);
    let events = type_chars(&mut fsa, ">");
    assert_eq!(
        events.len(),
        1,
        "> in visual linewise should produce one event"
    );
    assert_visual_operator(&events[0], VimOperator::Indent, MotionType::Linewise);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_visual_charwise_less_emits_visual_dedent_operator() {
    let mut fsa = enter_visual_mode(MotionType::Charwise);
    let events = type_chars(&mut fsa, "<");
    assert_eq!(
        events.len(),
        1,
        "< in visual charwise should produce one event"
    );
    assert_visual_operator(&events[0], VimOperator::Dedent, MotionType::Charwise);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_double_greater_is_dot_repeatable() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, ">>");
    assert_eq!(events.len(), 1, ">> should produce exactly one event");
    assert_operation_line(&events[0], VimOperator::Indent);

    let repeat_events = type_chars(&mut fsa, ".");
    assert_eq!(
        repeat_events.len(),
        1,
        ". should replay the indent operation"
    );
    assert_operation_line(&repeat_events[0], VimOperator::Indent);
    assert_eq!(fsa.mode, VimMode::Normal);
}

#[test]
fn test_normal_mode_double_less_is_dot_repeatable() {
    let mut fsa = enter_normal_mode();
    let events = type_chars(&mut fsa, "<<");
    assert_eq!(events.len(), 1, "<< should produce exactly one event");
    assert_operation_line(&events[0], VimOperator::Dedent);

    let repeat_events = type_chars(&mut fsa, ".");
    assert_eq!(
        repeat_events.len(),
        1,
        ". should replay the dedent operation"
    );
    assert_operation_line(&repeat_events[0], VimOperator::Dedent);
    assert_eq!(fsa.mode, VimMode::Normal);
}
