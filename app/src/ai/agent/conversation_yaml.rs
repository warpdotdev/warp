/// Walks a conversation's task tree and produces a hierarchical directory of YAML files
/// suitable for grep-based conversation search.
#[cfg(test)]
#[path = "conversation_yaml_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use warp_multi_agent_api as api;

use api::message::tool_call::Tool;
use api::message::tool_call_result::Result as ToolCallResultType;
use api::message::Message;

use super::task::helper::{SubagentExt, ToolExt};

const BASE_DIR_NAME: &str = "warp_conversation_search";

/// Returns the base directory for conversation search temp files.
///
/// Uses the platform temp directory so paths are fully qualified with
/// native separators on every OS (e.g. includes drive prefix on Windows).
pub(crate) fn base_dir() -> PathBuf {
    std::env::temp_dir().join(BASE_DIR_NAME)
}

/// Materializes a conversation's tasks into a directory of YAML files.
///
/// Returns the path to the root directory, or an error string.
pub fn materialize_tasks_to_yaml(tasks: &[api::Task]) -> Result<String, String> {
    let base_dir = base_dir();
    fs::create_dir_all(&base_dir).map_err(|e| format!("Failed to create base dir: {e}"))?;

    let dir = tempfile::tempdir_in(&base_dir)
        .map_err(|e| format!("Failed to create temp dir: {e}"))?
        .keep();

    // Build task lookup by id.
    let task_map: HashMap<&str, &api::Task> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();

    // Find root task (no parent or empty parent).
    let root = tasks
        .iter()
        .find(|t| {
            t.dependencies
                .as_ref()
                .is_none_or(|d| d.parent_task_id.is_empty())
        })
        .ok_or_else(|| "No root task found".to_string())?;

    let mut index: u32 = 0;
    if let Err(e) = write_task_messages(root, &dir, &mut index, &task_map) {
        let _ = fs::remove_dir_all(&dir);
        return Err(e);
    }

    Ok(dir.to_string_lossy().into_owned())
}

