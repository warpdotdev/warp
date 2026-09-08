use std::collections::HashMap;

use smol_str::SmolStr;

use super::*;
use crate::terminal::model::session::SessionInfo;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;

#[test]
fn test_is_expandable_alias_when_expandable() {
    // Alias is not in the alias value
    let expandable = is_expandable_alias("gco", "git checkout");
    assert!(expandable);

    // Alias is in the alias value but not in a command position
    let expandable = is_expandable_alias("gco", "git checkout gco");
    assert!(expandable);
}

#[test]
fn test_is_expandable_alias_when_unexpandable() {
    let expandable = is_expandable_alias("ls", "ls -G");
    assert!(!expandable);

    let expandable = is_expandable_alias("ls", "ls");
    assert!(!expandable);
}

#[test]
fn test_is_expandable_alias_when_alias_value_is_empty() {
    let expandable = is_expandable_alias("ls", "");
    assert!(!expandable);
}

#[test]
fn test_check_for_alias_has_alias() {
    let alias: SmolStr = "gco".into();
    let alias_value: String = "git checkout".into();
    let aliases = HashMap::from_iter([(alias.clone(), alias_value.clone())]);
    let session = Arc::new(Session::new(
        SessionInfo::new_for_test().with_aliases(aliases),
        Arc::new(TestCommandExecutor::default()),
    ));
    let expected = Some(AliasedCommand { alias, alias_value });

    // The alias is a command.
    let res = check_for_alias("gco branch", session.clone());
    assert_eq!(res, expected);

    // The alias is returned when it is not the first command
    let res = check_for_alias("echo hello && gco branch", session);
    assert_eq!(res, expected);
}

#[test]
fn test_check_for_alias_no_alias() {
    let aliases = HashMap::from_iter([
        ("gco".into(), "git checkout".into()),
        ("ls".into(), "ls -G".into()),
    ]);
    let session = Arc::new(Session::new(
        SessionInfo::new_for_test().with_aliases(aliases),
        Arc::new(TestCommandExecutor::default()),
    ));

    // No alias is returned if no aliases are in the command.
    let res = check_for_alias("echo hello", session.clone());
    assert_eq!(res, None);

    // No alias is returned if the alias is in a non-command position.
    let res = check_for_alias("echo gco", session.clone());
    assert_eq!(res, None);

    // No alias is returned if the alias is not expandable.
    let res = check_for_alias("ls", session);
    assert_eq!(res, None);
}
