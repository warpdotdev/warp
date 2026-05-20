use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use std::{fmt, future};

#[cfg(not(target_family = "wasm"))]
use async_compat::{Compat, CompatExt};
use async_stream::stream;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use http::HeaderValue;
use http::header::HeaderName;
pub use http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use reqwest::IntoUrl;
use reqwest_eventsource::RequestBuilderExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use warp_core::{
    channel::{Channel, ChannelState},
    execution_mode,
    operating_system_info::OperatingSystemInfo,
    report_error,
};

pub mod headers {
    /// Custom Warp header indicating the version of the Warp app.
    pub const CLIENT_RELEASE_VERSION_HEADER_KEY: &str = "X-Warp-Client-Version";

    /// Custom Warp header indicating the OS category the request was sent from.
    pub(crate) const WARP_OS_CATEGORY: &str = "X-Warp-OS-Category";
    /// Custom Warp header indicating the OS name the request was sent from. On Linux this is the
    /// name of the distribution. On all other platforms it should be equivalent to
    /// `WARP_OS_CATEGORY`.
    pub(crate) const WARP_OS_NAME: &str = "X-Warp-OS-Name";
    /// Custom Warp header indicating the version of the operating system. On Linux this is the
    /// version of the distribution, not the Linux kernel version.
    pub(crate) const WARP_OS_VERSION: &str = "X-Warp-OS-Version";

    /// Custom Warp header indicating the linux kernel version. This is only sent from Linux.
    pub(crate) const WARP_OS_LINUX_KERNEL_VERSION: &str = "X-Warp-OS-Linux-Kernel-Version";

    /// Custom Warp header indicating the client role. We don't use the User-Agent header
    /// because it can't be set from WASM.
    pub(crate) const WARP_CLIENT_ID: &str = "X-Warp-Client-ID";
}

/// The environment variable containing extra HTTP headers to attach to requests.
/// Only read when the channel is `Channel::Integration`. The value is a newline-separated
/// list of `Name:Value` pairs, where each pair is split on the first colon.
const EXTRA_HTTP_HEADERS_ENV_VAR: &str = "WARP_EXTRA_HTTP_HEADERS";

/// A wrapper around a `reqwest::Client` to execute requests. Returns a custom `RequestBuilder` type
/// that ensures any call to the underlying `reqwest::Client` are properly adapted so that they can
/// run outside of a Tokio context.
pub struct Client {
    wrapped: reqwest::Client,

    /// A callback that is executed before every request is sent with a cloned
    /// version of the outbound request.  If for some reason the request cannot be
    /// cloned the function is not called.
    before_request_sent: Option<RequestHookFn>,

    /// A callback that is executed on after each response is received.
    after_response_received: Option<ResponseHookFn>,
}

/// Type for 'hook' functions to be executed prior to sending a request. A reference to the
/// outbound request object is given as the first argument. The second argument the request's
/// serialized JSON payload, if any.
pub type RequestHookFn = Box<dyn Fn(&reqwest::Request, &Option<String>) + 'static + Send + Sync>;

/// Type for 'hook' functions to be executed after receiving a response. The sole argument is a
/// reference to the inbound response object.
pub type ResponseHookFn = Box<dyn Fn(&reqwest::Response) + 'static + Send + Sync>;

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        // The WASM version of this type has no bound on `Send`, which is not implemented on
        // `wasm_bindgen::JsValue`, which is ultimately used in reqwest_eventsource::Error.
        // Furthermore, `Send` is an unnecessary bound when targeting wasm because the browser is
        // single-threaded (and we don't leverage WebWorkers for async execution in WoW).
        pub type EventSourceStream = futures::stream::LocalBoxStream<
            'static,
            Result<reqwest_eventsource::Event, reqwest_eventsource::Error>,
        >;
    } else {
        pub type EventSourceStream = futures::stream::BoxStream<
            'static,
            Result<reqwest_eventsource::Event, reqwest_eventsource::Error>,
        >;
    }
}

/// A custom request builder that is a wrapper around a `request::RequestBuilder`. Ensures any async
/// call to the underyling `reqwest::RequestBuilder` are properly adapted to run outside of a Tokio
/// context via a call to `compat`.
pub struct RequestBuilder<'a> {
    wrapped: reqwest::RequestBuilder,
    client: &'a Client,

    // The JSON payload of the request, if any, serialized to a pretty-printed String.
    serialized_payload: Option<String>,

    prevent_sleep_reason: Option<&'static str>,
}

