//! Speech playback at the daemon boundary, with synthetic loopback HTTP and a
//! fake native helper.
#![allow(clippy::unwrap_used)]

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

const TOKEN: &str = "synthetic-speech-token";

async fn one_shot_http(headers: &str, body: &[u8]) -> Result<reqwest::Url> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = reqwest::Url::parse(&format!("http://{}", listener.local_addr()?))?;
    let response = format!(
        "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    let body = body.to_vec();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let Ok(count) = socket.read(&mut chunk).await else {
                return;
            };
            if count == 0 {
                return;
            }
            request.extend_from_slice(&chunk[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = socket.write_all(&response).await;
        let _ = socket.write_all(&body).await;
    });
    Ok(base)
}

fn settings(base: reqwest::Url) -> orbit_daemon::config::Settings {
    orbit_daemon::config::Settings {
        hub_url: Some(base.to_string()),
        ..orbit_daemon::config::Settings::default()
    }
}

#[tokio::test]
async fn clip_becomes_one_bounded_playback_clip_command() -> Result<()> {
    let audio = b"RIFFsynthetic-wave";
    let base = one_shot_http(
        "Content-Type: audio/wav\r\nX-Orbit-Speech-Kind: clip\r\nX-Orbit-Speech-Provider: synthetic-profile\r\n",
        audio,
    )
    .await?;
    let (native, mut helper) = mpsc::channel::<Value>(2);
    orbit_daemon::speech::play(
        settings(base),
        TOKEN.into(),
        true,
        "respuesta sintética".into(),
        None,
        "playback-speech".into(),
        native,
        orbit_ipc::new_op(),
    )
    .await?;
    let command = helper.recv().await.context("playback_clip ausente")?;
    assert_eq!(command["command"], "playback_clip");
    assert_eq!(command["playback_id"], "playback-speech");
    assert_eq!(command["mime_type"], "audio/wav");
    assert_eq!(command["provider"], "synthetic-profile");
    assert_eq!(command["audio_base64"], STANDARD.encode(audio));
    assert!(helper.try_recv().is_err());
    Ok(())
}
