//! Request headers that can be replaced while an MCP HTTP connection stays open.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::{Error as SseError, Sse};

/// Asked to replace the headers after a server rejected a request that carried them, given the
/// `WWW-Authenticate` challenge of the rejection. Resolves once the headers have been replaced
/// or the handler has given up on replacing them.
pub type RejectionHandler = Arc<dyn Fn(String) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Debug, thiserror::Error)]
#[error("'{name}' is not a valid HTTP header")]
pub struct InvalidHeaderError {
    name: String,
}

/// Headers sent with every request of an MCP HTTP connection, which their owner can replace
/// without reconnecting.
#[derive(Clone)]
pub struct RefreshableHeaders {
    inner: Arc<Inner>,
}

struct Inner {
    current: RwLock<Current>,
    on_rejected: Option<RejectionHandler>,
}

struct Current {
    headers: HashMap<String, String>,
    parsed: HashMap<HeaderName, HeaderValue>,
    /// Counts replacements, so a request can tell whether the headers it was sent with are still
    /// the current ones.
    generation: u64,
}

impl RefreshableHeaders {
    pub fn new(
        headers: HashMap<String, String>,
        on_rejected: Option<RejectionHandler>,
    ) -> Result<Self, InvalidHeaderError> {
        let parsed = parse_headers(&headers)?;
        Ok(Self {
            inner: Arc::new(Inner {
                current: RwLock::new(Current {
                    headers,
                    parsed,
                    generation: 0,
                }),
                on_rejected,
            }),
        })
    }

    /// Replaces the headers for every later request, including the retry of a rejected one.
    pub fn replace(&self, headers: HashMap<String, String>) -> Result<(), InvalidHeaderError> {
        let parsed = parse_headers(&headers)?;
        let mut current = self
            .inner
            .current
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        current.headers = headers;
        current.parsed = parsed;
        current.generation += 1;
        Ok(())
    }

    /// `configured` with the current headers taking the place of any it has under the same name,
    /// and the generation of the headers that were applied.
    pub(crate) fn applied_to(
        &self,
        configured: &HashMap<String, String>,
    ) -> (HashMap<String, String>, u64) {
        let current = self.read();
        let mut headers = without_names(configured, &current);
        headers.extend(current.headers.clone());
        (headers, current.generation)
    }

    /// `configured` without the headers this supplies on every request.
    pub(crate) fn excluded_from(
        &self,
        configured: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        without_names(configured, &self.read())
    }

    /// Gives the owner the chance to replace headers a server rejected, and reports whether
    /// headers newer than those of `sent_generation` are now in place.
    pub(crate) async fn refreshed_after_rejection(
        &self,
        sent_generation: u64,
        challenge: &str,
    ) -> bool {
        if self.read().generation != sent_generation {
            return true;
        }
        let Some(on_rejected) = &self.inner.on_rejected else {
            return false;
        };
        on_rejected(challenge.to_owned()).await;
        self.read().generation != sent_generation
    }

    /// The headers requests are sent with now.
    pub fn current(&self) -> HashMap<String, String> {
        self.read().headers.clone()
    }

    fn parsed(&self) -> (HashMap<HeaderName, HeaderValue>, u64) {
        let current = self.read();
        (current.parsed.clone(), current.generation)
    }

    fn read(&self) -> RwLockReadGuard<'_, Current> {
        self.inner
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for RefreshableHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let current = self.read();
        f.debug_struct("RefreshableHeaders")
            .field("names", &current.headers.keys().collect::<Vec<_>>())
            .field("generation", &current.generation)
            .finish_non_exhaustive()
    }
}

