import re

with open('app/src/workspace/view.rs', 'r') as f:
    content = f.read()

# Fix WorkspaceAction::AddProjectFolder missed dispatch
content = content.replace("ctx.dispatch_typed_action(WorkspaceAction::AddProjectFolder);", "ctx.dispatch_typed_action(&WorkspaceAction::AddProjectFolder);")

# Fix 7103
content = content.replace('''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            ctx,
        );''', '''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            None,
            ctx,
        );''')

# Fix 7388
content = content.replace('''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            group_working_directory,
            false,
            ctx,
        );''', '''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            group_working_directory,
            ctx,
        );''')

# Fix 12418
content = content.replace('''                me.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    Some(shell),
                    None,
                    true,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''', '''                me.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    Some(shell),
                    None,
                    true,
                    None,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''')

# Fix 19166
content = content.replace('''                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    None,
                    None,
                    false,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''', '''                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    None,
                    None,
                    false,
                    None,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''')

# Fix 19314
content = content.replace('''        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );''', '''        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            None,
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );''')

# Fix 24025
content = content.replace('''                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(ctx.window_id()),
                    None,
                    None,
                    *hide_homepage,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''', '''                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(ctx.window_id()),
                    None,
                    None,
                    *hide_homepage,
                    None,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );''')

with open('app/src/workspace/view.rs', 'w') as f:
    f.write(content)

