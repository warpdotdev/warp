use anyhow::anyhow;
use futures::future;

use super::*;

#[tokio::test]
async fn block_failure_retains_successfully_uploaded_usage() {
    let outcome = save_transcript_and_block(
        future::ready(Ok(UploadedTranscriptUsage::empty())),
        future::ready(Err(anyhow!("block unavailable"))),
    )
    .await;

    assert!(outcome.result.is_err());
    assert!(outcome.uploaded_usage.is_some());
}