pub struct Request {
    wrapped: reqwest::Request,
    serialized_payload: Option<String>,
    prevent_sleep_reason: Option<&'static str>,
}

/// A wrapper around a `reqwest::Response` that ensures any async calls to the underlying `Response`
/// a properly adapted to be run outside of a Tokio context.
pub struct Response(reqwest::Response);

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        #[cfg_attr(target_family = "wasm", expect(unused_mut))]
        let mut builder = reqwest::Client::builder();

        // Set some HTTP/2-related settings that aren't available on wasm.
        #[cfg(not(target_family = "wasm"))]
        {
            builder = builder
                .http2_keep_alive_interval(Duration::from_secs(60))
                // If a pong is not received within 15s, consider the connection dead.
                .http2_keep_alive_timeout(Duration::from_secs(15))
                // Send these even when there aren't active streams, to ensure that we detect
                // dead connections before we attempt to use them.
                .http2_keep_alive_while_idle(true);
        }

        Self::from_client_builder(builder).expect("should not fail to create client")
    }

    #[cfg(feature = "test-util")]
    pub fn new_for_test() -> Self {
        let client_builder = reqwest::ClientBuilder::new()
            // Don't load any SSL/TLS certificates, as doing so can be slow and we should
            // never be making real requests in tests.
            .tls_certs_only([])
            // Disable proxy usage in tests, as loading system proxy configuration can be
            // slow.
            .no_proxy();
        Self::from_client_builder(client_builder).expect("should not fail to create client")
    }

    pub fn from_client_builder(client_builder: reqwest::ClientBuilder) -> reqwest::Result<Self> {
        client_builder.build().map(|client| Self {
            wrapped: client,
            before_request_sent: None,
            after_response_received: None,
        })
    }

    pub fn set_before_request_fn(&mut self, hook_fn: RequestHookFn) {
        self.before_request_sent = Some(hook_fn);
    }

    pub fn set_after_response_fn(&mut self, hook_fn: ResponseHookFn) {
        self.after_response_received = Some(hook_fn);
    }

    fn builder(
        &self,
        wrapped: reqwest::RequestBuilder,
        include_warp_headers: bool,
    ) -> RequestBuilder<'_> {
        let mut builder = RequestBuilder {
            wrapped,
            client: self,
            serialized_payload: None,
            prevent_sleep_reason: None,
        };

        if include_warp_headers {
            builder = Self::add_warp_http_headers(builder);
        }

        builder
    }

    pub fn get<U: IntoUrl + Clone>(&self, url: U) -> RequestBuilder<'_> {
        self.builder(
            self.wrapped.get(url.clone()),
            Self::include_warp_http_headers(url),
        )
    }

    pub fn post<U: IntoUrl + Clone>(&self, url: U) -> RequestBuilder<'_> {
        self.builder(
            self.wrapped.post(url.clone()),
            Self::include_warp_http_headers(url),
        )
    }

    pub fn put<U: IntoUrl + Clone>(&self, url: U) -> RequestBuilder<'_> {
        self.builder(
            self.wrapped.put(url.clone()),
            Self::include_warp_http_headers(url),
        )
    }

    pub fn patch<U: IntoUrl + Clone>(&self, url: U) -> RequestBuilder<'_> {
        self.builder(
            self.wrapped.patch(url.clone()),
            Self::include_warp_http_headers(url),
        )
    }

    pub fn delete<U: IntoUrl + Clone>(&self, url: U) -> RequestBuilder<'_> {
        self.builder(
            self.wrapped.delete(url.clone()),
            Self::include_warp_http_headers(url),
        )
    }

    /// Helper method to determine if the request should include warp-specific headers. The only case
    /// where we should include custom headers is if the request is same-origin and is targetted to our server.
    /// For example, app.warp.dev --> app.warp.dev.
    #[cfg(target_family = "wasm")]
    fn include_warp_http_headers<U: IntoUrl + Clone>(url: U) -> bool {
        url.into_url().is_ok_and(|url| {
            url.host_str().is_some_and(|dest_host| {
                let window_hostname = gloo::utils::window()
                    .location()
                    .hostname()
                    .expect("Can't get window hostname");

                // If the request is going to our server, the destination host should be "app.warp.dev" or
                // "staging.warp.dev". The window hostname should also return the same.
                // Note that reqwest's host_str() method is described here: https://docs.rs/reqwest/latest/reqwest/struct.Url.html#method.domain and
                // gloo's hostname() method refers to this mozilla definition: https://developer.mozilla.org/en-US/docs/Web/API/Location/hostname.
                window_hostname == dest_host
            })
        })
    }

    #[cfg(not(target_family = "wasm"))]
    fn include_warp_http_headers<U: IntoUrl + Clone>(_url: U) -> bool {
        true
    }

    fn add_warp_http_headers(mut builder: RequestBuilder) -> RequestBuilder {
        // Include the client ID header.
        if let Some(client_id) = execution_mode::current_client_id() {
            builder = builder.header(headers::WARP_CLIENT_ID, client_id);
        }

        // If there's an app version, include it as an HTTP request header.
        if let Some(app_version) = ChannelState::app_version() {
            builder = builder.header(headers::CLIENT_RELEASE_VERSION_HEADER_KEY, app_version);
        }

        // On integration builds, attach any extra headers from the environment.
        if ChannelState::channel() == Channel::Integration
            && let Ok(raw) = std::env::var(EXTRA_HTTP_HEADERS_ENV_VAR)
        {
            for line in raw.lines() {
                let Some((name, value)) = line.split_once(':') else {
                    continue;
                };
                let name = name.trim();
                let value = value.trim();
                if name.is_empty() {
                    continue;
                }
                match (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    (Ok(name), Ok(value)) => {
                        builder = builder.header(name, value);
                    }
                    _ => {
                        log::warn!(
                            "Ignoring invalid entry in {EXTRA_HTTP_HEADERS_ENV_VAR}: {line}"
                        );
                    }
                }
            }
        }

        // Headers indicating the details of the client's operating system, if available here at runtime.
        if let Ok(os_system_info) = OperatingSystemInfo::get() {
            // Operating system category.
            let category = os_system_info.category().to_string();
            if let Ok(category) = HeaderValue::from_str(&category) {
                builder = builder.header(headers::WARP_OS_CATEGORY, category);
            }

            // Operating system name.
            builder = builder.header(
                headers::WARP_OS_NAME,
                HeaderValue::from_static(os_system_info.name()),
            );

            // Operating system version.
            if let Some(version) = os_system_info
                .version()
                .and_then(|version| HeaderValue::from_str(version).ok())
            {
                builder = builder.header(headers::WARP_OS_VERSION, version);
            }

            // Linux kernel version.
            if let Some(linux_kernel_version) = os_system_info
                .linux_kernel_version()
                .and_then(|kernel_version| HeaderValue::from_str(kernel_version).ok())
            {
                builder =
                    builder.header(headers::WARP_OS_LINUX_KERNEL_VERSION, linux_kernel_version);
            }
        }

        builder
    }

    pub async fn execute(&self, request: Request) -> reqwest::Result<Response> {
        let Request {
            wrapped: request,
            serialized_payload,
            prevent_sleep_reason,
        } = request;

        if let Some(before_response_send_fn) = &self.before_request_sent {
            before_response_send_fn(&request, &serialized_payload);
        }

        let _guard = prevent_sleep_reason.map(prevent_sleep::prevent_sleep);

        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                let result = self.wrapped.execute(request).await?;
            } else {
                // Explicitly await the future before converting from tokio -> futures. This is because
                // certain calls to tokio (such as tokio::time::sleep) will panic upon creation if they
                // are not in a tokio runtime. Wrapping the call in an async block first makes sure that it
                // is lazily evaluated, ensuring that it is created within a tokio runtime.
                let result = Compat::new(async { self.wrapped.execute(request).await }).await?;
            }
        }

        if let Some(after_response_received_fn) = &self.after_response_received {
            after_response_received_fn(&result);
        }

        Ok(Response(result))
    }
}

