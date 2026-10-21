//! DEI-14: the engine's catalogue does not fit through the hub frame either.
//!
//! DEI-13 stopped the catalogue riding in the republished state. This is the
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

/// A1/A3/R1/R2/R5: a catalogue of the size the real engine publishes reaches
/// whoever asked, entire and with its schemas, in frames the link accepts —
/// and the ceiling it travels under is the one that was already there.
#[tokio::test]
async fn dei14_a1_a_real_sized_catalogue_crosses_the_hub_frame_in_pages() -> Result<()> {
    let (catalogue, catalogue_bytes) = catalogue_of(150 * 1024)?;
    let declared = catalogue.as_array().context("catálogo")?.len();
    assert!(
        (REAL_CATALOGUE_BYTES..=REAL_CATALOGUE_BYTES * 2).contains(&catalogue_bytes),
        "la prueba usa un catálogo del orden del real ({REAL_CATALOGUE_BYTES} bytes), no uno de juguete: {catalogue_bytes} bytes"
    );
    assert!(
        catalogue_bytes > MAX_OUTGOING_FRAME_BYTES,
        "el catálogo entero no cabe en una trama: por eso se pide por partes"
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
            "R2: ninguna trama pide más de lo que el enlace admite: {bytes} bytes"
        );
        assert!(
            frame["params"]["refusal"].is_null(),
            "R1: pedir el catálogo no se rechaza: {frame}"
        );
        let result = &frame["params"]["result"];
        assert_eq!(result["after_index"].as_u64(), Some(after));
        assert_eq!(result["total"].as_u64(), Some(declared as u64));
        let page = result["tools"].as_array().context("una página")?;
        assert!(
            !page.is_empty(),
            "una página con herramientas pendientes nunca vuelve vacía"
        );
        assert!(
            serde_json::to_vec(page)?.len() <= MAX_CATALOGUE_PAGE_BYTES,
            "una página respeta su presupuesto"
        );
        assembled.extend(page.iter().cloned());
        rounds += 1;
        assert!(rounds <= 16, "el recorrido termina");
        match result["next_after_index"].as_u64() {
            Some(next) => {
                assert!(next > after, "el recorrido avanza y no se repite");
                after = next;
            }
            None => break,
        }
    }

    // R1: everything the engine declared arrived, with what Deimos needs to
    // name a tool and give it arguments.
    assert_eq!(assembled.len(), declared);
    assert_eq!(
        rounds, 2,
        "el catálogo real cuesta dos vueltas, no una docena"
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
        "los esquemas llegan intactos a quien pide el catálogo"
    );
    let assembled_bytes = serde_json::to_vec(&assembled)?.len();
    assert!(
        assembled_bytes > MAX_OUTGOING_FRAME_BYTES,
        "lo que se montó es el catálogo entero, que por sí solo no cabía: {assembled_bytes} bytes"
    );

    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A2/R3: a page asked for with a nonsense index is refused by name rather
/// than answered with an empty catalogue, which reads as "this Mac has no
/// tools" and is the wrong thing to make anyone believe.
#[tokio::test]
async fn dei14_a2_a_nonsense_page_index_is_refused_with_its_cause() -> Result<()> {
    let (catalogue, _) = catalogue_of(4 * 1024)?;
    let mut h = mac_with(catalogue).await?;
    for bogus in [json!("dos"), json!(-1), json!(1.5), json!(true)] {
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

/// A3, R3: a catalogue beyond its own limit is refused where it arrives, and
/// the refusal names it as a catalogue, not as an observation.
#[tokio::test]
async fn a3_an_absurd_catalogue_is_refused_by_its_own_name() -> Result<()> {
    let (catalogue, catalogue_bytes) = catalogue_of(MAX_ENGINE_CATALOGUE_BYTES + 1)?;
    let declared = catalogue.as_array().context("catálogo")?.len();
    assert!(catalogue_bytes > MAX_ENGINE_CATALOGUE_BYTES);
    assert!(
        declared <= MAX_ENGINE_TOOLS,
        "es el límite de bytes del catálogo el que rechaza, no el del número de herramientas"
    );

    let Err(error) = FakeEngine {
        tools: catalogue,
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    }
    .connect()
    .await
    else {
        anyhow::bail!("un catálogo absurdo no puede dejar el motor conectado");
    };
    let message = error.to_string();
    assert!(
        message.contains("catálogo"),
        "el rechazo tiene que nombrarlo como catálogo: {message}"
    );
    assert!(
        !message.contains("Observación"),
        "un catálogo no se rechaza con el mensaje de una observación de pantalla: {message}"
    );
    Ok(())
}
