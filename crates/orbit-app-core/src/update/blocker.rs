//! What holds an update back: a live capture or a call, or a state the app
//! does not know yet.
use super::UpdateService;
use crate::topics::{TopicCache, TopicFreshness};
use orbit_core::{
    CapturePhase,
    topics::{Knowledge, Pending, TopicData, TopicId},
};

impl UpdateService {
    pub fn installation_blocker(topics: &TopicCache) -> Option<String> {
        unknown_or_stale(topics)
            .or_else(|| recording_blocker(topics))
            .or_else(|| call_blocker(topics))
    }
}

fn unknown_or_stale(topics: &TopicCache) -> Option<String> {
    for (id, name) in [
        (TopicId::Recording, "la grabación"),
        (TopicId::Assistant, "la llamada"),
    ] {
        let Some(topic) = topics.topic(id) else {
            return Some(format!(
                "Aún no se conoce el estado de {name}; espera antes de actualizar."
            ));
        };
        if !matches!(topic.freshness, TopicFreshness::Current) {
            return Some(format!(
                "El estado de {name} está desactualizado; espera antes de actualizar."
            ));
        }
    }
    None
}

fn recording_blocker(topics: &TopicCache) -> Option<String> {
    let Some(TopicData::Recording(recording)) = topics
        .topic(TopicId::Recording)
        .map(|topic| &topic.knowledge)
    else {
        return Some("No se conoce si hay una grabación; espera antes de actualizar.".into());
    };
    let value = match recording {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return Some(format!(
                "El estado de la grabación está pendiente ({}); espera antes de actualizar.",
                pending_update_state(why)
            ));
        }
        Knowledge::Absent { refusal } => {
            return Some(format!(
                "No se pudo conocer el estado de la grabación: {refusal}"
            ));
        }
    };
    // Only a live capture: a recording a cut left interrupted, a paused one
    // or a closed one loses nothing to the restart, and an interrupted one
    // used to hold every update for as long as it stayed.
    if value
        .recording
        .as_ref()
        .is_some_and(|recording| live(&recording.phase))
    {
        return Some("Termina la grabación antes de actualizar.".into());
    }
    if value.call.as_ref().is_some_and(|call| call.panel.is_some()) {
        return Some("Termina la llamada antes de actualizar.".into());
    }
    None
}

/// A capture the Mac is running: starting, recording, or still draining what
/// it recorded after «Terminar». Quitting Orbit would cut it. Nothing of a
/// paused, interrupted or closed recording is being captured.
fn live(phase: &CapturePhase) -> bool {
    matches!(
        phase,
        CapturePhase::Starting | CapturePhase::Recording | CapturePhase::Finalizing
    )
}

fn call_blocker(topics: &TopicCache) -> Option<String> {
    let Some(TopicData::Assistant(assistant)) = topics
        .topic(TopicId::Assistant)
        .map(|topic| &topic.knowledge)
    else {
        return Some("No se conoce si hay una llamada; espera antes de actualizar.".into());
    };
    let value = match assistant {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return Some(format!(
                "El estado de la llamada está pendiente ({}); espera antes de actualizar.",
                pending_update_state(why)
            ));
        }
        Knowledge::Absent { refusal } => {
            return Some(format!(
                "No se pudo conocer el estado de la llamada: {refusal}"
            ));
        }
    };
    match &value.call {
        Knowledge::Present { value: None, .. } => None,
        Knowledge::Present { value: Some(_), .. } => {
            Some("Termina la llamada antes de actualizar.".into())
        }
        Knowledge::Unknown { why } => Some(format!(
            "El estado de la llamada está pendiente ({}); espera antes de actualizar.",
            pending_update_state(why)
        )),
        Knowledge::Absent { refusal } => Some(format!(
            "No se pudo conocer el estado de la llamada: {refusal}"
        )),
    }
}

fn pending_update_state(why: &Pending) -> String {
    match why {
        Pending::NotObservedYet => "aún no observado".into(),
        Pending::HelperStarting => "el ayudante del Mac está arrancando".into(),
        Pending::Reading { since_ms } => format!("se está leyendo desde {since_ms} ms"),
        Pending::Transcription {
            completed,
            pending,
            failed,
            queued,
        } => format!(
            "transcripción: {completed} terminados, {pending} pendientes, {failed} fallidos, en cola: {}",
            if *queued { "sí" } else { "no" }
        ),
        Pending::DaemonStarting => "el servicio de Orbit está arrancando".into(),
    }
}
