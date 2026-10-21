use super::*;

#[tokio::test]
async fn escaped_annotations_preserve_the_receipt_and_exact_digest_for_both_formats() -> Result<()>
{
    for format in [SourceFormat::Png, SourceFormat::SelectedText] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        list(&mut runtime, &mut receiver).await?;
        runtime
            .state
            .views
            .source_inventory
            .as_mut()
            .context("inventory")?
            .selected_text_available = true;
        let annotation = "\u{0001}".repeat(2200);
        runtime
            .source_capture_as(TOKEN, annotation.clone(), format)
            .await?;
        let request = receiver.recv().await.context("capture command")?;
        let (event, expected) = if format == SourceFormat::Png {
            (screenshot(&request)?, image()?)
        } else {
            let text = "¿Cafe\u{301}? 🪐\r\n";
            (
                json!({"event":"response","id":request["id"],"ok":true,
                "result":{"binding":request["window_binding"],"clock_domain":"mach_absolute_time",
                "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
                "mime_type":"text/plain; charset=utf-8","source":"native_selected_text","text":text,
                "range":{"location":0,"length":text.encode_utf16().count(),"unit":"utf16_code_units"},
                "ax_binding_quality":"unique_owner_title_bounds","selection_scope":"application_focused_element",
                "sensitivity":"not_certified"}}),
                text.as_bytes().to_vec(),
            )
        };
        runtime.native_event(event).await?;
        let receipt = runtime
            .state
            .views
            .source_receipt
            .as_ref()
            .context("memory receipt")?;
        assert_eq!(receipt.intent.annotation, annotation);
        assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&expected)));
        assert_eq!(receipt.bytes, expected.len() as u64);
        assert!(receiver.try_recv().is_err());
    }
    Ok(())
}
#[tokio::test]
async fn annotation_limit_is_checked_before_dispatch_without_a_disk_envelope_limit() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    list(&mut runtime, &mut receiver).await?;
    assert!(
        runtime
            .source_capture_as(TOKEN, "\u{0001}".repeat(4097), SourceFormat::Png)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    assert!(runtime.state.views.source_inventory.is_some());
    let annotation = "\u{0001}".repeat(4096);
    runtime
        .source_capture_as(TOKEN, annotation.clone(), SourceFormat::Png)
        .await?;
    let command = receiver.recv().await.context("capture")?;
    runtime.native_event(screenshot(&command)?).await?;
    let receipt = runtime
        .state
        .views
        .source_receipt
        .as_ref()
        .context("receipt")?;
    assert_eq!(receipt.intent.annotation, annotation);
    assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(image()?)));
    assert!(receiver.try_recv().is_err());
    Ok(())
}
