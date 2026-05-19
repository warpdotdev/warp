#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
#[cfg(not(target_family = "wasm"))]
use base64::{engine::general_purpose::STANDARD, Engine as _};
#[cfg(not(target_family = "wasm"))]
use crc::{Crc, CRC_32_ISCSI};
use std::future::Future;
use thiserror::Error;

#[cfg(feature = "local_fs")]
use super::ai::FileArtifactUploadTargetInfo;
use super::harness_support::{UploadFieldValue, UploadTarget};

/// Typed error for HTTP-backed operations so downstream classifiers (e.g. the agent-SDK
/// retry helper) can decide transient vs permanent failures without string-parsing the
/// anyhow Display.
///
/// Emitted as the source cause of an upload failure; callers typically also attach a
/// human-facing context message via `.context(...)` so `err.to_string()` remains useful.
#[derive(Debug, Error)]
#[error("HTTP request failed with status {status}: {body}")]
pub struct HttpStatusError {
    pub status: u16,
    pub body: String,
}

#[cfg(not(target_family = "wasm"))]
pub(crate) static CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
const CONTENT_LENGTH_HEADER_NAME: &str = "content-length";
#[cfg(feature = "local_fs")]
const FILE_UPLOAD_CHUNK_SIZE: usize = 64 * 1024;

struct NormalizedUploadTarget<'a> {
    url: &'a str,
    method: &'a str,
    headers: Vec<(&'a str, &'a str)>,
    fields: Vec<NormalizedField<'a>>,
}

/// Borrowed view of a single multipart form field.
#[derive(Debug, PartialEq, Eq)]
struct NormalizedField<'a> {
    name: &'a str,
    value: NormalizedFieldValue<'a>,
}

/// Borrowed view of a single multipart form field value.
#[derive(Debug, PartialEq, Eq)]
enum NormalizedFieldValue<'a> {
    Static(&'a str),
    ContentCrc32C,
    ContentData,
}

impl<'a> From<&'a UploadTarget> for NormalizedUploadTarget<'a> {
    fn from(target: &'a UploadTarget) -> Self {
        Self {
            url: &target.url,
            method: &target.method,
            headers: target
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect(),
            fields: target
                .fields
                .iter()
                .map(|field| NormalizedField {
                    name: field.name.as_str(),
                    value: match &field.value {
                        UploadFieldValue::Static { value } => {
                            NormalizedFieldValue::Static(value.as_str())
                        }
                        UploadFieldValue::ContentCrc32C => NormalizedFieldValue::ContentCrc32C,
                        UploadFieldValue::ContentData => NormalizedFieldValue::ContentData,
                    },
                })
                .collect(),
        }
    }
}

#[cfg(feature = "local_fs")]
impl<'a> From<&'a FileArtifactUploadTargetInfo> for NormalizedUploadTarget<'a> {
    fn from(target: &'a FileArtifactUploadTargetInfo) -> Self {
        Self {
            url: &target.url,
            method: &target.method,
            headers: target
                .headers
                .iter()
                .map(|header| (header.name.as_str(), header.value.as_str()))
                .collect(),
            fields: target
                .fields
                .iter()
                .map(|field| NormalizedField {
                    name: field.name.as_str(),
                    value: match &field.value {
                        UploadFieldValue::Static { value } => {
                            NormalizedFieldValue::Static(value.as_str())
                        }
                        UploadFieldValue::ContentCrc32C => NormalizedFieldValue::ContentCrc32C,
                        UploadFieldValue::ContentData => NormalizedFieldValue::ContentData,
                    },
                })
                .collect(),
        }
    }
}

/// A source of bytes to upload to a presigned upload target.
///
/// This abstracts over the requirements for file uploads, such as knowing
/// the payload size, so that we can avoid loading entire files into memory
/// wherever possible.
///
/// It also allows the upload target implementation to skip work that's
/// not needed for a particular target. For example, AWS S3 requires that
/// the client provide a checksum ahead of time, while GCS does not.
pub trait UploadBody {
    /// Total length of the body in bytes. Used for the `Content-Length`
    /// header on PUT uploads and for the multipart `ContentData` part length.
    fn length(&self) -> u64;

    /// Base64-encoded big-endian CRC32C of the body content.
    #[cfg(not(target_family = "wasm"))]
    fn compute_crc32c_base64(&self) -> impl Future<Output = Result<String>> + Send;

    /// Consume this body and produce a `reqwest::Body` for the upload
    /// request.
    fn into_reqwest_body(self) -> impl Future<Output = Result<reqwest::Body>> + Send;
}

/// In-memory implementation of [`UploadBody`].
impl UploadBody for Vec<u8> {
    fn length(&self) -> u64 {
        self.len() as u64
    }

