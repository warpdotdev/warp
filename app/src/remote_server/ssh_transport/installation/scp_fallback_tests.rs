use std::fs;
use std::io::Write as _;

use flate2::write::GzEncoder;
use flate2::Compression;
use mockito::Server;
use tempfile::tempdir;

use super::*;

fn valid_tarball() -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = tar::Builder::new(&mut encoder);
        let mut header = tar::Header::new_gnu();
        let body = b"fake remote server";
        header.set_path("oz").unwrap();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, &body[..]).unwrap();
        archive.finish().unwrap();
    }
    encoder.finish().unwrap()
}

fn invalid_tarball() -> Vec<u8> {
    b"not a gzip tarball".to_vec()
}

fn cache_path(tempdir: &tempfile::TempDir) -> PathBuf {
    tempdir.path().join("cache").join("oz.tar.gz")
}

fn cache_temp_dir(tempdir: &tempfile::TempDir) -> PathBuf {
    tempdir.path().join("cache").join(".tmp")
}

#[tokio::test]
async fn retry_uses_clean_temp_file_after_invalid_download() {
    let tempdir = tempdir().unwrap();
    let cache_path = cache_path(&tempdir);
    let mut server = Server::new_async().await;
    let invalid = invalid_tarball();
    let valid = valid_tarball();
    let first_attempt = server
        .mock("GET", "/oz.tar.gz")
        .with_status(200)
        .with_body(invalid)
        .expect(1)
        .create_async()
        .await;
    let retry_attempt = server
        .mock("GET", "/oz.tar.gz")
        .with_status(200)
        .with_body(valid.clone())
        .expect(1)
        .create_async()
        .await;

    download_remote_server_tarball_to_cache(&format!("{}/oz.tar.gz", server.url()), &cache_path)
        .await
        .unwrap();

    first_attempt.assert_async().await;
    retry_attempt.assert_async().await;
    assert_eq!(fs::read(&cache_path).unwrap(), valid);
    assert!(is_valid_cached_tarball(&cache_path).await);
    assert!(fs::read_dir(cache_temp_dir(&tempdir))
        .unwrap()
        .next()
        .is_none());
}

#[tokio::test]
async fn invalid_cached_tarball_is_discarded_and_redownloaded() {
    let tempdir = tempdir().unwrap();
    let cache_path = cache_path(&tempdir);
    fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    fs::write(&cache_path, invalid_tarball()).unwrap();

    let mut server = Server::new_async().await;
    let valid = valid_tarball();
    let download = server
        .mock("GET", "/oz.tar.gz")
        .with_status(200)
        .with_body(valid.clone())
        .expect(1)
        .create_async()
        .await;

    let cached_path =
        cached_remote_server_tarball_from(&format!("{}/oz.tar.gz", server.url()), &cache_path)
            .await
            .unwrap();

    download.assert_async().await;
    assert_eq!(cached_path, cache_path);
    assert_eq!(fs::read(&cache_path).unwrap(), valid);
    assert!(is_valid_cached_tarball(&cache_path).await);
}

#[tokio::test]
async fn valid_cached_tarball_is_reused_without_download() {
    let tempdir = tempdir().unwrap();
    let cache_path = cache_path(&tempdir);
    let valid = valid_tarball();
    fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    fs::write(&cache_path, &valid).unwrap();

    let mut server = Server::new_async().await;
    let unexpected_download = server
        .mock("GET", "/oz.tar.gz")
        .with_status(500)
        .expect(0)
        .create_async()
        .await;

    let cached_path =
        cached_remote_server_tarball_from(&format!("{}/oz.tar.gz", server.url()), &cache_path)
            .await
            .unwrap();

    unexpected_download.assert_async().await;
    assert_eq!(cached_path, cache_path);
    assert_eq!(fs::read(&cache_path).unwrap(), valid);
}

#[tokio::test]
async fn retry_exhaustion_returns_client_download_error_without_cache_file() {
    let tempdir = tempdir().unwrap();
    let cache_path = cache_path(&tempdir);
    let mut server = Server::new_async().await;
    let invalid = invalid_tarball();
    let invalid_downloads = server
        .mock("GET", "/oz.tar.gz")
        .with_status(200)
        .with_body(invalid)
        .expect(REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS)
        .create_async()
        .await;

    let err = download_remote_server_tarball_to_cache(
        &format!("{}/oz.tar.gz", server.url()),
        &cache_path,
    )
    .await
    .unwrap_err();

    invalid_downloads.assert_async().await;
    assert!(err
        .to_string()
        .contains("Remote-server tarball client download failed after 3 attempts"));
    assert!(!cache_path.exists());
    assert!(fs::read_dir(cache_temp_dir(&tempdir))
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn invalid_cached_tarball_fails_gzip_tar_validation() {
    let tempdir = tempdir().unwrap();
    let invalid_path = tempdir.path().join("oz.tar.gz");
    let mut file = fs::File::create(&invalid_path).unwrap();
    file.write_all(&invalid_tarball()).unwrap();

    assert!(validate_gzip_tarball(&invalid_path).is_err());
}
