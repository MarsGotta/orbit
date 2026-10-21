#![allow(clippy::unwrap_used)] // Fixture assertions only; never production.
//! What each download is measured against, and what the refusal says.
//!
//! The failure being fixed is a real one: the updater on 0.3.9 told the
//! owner that `install.sh` was "demasiado grande para un manifiesto". It is not
//! a manifest, and the manifest — 162 bytes — was fine. One `fetch` measured
//! both files against one 8 KiB number, and the installer had grown to
//! 11 171 bytes.
//!
//! These fixtures serve the bodies over a real socket, because what is being
//! fixed lives in `fetch`: which ceiling applies and when the measurement
//! happens. What they do NOT prove is anything about the published origin —
//! that it serves this installer, that macOS runs it, that the update
//! completes on the owner's machine — which needs the Mac.

use super::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

/// The installer this repository publishes, weighed as bash will see it.
fn published_installer() -> Vec<u8> {
    std::fs::read(installer_path()).unwrap()
}

fn installer_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release/install.sh")
}

/// An origin that answers one request with exactly these bytes and closes.
async fn origin_serving(response: Vec<u8>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        // Some refusals close after headers; keep the fixture's write cause
        // visible while allowing the client to stop reading the body.
        if let Err(error) = socket.write_all(&response).await {
            eprintln!("fixture peer closed while writing response: {error}");
            return;
        }
        if let Err(error) = socket.flush().await {
            eprintln!("fixture peer closed while flushing response: {error}");
        }
    });
    (origin, task)
}

async fn origin_serving_pair(manifest: &[u8], installer: &[u8]) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let manifest = declaring_its_length(manifest);
    let installer = declaring_its_length(installer);
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let count = socket.read(&mut request).await.unwrap();
            let route = String::from_utf8_lossy(&request[..count]);
            let response = if route.contains("/latest.txt") {
                &manifest
            } else {
                &installer
            };
            socket.write_all(response).await.unwrap();
            socket.flush().await.unwrap();
        }
    });
    (origin, task)
}

/// What a normal origin sends: a declared length, so the size is known before
/// a single byte of body is read.
fn declaring_its_length(body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// What an origin sends when it will not say how big the answer is: the body
/// ends when the connection does, so the only bound is the one we impose.
fn declaring_nothing(body: &[u8]) -> Vec<u8> {
    let mut bytes =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n".to_vec();
    bytes.extend_from_slice(body);
    bytes
}

async fn serve_and_fetch(response: Vec<u8>, download: &Download) -> Result<String, Refusal> {
    let (origin, server) = origin_serving(response).await;
    let fetched = Updater::new(origin).unwrap().fetch(download).await;
    server.await.unwrap();
    fetched
}

// ------------------------------------------- each file against its own ceiling

#[tokio::test]
async fn the_published_installer_that_broke_update_now_arrives_whole() {
    // The failure the owner saw, at the size that caused it: an installer of
    // the baseline's bytes measured against the manifest's ceiling is refused.
    // Built rather than read from disk, so the reproduction does not depend on
    // what `install.sh` happens to measure today.
    let as_big_as_then = vec![b'#'; INSTALLER_BASELINE_BYTES];
    assert!(as_big_as_then.len() > MANIFEST.ceiling);
    let as_a_manifest = serve_and_fetch(declaring_its_length(&as_big_as_then), &MANIFEST).await;
    assert!(as_a_manifest.is_err());

    // The installer this tree publishes, measured against its own ceiling,
    // arrives byte for byte.
    let installer = published_installer();
    let fetched = serve_and_fetch(declaring_its_length(&installer), &INSTALLER)
        .await
        .unwrap();
    assert_eq!(fetched.as_bytes(), installer.as_slice());
}

#[tokio::test]
async fn a_body_that_lands_exactly_on_its_ceiling_is_accepted() {
    for download in [&MANIFEST, &INSTALLER] {
        let exact = vec![b'x'; download.ceiling];
        assert_eq!(
            serve_and_fetch(declaring_its_length(&exact), download)
                .await
                .unwrap()
                .len(),
            download.ceiling,
            "{}",
            download.file
        );
    }
}

#[tokio::test]
async fn an_undeclared_body_that_fits_is_still_read_to_the_end() {
    let body = vec![b'x'; MANIFEST.ceiling];
    assert_eq!(
        serve_and_fetch(declaring_nothing(&body), &MANIFEST)
            .await
            .unwrap()
            .len(),
        MANIFEST.ceiling
    );
}

#[tokio::test]
async fn an_origin_that_answers_with_an_error_is_refused_before_any_ceiling() {
    let (origin, server) = origin_serving(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    )
    .await;
    let error = Updater::new(origin.clone())
        .unwrap()
        .fetch(&INSTALLER)
        .await
        .unwrap_err()
        .to_string();
    server.await.unwrap();
    assert!(error.contains("404"), "{error}");
    assert!(
        error.contains(&origin),
        "missing URL in HTTP refusal: {error}"
    );
}

#[tokio::test]
async fn a_redirect_on_the_same_origin_reaches_the_manifest() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for step in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let count = socket.read(&mut request).await.unwrap();
            let line = String::from_utf8_lossy(&request[..count]);
            let expected = if step == 0 {
                "/latest.txt"
            } else {
                "/mirror/latest.txt"
            };
            assert!(line.starts_with(&format!("GET {expected} ")), "{line}");
            let response = if step == 0 {
                b"HTTP/1.1 302 Found\r\nLocation: /mirror/latest.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            } else {
                declaring_its_length(b"version=0.3.12")
            };
            socket.write_all(&response).await.unwrap();
        }
    });
    let fetched = Updater::new(origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(fetched, "version=0.3.12");
}