impl<'a> RequestBuilder<'a> {
    pub fn build(self) -> reqwest::Result<Request> {
        self.build_split().1
    }

    pub fn build_split(self) -> (&'a Client, reqwest::Result<Request>) {
        let request = self.wrapped.build().map(|request| Request {
            wrapped: request,
            serialized_payload: self.serialized_payload,
            prevent_sleep_reason: self.prevent_sleep_reason,
        });
        (self.client, request)
    }

    pub async fn send(self) -> reqwest::Result<Response> {
        let (client, request) = self.build_split();
        client.execute(request?).await
    }

    pub fn json<T: Serialize + ?Sized>(self, json: &T) -> RequestBuilder<'a> {
        let serialized_payload =
            match serde_json::to_string_pretty(json).map_err(anyhow::Error::from) {
                Ok(payload) => Some(payload),
                Err(err) => {
                    report_error!(err.context("Failed to serialize JSON request payload."));
                    None
                }
            };
        Self {
            wrapped: self.wrapped.json(json),
            serialized_payload,
            ..self
        }
    }

    pub fn proto<T: prost::Message>(self, proto: &T) -> RequestBuilder<'a> {
        let bytes = proto.encode_to_vec();
        let serialized = String::from_utf8(bytes.clone());

        Self {
            wrapped: self
                .wrapped
                .header(
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/x-protobuf"),
                )
                .body(bytes),
            serialized_payload: serialized.ok(),
            ..self
        }
    }

    /// Sends the request to the endpoint, which is assumed to be a streaming server-sent-events
    /// endpoint, and returns a corresponding `EventSource`.
    pub fn eventsource(self) -> EventSourceStream {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                let mut stream = self
                    .wrapped
                    .eventsource()
                    .expect("Request type for SSE endpoint must be cloneable.");

                let stream = stream! {
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(event) => {
                                yield Ok(event);
                            }
                            Err(err) => {
                                yield Err(err);

                                // Close the stream if an error occurs.
                                stream.close();
                            }
                        }
                    }
                };
            } else {
                let mut stream = self
                    .wrapped
                    .eventsource()
                    .expect("Request type for SSE endpoint must be cloneable.");

                let stream = stream! {
                    // Wrap the stream with async-compat since reqwest requires Tokio.
                    while let Some(event) = stream.next().compat().await {
                        match event {
                            Ok(event) => {
                                yield Ok(event);
                            }
                            Err(err) => {
                                yield Err(err);

                                // Close the stream if an error occurs.
                                stream.close();
                            }
                        }
                    }
                };
            }
        }
        let stream = stream.take_while(|event| {
            if let Err(reqwest_eventsource::Error::StreamEnded) = event {
                return future::ready(false);
            }
            future::ready(true)
        });

        // Wrap the stream in one that holds onto a prevent_sleep guard, if one is required here.
        let stream = prevent_sleep::Stream::wrap(
            stream,
            self.prevent_sleep_reason.map(prevent_sleep::prevent_sleep),
        );

        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                stream.boxed_local()
            } else {
                stream.boxed()
            }
        }
    }

    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> RequestBuilder<'a>
    where
        U: fmt::Display,
        P: fmt::Display,
    {
        Self {
            wrapped: self.wrapped.basic_auth(username, password),
            ..self
        }
    }

    pub fn bearer_auth<T>(self, token: T) -> RequestBuilder<'a>
    where
        T: fmt::Display,
    {
        Self {
            wrapped: self.wrapped.bearer_auth(token),
            ..self
        }
    }

    // The `timeout` argument is unused on wasm.
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub fn timeout(self, timeout: Duration) -> RequestBuilder<'a> {
        cfg_if::cfg_if! {
            // reqwest provides no ability to configure a request timeout
            // on wasm, so make this a no-op (it's the best we can do).
            if #[cfg(target_family = "wasm")] {
                self
            } else {
                Self {
                    wrapped: self.wrapped.timeout(timeout),
                    ..self
                }
            }
        }
    }

    pub fn header<K, V>(self, key: K, value: V) -> RequestBuilder<'a>
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        Self {
            wrapped: self.wrapped.header(key, value),
            ..self
        }
    }

    pub fn body<T: Into<reqwest::Body>>(self, body: T) -> RequestBuilder<'a> {
        Self {
            wrapped: self.wrapped.body(body),
            ..self
        }
    }

    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> RequestBuilder<'a> {
        let serialized_payload =
            match serde_urlencoded::to_string(form).map_err(anyhow::Error::from) {
                Ok(payload) => Some(payload),
                Err(err) => {
                    report_error!(err.context("Failed to serialize url-encoded form payload"));
                    None
                }
            };
        Self {
            wrapped: self.wrapped.form(form),
            serialized_payload,
            ..self
        }
    }

    /// Attach a `multipart/form-data` body.
    /// Not available on wasm because reqwest's multipart builder API is native-only.
    #[cfg(not(target_family = "wasm"))]
    pub fn multipart(self, form: reqwest::multipart::Form) -> RequestBuilder<'a> {
        Self {
            wrapped: self.wrapped.multipart(form),
            ..self
        }
    }

    /// Prevents the system from sleeping due to idle while this request is in progress.
    ///
    /// The provided reason will be used in user-visible logging, so make sure it is
    /// descriptive and reasonably formatted (e.g. "Agent mode request in-progress").
    pub fn prevent_sleep(self, reason: &'static str) -> RequestBuilder<'a> {
        Self {
            prevent_sleep_reason: Some(reason),
            ..self
        }
    }
}

