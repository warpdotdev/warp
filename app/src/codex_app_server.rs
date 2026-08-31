use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};
use codex_app_server::{
    AccountStatus, AccountType, ApprovalPolicy, Client, ClientOptions, LoginChallenge, LoginMode,
    SandboxMode, ServerRequest, ServerRequestResponse, ThreadOptions,
};
use serde_json::{Map, Value, json};
use warp_cli::codex::{CodexArgs, CodexCommand};

pub(crate) fn run(args: &CodexArgs) -> Result<()> {
    futures_lite::future::block_on(run_async(args))
}

async fn run_async(args: &CodexArgs) -> Result<()> {
    match &args.command {
        CodexCommand::Login { device_code } => login(args, *device_code).await,
        CodexCommand::Status { json, refresh } => status(args, *json, *refresh).await,
        CodexCommand::Logout => logout(args).await,
        CodexCommand::Chat {
            prompt,
            interactive,
            model,
            cwd,
            read_only,
            dangerously_bypass_approvals_and_sandbox,
        } => {
            chat(
                args,
                prompt.clone(),
                *interactive,
                model.clone(),
                cwd.clone(),
                *read_only,
                *dangerously_bypass_approvals_and_sandbox,
            )
            .await
        }
    }
}

fn client_options(args: &CodexArgs) -> ClientOptions {
    args.codex_path
        .clone()
        .map(ClientOptions::with_program)
        .unwrap_or_default()
}

async fn login(args: &CodexArgs, device_code: bool) -> Result<()> {
    let mut client = Client::spawn(client_options(args)).await?;
    let challenge = client
        .start_login(if device_code {
            LoginMode::DeviceCode
        } else {
            LoginMode::Browser
        })
        .await?;

    match &challenge {
        LoginChallenge::Browser { auth_url, .. } => {
            println!("Open this URL to sign in with ChatGPT:\n{auth_url}");
            if launch_browser(auth_url) {
                println!("Opened the ChatGPT sign-in page in your browser.");
            }
        }
        LoginChallenge::DeviceCode {
            verification_url,
            user_code,
            ..
        } => {
            println!("Open this URL:\n{verification_url}\n\nEnter code: {user_code}");
            let _ = launch_browser(verification_url);
        }
    }

    client.wait_for_login(challenge.login_id()).await?;
    let status = client.account(false).await?;
    println!("{}", connected_account_message(&status));
    Ok(())
}

async fn status(args: &CodexArgs, as_json: bool, refresh: bool) -> Result<()> {
    let mut client = Client::spawn(client_options(args)).await?;
    let status = client.account(refresh).await?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&account_status_json(&status))?
        );
    } else {
        println!("{}", connected_account_message(&status));
    }
    Ok(())
}

