//! The bounded outbound link to Deimos. Queue admission is the only fact the
//! daemon can observe; the websocket has no remote receipt for these frames.
use orbit_core::notice::Op;
use serde_json::Value;
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// How many sent frames remember their op, so the hub's answer to one (its
/// error, above all) is said under the op of the request. The link
/// queues 16 frames; 64 covers four full queues of answers still on their
/// way. It bounds only this memory: an answer to an older frame gets a new
/// op, it is not refused.
const REMEMBERED_OPS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Retirement {
    PrivacyStop,
    AuthorityRetired,
    NetworkCancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SendResult {
    Queued,
    EncodingFailed { reason: String },
    Full { capacity: usize },
    Closed,
    Disconnected,
    Retired(Retirement),
}

impl SendResult {
    pub(super) fn delivered_to_queue(&self) -> bool {
        matches!(self, Self::Queued)
    }

    pub(super) fn cause(&self) -> String {
        match self {
            Self::Queued => "enviado a la cola local; recepción de Deimos sin confirmar".into(),
            Self::EncodingFailed { reason } => {
                format!("trama del hub no codificada: {reason}; esta trama no se entregó")
            }
            Self::Full { capacity } => format!(
                "canal del hub lleno (capacidad {capacity} tramas); esta trama no se entregó"
            ),
            Self::Closed => "canal del hub cerrado; esta trama no se entregó".into(),
            Self::Disconnected => "sin conexión con el hub; esta trama no se entregó".into(),
            Self::Retired(Retirement::PrivacyStop) => {
                "enlace retirado por parada de privacidad; esta trama no se entregó".into()
            }
            Self::Retired(Retirement::AuthorityRetired) => {
                "enlace retirado por retirada de autoridad; esta trama no se entregó".into()
            }
            Self::Retired(Retirement::NetworkCancelled) => {
                "enlace retirado por cancelación de red; esta trama no se entregó".into()
            }
        }
    }
}

/// The hub link's handle: the outbound frames, the connection task, the
/// device's credential and what the connected hub said about itself.
pub(super) struct LinkHandle {
    /// The bounded outbound link to Deimos.
    pub(super) outbound: HubLink,
    pub(super) task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    /// Where the connection task sends what the hub says.
    pub(super) events: mpsc::Sender<(u64, crate::hub::HubEvent)>,
    /// The device's credential with the hub; `None` before pairing.
    pub(super) token: Option<String>,
    pub(super) protocol_version: Option<u64>,
    pub(super) unrecognized_events: std::collections::BTreeSet<String>,
    /// Whether the hub last said it was nearly full (`ready.storage`,
    /// `storage.status`); `None` before it said anything. Kept in memory
    /// only, so a restart of Orbit says a nearly full hub once more.
    pub(super) storage_nearly_full: Option<bool>,
}

impl LinkHandle {
    pub(super) fn new(
        events: mpsc::Sender<(u64, crate::hub::HubEvent)>,
        token: Option<String>,
    ) -> Self {
        Self {
            outbound: HubLink::default(),
            task: None,
            events,
            token,
            protocol_version: None,
            unrecognized_events: std::collections::BTreeSet::new(),
            storage_nearly_full: None,
        }
    }
}

#[derive(Default)]
pub(super) struct HubLink {
    state: LinkState,
    sent_ops: VecDeque<(String, Op)>,
}

#[derive(Default)]
enum LinkState {
    #[default]
    Disconnected,
    Open(mpsc::Sender<Value>),
    Retired(Retirement),
}

impl HubLink {
    pub(super) fn open(&mut self, capacity: usize) -> mpsc::Receiver<Value> {
        let (sender, receiver) = mpsc::channel(capacity);
        self.install(sender);
        receiver
    }

    pub(super) fn install(&mut self, sender: mpsc::Sender<Value>) {
        self.state = LinkState::Open(sender);
    }

    pub(super) fn retire(&mut self, reason: Retirement) -> bool {
        let was_open = matches!(self.state, LinkState::Open(_));
        if reason == Retirement::PrivacyStop || !matches!(self.state, LinkState::Retired(_)) {
            self.state = LinkState::Retired(reason);
        }
        was_open
    }

    pub(super) fn is_some(&self) -> bool {
        matches!(self.state, LinkState::Open(_))
    }

    #[cfg(test)]
    pub(super) fn is_none(&self) -> bool {
        !self.is_some()
    }

    /// Sends one request frame on behalf of the action `op`: the frame carries
    /// it in its top-level `op` field (the contract with the hub).
    pub(super) fn send_or_report(&mut self, frame: anyhow::Result<Value>, op: &Op) -> SendResult {
        match &self.state {
            LinkState::Disconnected => SendResult::Disconnected,
            LinkState::Retired(reason) => SendResult::Retired(*reason),
            LinkState::Open(sender) => match frame {
                Err(error) => SendResult::EncodingFailed {
                    reason: error.to_string(),
                },
                Ok(mut frame) => {
                    if let Some(object) = frame.as_object_mut() {
                        object
                            .entry("op")
                            .or_insert_with(|| Value::String(op.as_str().to_owned()));
                    }
                    let id = frame.get("id").and_then(Value::as_str).map(str::to_owned);
                    match sender.try_send(frame) {
                        Ok(()) => {
                            if let Some(id) = id {
                                if self.sent_ops.len() == REMEMBERED_OPS {
                                    self.sent_ops.pop_front();
                                }
                                self.sent_ops.push_back((id, op.clone()));
                            }
                            SendResult::Queued
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => SendResult::Full {
                            capacity: sender.max_capacity(),
                        },
                        Err(mpsc::error::TrySendError::Closed(_)) => SendResult::Closed,
                    }
                }
            },
        }
    }

    /// The op of a frame this link sent, by the frame's id.
    pub(super) fn op_for(&self, id: &str) -> Option<Op> {
        self.sent_ops
            .iter()
            .rev()
            .find(|(sent, _)| sent == id)
            .map(|(_, op)| op.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every request frame carries the op of its action at the top
    /// level (`{v, id, op, method, params}`), and the answer to it can find
    /// that op again by the frame's id.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_request_frame_carries_its_op_and_its_answer_finds_it() {
        let mut link = HubLink::default();
        let mut frames = link.open(4);
        let op = Op::parse("0123abcd-frame").unwrap();
        let request = json!({"v":1,"id":"request-1","method":"ping","params":{}});
        assert_eq!(link.send_or_report(Ok(request), &op), SendResult::Queued);
        let sent = frames.try_recv().unwrap();
        assert_eq!(sent["op"], "0123abcd-frame");
        assert_eq!(sent["id"], "request-1");
        assert_eq!(link.op_for("request-1"), Some(op));
        assert_eq!(link.op_for("never-sent"), None);
    }

    #[test]
    fn distinguishes_unconnected_closed_and_all_retirement_causes() {
        let mut link = HubLink::default();
        assert_eq!(
            link.send_or_report(Ok(json!({})), &orbit_ipc::new_op()),
            SendResult::Disconnected
        );
        let receiver = link.open(1);
        drop(receiver);
        assert_eq!(
            link.send_or_report(Ok(json!({})), &orbit_ipc::new_op()),
            SendResult::Closed
        );
        for reason in [
            Retirement::PrivacyStop,
            Retirement::AuthorityRetired,
            Retirement::NetworkCancelled,
        ] {
            let receiver = link.open(1);
            link.retire(reason);
            drop(receiver);
            assert_eq!(
                link.send_or_report(Ok(json!({})), &orbit_ipc::new_op()),
                SendResult::Retired(reason)
            );
            assert!(
                SendResult::Retired(reason)
                    .cause()
                    .contains("no se entregó")
            );
        }
        let receiver = link.open(1);
        link.retire(Retirement::PrivacyStop);
        drop(receiver);
        link.retire(Retirement::NetworkCancelled);
        assert_eq!(
            link.send_or_report(Ok(json!({})), &orbit_ipc::new_op()),
            SendResult::Retired(Retirement::PrivacyStop)
        );
    }

    #[test]
    fn privacy_stop_overrides_an_earlier_network_retirement() {
        let mut link = HubLink::default();
        let receiver = link.open(1);
        link.retire(Retirement::NetworkCancelled);
        drop(receiver);
        link.retire(Retirement::PrivacyStop);
        assert_eq!(
            link.send_or_report(Ok(json!({})), &orbit_ipc::new_op()),
            SendResult::Retired(Retirement::PrivacyStop)
        );
    }
}
