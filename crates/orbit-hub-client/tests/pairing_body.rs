//! What the client says to the hub when it pairs: the code and the device's
//! name, and this installation's id only when it has one. A synthetic hub on
//! loopback takes the request; nothing here is evidence about the real hub.
use anyhow::{Context, Result};
use orbit_core::notice::Op;
use orbit_hub_client::{HubEndpoint, hub};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CODE: &str = "codigo-sintetico-7";
const NAME: &str = "Mesa Vega";
const INSTALLATION: &str = "3c9a6f1e-8b2d-4e7a-9c5b-1d0e2f3a4b6c";
const DEVICE: &str = "7a1b2c3d-4e5f-4061-8728-394a5b6c7d8e";
const TOKEN: &str = "synthetic-not-a-credential";

/// A hub that takes one `POST /v1/pair`, hands its JSON body to the test and
/// pairs the device.
async fn pairing_hub() -> Result<(String, tokio::task::JoinHandle<Result<Value>>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let hub = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let head_end = loop {
            if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
            let read = socket.read(&mut chunk).await?;
            anyhow::ensure!(read > 0, "the request ended before its headers");
            bytes.extend_from_slice(&chunk[..read]);
        };
        let head = String::from_utf8_lossy(&bytes[..head_end]).to_ascii_lowercase();
        anyhow::ensure!(head.starts_with("post /v1/pair "), "{head}");
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .context("the pairing has no content-length")?
            .trim()
            .parse()?;
        while bytes.len() < head_end + length {
            let read = socket.read(&mut chunk).await?;
            anyhow::ensure!(read > 0, "the body was cut");
            bytes.extend_from_slice(&chunk[..read]);
        }
        let body: Value = serde_json::from_slice(&bytes[head_end..head_end + length])?;
        let answer = json!({"device_id": DEVICE, "token": TOKEN}).to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                )
                .as_bytes(),
            )
            .await?;
        Ok(body)
    });
    Ok((url, hub))
}

/// Pairs as the daemon does, against the synthetic hub: the body the hub
/// received, once the device got the identity the hub gave.
async fn body_of_pairing(mut endpoint: HubEndpoint) -> Result<Value> {
    let (url, hub) = pairing_hub().await?;
    endpoint.hub_url = Some(url);
    let op: Op = uuid::Uuid::new_v4().into();
    let paired = hub::pair(&endpoint, CODE, true, &op).await;
    let body = hub.await??;
    assert_eq!(paired?, (DEVICE.to_owned(), TOKEN.to_owned()));
    Ok(body)
}

#[tokio::test]
async fn pairing_without_an_installation_id_sends_todays_body() -> Result<()> {
    let body = body_of_pairing(HubEndpoint {
        device_name: NAME.into(),
        ..HubEndpoint::default()
    })
    .await?;
    assert_eq!(body, json!({"code": CODE, "name": NAME}));
    Ok(())
}

#[tokio::test]
async fn pairing_with_an_installation_id_adds_it_and_nothing_else() -> Result<()> {
    let body = body_of_pairing(HubEndpoint {
        device_name: NAME.into(),
        installation_id: Some(INSTALLATION.into()),
        ..HubEndpoint::default()
    })
    .await?;
    assert_eq!(
        body,
        json!({"code": CODE, "name": NAME, "installation_id": INSTALLATION})
    );
    Ok(())
}