#[tokio::test]
async fn a_redirect_on_the_same_host_and_scheme_can_change_port() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", first.local_addr().unwrap());
    let target = format!("http://{}/latest.txt", second.local_addr().unwrap());
    let first_server = tokio::spawn(async move {
        let (mut socket, _) = first.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let second_server = tokio::spawn(async move {
        let (mut socket, _) = second.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(&declaring_its_length(b"version=0.3.12"))
            .await
            .unwrap();
    });
    let fetched = Updater::new(origin).unwrap().fetch(&MANIFEST).await;
    first_server.await.unwrap();
    second_server.abort();
    assert_eq!(fetched.unwrap(), "version=0.3.12");
}

#[tokio::test]
async fn a_redirect_to_another_host_names_the_rejected_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{}/latest.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            address.port()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let refusal = Updater::new(origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(matches!(
        &refusal,
        Refusal::Update {
            cause: UpdateCause::RedirectOutsideOrigin { target_origin, .. }
        } if target_origin.contains("localhost")
    ));
    let message = refusal.to_string();
    assert!(message.contains("localhost"), "{message}");
    assert!(message.contains("redirect_outside_origin"), "{message}");
}

#[tokio::test]
async fn size_refusals_distinguish_declared_from_observed() {
    for download in [MANIFEST, INSTALLER] {
        let body = vec![b'x'; download.ceiling + 1];
        let declared = serve_and_fetch(declaring_its_length(&body), &download)
            .await
            .unwrap_err();
        assert!(
            matches!(&declared, Refusal::Update { cause: UpdateCause::TooLarge { file, bytes, max_bytes, source: UpdateSizeSource::Declared } }
                if file == download.file && *bytes == body.len() as u64 && *max_bytes == download.ceiling as u64),
            "{declared:?}"
        );
        assert!(declared.to_string().contains(download.file), "{declared}");
        assert!(declared.to_string().contains("declared"), "{declared}");
        let observed = serve_and_fetch(declaring_nothing(&body), &download)
            .await
            .unwrap_err();
        assert!(
            matches!(&observed, Refusal::Update { cause: UpdateCause::TooLarge { file, bytes, max_bytes, source: UpdateSizeSource::Observed } }
                if file == download.file && *bytes > download.ceiling as u64 && *bytes <= body.len() as u64 && *max_bytes == download.ceiling as u64),
            "{observed:?}"
        );
        assert!(observed.to_string().contains(download.file), "{observed}");
        assert!(observed.to_string().contains("observed"), "{observed}");
    }
}

#[tokio::test]
async fn a_silent_local_origin_expires_with_measured_ms() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _socket = socket;
        std::future::pending::<()>().await;
    });
    let updater = Updater::with_timeout(origin.clone(), Duration::from_millis(20)).unwrap();
    let refusal = updater.fetch(&MANIFEST).await.unwrap_err();
    server.abort();
    match server.await {
        Err(error) if error.is_cancelled() => {}
        Ok(()) => {}
        Err(error) => panic!("loopback server failed: {error}"),
    }
    assert!(
        matches!(refusal, Refusal::Update { cause: UpdateCause::Deadline { url, waited_ms, deadline_ms } } if url == format!("{origin}/latest.txt") && waited_ms >= 20 && deadline_ms == 20)
    );
}

