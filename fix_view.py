import re

with open('app/src/workspace/view.rs', 'r') as f:
    content = f.read()

# Fix TabGroup initializer
content = content.replace('''                                TabGroup {
                                    id: group_snapshot.id,
                                    name: group_snapshot.name.clone(),
                                    color: group_snapshot.color,
                                    collapsed: group_snapshot.collapsed,
                                    draggable_state: Default::default(),
                                    // Only restore pinned state when the
                                    // Pinned Tabs feature is enabled.
                                    pinned: FeatureFlag::PinnedTabs.is_enabled()
                                        && group_snapshot.pinned,
                                },''', '''                                TabGroup {
                                    id: group_snapshot.id,
                                    name: group_snapshot.name.clone(),
                                    color: group_snapshot.color,
                                    collapsed: group_snapshot.collapsed,
                                    draggable_state: Default::default(),
                                    // Only restore pinned state when the
                                    // Pinned Tabs feature is enabled.
                                    pinned: FeatureFlag::PinnedTabs.is_enabled()
                                        && group_snapshot.pinned,
                                    working_directory: group_snapshot.working_directory.clone().map(std::path::PathBuf::from),
                                },''')

# Fix TabGroupSnapshot initializer
content = content.replace('''                .map(|group| TabGroupSnapshot {
                    id: group.id,
                    name: group.name.clone(),
                    color: group.color,
                    collapsed: group.collapsed,
                    pinned: FeatureFlag::PinnedTabs.is_enabled() && group.pinned,
                })''', '''                .map(|group| TabGroupSnapshot {
                    id: group.id,
                    name: group.name.clone(),
                    color: group.color,
                    collapsed: group.collapsed,
                    pinned: FeatureFlag::PinnedTabs.is_enabled() && group.pinned,
                    working_directory: group.working_directory.as_ref().map(|p| p.to_string_lossy().into_owned()),
                })''')

# Fix CreateProjectFolder dispatch
content = content.replace('''            NewSessionMenuItem::CreateProjectFolder => {
                ctx.dispatch_typed_action(WorkspaceAction::AddProjectFolder);
            }''', '''            NewSessionMenuItem::CreateProjectFolder => {
                ctx.dispatch_typed_action(&WorkspaceAction::AddProjectFolder);
            }''')

# Fix AddProjectFolder logic
content = content.replace('''            AddProjectFolder => {
                ctx.open_file_picker(
                    FilePickerConfiguration::new().folders_only(),
                    |result, app| {
                        if let Ok(Some(paths)) = result {
                            if let Some(path_str) = paths.into_iter().next() {
                                let path = std::path::PathBuf::from(path_str);
                                app.dispatch_typed_action(WorkspaceAction::AddProjectFolderConfirmed(path));
                            }
                        }
                    },
                );
            }''', '''            AddProjectFolder => {
                ctx.open_file_picker(
                    |result, app| {
                        if let Ok(paths) = result {
                            if let Some(path_str) = paths.into_iter().next() {
                                let path = std::path::PathBuf::from(path_str);
                                app.dispatch_typed_action(&WorkspaceAction::AddProjectFolderConfirmed(path));
                            }
                        }
                    },
                    FilePickerConfiguration::new().folders_only(),
                );
            }''')

# Update all `self.add_new_session_tab_with_default_mode(` calls to include `override_directory`.
# We find `self.add_new_session_tab_with_default_mode(` and the next 6 parameters, then insert `None,` or `group_working_directory,` before `ctx,`.

def replacer_fn(match):
    prefix = match.group(1)
    args = match.group(2)
    # The last argument is usually `ctx,` or `ctx`
    lines = args.split('\n')
    # Find where `ctx,` is
    ctx_idx = -1
    for i, line in enumerate(lines):
        if 'ctx,' in line or 'ctx' in line.strip():
            if 'ctx' == line.strip() or 'ctx,' == line.strip() or line.strip().startswith('ctx'):
                ctx_idx = i
                break
    if ctx_idx != -1:
        # Check if we already added override_directory.
        # If it's the one in new_tab_in_group, we insert `group_working_directory,`
        # Wait, the one in new_tab_in_group looks like this:
        #             false,
        #             group_working_directory,
        #             ctx,
        # So it already has group_working_directory! We don't want to add `None,` there.
        # Let's count args.
        # It's better to just do string replacements for each exact call site.
        pass
    return match.group(0) # fallback

