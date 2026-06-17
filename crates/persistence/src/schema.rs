// @generated automatically by Diesel CLI.

diesel::table! {
    active_mcp_servers (id) {
        id -> Integer,
        mcp_server_uuid -> Text,
    }
}

diesel::table! {
    agent_conversations (id) {
        id -> Integer,
        conversation_id -> Text,
        conversation_data -> Text,
        last_modified_at -> Timestamp,
    }
}

diesel::table! {
    agent_tasks (id) {
        id -> Integer,
        conversation_id -> Text,
        task_id -> Text,
        task -> Binary,
        last_modified_at -> Timestamp,
    }
}

diesel::table! {
    ai_document_panes (id) {
        id -> Integer,
        kind -> Text,
        document_id -> Text,
        version -> Integer,
        content -> Nullable<Text>,
        title -> Nullable<Text>,
    }
}

diesel::table! {
    ai_memory_panes (id) {
        id -> Integer,
        kind -> Text,
    }
}

diesel::table! {
    ai_queries (id) {
        id -> Integer,
        exchange_id -> Text,
        conversation_id -> Text,
        start_ts -> Timestamp,
        input -> Text,
        working_directory -> Nullable<Text>,
        output_status -> Text,
        model_id -> Text,
        planning_model_id -> Text,
        coding_model_id -> Text,
    }
}

diesel::table! {
    app (id) {
        id -> Nullable<Integer>,
        active_window_id -> Nullable<Integer>,
    }
}

diesel::table! {
    blocks (id) {
        id -> Nullable<Integer>,
        pane_leaf_uuid -> Binary,
        stylized_command -> Binary,
        stylized_output -> Binary,
        pwd -> Nullable<Text>,
        git_branch -> Nullable<Text>,
        virtual_env -> Nullable<Text>,
        conda_env -> Nullable<Text>,
        exit_code -> Integer,
        did_execute -> Bool,
        completed_ts -> Nullable<Timestamp>,
        start_ts -> Nullable<Timestamp>,
        ps1 -> Nullable<Text>,
        honor_ps1 -> Bool,
        shell -> Nullable<Text>,
        user -> Nullable<Text>,
        host -> Nullable<Text>,
        is_background -> Bool,
        rprompt -> Nullable<Text>,
        prompt_snapshot -> Nullable<Text>,
        block_id -> Text,
        ai_metadata -> Nullable<Text>,
        is_local -> Nullable<Bool>,
        agent_view_visibility -> Nullable<Text>,
        git_branch_name -> Nullable<Text>,
    }
}

diesel::table! {
    code_pane_tabs (id) {
        id -> Integer,
        code_pane_id -> Integer,
        tab_index -> Integer,
        local_path -> Nullable<Binary>,
    }
}

diesel::table! {
    code_panes (id) {
        id -> Integer,
        active_tab_index -> Integer,
        source_data -> Nullable<Text>,
    }
}

diesel::table! {
    code_review_panes (id) {
        id -> Integer,
        kind -> Text,
        terminal_uuid -> Binary,
        repo_path -> Text,
    }
}

diesel::table! {
    commands (id) {
        id -> Integer,
        command -> Text,
        exit_code -> Nullable<Integer>,
        start_ts -> Nullable<Timestamp>,
        completed_ts -> Nullable<Timestamp>,
        pwd -> Nullable<Text>,
        shell -> Nullable<Text>,
        username -> Nullable<Text>,
        hostname -> Nullable<Text>,
        session_id -> Nullable<BigInt>,
        git_branch -> Nullable<Text>,
        cloud_workflow_id -> Nullable<Text>,
        workflow_command -> Nullable<Text>,
        is_agent_executed -> Nullable<Bool>,
    }
}

diesel::table! {
    env_var_collection_panes (id) {
        id -> Integer,
        kind -> Text,
        env_var_collection_id -> Nullable<Text>,
    }
}

diesel::table! {
    folders (id) {
        id -> Integer,
        name -> Text,
        is_open -> Bool,
        is_warp_pack -> Bool,
    }
}

diesel::table! {
    ignored_suggestions (id) {
        id -> Integer,
        suggestion -> Text,
        suggestion_type -> Text,
    }
}

diesel::table! {
    mcp_environment_variables (mcp_server_uuid) {
        mcp_server_uuid -> Binary,
        environment_variables -> Text,
    }
}

diesel::table! {
    mcp_server_installations (id) {
        id -> Text,
        templatable_mcp_server -> Text,
        template_version_ts -> Timestamp,
        variable_values -> Text,
        restore_running -> Bool,
        last_modified_at -> Timestamp,
    }
}

diesel::table! {
    mcp_server_panes (id) {
        id -> Integer,
        kind -> Text,
    }
}