/// Writes YAML files for each message in a task, recursing into subtasks.
fn write_task_messages(
    task: &api::Task,
    dir: &Path,
    index: &mut u32,
    task_map: &HashMap<&str, &api::Task>,
) -> Result<(), String> {
    // Build a tool_call_id → tool name map for resolving ToolCallResult names.
    let tool_call_names: HashMap<&str, &'static str> = task
        .messages
        .iter()
        .filter_map(|m| {
            if let Some(Message::ToolCall(tc)) = &m.message {
                let name = tc.tool.as_ref().map(|t| t.name()).unwrap_or("unknown");
                Some((tc.tool_call_id.as_str(), name))
            } else {
                None
            }
        })
        .collect();

    for msg in &task.messages {
        let msg_id = &msg.id;
        let Some(message) = &msg.message else {
            continue;
        };

        match message {
            Message::UserQuery(uq) => {
                let filename = format!("{:03}.{msg_id}.user_query.yaml", *index);
                let mut content = String::new();
                content.push_str("type: user_query\n");
                content.push_str("query: |\n");
                write_block_scalar(&mut content, &uq.query);
                if let Some(ctx) = &uq.context {
                    if let Some(dir_ctx) = &ctx.directory {
                        if !dir_ctx.pwd.is_empty() {
                            content.push_str(&format!("working_directory: {}\n", dir_ctx.pwd));
                        }
                    }
                }
                write_yaml_file(dir, &filename, &content)?;
                *index += 1;
            }
            Message::AgentOutput(ao) => {
                let filename = format!("{:03}.{msg_id}.agent_output.yaml", *index);
                let mut content = String::new();
                content.push_str("type: agent_output\n");
                content.push_str("text: |\n");
                write_block_scalar(&mut content, &ao.text);
                write_yaml_file(dir, &filename, &content)?;
                *index += 1;
            }
            Message::AgentReasoning(ar) => {
                let filename = format!("{:03}.{msg_id}.agent_reasoning.yaml", *index);
                let mut content = String::new();
                content.push_str("type: agent_reasoning\n");
                content.push_str("reasoning: |\n");
                write_block_scalar(&mut content, &ar.reasoning);
                write_yaml_file(dir, &filename, &content)?;
                *index += 1;
            }
            Message::Summarization(s) => {
                if let Some(api::message::summarization::SummaryType::ConversationSummary(cs)) =
                    &s.summary_type
                {
                    let filename = format!("{:03}.{msg_id}.summarization.yaml", *index);
                    let mut content = String::new();
                    content.push_str("type: summarization\n");
                    content.push_str("summary: |\n");
                    write_block_scalar(&mut content, &cs.summary);
                    write_yaml_file(dir, &filename, &content)?;
                    *index += 1;
                }
            }
            Message::ToolCall(tc) => {
                let tool_call_id = &tc.tool_call_id;
                match &tc.tool {
                    Some(Tool::Server(_)) => {
                        // Skip opaque server tool calls.
                    }
                    Some(Tool::Subagent(sub)) => {
                        let subtask_id = &sub.task_id;
                        let subagent_type = sub.type_name();
                        let subagent_index = *index;
                        let filename = format!(
                            "{:03}.{msg_id}.subagent.{tool_call_id}.{subtask_id}.{subagent_type}.yaml",
                            subagent_index
                        );
                        let mut content = String::new();
                        content.push_str("type: subagent\n");
                        content.push_str(&format!("tool_call_id: {tool_call_id}\n"));
                        content.push_str(&format!("task_id: {subtask_id}\n"));
                        content.push_str(&format!("subagent_type: {subagent_type}\n"));
                        if !sub.payload.is_empty() {
                            content.push_str("payload: |\n");
                            write_block_scalar(&mut content, &sub.payload);
                        }
                        write_yaml_file(dir, &filename, &content)?;
                        *index += 1;

                        // Recurse into the subtask directory.
                        if let Some(subtask) = task_map.get(subtask_id.as_str()) {
                            let subdir_name = format!("{:03}.{subtask_id}", subagent_index);
                            let subdir = dir.join(&subdir_name);
                            fs::create_dir_all(&subdir).map_err(|e| {
                                format!("Failed to create subdir {subdir_name}: {e}")
                            })?;
                            let mut sub_index: u32 = 0;
                            write_task_messages(subtask, &subdir, &mut sub_index, task_map)?;
                        }
                    }
                    Some(tool) => {
                        let name = tool.name();
                        let filename = format!(
                            "{:03}.{msg_id}.tool_call.{tool_call_id}.{name}.yaml",
                            *index
                        );
                        let mut content = String::new();
                        content.push_str("type: tool_call\n");
                        content.push_str(&format!("tool_name: {name}\n"));
                        content.push_str(&format!("tool_call_id: {tool_call_id}\n"));
                        write_tool_call_args(&mut content, tool);
                        write_yaml_file(dir, &filename, &content)?;
                        *index += 1;
                    }
                    None => {}
                }
            }
            Message::ToolCallResult(tcr) => {
                let tool_call_id = &tcr.tool_call_id;
                match &tcr.result {
                    Some(ToolCallResultType::Server(_)) => {
                        // Skip opaque server results.
                    }
                    Some(ToolCallResultType::Subagent(_)) => {
                        let subtask_id = find_subtask_id_for_tool_call(task, tool_call_id);
                        let tid = subtask_id.as_deref().unwrap_or("unknown");
                        let filename = format!(
                            "{:03}.{msg_id}.subagent_result.{tool_call_id}.{tid}.yaml",
                            *index
                        );
                        let mut content = String::new();
                        content.push_str("type: subagent_result\n");
                        content.push_str(&format!("tool_call_id: {tool_call_id}\n"));
                        content.push_str(&format!("task_id: {tid}\n"));
                        write_yaml_file(dir, &filename, &content)?;
                        *index += 1;
                    }
                    Some(result) => {
                        let name = tool_call_names
                            .get(tool_call_id.as_str())
                            .copied()
                            .unwrap_or("unknown");
                        let filename = format!(
                            "{:03}.{msg_id}.tool_call_result.{tool_call_id}.{name}.yaml",
                            *index
                        );
                        let mut content = String::new();
                        content.push_str("type: tool_call_result\n");
                        content.push_str(&format!("tool_name: {name}\n"));
                        content.push_str(&format!("tool_call_id: {tool_call_id}\n"));
                        write_tool_call_result_content(&mut content, result);
                        write_yaml_file(dir, &filename, &content)?;
                        *index += 1;
                    }
                    None => {}
                }
            }
            // No searchable content.
            Message::WebSearch(_)
            | Message::WebFetch(_)
            | Message::ModelUsed(_)
            | Message::UpdateTodos(_)
            | Message::UpdateReviewComments(_)
            | Message::DebugOutput(_)
            | Message::ArtifactEvent(_)
            | Message::MessagesReceivedFromAgents(_)
            | Message::EventsFromAgents(_)
            | Message::PassiveSuggestionResult(_)
            | Message::SystemQuery(_)
            | Message::CodeReview(_)
            | Message::ServerEvent(_)
            | Message::InvokeSkill(_)
            | Message::OrchestrationConfigSnapshot(_) => {}
        }
    }
    Ok(())
}