# I will just manually replace the 8 calls exactly:

content = content.replace('''                    self.add_new_session_tab_with_default_mode(
                        NewSessionSource::Window,
                        None,  /* previous_active_window */
                        None,  /* chosen_shell */
                        None,  /* ai_conversation */
                        false, /* hide_homepage */
                        ctx,
                    );''', '''                    self.add_new_session_tab_with_default_mode(
                        NewSessionSource::Window,
                        None,  /* previous_active_window */
                        None,  /* chosen_shell */
                        None,  /* ai_conversation */
                        false, /* hide_homepage */
                        None,  /* override_directory */
                        ctx,
                    );''')

content = content.replace('''                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Window,
                    previous_active_window,
                    shell,
                    None,  /* ai_conversation */
                    false, /* hide_homepage */
                    ctx,
                );''', '''                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Window,
                    previous_active_window,
                    shell,
                    None,  /* ai_conversation */
                    false, /* hide_homepage */
                    None,  /* override_directory */
                    ctx,
                );''')

content = content.replace('''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Window,
            previous_active_window,
            shell,
            None,
            false,
            ctx,
        );''', '''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Window,
            previous_active_window,
            shell,
            None,
            false,
            None,
            ctx,
        );''')

content = content.replace('''                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Tab,
                    None,
                    None,
                    None,
                    false,
                    ctx,
                );''', '''                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Tab,
                    None,
                    None,
                    None,
                    false,
                    None,
                    ctx,
                );''')

content = content.replace('''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            hide_homepage,
            ctx,
        );''', '''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            hide_homepage,
            None,
            ctx,
        );''')

content = content.replace('''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            Some(shell),
            None,
            false,
            ctx,
        );''', '''        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            Some(shell),
            None,
            false,
            None,
            ctx,
        );''')

content = content.replace('''            self.add_new_session_tab_with_default_mode(
                NewSessionSource::Tab,
                Some(window_id),
                None,
                Some(ConversationRestorationInNewPaneType::Forked {
                    conversation: forked_conversation,
                    has_initial_query,
                }),
                false,
                ctx,
            );''', '''            self.add_new_session_tab_with_default_mode(
                NewSessionSource::Tab,
                Some(window_id),
                None,
                Some(ConversationRestorationInNewPaneType::Forked {
                    conversation: forked_conversation,
                    has_initial_query,
                }),
                false,
                None,
                ctx,
            );''')

# Replace the function signature of add_new_session_tab_with_default_mode
content = content.replace('''    fn add_new_session_tab_with_default_mode(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            new_session_source,
            previous_session_window_id,
            chosen_shell,
            conversation_restoration,
            hide_homepage,
            DefaultSessionModeBehavior::Apply,
            ctx,
        );
    }''', '''    fn add_new_session_tab_with_default_mode(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        override_directory: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            new_session_source,
            previous_session_window_id,
            chosen_shell,
            conversation_restoration,
            hide_homepage,
            override_directory,
            DefaultSessionModeBehavior::Apply,
            ctx,
        );
    }''')

# Replace the signature of add_new_session_tab_internal_with_default_session_mode_behavior
content = content.replace('''    fn add_new_session_tab_internal_with_default_session_mode_behavior(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        default_session_mode_behavior: DefaultSessionModeBehavior,
        ctx: &mut ViewContext<Self>,
    ) {''', '''    fn add_new_session_tab_internal_with_default_session_mode_behavior(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        override_directory: Option<PathBuf>,
        default_session_mode_behavior: DefaultSessionModeBehavior,
        ctx: &mut ViewContext<Self>,
    ) {''')

# Use override_directory in add_new_session_tab_internal_with_default_session_mode_behavior
content = content.replace('''        let startup_directory = startup_directory_from_conversation.or_else(|| {
            self.get_new_tab_startup_directory(
                new_session_source,
                previous_session_window_id,
                chosen_shell.as_ref(),
                ctx,
            )
        });''', '''        let startup_directory = startup_directory_from_conversation.or_else(|| {
            override_directory.or_else(|| {
                self.get_new_tab_startup_directory(
                    new_session_source,
                    previous_session_window_id,
                    chosen_shell.as_ref(),
                    ctx,
                )
            })
        });''')

with open('app/src/workspace/view.rs', 'w') as f:
    f.write(content)

