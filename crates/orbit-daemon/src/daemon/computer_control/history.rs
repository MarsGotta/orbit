//! The control history, kept in a private file of its own: every attempt
//! recorded before it is made, with its outcome once known.
use super::*;

// Bounds the private file's read allocation and atomic writes, not the number
// of actions an assistant can perform. Retain newest rows within both budgets.
const MAX_CONTROL_STATE_BYTES: usize = 256 * 1024;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DurableControlState {
    pub(super) schema: u8,
    pub(super) history: Vec<ControlHistoryEntry>,
}

impl Default for DurableControlState {
    fn default() -> Self {
        Self {
            schema: 1,
            history: Vec::new(),
        }
    }
}

impl Runtime {
    pub(in crate::daemon) fn control_history(&self) -> Result<Vec<ControlHistoryEntry>> {
        Ok(self.load_control_state()?.history)
    }

    pub(super) fn control_state_path(&self) -> PathBuf {
        self.paths.root.join("computer-control.json")
    }

    pub(super) fn load_control_state(&self) -> Result<DurableControlState> {
        let bytes = match std::fs::read(self.control_state_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DurableControlState::default());
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() > MAX_CONTROL_STATE_BYTES {
            bail!("El estado local de control supera 256 KiB");
        }
        let value: DurableControlState = serde_json::from_slice(&bytes)?;
        if value.schema != 1
            || value.history.len() > MAX_CONTROL_HISTORY
            || value.history.iter().any(|entry| !entry.is_valid())
        {
            bail!("El estado local de control no es válido");
        }
        Ok(value)
    }

    pub(super) fn save_control_state(&self, value: &DurableControlState) -> Result<()> {
        if value.schema != 1
            || value.history.len() > MAX_CONTROL_HISTORY
            || value.history.iter().any(|entry| !entry.is_valid())
        {
            bail!("El estado local de control no es válido");
        }
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > MAX_CONTROL_STATE_BYTES {
            bail!("El estado local de control supera 256 KiB");
        }
        crate::config::atomic_private_write(&self.control_state_path(), &bytes)
    }

    pub(super) fn upsert_control_history(&self, entry: ControlHistoryEntry) -> Result<()> {
        let mut state = self.load_control_state()?;
        if let Some(existing) = state
            .history
            .iter_mut()
            .find(|existing| existing.operation_id == entry.operation_id)
        {
            // A receipt updates the same action. Moving it before a cursor
            // would make the next history page repeat earlier entries.
            *existing = entry;
        } else {
            state.history.insert(0, entry);
        }
        state.history.truncate(MAX_CONTROL_HISTORY);
        let mut bytes = serde_json::to_vec(&state)?.len();
        while bytes > MAX_CONTROL_STATE_BYTES {
            let oldest = state.history.pop().context("control history size")?;
            bytes -= serde_json::to_vec(&oldest)?.len() + usize::from(!state.history.is_empty());
        }
        self.save_control_state(&state)
    }
}
