use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use prost::Message;
use warp_multi_agent_api::ResponseEvent;

/// Decodes a serialized response event string by base64-decoding
/// and then decoding the protobuf payload into a ResponseEvent.
pub fn decode_agent_response_event(encoded: &str) -> Result<ResponseEvent> {
    let bytes = STANDARD_NO_PAD.decode(encoded)?;
    let event = ResponseEvent::decode(bytes.as_slice())?;
    Ok(event)
}

/// Encodes a ResponseEvent by protobuf-encoding it and base64-encoding the bytes.
pub fn encode_agent_response_event(event: &ResponseEvent) -> String {
    let bytes = event.encode_to_vec();
    STANDARD_NO_PAD.encode(bytes)
}