#[tokio::test]
async fn a_refused_local_connection_names_the_cause() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let refusal = Updater::new(origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap_err();
    assert!(
        matches!(
            refusal,
            Refusal::Update {
                cause: UpdateCause::NoResponse {
                    cause: UpdateNetworkCause::ConnectionRefused,
                    ..
                }
            }
        ),
        "{refusal}"
    );
}

struct FailingDns;
impl reqwest::dns::Resolve for FailingDns {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async {
            Err(
                Box::new(std::io::Error::other("synthetic DNS lookup failure"))
                    as Box<dyn std::error::Error + Send + Sync>,
            )
        })
    }
}

#[tokio::test]
async fn a_failing_synthetic_resolver_names_dns_without_external_traffic() {
    let timeout = Duration::from_secs(1);
    let updater = Updater {
        origin: "http://fixture.invalid".into(),
        client: reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(FailingDns))
            .timeout(timeout)
            .build()
            .unwrap(),
        timeout,
    };
    let refusal = updater.fetch(&MANIFEST).await.unwrap_err();
    assert!(
        matches!(
            refusal,
            Refusal::Update {
                cause: UpdateCause::NoResponse {
                    cause: UpdateNetworkCause::Dns,
                    ..
                }
            }
        ),
        "{refusal}"
    );
}

#[tokio::test]
async fn plaintext_from_a_local_tls_origin_names_tls() {
    let (origin, server) = origin_serving(declaring_its_length(b"not TLS")).await;
    let https_origin = origin.replacen("http://", "https://", 1);
    let refusal = Updater::new(https_origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(
        matches!(
            refusal,
            Refusal::Update {
                cause: UpdateCause::NoResponse {
                    cause: UpdateNetworkCause::Tls,
                    ..
                }
            }
        ),
        "{refusal}"
    );
}

#[tokio::test]
async fn a_redirect_loop_is_bounded_with_its_count() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..=MAX_REDIRECTS {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /latest.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        }
    });
    let refusal = Updater::new(origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(matches!(
        refusal,
        Refusal::Update {
            cause: UpdateCause::RedirectLimit { followed, max_redirects, .. }
        } if followed == MAX_REDIRECTS && max_redirects == MAX_REDIRECTS
    ));
}

#[tokio::test]
async fn a_redirect_without_location_names_the_missing_header() {
    let refusal = serve_and_fetch(
        b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        &MANIFEST,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        refusal,
        Refusal::Update {
            cause: UpdateCause::InvalidRedirect { cause, .. }
        } if cause.contains("Location")
    ));
}

#[test]
fn an_http_client_build_failure_has_its_own_cause() {
    let error = reqwest::Client::builder()
        .user_agent("\n")
        .build()
        .unwrap_err();
    let refusal = client_build_error(error);
    assert!(matches!(
        &refusal,
        Refusal::Update {
            cause: UpdateCause::ClientBuild { cause }
        } if !cause.is_empty()
    ));
    assert!(refusal.to_string().contains("client_build"));
}

