use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Mutex, OnceLock};

use cynic::QueryBuilder as _;
use log::{Level, Log, Metadata, Record};
use warp_core::channel::ChannelState;

use super::{GraphQLError, RequestOptions, build_graphql_request, send_graphql_request};
use crate::queries::list_warp_dev_images::{ListWarpDevImages, ListWarpDevImagesVariables};

struct TestLogger;

static LOGGER: TestLogger = TestLogger;
static LOGS: OnceLock<Mutex<Vec<(Level, String)>>> = OnceLock::new();

impl Log for TestLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        logs()
            .lock()
            .unwrap()
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

fn logs() -> &'static Mutex<Vec<(Level, String)>> {
    LOGS.get_or_init(|| Mutex::new(Vec::new()))
}

fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
    logs().lock().unwrap().clear();
}

struct RestoreServerRootUrl(String);

impl Drop for RestoreServerRootUrl {
    fn drop(&mut self) {
        let _ = ChannelState::override_server_root_url(self.0.clone());
    }
}

fn serve_one_html_403(listener: TcpListener) {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("listener should accept a client");
        let mut buf = [0u8; 16 * 1024];
        let _ = stream.read(&mut buf);
        let body = b"<html>not authorized</html>";
        let header = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(header.as_bytes())
            .expect("should write 403 headers");
        stream.write_all(body).expect("should write 403 body");
    });
}

#[test]
fn html_403_from_staging_logs_and_returns_staging_access_blocked() {
    init_logger();

    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind loopback listener");
    let addr: SocketAddr = listener
        .local_addr()
        .expect("listener should have an address");
    serve_one_html_403(listener);

    let previous_url = ChannelState::server_root_url().into_owned();
    let _restore = RestoreServerRootUrl(previous_url);
    ChannelState::override_server_root_url(format!("http://staging.warp.dev:{}", addr.port()))
        .expect("staging URL should parse");

    let client = http_client::Client::from_client_builder(
        reqwest::Client::builder()
            .no_proxy()
            .http1_only()
            .resolve("staging.warp.dev", addr),
    )
    .expect("should build HTTP client");

    let operation = ListWarpDevImages::build(ListWarpDevImagesVariables {});
    let req = build_graphql_request(&client, operation, RequestOptions::default())
        .expect("graphql request should build");

    let error =
        futures::executor::block_on(send_graphql_request::<ListWarpDevImages>(&client, req))
            .expect_err("staging HTML 403 should fail");

    assert!(
        matches!(error, GraphQLError::StagingAccessBlocked),
        "expected StagingAccessBlocked, got {error:?}"
    );
    assert!(
        logs().lock().unwrap().iter().any(|(level, message)| {
            *level == Level::Error && message.contains("not authorized for staging")
        }),
        "expected an error-level staging authorization log, got {:?}",
        logs().lock().unwrap()
    );
}
