use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE;
use futures::StreamExt as _;
use prost::Message as _;
use tracing_futures::Instrument as _;
use warp_core::channel::ChannelState;
#[cfg(feature = "agent_mode_evals")]
use warp_server_client::base_client::EVAL_USER_ID_HEADER;
use warp_server_client::base_client::{AmbientHeaderPolicy, BaseClient};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to authenticate multi-agent request")]
    Authentication(#[source] anyhow::Error),

    #[error("Failed to resolve ambient headers for multi-agent request")]
    AmbientHeaders(#[source] anyhow::Error),

    #[error("Failed to decode base64 multi-agent response event")]
    Base64Decode(#[source] base64::DecodeError),

    #[error("Failed to decode protobuf multi-agent response event")]
    ProtobufDecode(#[source] prost::DecodeError),

    #[error("Multi-agent eventsource stream failed: {0:?}")]
    EventSource(Box<reqwest_eventsource::Error>),
}

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        /// A multi-agent response event stream without an unnecessary `Send` bound on WASM.
        pub type OutputStream = futures::stream::LocalBoxStream<
            'static,
            Result<warp_multi_agent_api::ResponseEvent, Error>,
        >;
    } else {
        /// A multi-agent response event stream that can be sent between native threads.
        pub type OutputStream = futures::stream::BoxStream<
            'static,
            Result<warp_multi_agent_api::ResponseEvent, Error>,
        >;
    }
}

/// Opens a decoded multi-agent response event stream.
pub async fn generate_multi_agent_output(
    client: &BaseClient,
    request: &warp_multi_agent_api::Request,
) -> Result<OutputStream, Error> {
    let auth_token = client
        .get_or_refresh_access_token()
        .await
        .map_err(Error::Authentication)?;
    let is_passive = is_passive_suggestion_request(request);
    let url = endpoint_url(is_passive);

    let mut request_builder = client
        .http_client()
        .post(url)
        .proto(request)
        .prevent_sleep("Agent Mode request in-progress");
    if let Some(token) = auth_token.as_bearer_token() {
        request_builder = request_builder.bearer_auth(token);
    }

    for (name, value) in client
        .ambient_headers(ambient_policy(is_passive))
        .await
        .map_err(Error::AmbientHeaders)?
    {
        request_builder = request_builder.header(name, value);
    }

    #[cfg(feature = "agent_mode_evals")]
    if let Some(eval_user_id) = client.eval_user_id() {
        request_builder = request_builder.header(EVAL_USER_ID_HEADER, eval_user_id.to_string());
    }

    let raw_stream = client.wrap_eventsource_with_iap_detection(request_builder.eventsource());
    let output_stream = raw_stream.filter_map(|event| async {
        match event {
            Ok(reqwest_eventsource::Event::Message(message_event)) => {
                Some(decode_response_event(&message_event.data))
            }
            Ok(reqwest_eventsource::Event::Open) => None,
            Err(error) => Some(Err(Error::EventSource(Box::new(error)))),
        }
    });

    // Once we get the init event, add some identifiers to the trace span.
    let output_stream = output_stream.inspect(|event| {
        if let Ok(event) = &event {
            match &event.r#type {
                Some(warp_multi_agent_api::response_event::Type::Init(init)) => {
                    tracing::info!("StreamInit");
                    tracing::Span::current().record("conversation_id", &init.conversation_id);
                    tracing::Span::current().record("request_id", &init.request_id);
                    tracing::Span::current().record("run_id", &init.run_id);
                }
                Some(warp_multi_agent_api::response_event::Type::Finished(_finished)) => {
                    tracing::info!("StreamFinished");
                }
                _ => {}
            }
        }
    });
    // Wrap the output stream with a trace span.
    let output_stream = output_stream.instrument(tracing::info_span!(
        "generate_multi_agent_output",
        tags.cloud_agent = true,
        conversation_id = tracing::field::Empty,
        request_id = tracing::field::Empty,
        run_id = tracing::field::Empty,
    ));

    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            Ok(output_stream.boxed_local())
        } else {
            Ok(output_stream.boxed())
        }
    }
}

fn is_passive_suggestion_request(request: &warp_multi_agent_api::Request) -> bool {
    request.input.as_ref().is_some_and(|input| {
        matches!(
            input.r#type,
            Some(warp_multi_agent_api::request::input::Type::GeneratePassiveSuggestions(_))
        )
    })
}

fn endpoint_url(is_passive: bool) -> String {
    format!(
        "{}/{}/{}",
        ChannelState::server_root_url(),
        if cfg!(feature = "agent_mode_evals") {
            "agent-mode-evals"
        } else {
            "ai"
        },
        if is_passive {
            "passive-suggestions"
        } else {
            "multi-agent"
        }
    )
}

fn ambient_policy(is_passive: bool) -> AmbientHeaderPolicy {
    if is_passive {
        // Passive suggestions read from the main conversation, but cannot modify it.
        AmbientHeaderPolicy::omit_all()
    } else {
        AmbientHeaderPolicy::workload_only()
    }
}

fn decode_response_event(data: &str) -> Result<warp_multi_agent_api::ResponseEvent, Error> {
    let decoded_data = BASE64_URL_SAFE
        .decode(data.trim_matches('"'))
        .map_err(Error::Base64Decode)?;
    warp_multi_agent_api::ResponseEvent::decode(decoded_data.as_slice())
        .map_err(Error::ProtobufDecode)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
