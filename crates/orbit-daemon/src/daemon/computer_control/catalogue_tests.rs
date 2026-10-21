//! The engine's catalogue does not fit through the hub frame either.
//!
//! The catalogue no longer rides in the republished state. This is the
//! second ceiling, downstream of it: the link refuses any outgoing frame over
//! 128 KiB, and the whole catalogue with its schemas is 147 909 bytes. Every
//! earlier test of this path declared a toy catalogue of a few hundred bytes,
//! which is exactly what let the size reach the Mac twice, so these cases use a
//! catalogue of the real order and drive the real entry point: the frame the
//! hub delivers, answered by the frames the link would have to carry.
use super::*;
use crate::hub::MAX_OUTGOING_FRAME_BYTES;
use orbit_core::computer_control::{MAX_ENGINE_CATALOGUE_BYTES, MAX_ENGINE_TOOLS};
use orbit_core::device_bridge::MAX_CATALOGUE_PAGE_BYTES;

async fn mac_with(catalogue: Value) -> Result<Mac> {
    Mac::with_engine(FakeEngine {
        tools: catalogue,
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    })
    .await
}

/// A catalogue of the size the real engine publishes reaches
/// whoever asked, entire and with its schemas, in frames the link accepts —
/// and the ceiling it travels under is the one that was already there.
#[tokio::test]
async fn a_real_sized_catalogue_crosses_the_hub_frame_in_pages() -> Result<()> {
    let (catalogue, catalogue_bytes) = catalogue_of(150 * 1024)?;
    let declared = catalogue.as_array().context("catalogue")?.len();
    assert!(
        (REAL_CATALOGUE_BYTES..=REAL_CATALOGUE_BYTES * 2).contains(&catalogue_bytes),
        "the test uses a catalogue of the real one's order ({REAL_CATALOGUE_BYTES} bytes), not a toy one: {catalogue_bytes} bytes"
    );
    assert!(
        catalogue_bytes > MAX_OUTGOING_FRAME_BYTES,
        "the whole catalogue does not fit in one frame: that is why it is asked for in pages"
    );

    let mut h = mac_with(catalogue).await?;
    assert!(
        h.runtime.engine_is_ready(),
        "a real-sized catalogue connects"
    );
    let whole = h.runtime.engine_tools();
    assert_eq!(whole.len(), declared);
    assert!(
        whole
            .iter()
            .filter(|tool| tool.name.starts_with("engine_tool_"))
            .all(|tool| tool.input_schema.is_object() && !tool.description.is_empty())
    );
    let published = serde_json::to_vec(&crate::daemon::topic(
        &h.runtime,
        orbit_core::topics::TopicId::Control,
    ))?;
    assert!(
        published.len() < MAX_OBSERVATION_BYTES,
        "the published state carries no catalogue: {} bytes",
        published.len()
    );
    assert!(!String::from_utf8_lossy(&published).contains("parameter_0"));
    let mut assembled: Vec<Value> = Vec::new();
    let mut after = 0u64;
    let mut rounds = 0usize;
    loop {
        let frame = h
            .ask(
                "control_tools",
                if after == 0 {
                    json!({})
                } else {
                    json!({"after_index": after})
                },
            )
            .await?;
        let bytes = serde_json::to_vec(&frame)?.len();
        assert!(
            bytes <= MAX_OUTGOING_FRAME_BYTES,
            "no frame asks for more than the link admits: {bytes} bytes"
        );
        assert!(
            frame["params"]["refusal"].is_null(),
            "asking for the catalogue is not refused: {frame}"
        );
        let result = &frame["params"]["result"];
        assert_eq!(result["after_index"].as_u64(), Some(after));
        assert_eq!(result["total"].as_u64(), Some(declared as u64));
        let page = result["tools"].as_array().context("a page")?;
        assert!(
            !page.is_empty(),
            "a page with tools still pending never comes back empty"
        );
        assert!(
            serde_json::to_vec(page)?.len() <= MAX_CATALOGUE_PAGE_BYTES,
            "a page keeps to its budget"
        );
        assembled.extend(page.iter().cloned());
        rounds += 1;
        assert!(rounds <= 16, "the walk ends");
        match result["next_after_index"].as_u64() {
            Some(next) => {
                assert!(next > after, "the walk advances and does not repeat");
                after = next;
            }
            None => break,
        }
    }

    // Everything the engine declared arrived, with what Deimos needs to
    // name a tool and give it arguments.
    assert_eq!(assembled.len(), declared);
    assert_eq!(
        rounds, 2,
        "the real catalogue costs two rounds, not a dozen"
    );
    let names: Vec<&str> = assembled
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names.len(), declared);
    assert!(names.contains(&"engine_tool_000"));
    assert!(
        assembled
            .iter()
            .filter(|tool| tool["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("engine_tool_")))
            .all(|tool| tool["input_schema"].is_object()
                && !tool["description"].as_str().unwrap_or_default().is_empty()),
        "the schemas reach whoever asks for the catalogue intact"
    );
    let assembled_bytes = serde_json::to_vec(&assembled)?.len();
    assert!(
        assembled_bytes > MAX_OUTGOING_FRAME_BYTES,
        "what was assembled is the whole catalogue, which did not fit on its own: {assembled_bytes} bytes"
    );

    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A page asked for with a nonsense index is refused by name rather
/// than answered with an empty catalogue, which reads as "this Mac has no
/// tools" and is the wrong thing to make anyone believe.
#[tokio::test]
async fn a_nonsense_page_index_is_refused_with_its_cause() -> Result<()> {
    let (catalogue, _) = catalogue_of(4 * 1024)?;
    let mut h = mac_with(catalogue).await?;
    for bogus in [json!("two"), json!(-1), json!(1.5), json!(true)] {
        let frame = h
            .ask("control_tools", json!({"after_index": bogus}))
            .await?;
        assert_eq!(
            frame["params"]["refusal"]["code"], "invalid_parameter",
            "{frame}"
        );
        assert_eq!(
            frame["params"]["refusal"]["expected"]["field"],
            "after_index"
        );
        assert_eq!(
            frame["params"]["refusal"]["expected"]["expected"],
            "nonnegative_integer"
        );
        assert!(frame["params"]["result"].is_null());
    }
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A catalogue beyond its own limit is refused where it arrives, and
/// the refusal names it as a catalogue, not as an observation.
#[tokio::test]
async fn an_absurd_catalogue_is_refused_by_its_own_name() -> Result<()> {
    let (catalogue, catalogue_bytes) = catalogue_of(MAX_ENGINE_CATALOGUE_BYTES + 1)?;
    let declared = catalogue.as_array().context("catalogue")?.len();
    assert!(catalogue_bytes > MAX_ENGINE_CATALOGUE_BYTES);
    assert!(
        declared <= MAX_ENGINE_TOOLS,
        "it is the catalogue's byte limit that refuses, not the tool-count one"
    );

    let Err(error) = FakeEngine {
        tools: catalogue,
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    }
    .connect()
    .await
    else {
        anyhow::bail!("an absurd catalogue cannot leave the engine connected");
    };
    let message = error.to_string();
    assert!(
        message.contains("catálogo"),
        "the refusal has to name it as a catalogue: {message}"
    );
    assert!(
        !message.contains("Observación"),
        "a catalogue is not refused with a screen observation's message: {message}"
    );
    Ok(())
}