fn write_yaml_file(dir: &Path, filename: &str, content: &str) -> Result<(), String> {
    let path = dir.join(filename);
    let mut file =
        fs::File::create(&path).map_err(|e| format!("Failed to create {}: {e}", path.display()))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(())
}

/// Writes a YAML block scalar (|) with each line indented by 2 spaces.
fn write_block_scalar(out: &mut String, text: &str) {
    write_block_scalar_with_indent(out, text, 2);
}

fn write_block_scalar_with_indent(out: &mut String, text: &str, indent: usize) {
    let indent = " ".repeat(indent);
    for line in text.lines() {
        out.push_str(&indent);
        out.push_str(line);
        out.push('\n');
    }
    // Ensure trailing newline for empty text.
    if text.is_empty() {
        out.push_str(&indent);
        out.push('\n');
    }
}

/// Writes key arguments from structured tool calls into the YAML content.
fn write_tool_call_args(out: &mut String, tool: &Tool) {
    match tool {
        Tool::RunShellCommand(cmd) => {
            out.push_str("command: |\n");
            write_block_scalar(out, &cmd.command);
        }
        Tool::SearchCodebase(sc) => {
            out.push_str(&format!("query: \"{}\"\n", escape_yaml_string(&sc.query)));
            if !sc.codebase_path.is_empty() {
                out.push_str(&format!("codebase_path: {}\n", sc.codebase_path));
            }
            if !sc.path_filters.is_empty() {
                out.push_str("path_filters:\n");
                for p in &sc.path_filters {
                    out.push_str(&format!("  - \"{}\"\n", escape_yaml_string(p)));
                }
            }
        }
        Tool::ReadFiles(rf) => {
            out.push_str("files:\n");
            for f in &rf.files {
                out.push_str(&format!("  - name: {}\n", f.name));
                if !f.line_ranges.is_empty() {
                    out.push_str("    ranges:\n");
                    for r in &f.line_ranges {
                        out.push_str(&format!("      - {}-{}\n", r.start, r.end));
                    }
                }
            }
        }
        Tool::UploadFileArtifact(upload) => {
            if let Some(file) = &upload.file {
                out.push_str("file:\n");
                out.push_str(&format!("  file_path: {}\n", file.file_path));
            }
            if !upload.description.is_empty() {
                out.push_str(&format!(
                    "description: \"{}\"\n",
                    escape_yaml_string(&upload.description)
                ));
            }
        }
        Tool::Grep(g) => {
            out.push_str("queries:\n");
            for q in &g.queries {
                out.push_str(&format!("  - \"{}\"\n", escape_yaml_string(q)));
            }
            out.push_str(&format!("path: {}\n", g.path));
        }
        Tool::FileGlobV2(fg) => {
            out.push_str("patterns:\n");
            for p in &fg.patterns {
                out.push_str(&format!("  - \"{}\"\n", escape_yaml_string(p)));
            }
            if !fg.search_dir.is_empty() {
                out.push_str(&format!("search_dir: {}\n", fg.search_dir));
            }
        }
        Tool::CallMcpTool(mcp) => {
            out.push_str(&format!("name: {}\n", mcp.name));
            if let Some(args) = &mcp.args {
                out.push_str("args: |\n");
                let json =
                    serde_json::to_string_pretty(&prost_struct_to_json(args)).unwrap_or_default();
                write_block_scalar(out, &json);
            }
        }
        Tool::ReadMcpResource(r) => {
            out.push_str(&format!("uri: {}\n", r.uri));
        }
        Tool::FetchConversation(fc) => {
            out.push_str(&format!("conversation_id: {}\n", fc.conversation_id));
        }
        Tool::CreateDocuments(cd) => {
            out.push_str("documents:\n");
            for doc in &cd.new_documents {
                out.push_str(&format!(
                    "  - title: \"{}\"\n",
                    escape_yaml_string(&doc.title)
                ));
                let preview = truncate_content(&doc.content, 2048);
                if !preview.is_empty() {
                    out.push_str("    content: |\n");
                    for line in preview.lines() {
                        out.push_str("      ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
        }
        Tool::ReadDocuments(rd) => {
            out.push_str("documents:\n");
            for doc in &rd.documents {
                out.push_str(&format!("  - document_id: {}\n", doc.document_id));
            }
        }
        Tool::EditDocuments(ed) => {
            out.push_str(&format!("diff_count: {}\n", ed.diffs.len()));
            for diff in &ed.diffs {
                out.push_str(&format!("  - document_id: {}\n", diff.document_id));
                if !diff.search.is_empty() {
                    out.push_str("    search: |\n");
                    write_block_scalar(out, truncate_content(&diff.search, 1024));
                }
                if !diff.replace.is_empty() {
                    out.push_str("    replace: |\n");
                    write_block_scalar(out, truncate_content(&diff.replace, 1024));
                }
            }
        }
        Tool::StartAgent(sa) => {
            out.push_str(&format!("name: \"{}\"\n", escape_yaml_string(&sa.name)));
            out.push_str("prompt: |\n");
            write_block_scalar(out, &sa.prompt);
        }
        Tool::StartAgentV2(sa) => {
            out.push_str(&format!("name: \"{}\"\n", escape_yaml_string(&sa.name)));
            out.push_str("prompt: |\n");
            write_block_scalar(out, &sa.prompt);
        }
        #[allow(deprecated)]
        Tool::FileGlob(fg) => {
            out.push_str("patterns:\n");
            for p in &fg.patterns {
                out.push_str(&format!("  - \"{}\"\n", escape_yaml_string(p)));
            }
            if !fg.path.is_empty() {
                out.push_str(&format!("search_dir: {}\n", fg.path));
            }
        }
        Tool::WriteToLongRunningShellCommand(w) => {
            out.push_str(&format!("command_id: {}\n", w.command_id));
            if let Ok(text) = std::str::from_utf8(&w.input) {
                out.push_str("input: |\n");
                write_block_scalar(out, truncate_content(text, 1024));
            }
        }
        Tool::InsertReviewComments(i) => {
            out.push_str(&format!("repo_path: {}\n", i.repo_path));
            if !i.base_branch.is_empty() {
                out.push_str(&format!("base_branch: {}\n", i.base_branch));
            }
            out.push_str("comments:\n");
            for c in &i.comments {
                if let Some(loc) = &c.location {
                    out.push_str(&format!("  - file: {}\n", loc.file_path));
                } else {
                    out.push_str("  - file: <pr-level>\n");
                }
                out.push_str("    body: |\n");
                write_block_scalar(out, truncate_content(&c.comment_body, 512));
            }
        }
        Tool::ReadSkill(rs) => {
            use api::message::tool_call::read_skill::SkillReference;
            match &rs.skill_reference {
                Some(SkillReference::SkillPath(path)) => {
                    out.push_str(&format!("skill_path: {}\n", path));
                }
                Some(SkillReference::BundledSkillId(id)) => {
                    out.push_str(&format!("bundled_skill_id: {}\n", id));
                }
                None => {}
            }
        }
        Tool::SendMessageToAgent(s) => {
            out.push_str(&format!(
                "subject: \"{}\"\n",
                escape_yaml_string(&s.subject)
            ));
            out.push_str("message: |\n");
            write_block_scalar(out, truncate_content(&s.message, 2048));
        }
        Tool::AskUserQuestion(ask) => {
            if ask.questions.is_empty() {
                out.push_str("questions: []\n");
                return;
            }
            out.push_str("questions:\n");
            for question in &ask.questions {
                out.push_str(&format!(
                    "  - question_id: \"{}\"\n",
                    escape_yaml_string(&question.question_id)
                ));
                out.push_str("    question: |\n");
                write_block_scalar_with_indent(out, &question.question, 6);
                match &question.question_type {
                    Some(api::ask_user_question::question::QuestionType::MultipleChoice(mc)) => {
                        out.push_str("    question_type: multiple_choice\n");
                        out.push_str(&format!("    is_multiselect: {}\n", mc.is_multiselect));
                        out.push_str(&format!("    supports_other: {}\n", mc.supports_other));
                        if mc.options.is_empty() {
                            out.push_str("    options: []\n");
                        } else {
                            out.push_str("    options:\n");
                            for option in &mc.options {
                                out.push_str(&format!(
                                    "      - label: \"{}\"\n",
                                    escape_yaml_string(&option.label)
                                ));
                            }
                        }
                    }
                    None => {
                        out.push_str("    question_type: missing\n");
                    }
                }
            }
        }
        Tool::ApplyFileDiffs(afd) => {
            out.push_str(&format!(
                "summary: \"{}\"\n",
                escape_yaml_string(&afd.summary)
            ));
            out.push_str("files:\n");
            for d in &afd.diffs {
                out.push_str(&format!("  - {}\n", d.file_path));
            }
            for d in &afd.v4a_updates {
                out.push_str(&format!("  - {}\n", d.file_path));
            }
            for nf in &afd.new_files {
                out.push_str(&format!("  - new: {}\n", nf.file_path));
            }
            for df in &afd.deleted_files {
                out.push_str(&format!("  - deleted: {}\n", df.file_path));
            }
        }
        Tool::RunAgents(o) => {
            out.push_str(&format!(
                "summary: \"{}\"\n",
                escape_yaml_string(&o.summary)
            ));
            out.push_str("agents:\n");
            for cfg in &o.agent_run_configs {
                out.push_str(&format!(
                    "  - name: \"{}\"\n",
                    escape_yaml_string(&cfg.name)
                ));
            }
        }
        // No additional args worth serializing.
        Tool::ReadShellCommandOutput(_)
        | Tool::UseComputer(_)
        | Tool::RequestComputerUse(_)
        | Tool::SuggestPlan(_)
        | Tool::SuggestCreatePlan(_)
        | Tool::SuggestNewConversation(_)
        | Tool::SuggestPrompt(_)
        | Tool::OpenCodeReview(_)
        | Tool::InitProject(_)
        | Tool::Server(_)
        | Tool::Subagent(_)
        | Tool::TransferShellCommandControlToUser(_) => {}
    }
}

/// Writes content from structured tool call results.
fn write_tool_call_result_content(out: &mut String, result: &ToolCallResultType) {
    match result {
        ToolCallResultType::RunAgentsResult(r) => match &r.outcome {
            Some(api::run_agents_result::Outcome::Launched(launched)) => {
                out.push_str("status: launched\n");
                out.push_str(&format!("agent_count: {}\n", launched.agents.len()));
            }
            Some(api::run_agents_result::Outcome::Denied(denied)) => {
                out.push_str("status: launch_denied\n");
                out.push_str(&format!(
                    "reason: \"{}\"\n",
                    escape_yaml_string(&denied.reason)
                ));
            }
            Some(api::run_agents_result::Outcome::Failure(failure)) => {
                out.push_str("status: failure\n");
                out.push_str(&format!(
                    "error: \"{}\"\n",
                    escape_yaml_string(&failure.error)
                ));
            }
            None => {}
        },
        ToolCallResultType::StartAgentV2(r) => match &r.result {
            Some(api::start_agent_v2_result::Result::Success(s)) => {
                out.push_str(&format!("agent_id: {}\n", s.agent_id));
            }
            Some(api::start_agent_v2_result::Result::Error(e)) => {
                out.push_str(&format!("error: {}\n", e.error));
            }
            None => {}
        },
        ToolCallResultType::RunShellCommand(r) => {
            if let Some(res) = &r.result {
                use api::run_shell_command_result::Result;
                match res {
                    Result::CommandFinished(c) => {
                        out.push_str(&format!("exit_code: {}\n", c.exit_code));
                        out.push_str("output: |\n");
                        write_block_scalar(out, &c.output);
                    }
                    Result::LongRunningCommandSnapshot(s) => {
                        out.push_str("status: long_running\n");
                        out.push_str("output: |\n");
                        write_block_scalar(out, &s.output);
                    }
                    Result::PermissionDenied(_) => {
                        out.push_str("status: permission_denied\n");
                    }
                }
            }
        }
        ToolCallResultType::SearchCodebase(r) => {
            if let Some(res) = &r.result {
                use api::search_codebase_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("files:\n");
                        for f in &s.files {
                            out.push_str(&format!("  - name: {}\n", f.file_path));
                            let preview = truncate_content(&f.content, 2048);
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::ReadFiles(r) => {
            if let Some(res) = &r.result {
                use api::read_files_result::Result;
                match res {
                    Result::TextFilesSuccess(s) => {
                        out.push_str("files:\n");
                        for f in &s.files {
                            out.push_str(&format!("  - name: {}\n", f.file_path));
                            let preview = truncate_content(&f.content, 4096);
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::AnyFilesSuccess(s) => {
                        out.push_str("files:\n");
                        for f in &s.files {
                            let (path, preview) = any_file_content_summary(f);
                            out.push_str(&format!("  - name: {}\n", path));
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::UploadFileArtifact(r) => {
            if let Some(res) = &r.result {
                use api::upload_file_artifact_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str(&format!("artifact_uid: {}\n", s.artifact_uid));
                        out.push_str(&format!("mime_type: {}\n", s.mime_type));
                        out.push_str(&format!("size_bytes: {}\n", s.size_bytes));
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::Grep(r) => {
            if let Some(res) = &r.result {
                use api::grep_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("matched_files:\n");
                        for f in &s.matched_files {
                            out.push_str(&format!("  - file: {}\n", f.file_path));
                            if !f.matched_lines.is_empty() {
                                out.push_str("    lines:\n");
                                for line in &f.matched_lines {
                                    out.push_str(&format!("      - {}\n", line.line_number));
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::ApplyFileDiffs(r) => {
            if let Some(res) = &r.result {
                use api::apply_file_diffs_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("status: success\n");
                        out.push_str("files:\n");
                        for uf in &s.updated_files_v2 {
                            if let Some(f) = &uf.file {
                                out.push_str(&format!("  - {}\n", f.file_path));
                            }
                        }
                        for df in &s.deleted_files {
                            out.push_str(&format!("  - deleted: {}\n", df.file_path));
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("is_error: true\nerror: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::FetchConversation(r) => {
            if let Some(res) = &r.result {
                use api::fetch_conversation_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str(&format!("directory_path: {}\n", s.directory_path));
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        #[allow(deprecated)]
        ToolCallResultType::FileGlob(r) => {
            if let Some(res) = &r.result {
                use api::file_glob_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("matched_files: |\n");
                        write_block_scalar(out, truncate_content(&s.matched_files, 4096));
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::FileGlobV2(r) => {
            if let Some(res) = &r.result {
                use api::file_glob_v2_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("matched_files:\n");
                        for f in &s.matched_files {
                            out.push_str(&format!("  - {}\n", f.file_path));
                        }
                        if !s.warnings.is_empty() {
                            out.push_str(&format!(
                                "warnings: \"{}\"\n",
                                escape_yaml_string(&s.warnings)
                            ));
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::WriteToLongRunningShellCommand(r) => {
            if let Some(res) = &r.result {
                use api::write_to_long_running_shell_command_result::Result;
                match res {
                    Result::LongRunningCommandSnapshot(s) => {
                        out.push_str("status: long_running\n");
                        out.push_str("output: |\n");
                        write_block_scalar(out, truncate_content(&s.output, 4096));
                    }
                    Result::CommandFinished(c) => {
                        out.push_str(&format!("exit_code: {}\n", c.exit_code));
                        out.push_str("output: |\n");
                        write_block_scalar(out, truncate_content(&c.output, 4096));
                    }
                    Result::Error(_) => {
                        out.push_str("status: error\n");
                    }
                }
            }
        }
        ToolCallResultType::ReadShellCommandOutput(r) => {
            if let Some(res) = &r.result {
                use api::read_shell_command_output_result::Result;
                match res {
                    Result::CommandFinished(c) => {
                        out.push_str(&format!("exit_code: {}\n", c.exit_code));
                        out.push_str("output: |\n");
                        write_block_scalar(out, truncate_content(&c.output, 4096));
                    }
                    Result::LongRunningCommandSnapshot(s) => {
                        out.push_str("status: long_running\n");
                        out.push_str("output: |\n");
                        write_block_scalar(out, truncate_content(&s.output, 4096));
                    }
                    Result::Error(_) => {
                        out.push_str("status: error\n");
                    }
                }
            }
        }
        ToolCallResultType::CallMcpTool(r) => {
            if let Some(res) = &r.result {
                use api::call_mcp_tool_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("results:\n");
                        for item in &s.results {
                            use api::call_mcp_tool_result::success::result::Result as ItemResult;
                            match &item.result {
                                Some(ItemResult::Text(t)) => {
                                    out.push_str("  - type: text\n");
                                    out.push_str("    content: |\n");
                                    for line in truncate_content(&t.text, 4096).lines() {
                                        out.push_str("      ");
                                        out.push_str(line);
                                        out.push('\n');
                                    }
                                }
                                Some(ItemResult::Image(_)) => {
                                    out.push_str("  - type: image\n");
                                }
                                Some(ItemResult::Resource(r)) => {
                                    out.push_str(&format!(
                                        "  - type: resource\n    uri: {}\n",
                                        r.uri
                                    ));
                                }
                                None => {}
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::ReadMcpResource(r) => {
            if let Some(res) = &r.result {
                use api::read_mcp_resource_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("contents:\n");
                        for content in &s.contents {
                            out.push_str(&format!("  - uri: {}\n", content.uri));
                            if let Some(api::mcp_resource_content::ContentType::Text(t)) =
                                &content.content_type
                            {
                                out.push_str("    text: |\n");
                                for line in truncate_content(&t.content, 4096).lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::ReadSkill(r) => {
            if let Some(res) = &r.result {
                use api::read_skill_result::Result;
                match res {
                    Result::Success(s) => {
                        if let Some(content) = &s.content {
                            out.push_str(&format!("file_path: {}\n", content.file_path));
                            out.push_str("content: |\n");
                            write_block_scalar(out, truncate_content(&content.content, 4096));
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::ReadDocuments(r) => {
            if let Some(res) = &r.result {
                use api::read_documents_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("documents:\n");
                        for doc in &s.documents {
                            out.push_str(&format!("  - document_id: {}\n", doc.document_id));
                            let preview = truncate_content(&doc.content, 4096);
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::EditDocuments(r) => {
            if let Some(res) = &r.result {
                use api::edit_documents_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("updated_documents:\n");
                        for doc in &s.updated_documents {
                            out.push_str(&format!("  - document_id: {}\n", doc.document_id));
                            let preview = truncate_content(&doc.content, 4096);
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::CreateDocuments(r) => {
            if let Some(res) = &r.result {
                use api::create_documents_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str("created_documents:\n");
                        for doc in &s.created_documents {
                            out.push_str(&format!("  - document_id: {}\n", doc.document_id));
                            let preview = truncate_content(&doc.content, 4096);
                            if !preview.is_empty() {
                                out.push_str("    content: |\n");
                                for line in preview.lines() {
                                    out.push_str("      ");
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            }
                        }
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::StartAgent(r) => {
            if let Some(res) = &r.result {
                use api::start_agent_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str(&format!("agent_id: {}\n", s.agent_id));
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.error));
                    }
                }
            }
        }
        ToolCallResultType::SendMessageToAgent(r) => {
            if let Some(res) = &r.result {
                use api::send_message_to_agent_result::Result;
                match res {
                    Result::Success(s) => {
                        out.push_str(&format!("message_id: {}\n", s.message_id));
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::InsertReviewComments(r) => {
            if let Some(res) = &r.result {
                use api::insert_review_comments_result::Result;
                match res {
                    Result::Success(_) => {
                        out.push_str("status: success\n");
                    }
                    Result::Error(e) => {
                        out.push_str(&format!("error: {}\n", e.message));
                    }
                }
            }
        }
        ToolCallResultType::AskUserQuestion(r) => {
            use api::ask_user_question_result::Result;
            match &r.result {
                Some(Result::Success(success)) => {
                    out.push_str("status: completed\n");
                    if success.answers.is_empty() {
                        out.push_str("answers: []\n");
                    } else {
                        out.push_str("answers:\n");
                        for answer in &success.answers {
                            out.push_str(&format!(
                                "  - question_id: \"{}\"\n",
                                escape_yaml_string(&answer.question_id)
                            ));
                            match &answer.answer {
                                Some(
                                    api::ask_user_question_result::answer_item::Answer::MultipleChoice(
                                        multiple_choice,
                                    ),
                                ) => {
                                    out.push_str("    answer_type: multiple_choice\n");
                                    if multiple_choice.selected_options.is_empty() {
                                        out.push_str("    selected_options: []\n");
                                    } else {
                                        out.push_str("    selected_options:\n");
                                        for option in &multiple_choice.selected_options {
                                            out.push_str(&format!(
                                                "      - \"{}\"\n",
                                                escape_yaml_string(option)
                                            ));
                                        }
                                    }
                                    if !multiple_choice.other_text.is_empty() {
                                        out.push_str("    other_text: |\n");
                                        write_block_scalar_with_indent(
                                            out,
                                            &multiple_choice.other_text,
                                            6,
                                        );
                                    }
                                }
                                Some(api::ask_user_question_result::answer_item::Answer::Skipped(()))
                                | None => {
                                    out.push_str("    answer_type: skipped\n");
                                }
                            }
                        }
                    }
                }
                Some(Result::Error(error)) => {
                    out.push_str("status: error\n");
                    out.push_str("error: |\n");
                    write_block_scalar_with_indent(out, &error.message, 2);
                }
                None => {
                    out.push_str("status: cancelled\n");
                }
            }
        }
        ToolCallResultType::Cancel(_) => {
            out.push_str("status: cancelled\n");
        }
        // No structured content worth serializing.
        ToolCallResultType::Server(_)
        | ToolCallResultType::Subagent(_)
        | ToolCallResultType::UseComputer(_)
        | ToolCallResultType::RequestComputerUseResult(_)
        | ToolCallResultType::SuggestNewConversation(_)
        | ToolCallResultType::SuggestPrompt(_)
        | ToolCallResultType::OpenCodeReview(_)
        | ToolCallResultType::InitProject(_)
        | ToolCallResultType::TransferShellCommandControlToUser(_)
        | ToolCallResultType::SuggestCreatePlan(_)
        | ToolCallResultType::SuggestPlan(_) => {
            out.push_str("status: completed\n");
        }
    }
}

/// Looks up the subtask_id for a given tool_call_id by scanning the task's messages for a
/// matching Subagent tool call.
fn find_subtask_id_for_tool_call(task: &api::Task, tool_call_id: &str) -> Option<String> {
    task.messages.iter().find_map(|m| {
        if let Some(Message::ToolCall(tc)) = &m.message {
            if tc.tool_call_id == tool_call_id {
                if let Some(Tool::Subagent(sub)) = &tc.tool {
                    return Some(sub.task_id.clone());
                }
            }
        }
        None
    })
}

fn escape_yaml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn truncate_content(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find a safe UTF-8 boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Returns (file_path, truncated_content_preview) for an AnyFileContent.
fn any_file_content_summary(content: &api::AnyFileContent) -> (String, String) {
    match &content.content {
        Some(api::any_file_content::Content::TextContent(f)) => (
            f.file_path.clone(),
            truncate_content(&f.content, 4096).to_string(),
        ),
        Some(api::any_file_content::Content::BinaryContent(b)) => {
            (b.file_path.clone(), "<binary>".to_string())
        }
        None => (String::new(), String::new()),
    }
}

/// Converts a prost Struct to a serde_json Value for pretty-printing.
fn prost_struct_to_json(s: &prost_types::Struct) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = s
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), prost_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

fn prost_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::NumberValue(n)) => serde_json::json!(*n),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::StructValue(s)) => prost_struct_to_json(s),
        Some(Kind::ListValue(l)) => {
            serde_json::Value::Array(l.values.iter().map(prost_value_to_json).collect())
        }
        None => serde_json::Value::Null,
    }
}
