use std::ffi::OsString;
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail};
use async_process::{Child, ChildStdin, ChildStdout};
use command::r#async::Command;
use futures_lite::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use serde_json::{Map, Value, json};

use crate::types::{
    AccountStatus, LoginChallenge, LoginMode, Notification, ServerRequest, ServerRequestResponse,
    ThreadOptions, TurnEvent, TurnResult,
};

const CLIENT_NAME: &str = "warp";
const CLIENT_TITLE: &str = "Warp Codex integration";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub codex_program: OsString,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            codex_program: std::env::var_os("WARP_CODEX_PATH")
                .unwrap_or_else(|| OsString::from("codex")),
        }
    }
}

impl ClientOptions {
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            codex_program: program.into(),
        }
    }
}

enum WireMessage {
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<Value>,
    },
    Notification(Notification),
    ServerRequest(ServerRequest),
}

pub struct Client {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_request_id: u64,
}

impl Client {
    pub async fn spawn(options: ClientOptions) -> Result<Self> {
        let mut command = Command::new(&options.codex_program);
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command.spawn().with_context(|| {
            format!(
                "Failed to launch Codex app-server using {:?}; install Codex or set WARP_CODEX_PATH",
                options.codex_program
            )
        })?;
        let input = child
            .stdin
            .take()
            .context("Codex app-server did not expose stdin")?;
        let output = child
            .stdout
            .take()
            .context("Codex app-server did not expose stdout")?;

        let mut client = Self {
            child,
            input: BufWriter::new(input),
            output: BufReader::new(output),
            next_request_id: 1,
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&mut self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "title": CLIENT_TITLE,
                    "version": CLIENT_VERSION,
                }
            }),
        )
        .await
        .context("Failed to initialize Codex app-server")?;
        self.send_value(&json!({ "method": "initialized" }))
            .await
            .context("Failed to acknowledge Codex app-server initialization")
    }

    pub async fn account(&mut self, refresh_token: bool) -> Result<AccountStatus> {
        let value = self
            .request("account/read", json!({ "refreshToken": refresh_token }))
            .await
            .context("Failed to read the Codex account")?;
        AccountStatus::from_value(value)
    }

    pub async fn start_login(&mut self, mode: LoginMode) -> Result<LoginChallenge> {
        let params = match mode {
            LoginMode::Browser => json!({ "type": "chatgpt" }),
            LoginMode::DeviceCode => json!({ "type": "chatgptDeviceCode" }),
        };
        let value = self
            .request("account/login/start", params)
            .await
            .context("Failed to start ChatGPT login through Codex")?;
        LoginChallenge::from_value(value)
    }

    pub async fn wait_for_login(&mut self, login_id: &str) -> Result<()> {
        loop {
            match self.read_message().await? {
                WireMessage::Notification(notification)
                    if notification.method == "account/login/completed" =>
                {
                    let completed_login_id =
                        notification.params.get("loginId").and_then(Value::as_str);
                    if completed_login_id.is_some_and(|id| id != login_id) {
                        continue;
                    }
                    if notification
                        .params
                        .get("success")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        return Ok(());
                    }
                    let error = notification
                        .params
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("ChatGPT login was not completed");
                    bail!("{error}");
                }
                WireMessage::ServerRequest(request) => {
                    let response = deny_server_request(&request);
                    self.send_server_response(request.id, response).await?;
                }
                WireMessage::Response { .. } | WireMessage::Notification(_) => {}
            }
        }
    }

    pub async fn logout(&mut self) -> Result<()> {
        self.request("account/logout", json!({}))
            .await
            .context("Failed to log out of Codex")?;
        Ok(())
    }

    pub async fn start_thread(&mut self, options: &ThreadOptions) -> Result<String> {
        let value = self
            .request("thread/start", thread_params(options, None))
            .await
            .context("Failed to start a Codex thread")?;
        thread_id_from_response(&value, "start")
    }

    /// Resume a persisted Codex thread and apply the current request's runtime
    /// overrides. App-server returns the same thread shape as `thread/start`.
    pub async fn resume_thread(
        &mut self,
        thread_id: &str,
        options: &ThreadOptions,
    ) -> Result<String> {
        let value = self
            .request("thread/resume", thread_params(options, Some(thread_id)))
            .await
            .with_context(|| format!("Failed to resume Codex thread {thread_id}"))?;
        thread_id_from_response(&value, "resume")
    }

    /// Begin a turn and return its id without consuming the subsequent event
    /// stream. Notifications received before the response are forwarded to the
    /// supplied callback.
    pub async fn start_turn<N, R>(
        &mut self,
        thread_id: &str,
        prompt: &str,
        on_notification: &mut N,
        on_server_request: &mut R,
    ) -> Result<String>
    where
        N: FnMut(&Notification),
        R: FnMut(&ServerRequest) -> ServerRequestResponse,
    {
        let value = self
            .request_with_hooks(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{ "type": "text", "text": prompt }],
                }),
                on_notification,
                on_server_request,
            )
            .await
            .context("Failed to start a Codex turn")?;
        value
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .context("Codex turn response did not include a turn id")
            .map(str::to_owned)
    }

    /// Read one notification or the terminal result for an in-flight turn.
    /// Server-initiated requests are answered through `on_server_request` and
    /// are not exposed as stream events.
    pub async fn next_turn_event<R>(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        on_server_request: &mut R,
    ) -> Result<TurnEvent>
    where
        R: FnMut(&ServerRequest) -> ServerRequestResponse,
    {
        loop {
            match self.read_message().await? {
                WireMessage::Notification(notification) => {
                    if notification.method == "turn/completed"
                        && notification
                            .params
                            .get("turn")
                            .and_then(|turn| turn.get("id"))
                            .and_then(Value::as_str)
                            .is_none_or(|id| id == turn_id)
                    {
                        return Ok(TurnEvent::Completed(turn_result_from_notification(
                            thread_id,
                            turn_id,
                            &notification,
                        )?));
                    }
                    return Ok(TurnEvent::Notification(notification));
                }
                WireMessage::ServerRequest(request) => {
                    let response = on_server_request(&request);
                    self.send_server_response(request.id, response).await?;
                }
                WireMessage::Response { .. } => {
                    // No client request is outstanding while consuming a turn.
                    // Ignore late responses from optional app-server facilities.
                }
            }
        }
    }

    pub async fn run_turn<N, R>(
        &mut self,
        thread_id: &str,
        prompt: &str,
        mut on_notification: N,
        mut on_server_request: R,
    ) -> Result<TurnResult>
    where
        N: FnMut(&Notification),
        R: FnMut(&ServerRequest) -> ServerRequestResponse,
    {
        let turn_id = self
            .start_turn(
                thread_id,
                prompt,
                &mut on_notification,
                &mut on_server_request,
            )
            .await?;

        loop {
            match self
                .next_turn_event(thread_id, &turn_id, &mut on_server_request)
                .await?
            {
                TurnEvent::Notification(notification) => on_notification(&notification),
                TurnEvent::Completed(result) => return Ok(result),
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_with_hooks(method, params, &mut |_| {}, &mut deny_server_request)
            .await
    }

    async fn request_with_hooks<N, R>(
        &mut self,
        method: &str,
        params: Value,
        on_notification: &mut N,
        on_server_request: &mut R,
    ) -> Result<Value>
    where
        N: FnMut(&Notification),
        R: FnMut(&ServerRequest) -> ServerRequestResponse,
    {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.send_value(&json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        loop {
            match self.read_message().await? {
                WireMessage::Response {
                    id: response_id,
                    result,
                    error,
                } => {
                    if response_id.as_u64() != Some(id) {
                        bail!(
                            "Codex app-server returned response id {response_id} while waiting for {id}"
                        );
                    }
                    if let Some(error) = error {
                        let code = error.get("code").and_then(Value::as_i64);
                        let message = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown Codex app-server error");
                        if let Some(code) = code {
                            bail!("Codex app-server error {code}: {message}");
                        }
                        bail!("Codex app-server error: {message}");
                    }
                    return result.context("Codex app-server response did not include a result");
                }
                WireMessage::Notification(notification) => on_notification(&notification),
                WireMessage::ServerRequest(request) => {
                    let response = on_server_request(&request);
                    self.send_server_response(request.id, response).await?;
                }
            }
        }
    }

    async fn send_server_response(
        &mut self,
        id: Value,
        response: ServerRequestResponse,
    ) -> Result<()> {
        let message = match response {
            ServerRequestResponse::Result(result) => json!({ "id": id, "result": result }),
            ServerRequestResponse::Error { code, message } => {
                json!({ "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.send_value(&message).await
    }

    async fn send_value(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value).context("Failed to encode Codex RPC message")?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .context("Failed to write to Codex app-server")?;
        self.input
            .flush()
            .await
            .context("Failed to flush Codex app-server request")
    }

    async fn read_message(&mut self) -> Result<WireMessage> {
        loop {
            let mut line = String::new();
            let bytes_read = self
                .output
                .read_line(&mut line)
                .await
                .context("Failed to read from Codex app-server")?;
            if bytes_read == 0 {
                let status = self.child.try_status().ok().flatten();
                return Err(anyhow!(
                    "Codex app-server closed its output{}",
                    status
                        .map(|status| format!(" with status {status}"))
                        .unwrap_or_default()
                ));
            }
            if line.trim().is_empty() {
                continue;
            }

            let mut value: Value = serde_json::from_str(&line)
                .with_context(|| format!("Codex app-server emitted invalid JSON: {line:?}"))?;
            let object = value
                .as_object_mut()
                .context("Codex app-server emitted a non-object JSON message")?;
            let method = object
                .remove("method")
                .and_then(|method| method.as_str().map(str::to_owned));
            let id = object.remove("id");
            let params = object.remove("params").unwrap_or(Value::Null);

            return match (method, id) {
                (Some(method), Some(id)) => Ok(WireMessage::ServerRequest(ServerRequest {
                    id,
                    method,
                    params,
                })),
                (Some(method), None) => {
                    Ok(WireMessage::Notification(Notification { method, params }))
                }
                (None, Some(id)) => Ok(WireMessage::Response {
                    id,
                    result: object.remove("result"),
                    error: object.remove("error"),
                }),
                (None, None) => Err(anyhow!("Codex app-server emitted an unrecognized message")),
            };
        }
    }
}

fn thread_params(options: &ThreadOptions, thread_id: Option<&str>) -> Value {
    let mut params = Map::new();
    if let Some(thread_id) = thread_id {
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    }
    params.insert(
        "cwd".to_owned(),
        Value::String(options.cwd.to_string_lossy().into_owned()),
    );
    params.insert(
        "approvalPolicy".to_owned(),
        Value::String(options.approval_policy.as_protocol().to_owned()),
    );
    params.insert(
        "sandbox".to_owned(),
        Value::String(options.sandbox.as_protocol().to_owned()),
    );
    if thread_id.is_none() {
        params.insert(
            "threadSource".to_owned(),
            Value::String(options.thread_source.clone()),
        );
    }
    if let Some(model) = &options.model {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    Value::Object(params)
}

fn thread_id_from_response(value: &Value, operation: &str) -> Result<String> {
    value
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("Codex thread/{operation} response did not include a thread id"))
}

fn turn_result_from_notification(
    thread_id: &str,
    turn_id: &str,
    notification: &Notification,
) -> Result<TurnResult> {
    let turn = notification
        .params
        .get("turn")
        .context("Codex turn/completed notification did not include a turn")?;
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed")
        .to_owned();
    let error = turn
        .get("error")
        .filter(|error| !error.is_null())
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string())
        });
    Ok(TurnResult {
        thread_id: thread_id.to_owned(),
        turn_id: turn_id.to_owned(),
        status,
        error,
    })
}

pub fn deny_server_request(request: &ServerRequest) -> ServerRequestResponse {
    match request.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            ServerRequestResponse::Result(json!({ "decision": "decline" }))
        }
        "execCommandApproval" | "applyPatchApproval" => ServerRequestResponse::Result(json!({
            "decision": { "denied": { "rejection": "Denied by Warp" } },
        })),
        "item/permissions/requestApproval" => ServerRequestResponse::Result(json!({
            "permissions": {
                "fileSystem": null,
                "network": null,
            },
            "scope": "turn",
        })),
        "item/tool/requestUserInput" => ServerRequestResponse::Result(json!({ "answers": {} })),
        "mcpServer/elicitation/request" => {
            ServerRequestResponse::Result(json!({ "action": "decline" }))
        }
        method => ServerRequestResponse::method_not_found(method),
    }
}
