#![cfg(all(feature = "sdk", not(target_family = "wasm")))]

use std::{
    io::{Cursor, Read},
    time::Duration,
};

use matrix_engine::sdk::{
    Client, Error, HttpError,
    config::RequestConfig,
    media::{
        DefaultMediaFetcher, MediaFetcher, MediaFormat, MediaRequestParameters,
        MediaThumbnailSettings,
    },
    ruma::{
        api::MatrixVersion,
        events::room::{EncryptedFile, EncryptedFileHashes, MediaSource},
        owned_mxc_uri, uint,
    },
};
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
        .server_versions([MatrixVersion::V1_0])
        .build()
        .await
        .unwrap()
}

fn thumbnail(source: MediaSource) -> MediaRequestParameters {
    MediaRequestParameters {
        source,
        format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(uint!(320), uint!(240))),
    }
}

async fn serve(
    listener: TcpListener,
    body: Vec<u8>,
    mut done: oneshot::Receiver<()>,
) -> Vec<String> {
    let mut requests = Vec::new();
    loop {
        tokio::select! {
            biased;
            accepted = listener.accept() => {
                let (mut stream, _) = accepted.unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(stream.read_u8().await.unwrap());
                    assert!(headers.len() < 16_384);
                }
                requests.push(String::from_utf8(headers).unwrap());
                // Chunked avoids letting Content-Length alone exercise the cap.
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", body.len());
                stream.write_all(header.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
                stream.write_all(b"\r\n0\r\n\r\n").await.unwrap();
                stream.shutdown().await.unwrap();
            }
            _ = &mut done => return requests,
        }
    }
}

fn encrypted(plaintext: &[u8]) -> (Vec<u8>, Box<EncryptedFile>) {
    let mut input = Cursor::new(plaintext);
    let mut encryptor = matrix_sdk_crypto::AttachmentEncryptor::new(&mut input);
    let mut ciphertext = Vec::new();
    encryptor.read_to_end(&mut ciphertext).unwrap();
    let info = encryptor.finish();
    (
        ciphertext,
        Box::new(EncryptedFile::new(
            owned_mxc_uri!("mxc://fixture.invalid/encrypted-thumbnail"),
            info.encryption_info,
            info.hashes,
        )),
    )
}

#[test]
fn encrypted_thumbnail_over_cap_returns_typed_error_before_decrypt() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = client(listener.local_addr().unwrap()).await;
        let (ciphertext, file) = encrypted(b"thumbnail");
        assert_eq!(ciphertext.len(), 9);
        let request = thumbnail(MediaSource::Encrypted(file));
        let (done, completion) = oneshot::channel();
        let server = tokio::spawn(serve(listener, ciphertext, completion));
        let result = tokio::time::timeout(Duration::from_secs(5),
            DefaultMediaFetcher.fetch_media_content_with_request_config(
                &client, &request, RequestConfig::new().retry_limit(3).max_response_size(Some(8)),
            )).await.unwrap();
        done.send(()).unwrap();
        let requests = server.await.unwrap();
        assert!(matches!(result, Err(Error::Http(ref error)) if matches!(**error, HttpError::ResponseTooLarge { limit: 8 })), "{result:?}");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /_matrix/media/r0/download/fixture.invalid/encrypted-thumbnail"), "{}", requests[0]);
    });
}

#[test]
fn encrypted_thumbnail_corrupt_hash_within_cap_returns_no_plaintext() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = client(listener.local_addr().unwrap()).await;
        let (ciphertext, mut file) = encrypted(b"thumbnail");
        file.hashes = EncryptedFileHashes::with_sha256([0; 32]);
        let request = thumbnail(MediaSource::Encrypted(file));
        let (done, completion) = oneshot::channel();
        let server = tokio::spawn(serve(listener, ciphertext, completion));
        let result = tokio::time::timeout(Duration::from_secs(5),
            DefaultMediaFetcher.fetch_media_content_with_request_config(
                &client, &request, RequestConfig::new().max_response_size(Some(9)),
            )).await.unwrap();
        done.send(()).unwrap();
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(matches!(result, Err(Error::Io(ref error)) if error.kind() == std::io::ErrorKind::Other && error.to_string().contains("Hash mismatch")), "{result:?}");
    });
}

#[test]
fn default_trait_and_explicit_config_preserve_plain_and_encrypted_bytes() {
    runtime().block_on(async {
        let plaintext = vec![42; 4096];
        let (ciphertext, file) = encrypted(&plaintext);
        for (source, body) in [
            (
                MediaSource::Plain(owned_mxc_uri!("mxc://fixture.invalid/thumbnail")),
                plaintext.clone(),
            ),
            (MediaSource::Encrypted(file), ciphertext),
        ] {
            for cap in [None, Some(plaintext.len())] {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let client = client(listener.local_addr().unwrap()).await;
                let request = thumbnail(source.clone());
                let (done, completion) = oneshot::channel();
                let server = tokio::spawn(serve(listener, body.clone(), completion));
                let result = tokio::time::timeout(Duration::from_secs(5), async {
                    if cap.is_none() {
                        let fetcher: &dyn MediaFetcher = &DefaultMediaFetcher;
                        fetcher.fetch_media_content(&client, &request).await
                    } else {
                        DefaultMediaFetcher
                            .fetch_media_content_with_request_config(
                                &client,
                                &request,
                                RequestConfig::new().max_response_size(cap),
                            )
                            .await
                    }
                })
                .await
                .unwrap();
                done.send(()).unwrap();
                assert_eq!(server.await.unwrap().len(), 1);
                assert_eq!(result.unwrap(), plaintext);
            }
        }
    });
}

#[test]
fn plain_thumbnail_over_cap_returns_typed_error_without_retry() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = client(listener.local_addr().unwrap()).await;
        let request = thumbnail(MediaSource::Plain(owned_mxc_uri!("mxc://fixture.invalid/thumbnail")));
        let (done, completion) = oneshot::channel();
        let server = tokio::spawn(serve(listener, vec![7; 9], completion));
        let result = tokio::time::timeout(Duration::from_secs(5),
            DefaultMediaFetcher.fetch_media_content_with_request_config(
                &client, &request, RequestConfig::new().retry_limit(3).max_response_size(Some(8)),
            )).await.unwrap();
        done.send(()).unwrap();
        let requests = server.await.unwrap();
        assert!(matches!(result, Err(Error::Http(ref error)) if matches!(**error, HttpError::ResponseTooLarge { limit: 8 })), "{result:?}");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /_matrix/media/r0/thumbnail/fixture.invalid/thumbnail?"), "{}", requests[0]);
        assert!(requests[0].contains("width=320"));
        assert!(requests[0].contains("height=240"));
    });
}
