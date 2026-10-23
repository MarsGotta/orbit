//! Exact synthetic authority states for core unit tests only.

use crate::State;

pub(crate) fn stopped_state(mut state: State) -> State {
    state.durable.privacy.stopped = true;
    state
}
