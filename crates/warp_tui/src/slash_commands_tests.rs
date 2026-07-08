use ai::skills::SkillReference;
use warp::tui_export::{DetectedSkillCommand, ParsedSlashCommandInput};

use super::menu_query_for_parsed_input;

fn parsed_skill(argument: Option<&str>) -> ParsedSlashCommandInput {
    ParsedSlashCommandInput::SkillCommand(DetectedSkillCommand {
        reference: SkillReference::BundledSkillId("write-product-spec".to_owned()),
        name: "write-product-spec".to_owned(),
        argument: argument.map(str::to_owned),
    })
}

#[test]
fn skill_without_argument_remains_searchable() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(None)).as_deref(),
        Some("write-product-spec")
    );
}

#[test]
fn skill_argument_entry_closes_menu() {
    assert_eq!(menu_query_for_parsed_input(&parsed_skill(Some(""))), None);
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(Some("here is my prompt"))),
        None
    );
}
