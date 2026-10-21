//! Synthetic loopback hub: its refusal must survive the upload adapter.
use anyhow::Result;
use orbit_core::refusal::{Refusal, Service};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn capture_capacity_response_reaches_the_daemon_with_its_counts() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await?;
        let body = br#"{"code":"too_many","refusal":{"code":"too_many","resource":"capture","count":5,"max":4,"retry":{"kind":"after_fix"}}}"#;
        let response = format!(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(response.as_bytes()).await?;
        socket.write_all(body).await?;
        Result::<()>::Ok(())
    });
    let upload = orbit_daemon::hub::upload_capture(
        &format!("http://{address}/"),
        "synthetic-token",
        true,
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000002",
        vec![137, 80, 78, 71],
        &orbit_ipc::new_op(),
    )
    .await;
    let error = match upload {
        Ok(_) => anyhow::bail!("hub accepted synthetic capacity refusal"),
        Err(error) => error,
    };
    server.await??;
    let refusal = error.downcast_ref::<Refusal>();
    assert!(
        matches!(
            refusal,
            Some(Refusal::TooMany {
                count: 5,
                max: 4,
                ..
            })
        ),
        "the upload error lost the typed refusal: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn old_hub_busy_capture_does_not_claim_the_mac_is_busy() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await?;
        let body = br#"{"code":"capture_capacity","refusal":{"code":"busy","operation":"capture","retry_after_ms":null,"retry":{"kind":"after_fix"}}}"#;
        let response = format!(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(response.as_bytes()).await?;
        socket.write_all(body).await?;
        Result::<()>::Ok(())
    });
    let upload = orbit_daemon::hub::upload_capture(
        &format!("http://{address}/"),
        "synthetic-token",
        true,
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000002",
        vec![137, 80, 78, 71],
        &orbit_ipc::new_op(),
    )
    .await;
    let error = match upload {
        Ok(_) => anyhow::bail!("old hub accepted synthetic capacity refusal"),
        Err(error) => error,
    };
    server.await??;
    assert!(
        matches!(
            error.downcast_ref::<Refusal>(),
            Some(Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(429)
            })
        ),
        "old hub must report what it knows, HTTP 429, without claiming the Mac is busy: {error}"
    );
    Ok(())
}
