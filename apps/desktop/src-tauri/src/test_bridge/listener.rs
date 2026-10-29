use super::{IO_BOUND, View, serve, write_reply};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{net::UnixListener, task::JoinSet};

// Limits simultaneous connections and pending evaluations, not the size of a
// DOM or a recording. Four lets a read inspect a slow view without allowing an
// unbounded task/oneshot per peer. Extra peers get a refusal, never a retry.
const CONNECTIONS: usize = 4;
pub(super) async fn run(listener: UnixListener, view: Arc<dyn View>, bound: Duration) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            accepted = listener.accept() => {
                let Ok((mut stream, _)) = accepted else { break };
                while tasks.try_join_next().is_some() {}
                if tasks.len() >= CONNECTIONS {
                    let reply = json!({"ok":false,"cause":"bridge_busy",
                        "in_flight":tasks.len(),"max_connections":CONNECTIONS,"write_bound_ms":IO_BOUND.as_millis()});
                    write_reply(&mut stream, reply).await;
                } else {
                    let view = view.clone();
                    tasks.spawn(async move { serve(stream, view.as_ref(), bound).await; });
                }
            }
        }
    }
}