    #[cfg(not(target_family = "wasm"))]
    async fn compute_crc32c_base64(&self) -> Result<String> {
        Ok(encode_crc32c_base64(CRC32C.checksum(self)))
    }

    async fn into_reqwest_body(self) -> Result<reqwest::Body> {
        Ok(reqwest::Body::from(self))
    }
}

/// `UploadBody` implementation backed by a file on disk. Used for streaming
/// artifact uploads without buffering the file content in memory.
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone)]
pub struct FileUploadBody {
    path: PathBuf,
}

#[cfg(feature = "local_fs")]
impl FileUploadBody {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[cfg(feature = "local_fs")]
impl UploadBody for FileUploadBody {
    fn length(&self) -> u64 {
        // Read metadata on demand so callers don't have to keep a pre-computed
        // size in sync with the file on disk. Returning 0 on a stat failure is
        // safe: reqwest will still stream the actual bytes, and any PUT paths
        // that use this for `Content-Length` already tolerate unknown lengths.
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    async fn compute_crc32c_base64(&self) -> Result<String> {
        use std::io::Read as _;
        let path = self.path.clone();

        // Compute the checksum as a blocking task for 2 reasons:
        // - Filesystem operations generally aren't natively async to begin with
        // - Checksum calculation is CPU-bound, and shouldn't tie up an async worker thread
        tokio::task::spawn_blocking(move || -> Result<String> {
            let mut file = std::fs::File::open(&path)
                .with_context(|| format!("Failed to open artifact file '{}'", path.display()))?;
            let mut digest = CRC32C.digest();
            let mut buf = vec![0u8; FILE_UPLOAD_CHUNK_SIZE];
            loop {
                let n = file.read(&mut buf).with_context(|| {
                    format!("Failed to read artifact file '{}'", path.display())
                })?;
                if n == 0 {
                    break;
                }
                digest.update(&buf[..n]);
            }
            Ok(encode_crc32c_base64(digest.finalize()))
        })
        .await
        .context("CRC32C computation task failed to join")?
    }

    async fn into_reqwest_body(self) -> Result<reqwest::Body> {
        let file = tokio::fs::File::open(&self.path)
            .await
            .with_context(|| format!("Failed to open artifact file '{}'", self.path.display()))?;
        Ok(reqwest::Body::wrap_stream(
            tokio_util::io::ReaderStream::new(file),
        ))
    }
}

#[derive(Copy, Clone)]
struct UploadErrorContext {
    transport: &'static str,
    failure: &'static str,
}

fn build_upload_request<'a>(
    http_client: &'a http_client::Client,
    target: &NormalizedUploadTarget<'_>,
    content_length: Option<u64>,
) -> Result<http_client::RequestBuilder<'a>> {
    let method = target.method.to_ascii_uppercase();
    let has_content_length = target
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(CONTENT_LENGTH_HEADER_NAME));

    let mut request = match method.as_str() {
        "GET" => http_client.get(target.url),
        "POST" => http_client.post(target.url),
        "PUT" => http_client.put(target.url),
        "DELETE" => http_client.delete(target.url),
        other => return Err(anyhow!("Unsupported HTTP method: {other}")),
    };

    for (name, value) in &target.headers {
        request = request.header(*name, *value);
    }

    if let Some(content_length) = content_length.filter(|_| !has_content_length) {
        request = request.header(CONTENT_LENGTH_HEADER_NAME, content_length.to_string());
    }

    Ok(request)
}

async fn ensure_upload_succeeded(
    response: http_client::Response,
    error_context: UploadErrorContext,
) -> Result<()> {
    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let status_err = HttpStatusError {
        status: status.as_u16(),
        body: body.clone(),
    };
    Err(anyhow::Error::new(status_err).context(format!(
        "{} failed with status {status}: {body}",
        error_context.failure
    )))
}

async fn send_upload_request(
    http_client: &http_client::Client,
    target: &NormalizedUploadTarget<'_>,
    body: impl Into<reqwest::Body>,
    content_length: u64,
    error_context: UploadErrorContext,
) -> Result<()> {
    let response = build_upload_request(http_client, target, Some(content_length))?
        .body(body)
        .send()
        .await
        .context(error_context.transport)?;

    ensure_upload_succeeded(response, error_context).await
}

pub(crate) async fn upload_to_target(
    http_client: &http_client::Client,
    target: &UploadTarget,
    body: impl UploadBody,
) -> Result<()> {
    let normalized = NormalizedUploadTarget::from(target);
    let error_context = UploadErrorContext {
        transport: "Failed to upload to presigned URL",
        failure: "Upload",
    };
    send_upload(http_client, &normalized, body, None, error_context).await
}