/// An error returned from `Response::error_for_status` that includes response metadata.
/// This allows callers to inspect headers (like X-Warp-Error-Code) and the response body when
/// handling errors.
#[derive(Debug)]
pub struct ResponseError {
    pub source: reqwest::Error,
    pub headers: Box<HeaderMap>,
    pub body: Option<String>,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for ResponseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl Response {
    pub async fn text(self) -> reqwest::Result<String> {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                self.0.text().await
            } else {
                Compat::new(async { self.0.text().compat().await }).await
            }
        }
    }

    pub fn status(&self) -> StatusCode {
        self.0.status()
    }

    pub async fn json<T: DeserializeOwned>(self) -> reqwest::Result<T> {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                self.0.json().await
            } else {
                Compat::new(async { self.0.json().compat().await }).await
            }
        }
    }

    /// Checks the response status and returns an error if it's not successful.
    /// Unlike `reqwest::Response::error_for_status`, this returns a `ResponseError`
    /// that includes the response headers, allowing callers to inspect them.
    pub fn error_for_status(self) -> Result<Self, ResponseError> {
        let headers = self.0.headers().clone();
        match self.0.error_for_status() {
            Ok(response) => Ok(Self(response)),
            Err(source) => Err(ResponseError {
                source,
                headers: Box::new(headers),
                body: None,
            }),
        }
    }

    /// Checks the response status and returns an error if it's not successful.
    /// Unlike `error_for_status`, this also reads and preserves the response body on errors.
    pub async fn error_for_status_with_body(self) -> Result<Self, ResponseError> {
        let headers = self.0.headers().clone();
        match self.0.error_for_status_ref() {
            Ok(_) => Ok(self),
            Err(source) => {
                let body = self.text().await.ok();
                Err(ResponseError {
                    source,
                    headers: Box::new(headers),
                    body,
                })
            }
        }
    }

    /// Returns a reference to the underlying response if the status is successful,
    /// otherwise returns an error with headers preserved.
    pub fn error_for_status_ref(&self) -> Result<&reqwest::Response, ResponseError> {
        let headers = self.0.headers().clone();
        match self.0.error_for_status_ref() {
            Ok(response) => Ok(response),
            Err(source) => Err(ResponseError {
                source,
                headers: Box::new(headers),
                body: None,
            }),
        }
    }

    pub async fn bytes(self) -> reqwest::Result<Bytes> {
        self.0.bytes().await
    }

    pub fn bytes_stream(self) -> impl Stream<Item = reqwest::Result<Bytes>> {
        self.0.bytes_stream()
    }

    pub fn headers(&self) -> &http::HeaderMap {
        self.0.headers()
    }

    pub fn url(&self) -> &reqwest::Url {
        self.0.url()
    }
}