fn without_names(
    configured: &HashMap<String, String>,
    current: &Current,
) -> HashMap<String, String> {
    configured
        .iter()
        .filter(|(name, _)| {
            !current
                .headers
                .keys()
                .any(|refreshable| refreshable.eq_ignore_ascii_case(name))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn parse_headers(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, InvalidHeaderError> {
    headers
        .iter()
        .map(|(name, value)| {
            let invalid = || InvalidHeaderError { name: name.clone() };
            let parsed_name = HeaderName::try_from(name.as_str()).map_err(|_| invalid())?;
            let mut parsed_value = HeaderValue::try_from(value.as_str()).map_err(|_| invalid())?;
            parsed_value.set_sensitive(true);
            Ok((parsed_name, parsed_value))
        })
        .collect()
}

/// Sends every request with the current [`RefreshableHeaders`], and repeats a request the server
/// rejected once when that rejection got the headers replaced.
#[derive(Clone)]
pub(crate) struct RefreshableHeaderClient<C> {
    http_client: C,
    headers: RefreshableHeaders,
}

impl<C> RefreshableHeaderClient<C>
where
    C: StreamableHttpClient + Send + Sync,
{
    pub(crate) fn new(http_client: C, headers: RefreshableHeaders) -> Self {
        Self {
            http_client,
            headers,
        }
    }

    async fn call_with_current_headers<T, F, Fut>(
        &self,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        call: F,
    ) -> Result<T, StreamableHttpError<C::Error>>
    where
        F: Fn(HashMap<HeaderName, HeaderValue>) -> Fut,
        Fut: Future<Output = Result<T, StreamableHttpError<C::Error>>>,
    {
        let with_current = |current: HashMap<HeaderName, HeaderValue>| {
            let mut headers = custom_headers.clone();
            headers.extend(current);
            headers
        };
        let (current, sent_generation) = self.headers.parsed();
        match call(with_current(current)).await {
            Err(StreamableHttpError::AuthRequired(challenge)) => {
                if !self
                    .headers
                    .refreshed_after_rejection(sent_generation, &challenge.www_authenticate_header)
                    .await
                {
                    return Err(StreamableHttpError::AuthRequired(challenge));
                }
                let (refreshed, _) = self.headers.parsed();
                call(with_current(refreshed)).await
            }
            result => result,
        }
    }
}

impl<C> StreamableHttpClient for RefreshableHeaderClient<C>
where
    C: StreamableHttpClient + Send + Sync,
{
    type Error = C::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.call_with_current_headers(custom_headers, |headers| {
            let uri = uri.clone();
            let message = message.clone();
            let session_id = session_id.clone();
            let auth_header = auth_header.clone();
            async move {
                self.http_client
                    .post_message(uri, message, session_id, auth_header, headers)
                    .await
            }
        })
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.call_with_current_headers(custom_headers, |headers| {
            let uri = uri.clone();
            let message = message.clone();
            let session_id = session_id.clone();
            let auth_header = auth_header.clone();
            async move {
                self.http_client
                    .post_message_with_max_sse_event_size(
                        uri,
                        message,
                        session_id,
                        auth_header,
                        headers,
                        max_sse_event_size,
                    )
                    .await
            }
        })
        .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        // rmcp's reqwest client reports a rejected `DELETE` as a client error rather than
        // `AuthRequired`, so closing a session after the headers have expired is not retried.
        self.call_with_current_headers(custom_headers, |headers| {
            let uri = uri.clone();
            let session_id = session_id.clone();
            let auth_header = auth_header.clone();
            async move {
                self.http_client
                    .delete_session(uri, session_id, auth_header, headers)
                    .await
            }
        })
        .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.call_with_current_headers(custom_headers, |headers| {
            let uri = uri.clone();
            let session_id = session_id.clone();
            let last_event_id = last_event_id.clone();
            let auth_header = auth_header.clone();
            async move {
                self.http_client
                    .get_stream(uri, session_id, last_event_id, auth_header, headers)
                    .await
            }
        })
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.call_with_current_headers(custom_headers, |headers| {
            let uri = uri.clone();
            let session_id = session_id.clone();
            let last_event_id = last_event_id.clone();
            let auth_header = auth_header.clone();
            async move {
                self.http_client
                    .get_stream_with_max_sse_event_size(
                        uri,
                        session_id,
                        last_event_id,
                        auth_header,
                        headers,
                        max_sse_event_size,
                    )
                    .await
            }
        })
        .await
    }
}

#[cfg(test)]
#[path = "refreshable_headers_tests.rs"]
mod tests;
