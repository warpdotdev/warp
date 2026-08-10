use super::*;

fn instance(scope: PluginScopeId, kind: PluginSourceKind) -> PluginInstanceId {
    PluginInstanceId::new(scope, PluginSourceId::new(kind, "/root"), "acme-tools")
}

#[test]
fn qualified_names_use_the_plugin_and_component_names() {
    let component = PluginComponentId::new(
        instance(PluginScopeId::Repository, PluginSourceKind::AgentsDirectory),
        PluginComponentKind::Skill,
        "deploy",
    );
    assert_eq!(component.qualified_name(), "acme-tools:deploy");
    assert_eq!(component.to_string(), "acme-tools:deploy");
}

/// Precedence: repository outranks user, and `.agents` outranks `.warp` within a scope.
#[test]
fn interactive_precedence_orders_repository_over_user_and_agents_over_warp() {
    let repository_agents =
        instance(PluginScopeId::Repository, PluginSourceKind::AgentsDirectory).precedence();
    let repository_warp =
        instance(PluginScopeId::Repository, PluginSourceKind::WarpDirectory).precedence();
    let user_agents = instance(PluginScopeId::User, PluginSourceKind::AgentsDirectory).precedence();
    let user_warp = instance(PluginScopeId::User, PluginSourceKind::WarpDirectory).precedence();

    assert!(repository_agents < repository_warp);
    assert!(repository_warp < user_agents);
    assert!(user_agents < user_warp);
}

/// Factory precedence: automation over agent over factory, and every Factory scope outranks the
/// local scopes so a Factory definition is never displaced by a checkout's own plugins.
#[test]
fn factory_precedence_orders_automation_over_agent_over_factory() {
    let automation = PluginScopeId::Automation {
        name: "nightly".to_owned(),
    }
    .scope_rank();
    let agent = PluginScopeId::Agent {
        name: "release".to_owned(),
    }
    .scope_rank();
    let factory = PluginScopeId::Factory.scope_rank();

    assert!(automation < agent);
    assert!(agent < factory);
    assert!(factory < PluginScopeId::Repository.scope_rank());
}

#[test]
fn qualified_names_split_on_the_first_separator() {
    assert_eq!(
        split_qualified_name("acme-tools:deploy"),
        Some(("acme-tools", "deploy"))
    );
    // A plugin name may contain periods, and a component name may contain a colon of its own.
    assert_eq!(
        split_qualified_name("acme.tools:deploy:prod"),
        Some(("acme.tools", "deploy:prod"))
    );
    assert_eq!(split_qualified_name("deploy"), None);
    assert_eq!(split_qualified_name(":deploy"), None);
    assert_eq!(split_qualified_name("acme-tools:"), None);
}

/// Scope key tokens feed the persistent data key, so two differently named agents must not
/// collapse onto one directory.
#[test]
fn scope_key_tokens_distinguish_named_scopes() {
    assert_eq!(
        PluginScopeId::Agent {
            name: "release".to_owned()
        }
        .key_token(),
        "agent/release"
    );
    assert_ne!(
        PluginScopeId::Agent {
            name: "a".to_owned()
        }
        .key_token(),
        PluginScopeId::Agent {
            name: "b".to_owned()
        }
        .key_token()
    );
    assert_ne!(
        PluginScopeId::Agent {
            name: "a".to_owned()
        }
        .key_token(),
        PluginScopeId::Automation {
            name: "a".to_owned()
        }
        .key_token()
    );
}
