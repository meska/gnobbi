#![cfg(all(feature = "sdk", not(target_family = "wasm")))]

use std::time::Duration;

use matrix_engine::sdk::{Client, HttpError, config::RequestConfig};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn client(address: std::net::SocketAddr) -> Client {
    Client::builder()
        .homeserver_url(format!("http://{address}"))
        .sliding_sync_version_builder(matrix_engine::sdk::sliding_sync::VersionBuilder::None)
        .server_versions([matrix_engine::sdk::ruma::api::MatrixVersion::V1_0])
        .build()
        .await
        .unwrap()
}

fn config(limit: Option<usize>) -> RequestConfig {
    RequestConfig::new()
        .retry_limit(3)
        .timeout(Duration::from_secs(2))
        .max_response_size(limit)
}

async fn download(client: &Client, limit: usize) -> Result<Vec<u8>, HttpError> {
    download_with_config(client, config(Some(limit))).await
}

async fn download_with_config(
    client: &Client,
    config: RequestConfig,
) -> Result<Vec<u8>, HttpError> {
    #[allow(deprecated)]
    let request = matrix_engine::sdk::ruma::api::client::media::get_content::v3::Request::from_url(
        matrix_engine::sdk::ruma::mxc_uri!("mxc://fixture.invalid/body"),
    )
    .unwrap();
    client
        .send(request)
        .with_request_config(config)
        .await
        .map(|response| response.file)
}

// Keep serving until the SDK returns, so retries are counted rather than hidden by a closed listener.
async fn serve_response(
    listener: TcpListener,
    response: Vec<u8>,
    mut done: oneshot::Receiver<()>,
) -> usize {
    let mut requests = 0;
    loop {
        tokio::select! {
            biased;
            accepted = listener.accept() => {
                let (mut stream, _) = accepted.unwrap();
                read_request(&mut stream).await;
                requests += 1;
                stream.write_all(&response).await.unwrap();
                stream.shutdown().await.unwrap();
            }
            _ = &mut done => return requests,
        }
    }
}

#[test]
fn edge_content_length_rejects_before_any_body_is_sent() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = client(listener.local_addr().unwrap()).await;
        let (rejected, rejection) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n")
                .await
                .unwrap();
            // Withhold the entire body: only the header can cause rejection.
            rejection.await.unwrap();
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(150), listener.accept())
                    .await
                    .is_err(),
                "oversize must not retry"
            );
            1
        });
        let result = tokio::time::timeout(Duration::from_secs(3), download(&client, 8))
            .await
            .unwrap();
        assert!(
            matches!(result, Err(HttpError::ResponseTooLarge { limit: 8 })),
            "{result:?}"
        );
        rejected.send(()).unwrap();
        assert_eq!(server.await.unwrap(), 1);
    });
}

#[test]
fn edge_chunked_success_and_error_bodies_are_typed_and_never_retried() {
    runtime().block_on(async {
        for status in ["200 OK", "429 Too Many Requests", "500 Internal Server Error"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = client(listener.local_addr().unwrap()).await;
            let (done, completion) = oneshot::channel();
            // Valid Matrix error JSON, large enough to exceed the cap even without Content-Length.
            let body = br#"{"errcode":"M_LIMIT_EXCEEDED","error":"too large","retry_after_ms":1}"#;
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", body.len(), std::str::from_utf8(body).unwrap());
            let server = tokio::spawn(serve_response(listener, response.into_bytes(), completion));
            let result = tokio::time::timeout(Duration::from_secs(10), download(&client, 8)).await.unwrap();
            done.send(()).unwrap();
            let requests = server.await.unwrap();
            assert!(matches!(result, Err(HttpError::ResponseTooLarge { limit: 8 })), "{status}: {result:?}");
            assert_eq!(requests, 1, "{status}: permanent errors must not retry despite retry_limit(3)");
        }
    });
}

#[test]
fn edge_none_preserves_complete_media_and_non_media_responses() {
    runtime().block_on(async {
        for explicit_none in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = client(listener.local_addr().unwrap()).await;
            let (done, completion) = oneshot::channel();
            let body = "x".repeat(4096);
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let server = tokio::spawn(serve_response(listener, response.into_bytes(), completion));
            let request_config = if explicit_none { config(Some(8)).max_response_size(None) } else { RequestConfig::new() };
            assert_eq!(download_with_config(&client, request_config).await.unwrap(), body.as_bytes());
            done.send(()).unwrap();
            assert_eq!(server.await.unwrap(), 1);

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = self::client(listener.local_addr().unwrap()).await;
            let (done, completion) = oneshot::channel();
            let version = format!("v1.{}", "1".repeat(4096));
            let body = format!(r#"{{"versions":["v1.0","{version}"],"unstable_features":{{}}}}"#);
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let server = tokio::spawn(serve_response(listener, response.into_bytes(), completion));
            let mut request = client.send(matrix_engine::sdk::ruma::api::client::discovery::get_supported_versions::Request::new());
            if explicit_none {
                request = request.with_request_config(config(Some(8)).max_response_size(None));
            }
            let response = request.await.unwrap();
            assert_eq!(response.versions, ["v1.0".to_owned(), version]);
            done.send(()).unwrap();
            assert_eq!(server.await.unwrap(), 1);
        }
    });
}

async fn read_request(stream: &mut tokio::net::TcpStream) {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        headers.push(stream.read_u8().await.unwrap());
        assert!(headers.len() < 16_384);
    }
}

#[test]
fn bounded_response_accepts_n_rejects_n_plus_one_and_handles_zero() {
    runtime().block_on(async {
        for (limit, size) in [(8, 8), (8, 9), (0, 0), (0, 1)] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = client(listener.local_addr().unwrap()).await;
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_request(&mut stream).await;
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                stream.write_all(&vec![b'x'; size]).await.unwrap();
                stream.shutdown().await.unwrap();
                assert!(tokio::time::timeout(Duration::from_millis(150), listener.accept()).await.is_err(), "oversize must not retry");
            });
            let result = tokio::time::timeout(Duration::from_secs(3), download(&client, limit)).await.unwrap();
            if size <= limit {
                assert_eq!(result.unwrap(), vec![b'x'; size]);
            } else {
                assert!(matches!(result, Err(HttpError::ResponseTooLarge { limit: actual }) if actual == limit), "{result:?}");
            }
            server.await.unwrap();
        }
    });
}

#[test]
fn bounded_response_stops_before_server_releases_tail() {
    runtime().block_on(async {
        const LIMIT: usize = 8;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = client(listener.local_addr().unwrap()).await;
        let (rejected, rejection) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            // No Content-Length: only received bytes can trigger the limit.
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n8\r\nxxxxxxxx\r\n1\r\nx\r\n").await.unwrap();
            // The terminal chunk and remaining body are withheld until rejection.
            assert!(tokio::time::timeout(Duration::from_secs(3), rejection).await.unwrap().is_ok(), "client must reject before tail is sent");
            let mut byte = [0];
            assert_eq!(tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await.unwrap().unwrap(), 0, "client must drop the incomplete response");
            assert!(tokio::time::timeout(Duration::from_millis(150), listener.accept()).await.is_err(), "oversize must not retry");
        });
        let result = tokio::time::timeout(Duration::from_secs(3), download(&client, LIMIT)).await.unwrap();
        assert!(matches!(result, Err(HttpError::ResponseTooLarge { limit: LIMIT })), "{result:?}");
        rejected.send(()).unwrap();
        server.await.unwrap();
    });
}