/// Internal dispatcher shared between [`upload_to_target`] and
/// [`upload_file_to_target`]. Routes the request to either a plain body upload
/// or a multipart form upload based on whether the target has form fields.
async fn send_upload(
    http_client: &http_client::Client,
    target: &NormalizedUploadTarget<'_>,
    body: impl UploadBody,
    precomputed_crc32c: Option<String>,
    error_context: UploadErrorContext,
) -> Result<()> {
    if target.fields.is_empty() {
        let length = body.length();
        let reqwest_body = body.into_reqwest_body().await?;
        send_upload_request(http_client, target, reqwest_body, length, error_context).await
    } else {
        #[cfg(not(target_family = "wasm"))]
        {
            send_multipart_upload(http_client, target, body, precomputed_crc32c, error_context)
                .await
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (http_client, body, precomputed_crc32c, error_context);
            Err(anyhow!(
                "Multipart upload targets are not supported on wasm"
            ))
        }
    }
}

/// Send a `multipart/form-data` request.
///
/// Callers that already computed the CRC32C (for example `upload_file_to_target`,
/// which needs it for `confirmFileArtifactUpload`) pass it through via
/// `precomputed_crc32c` so we don't hash the body twice. Otherwise we only
/// compute the checksum when requested by the upload target.
#[cfg(not(target_family = "wasm"))]
async fn send_multipart_upload(
    http_client: &http_client::Client,
    target: &NormalizedUploadTarget<'_>,
    body: impl UploadBody,
    precomputed_crc32c: Option<String>,
    error_context: UploadErrorContext,
) -> Result<()> {
    let form = build_multipart_form(target, body, precomputed_crc32c).await?;
    let request = build_upload_request(http_client, target, None)?;
    let response = request
        .multipart(form)
        .send()
        .await
        .context(error_context.transport)?;
    ensure_upload_succeeded(response, error_context).await
}

#[cfg(not(target_family = "wasm"))]
async fn build_multipart_form(
    target: &NormalizedUploadTarget<'_>,
    body: impl UploadBody,
    precomputed_crc32c: Option<String>,
) -> Result<reqwest::multipart::Form> {
    use reqwest::multipart::{Form, Part};

    let needs_crc32c = target
        .fields
        .iter()
        .any(|field| matches!(field.value, NormalizedFieldValue::ContentCrc32C));
    let crc32c_base64 = match (precomputed_crc32c, needs_crc32c) {
        (Some(crc), _) => Some(crc),
        (None, true) => Some(body.compute_crc32c_base64().await?),
        (None, false) => None,
    };
    let content_length = body.length();
    let content_body = body.into_reqwest_body().await?;

    let mut form = Form::new();
    let mut content_body_cell = Some((content_body, content_length));
    for field in &target.fields {
        let name = field.name.to_string();
        match &field.value {
            NormalizedFieldValue::Static(value) => {
                form = form.text(name, value.to_string());
            }
            NormalizedFieldValue::ContentCrc32C => {
                let crc = crc32c_base64.as_deref().ok_or_else(|| {
                    anyhow!(
                        "Internal error: ContentCrc32C field '{}' requires a computed checksum",
                        field.name
                    )
                })?;
                form = form.text(name, crc.to_string());
            }
            NormalizedFieldValue::ContentData => {
                let (body, length) = content_body_cell.take().ok_or_else(|| {
                    anyhow!("Multipart upload must contain exactly one ContentData field")
                })?;
                form = form.part(name, Part::stream_with_length(body, length));
            }
        }
    }
    if content_body_cell.is_some() {
        return Err(anyhow!(
            "Multipart upload must contain exactly one ContentData field"
        ));
    }
    Ok(form)
}

#[cfg(not(target_family = "wasm"))]
fn encode_crc32c_base64(crc32c: u32) -> String {
    // Storage providers expect the checksum as base64 of the raw big-endian CRC32C bytes,
    // not the more human-readable hex string we typically log.
    STANDARD.encode(crc32c.to_be_bytes())
}

/// Upload a file artifact. Always computes the base64 CRC32C so callers can
/// pass it to `confirmFileArtifactUpload`.
#[cfg(feature = "local_fs")]
pub(crate) async fn upload_file_to_target(
    http_client: &http_client::Client,
    target: &FileArtifactUploadTargetInfo,
    body: impl UploadBody,
) -> Result<String> {
    let normalized = NormalizedUploadTarget::from(target);
    let error_context = UploadErrorContext {
        transport: "Failed to upload artifact bytes",
        failure: "Artifact upload",
    };

    // `confirmFileArtifactUpload` always needs the CRC32C, so we always compute
    // it up front and then hand it to the dispatcher so the multipart path
    // doesn't recompute it.
    let crc32c_base64 = body.compute_crc32c_base64().await?;
    send_upload(
        http_client,
        &normalized,
        body,
        Some(crc32c_base64.clone()),
        error_context,
    )
    .await?;
    Ok(crc32c_base64)
}

#[cfg(test)]
#[path = "presigned_upload_tests.rs"]
mod tests;