/// Adapter to use our HTTP client wrapper with [`oauth2`]. This is modeled on the [`reqwest`]
/// implementation of [`oauth2::AsyncHttpClient`].
impl<'c> oauth2::AsyncHttpClient<'c> for Client {
    type Error = oauth2::HttpClientError<reqwest::Error>;

    #[cfg(target_arch = "wasm32")]
    type Future = Pin<Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + 'c>>;
    #[cfg(not(target_arch = "wasm32"))]
    type Future =
        Pin<Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + Send + Sync + 'c>>;

    fn call(&'c self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(async move {
            let include_warp_headers = Self::include_warp_http_headers(request.uri().to_string());
            let builder = reqwest::RequestBuilder::from_parts(
                self.wrapped.clone(),
                request.try_into().map_err(Box::new)?,
            );

            let response = self
                .builder(builder, include_warp_headers)
                .send()
                .await
                .map_err(Box::new)?;

            let mut builder = ::http::Response::builder().status(response.status());

            #[cfg(not(target_arch = "wasm32"))]
            {
                builder = builder.version(response.0.version());
            }

            for (name, value) in response.0.headers().iter() {
                builder = builder.header(name, value);
            }

            let response_body = response.bytes().await.map_err(Box::new)?.to_vec();
            builder
                .body(response_body)
                .map_err(oauth2::HttpClientError::Http)
        })
    }
}