diesel::table! {
    notebook_panes (id) {
        id -> Integer,
        kind -> Text,
        notebook_id -> Nullable<Text>,
        local_path -> Nullable<Binary>,
    }
}

diesel::table! {
    notebooks (id) {
        id -> Integer,
        title -> Nullable<Text>,
        data -> Nullable<Text>,
        ai_document_id -> Nullable<Text>,
    }
}

diesel::table! {
    pane_branches (id) {
        id -> Integer,
        pane_node_id -> Integer,
        horizontal -> Bool,
    }
}

diesel::table! {
    pane_leaves (pane_node_id, kind) {
        pane_node_id -> Integer,
        kind -> Text,
        is_focused -> Bool,
        custom_vertical_tabs_title -> Nullable<Text>,
    }
}

diesel::table! {
    pane_nodes (id) {
        id -> Integer,
        tab_id -> Integer,
        parent_pane_node_id -> Nullable<Integer>,
        flex -> Nullable<Float>,
        is_leaf -> Bool,
    }
}

diesel::table! {
    panels (id) {
        id -> Integer,
        tab_id -> Integer,
        left_panel -> Nullable<Text>,
        right_panel -> Nullable<Text>,
    }
}

diesel::table! {
    project_rules (id) {
        id -> Integer,
        path -> Text,
        project_root -> Text,
    }
}

diesel::table! {
    projects (path) {
        path -> Text,
        added_ts -> Timestamp,
        last_opened_ts -> Nullable<Timestamp>,
    }
}

diesel::table! {
    server_experiments (experiment) {
        experiment -> Text,
    }
}

diesel::table! {
    settings_panes (id) {
        id -> Integer,
        kind -> Text,
        current_page -> Text,
    }
}

diesel::table! {
    tabs (id) {
        id -> Integer,
        window_id -> Integer,
        custom_title -> Nullable<Text>,
        color -> Nullable<Text>,
    }
}

diesel::table! {
    terminal_panes (id) {
        id -> Integer,
        kind -> Text,
        uuid -> Binary,
        cwd -> Nullable<Text>,
        is_active -> Bool,
        shell_launch_data -> Nullable<Text>,
        input_config -> Nullable<Text>,
        llm_model_override -> Nullable<Text>,
        active_profile_id -> Nullable<Text>,
        conversation_ids -> Nullable<Text>,
        active_conversation_id -> Nullable<Text>,
    }
}

diesel::table! {
    welcome_panes (id) {
        id -> Integer,
        kind -> Text,
        startup_directory -> Nullable<Text>,
    }
}

diesel::table! {
    windows (id) {
        id -> Integer,
        active_tab_index -> Integer,
        window_width -> Nullable<Float>,
        window_height -> Nullable<Float>,
        origin_x -> Nullable<Float>,
        origin_y -> Nullable<Float>,
        quake_mode -> Bool,
        universal_search_width -> Nullable<Float>,
        warp_ai_width -> Nullable<Float>,
        voltron_width -> Nullable<Float>,
        fullscreen_state -> Integer,
        agent_management_filters -> Nullable<Text>,
        left_panel_open -> Nullable<Bool>,
        vertical_tabs_panel_open -> Nullable<Bool>,
    }
}

diesel::table! {
    workflow_panes (id) {
        id -> Integer,
        kind -> Text,
        workflow_id -> Nullable<Text>,
    }
}

diesel::table! {
    workflows (id) {
        id -> Integer,
        data -> Text,
    }
}

diesel::table! {
    workspace_language_server (id) {
        id -> Integer,
        workspace_id -> Integer,
        language_server_name -> Text,
        enabled -> Text,
    }
}

diesel::table! {
    workspace_metadata (id) {
        id -> Integer,
        repo_path -> Text,
        navigated_ts -> Nullable<Timestamp>,
        modified_ts -> Nullable<Timestamp>,
        queried_ts -> Nullable<Timestamp>,
    }
}

diesel::joinable!(app -> windows (active_window_id));
diesel::joinable!(code_pane_tabs -> code_panes (code_pane_id));
diesel::joinable!(pane_branches -> pane_nodes (pane_node_id));
diesel::joinable!(pane_leaves -> pane_nodes (pane_node_id));
diesel::joinable!(pane_nodes -> tabs (tab_id));
diesel::joinable!(panels -> tabs (tab_id));
diesel::joinable!(tabs -> windows (window_id));
diesel::joinable!(workspace_language_server -> workspace_metadata (workspace_id));

diesel::allow_tables_to_appear_in_same_query!(
    app,
    pane_branches,
    pane_leaves,
    pane_nodes,
    panels,
    tabs,
    windows,
);
diesel::allow_tables_to_appear_in_same_query!(code_pane_tabs, code_panes,);
diesel::allow_tables_to_appear_in_same_query!(workspace_language_server, workspace_metadata,);
