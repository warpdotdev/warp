use futures::{Sink, Stream, StreamExt};
use itertools::Itertools;
pub use ws_stream_wasm::WsMessage as Message;
use ws_stream_wasm::{WsErr, WsMessage, WsMeta};

use crate::WebsocketMessage;

pub async fn connect(
    url: impl AsRef<str>,
    protocols: impl IntoIterator<Item = &str>,
) -> anyhow::Result<WebSocket> {
    let protocols = protocols.into_iter().collect_vec();
    let (meta, stream) = WsMeta::connect(url, (!protocols.is_empty()).then_some(protocols)).await?;
    Ok(WebSocket { stream, meta })
}

pub type Error = WsErr;

pub struct WebSocket {
    stream: ws_stream_wasm::WsStream,
    meta: WsMeta,
}

impl WebSocket {
    pub async fn split(
        self,
    ) -> (
        impl Sink<Message, Error = WsErr>,
        impl Stream<Item = Result<Message, WsErr>>,
    ) {
        let (sink, stream) = self.stream.split();
        (sink, stream.map(Ok::<_, ws_stream_wasm::WsErr>))
    }

    pub async fn into_graphql_client_builder(self) -> graphql_ws_client::ClientBuilder {
        graphql_ws_client::Client::build(
            graphql_ws_client::ws_stream_wasm::Connection::new((self.meta, self.stream)).await,
        )
    }
}

impl WebsocketMessage for WsMessage {
    fn new_binary(bytes: Vec<u8>) -> Self {
        Self::Binary(bytes)
    }

    fn binary(&self) -> Option<&[u8]> {
        match self {
            Self::Binary(bytes) => Some(bytes.as_ref()),
            _ => None,
        }
    }

    fn new(text: String) -> Self {
        Self::new_text(text)
    }

    fn new_text(text: String) -> Self {
        Self::Text(text)
    }

    fn text(&self) -> Option<&str> {
        match self {
            WsMessage::Text(text) => Some(text.as_str()),
            _ => None,
        }
    }
}
