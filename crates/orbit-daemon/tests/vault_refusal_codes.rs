//! Refusal metadata crosses the real HTTP adapter without interpreting fallback prose.
use anyhow::Result;
use orbit_core::vault::VaultCall;
use orbit_daemon::hub_notes::NoteHub;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn vault_refusals_keep_the_cause_fields_and_signed_upstream_number() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0_u8; 4096];
        let received = socket.read(&mut request).await?;
        anyhow::ensure!(received > 0, "the HTTP adapter sent no request");
        let body = json!({"code":"vault_sync_refused", "message":"The sync service refused the read.",
            "message_code":"vault_invalid_request", "message_fields":{"field":"page","rule":"greater_than_equal"},
            "refusal":{"code":"vault_sync_refused","upstream_code":-7,"max_page_size":100}}).to_string();
        socket.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        Result::<()>::Ok(())
    });
    let hub = NoteHub {
        base: format!("http://{address}/").parse()?,
        token: "synthetic-token".into(),
        op: orbit_ipc::new_op(),
    };
    let outcome = orbit_daemon::hub_vault::call(&hub, &VaultCall::Config).await;
    server.await??;
    let wire = serde_json::to_value(outcome)?;
    assert_eq!(wire["outcome"], "refused");
    assert_eq!(wire["message_code"], "vault_invalid_request");
    assert_eq!(
        wire["message_fields"],
        json!({"field":"page","rule":"greater_than_equal"})
    );
    assert_eq!(wire["numbers"]["upstream_code"], -7);
    assert_eq!(wire["numbers"]["max_page_size"], 100);
    assert_eq!(wire["message"], "The sync service refused the read.");
    Ok(())
}