async fn logout(args: &CodexArgs) -> Result<()> {
    let mut client = Client::spawn(client_options(args)).await?;
    client.logout().await?;
    println!("Signed out of Codex.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn chat(
    args: &CodexArgs,
    initial_prompt: Option<String>,
    interactive_after_prompt: bool,
    model: Option<String>,
    cwd: Option<PathBuf>,
    read_only: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
) -> Result<()> {
    let mut client = Client::spawn(client_options(args)).await?;
    let account = client.account(false).await?;
    if account.account.is_none() {
        bail!("Codex is not signed in. Run `warp codex login` first");
    }

    let cwd = cwd
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .context("Could not determine the Codex working directory")?;
    let mut options = ThreadOptions::new(cwd);
    options.model = model;
    if read_only {
        options.sandbox = SandboxMode::ReadOnly;
    }
    if dangerously_bypass_approvals_and_sandbox {
        options.approval_policy = ApprovalPolicy::Never;
        options.sandbox = SandboxMode::DangerFullAccess;
    }

    let thread_id = client.start_thread(&options).await?;
    let should_be_interactive = initial_prompt.is_none() || interactive_after_prompt;
    let mut next_prompt = initial_prompt;

    loop {
        let prompt = match next_prompt.take() {
            Some(prompt) => prompt,
            None if should_be_interactive => match read_chat_prompt()? {
                Some(prompt) => prompt,
                None => break,
            },
            None => break,
        };

        let mut emitted_agent_text = false;
        let result = client
            .run_turn(
                &thread_id,
                &prompt,
                |notification| {
                    if let Some(delta) = notification.agent_message_delta() {
                        print!("{delta}");
                        let _ = io::stdout().flush();
                        emitted_agent_text = true;
                    } else if !emitted_agent_text
                        && let Some(message) = notification.completed_agent_message()
                    {
                        print!("{message}");
                        let _ = io::stdout().flush();
                        emitted_agent_text = true;
                    } else if let Some(delta) = notification.command_output_delta() {
                        eprint!("{delta}");
                        let _ = io::stderr().flush();
                    } else if let Some(delta) = notification.file_change_output_delta() {
                        eprint!("{delta}");
                        let _ = io::stderr().flush();
                    } else if let Some(warning) = notification.warning_message() {
                        eprintln!("[codex warning] {warning}");
                    } else if let Some(error) = notification.error_message() {
                        eprintln!("[codex error] {error}");
                    }
                },
                interactive_server_request,
            )
            .await?;
        if emitted_agent_text {
            println!();
        }
        if let Some(error) = result.error {
            bail!("Codex turn failed: {error}");
        }
        if result.status != "completed" {
            bail!("Codex turn ended with status {}", result.status);
        }

        if !should_be_interactive {
            break;
        }
    }

    Ok(())
}

fn read_chat_prompt() -> Result<Option<String>> {
    loop {
        print!("codex> ");
        io::stdout().flush()?;
        let mut prompt = String::new();
        if io::stdin().read_line(&mut prompt)? == 0 {
            println!();
            return Ok(None);
        }
        let prompt = prompt.trim_end();
        if matches!(prompt, "/exit" | "/quit") {
            return Ok(None);
        }
        if !prompt.is_empty() {
            return Ok(Some(prompt.to_owned()));
        }
    }
}

fn interactive_server_request(request: &ServerRequest) -> ServerRequestResponse {
    match request.method.as_str() {
        "item/commandExecution/requestApproval" => {
            let command = request
                .params
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<command unavailable>");
            let reason = request.params.get("reason").and_then(Value::as_str);
            let cwd = request.params.get("cwd").and_then(Value::as_str);
            let detail = format_approval_detail(command, reason, cwd);
            approval_response(&format!("Codex wants to run:\n{detail}"), false)
        }
        "item/fileChange/requestApproval" => {
            let reason = request
                .params
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Codex requested permission to change files");
            approval_response(reason, false)
        }
        "execCommandApproval" | "applyPatchApproval" => {
            let command = request
                .params
                .get("command")
                .or_else(|| request.params.get("fileChanges"))
                .map(Value::to_string)
                .unwrap_or_else(|| "Codex requested an action".to_owned());
            approval_response(&command, true)
        }
        "item/tool/requestUserInput" => answer_user_input(request),
        "item/permissions/requestApproval" => {
            eprintln!(
                "Codex requested additional permissions; Warp declined the broad permission grant."
            );
            ServerRequestResponse::result(json!({
                "permissions": { "fileSystem": null, "network": null },
                "scope": "turn",
            }))
        }
        "mcpServer/elicitation/request" => {
            eprintln!("Codex requested MCP input; Warp declined this unsupported form prompt.");
            ServerRequestResponse::result(json!({ "action": "decline" }))
        }
        "item/tool/call" => ServerRequestResponse::result(json!({
            "success": false,
            "contentItems": [{
                "type": "inputText",
                "text": "Warp has no matching dynamic tool implementation",
            }],
        })),
        method => ServerRequestResponse::method_not_found(method),
    }
}

fn format_approval_detail(command: &str, reason: Option<&str>, cwd: Option<&str>) -> String {
    let mut detail = command.to_owned();
    if let Some(cwd) = cwd {
        detail.push_str(&format!("\nDirectory: {cwd}"));
    }
    if let Some(reason) = reason {
        detail.push_str(&format!("\nReason: {reason}"));
    }
    detail
}

fn approval_response(message: &str, legacy: bool) -> ServerRequestResponse {
    eprintln!("\n{message}");
    eprint!("Allow? [y]es / [a]ll session / [N]o / [q]uit: ");
    let _ = io::stderr().flush();
    let mut response = String::new();
    let choice = io::stdin()
        .read_line(&mut response)
        .ok()
        .filter(|read| *read > 0)
        .map(|_| response.trim().to_ascii_lowercase())
        .unwrap_or_default();

    let decision = if legacy {
        match choice.as_str() {
            "y" | "yes" => Value::String("approved".to_owned()),
            "a" | "all" => Value::String("approved_for_session".to_owned()),
            "q" | "quit" | "cancel" => Value::String("abort".to_owned()),
            _ => json!({ "denied": { "rejection": "Denied by the user in Warp" } }),
        }
    } else {
        Value::String(
            match choice.as_str() {
                "y" | "yes" => "accept",
                "a" | "all" => "acceptForSession",
                "q" | "quit" | "cancel" => "cancel",
                _ => "decline",
            }
            .to_owned(),
        )
    };
    ServerRequestResponse::result(json!({ "decision": decision }))
}

fn answer_user_input(request: &ServerRequest) -> ServerRequestResponse {
    let mut answers = Map::new();
    let questions = request
        .params
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for question in questions {
        let Some(id) = question.get("id").and_then(Value::as_str) else {
            continue;
        };
        let prompt = question
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("Codex needs input");
        eprint!("\n{prompt}: ");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).unwrap_or(0) == 0 {
            continue;
        }
        answers.insert(
            id.to_owned(),
            json!({ "answers": [answer.trim_end().to_owned()] }),
        );
    }

    ServerRequestResponse::result(json!({ "answers": answers }))
}

