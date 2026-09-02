use std::sync::Arc;

use async_trait::async_trait;
use warpui::{Entity, SingletonEntity};

use super::server_api::{ServerApi, TranscribeError};
use super::team_scope::RequestTeamScope;
use crate::ai::voice::transcribe::{Provider, TranscribeRequest};
use crate::voice::transcriber::Transcriber;

pub struct ServerVoiceTranscriber {
    server_api: Arc<ServerApi>,
}

impl ServerVoiceTranscriber {
    pub fn new(server_api: Arc<ServerApi>) -> Self {
        Self { server_api }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl Transcriber for ServerVoiceTranscriber {
    async fn transcribe(
        &self,
        wav_base64: String,
        language: Option<String>,
        team_scope: RequestTeamScope,
    ) -> Result<String, TranscribeError> {
        let request = TranscribeRequest {
            provider: Provider::Wispr,
            audio: Some(wav_base64),
            language,
            ..Default::default()
        };
        let response = self.server_api.transcribe(&request, team_scope).await;
        match response {
            Ok(response) => Ok(response.text),
            Err(e) => Err(e),
        }
    }
}

impl Entity for ServerVoiceTranscriber {
    type Event = ();
}

impl SingletonEntity for ServerVoiceTranscriber {}
