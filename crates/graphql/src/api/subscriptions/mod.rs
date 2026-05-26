pub mod get_warp_drive_updates;

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use async_channel::Sender;
use cynic::{QueryFragment, QueryVariables, StreamingOperation as CynicStreamingOperation};
use futures::StreamExt as _;
use serde::de::DeserializeOwned;
use serde::Serialize;

const PROTOCOL: &str = "graphql-transport-ws";

/// This function is used to start a GraphQL subscription given an operation.
/// Messages coming over the subscription will be sent over the message_sender stream.
/// transform_stream_message is a function that converts a message received
/// over the subscription (i.e. a GraphQL Response type) to a custom type T
/// before it sent to the message_sender.
///
/// The `init_payload`` is a payload that is sent as part of the websocket handshake
/// See: https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md#connectioninit
///
/// Note that the future returned by this method only resolves once the stream is done.
/// However, a message is sent over stream_ready_sender when the stream is ready to receive messages.
/// Any errant message sent over the stream will also terminate it.
pub async fn start_graphql_streaming_operation<Q, V, F, T>(
    server_url: &str,
    init_payload: HashMap<&str, String>,
    operation: CynicStreamingOperation<Q, V>,
    transform_stream_message: F,
    message_sender: Sender<T>,
    stream_ready_sender: Sender<()>,
) -> Result<()>
where
    Q: QueryFragment + DeserializeOwned + Unpin + Send + 'static,
    F: Fn(Option<Q>) -> Result<T> + Unpin + Send,
    V: QueryVariables + Serialize + Unpin + Send + 'static,
{
    // TODO: eventually, we might want to explore re-using the client. But for now,
    // we only have one subscription so let's just create a new one (we have to
    // anyways for retry logic).

    let socket = websocket::WebSocket::connect(server_url, Some(PROTOCOL))
        .await
        .context("failed to create websocket connection")?;
    let mut stream = socket
        .into_graphql_client_builder()
        .await
        .payload(init_payload)?
        .subscribe(operation)
        .await
        .context("failed to initialize graphql subscription")?;

    stream_ready_sender.send(()).await?;

    while let Some(stream_item) = stream.next().await {
        match stream_item {
            Ok(response) => {
                // If the response has errors, let's stop the subscription.
                // A response error could be a schema mismatch for example.
                if let Some(err) = response.errors.as_ref().and_then(|errs| errs.first()) {
                    return Err(anyhow!(
                        "Error in subscription message: {}. Stopping websocket.",
                        err
                    ));
                } else {
                    // Try to transform the stream message to the liking of the caller.
                    // If it results in an error, don't stop the subscription; it was
                    // the caller's choice to return an error in that case (e.g. a message
                    // type the client doesn't yet know how to handle).
                    match transform_stream_message(response.data) {
                        Ok(transformed_message) => {
                            if let Err(e) = message_sender.send(transformed_message).await {
                                log::warn!("Failed to send transformed message over channel: {e}")
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to transform stream message: {e}");
                        }
                    }
                }
            }
            // If we received an error over the stream, we should stop the subscription.
            Err(e) => {
                return Err(anyhow!(
                    "Message received in subscriptions stream has error: {e}. Stopping websocket."
                ));
            }
        }
    }

    Ok(())
}