pub(crate) fn launch_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let result = ProcessCommand::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = ProcessCommand::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = ProcessCommand::new("xdg-open").arg(url).spawn();

    result.is_ok()
}

pub(crate) async fn read_account(options: ClientOptions) -> Result<AccountStatus> {
    let mut client = Client::spawn(options).await?;
    client.account(false).await
}

pub(crate) async fn login_in_browser(options: ClientOptions) -> Result<AccountStatus> {
    let mut client = Client::spawn(options).await?;
    let challenge = client.start_login(LoginMode::Browser).await?;
    let LoginChallenge::Browser { login_id, auth_url } = challenge else {
        bail!("Codex returned an unexpected login challenge")
    };
    if !launch_browser(&auth_url) {
        bail!("Could not open the ChatGPT login page: {auth_url}");
    }
    client.wait_for_login(&login_id).await?;
    client.account(false).await
}

pub(crate) async fn disconnect(options: ClientOptions) -> Result<AccountStatus> {
    let mut client = Client::spawn(options).await?;
    client.logout().await?;
    client.account(false).await
}

pub(crate) fn connected_account_message(status: &AccountStatus) -> String {
    let Some(account) = &status.account else {
        return "Not signed in to Codex. Run `warp codex login`.".to_owned();
    };
    let provider = match &account.account_type {
        AccountType::ChatGpt => "ChatGPT",
        AccountType::ApiKey => "OpenAI API key",
        AccountType::AwsBedrock => "AWS Bedrock",
        AccountType::Other(kind) => kind,
    };
    let mut message = format!("Signed in to Codex with {provider}");
    if let Some(email) = &account.email {
        message.push_str(&format!(" as {email}"));
    }
    if let Some(plan_type) = &account.plan_type {
        message.push_str(&format!(" ({plan_type})"));
    }
    message.push('.');
    message
}

fn account_status_json(status: &AccountStatus) -> Value {
    let account = status.account.as_ref().map(|account| {
        json!({
            "type": match &account.account_type {
                AccountType::ChatGpt => "chatgpt",
                AccountType::ApiKey => "apiKey",
                AccountType::AwsBedrock => "awsBedrock",
                AccountType::Other(kind) => kind,
            },
            "email": account.email,
            "planType": account.plan_type,
        })
    });
    json!({
        "signedIn": account.is_some(),
        "account": account,
        "requiresOpenaiAuth": status.requires_openai_auth,
    })
}

#[cfg(test)]
#[path = "codex_app_server_tests.rs"]
mod tests;