#[tokio::test]
async fn a_redirect_to_another_scheme_is_rejected_before_tls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: https://{address}/latest.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let refusal = Updater::new(origin)
        .unwrap()
        .fetch(&MANIFEST)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(matches!(
        refusal,
        Refusal::Update {
            cause: UpdateCause::RedirectOutsideOrigin { target_origin, .. }
        } if target_origin.starts_with("https://")
    ));
}

#[tokio::test]
async fn a_malformed_local_response_keeps_the_transport_detail() {
    let refusal = serve_and_fetch(b"not an HTTP response".to_vec(), &MANIFEST)
        .await
        .unwrap_err();
    assert!(
        matches!(&refusal, Refusal::Update { cause: UpdateCause::NoResponse { cause: UpdateNetworkCause::Other { detail }, .. } } if !detail.is_empty()),
        "{refusal}"
    );
}

#[test]
fn invalid_origin_does_not_echo_credentials() {
    let refusal = Updater::new("https://user:secret@origin.invalid")
        .err()
        .unwrap();
    assert!(matches!(
        refusal,
        Refusal::Update {
            cause: UpdateCause::InvalidOrigin { .. }
        }
    ));
    assert!(!refusal.to_string().contains("secret"));
}

#[tokio::test]
async fn http_and_utf8_failures_are_typed_refusals() {
    let http = serve_and_fetch(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        &MANIFEST,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            http,
            Refusal::Update {
                cause: UpdateCause::HttpStatus { status: 404, .. }
            }
        ),
        "{http:#}"
    );

    let utf8 = serve_and_fetch(declaring_its_length(&[b'a', 0xff]), &MANIFEST)
        .await
        .unwrap_err();
    assert!(
        matches!(
            utf8,
            Refusal::Update {
                cause: UpdateCause::InvalidUtf8 { byte: 1, .. }
            }
        ),
        "{utf8:#}"
    );
    assert!(utf8.to_string().contains('1'), "{utf8}");
}

// ------------------------------------------- the installer this tree publishes

#[test]
fn the_installer_this_tree_publishes_fits_under_its_ceiling() {
    // Weighed on every run, so an installer that outgrows what the updater
    // will download is caught here and not on the owner's machine after
    // publishing. Growing or shrinking inside the ceiling needs no edit here
    // or in `update.rs`; only crossing it does.
    assert_eq!(
        INSTALLER_CEILING_BYTES, 16_756,
        "11 171 B plus 50 % for the new adapter"
    );
    let size = published_installer().len();
    assert!(
        size <= INSTALLER.ceiling,
        "scripts/release/install.sh measures {size} bytes and the installer's ceiling is {} \
         ({INSTALLER_BASELINE_BYTES} of baseline plus {INSTALLER_HEADROOM_PERCENT} %). \
         Do not just raise the ceiling: a larger ceiling with no reason is the same \
         defect with a later date. Derive the baseline again from what it measures \
         today, in crates/orbit-app-core/src/update/download.rs.",
        INSTALLER.ceiling
    );
}

#[tokio::test]
async fn a_mismatched_installer_digest_is_refused_before_execution() {
    let script = "#!/bin/bash\nexit 0\n";
    let manifest = format!("version=0.3.14\ninstaller_sha256={}\n", "a".repeat(64));
    let (origin, server) = origin_serving_pair(manifest.as_bytes(), script.as_bytes()).await;
    let result = Updater::new(origin).unwrap().installer_script().await;
    server.abort();
    let error = result.unwrap_err();
    assert!(error.to_string().contains(&"a".repeat(64)), "{error}");
}

#[tokio::test]
async fn a_matching_installer_digest_is_verified() {
    let script = "#!/bin/bash\nexit 0\n";
    let digest = format!("{:x}", Sha256::digest(script.as_bytes()));
    let manifest = format!("version=0.3.14\ninstaller_sha256={digest}\n");
    let (origin, server) = origin_serving_pair(manifest.as_bytes(), script.as_bytes()).await;
    let (downloaded, verified) = Updater::new(origin)
        .unwrap()
        .installer_script()
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(downloaded, script);
    assert!(verified);
}
