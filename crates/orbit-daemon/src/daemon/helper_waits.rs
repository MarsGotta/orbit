//! Helper orders whose answer a task off the loop waits for: the window's
//! text for «Adjuntar ventana» and a photo written as JPEG before it is
//! attached. The loop sends the order and hands the reply over when it
//! arrives; the task bounds its own wait. A helper that goes away ends every
//! wait, which then says so. Nothing here logs what an answer carries.
use super::Runtime;
use orbit_core::refusal::{Operation, Refusal};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::oneshot;

/// The helper's result, or the code of its refusal or of the missing reply.
pub(super) type HelperAnswer = Result<Value, String>;

impl Runtime {
    /// Send `name` with `fields` to the helper; its reply arrives on the
    /// receiver. Refused at once when there is no helper or it is busy.
    pub(super) fn ask_helper(
        &mut self,
        name: &str,
        mut fields: Value,
        operation: Operation,
    ) -> Result<oneshot::Receiver<HelperAnswer>, Refusal> {
        if !self.state.capabilities().native_helper() {
            return Err(Refusal::HelperNotRunning);
        }
        let id = format!("{name}-{}", super::id());
        if let Some(object) = fields.as_object_mut() {
            object.insert("id".into(), Value::from(id.clone()));
        }
        let command = crate::native::command(name, fields).map_err(|_| Refusal::Internal {
            cause: orbit_core::refusal::InternalCause::NativeCommand,
        })?;
        self.helper
            .sender
            .try_send(command.clone())
            .map_err(|_| Refusal::Busy {
                operation,
                retry_after_ms: None,
            })?;
        self.helper.record_pending(&command);
        let (sender, receiver) = oneshot::channel();
        // A wait that gave up leaves its sender here; it is dropped now.
        self.helper.waits.retain(|_, waiting| !waiting.is_closed());
        self.helper.waits.insert(id, sender);
        Ok(receiver)
    }

    /// The helper went away or started again: what it was asked has no
    /// answer, and each wait ends with `helper_restarted`.
    pub(super) fn end_helper_waits(&mut self) {
        self.helper.waits.clear();
        // «Silenciar» asked of the helper that went: never answered, and
        // the microphone stays as it was shown.
        self.audio.forget_mute_orders();
    }
}

/// The helper's answer within `deadline`, or why there is none.
pub(super) async fn wait(
    receiver: oneshot::Receiver<HelperAnswer>,
    deadline: Duration,
) -> HelperAnswer {
    match tokio::time::timeout(deadline, receiver).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err("helper_restarted".into()),
        Err(_) => Err("timeout".into()),
    }
}
