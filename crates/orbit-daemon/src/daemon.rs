//! Single state owner. Native, network and IPC feed facts into this actor.
use crate::{
    config::{Paths, Settings, get_secret, storage_key},
    hub::{self, HubEvent},
    ipc::{self, Call},
    native,
};
use anyhow::{Context, Result, bail};
use orbit_core::{
    AccountSummary, Approval, CapturePhase, Connection, DurableState, Effect, Event, State, decide,
    failure::Outcome,
    protocol::{Command, DaemonStatus, Response, TopicResponse, VoiceNoteAudioResponse},
    refusal::{Refusal, Service},
};
use orbit_store::{Job, JobState, Store};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

#[cfg(test)]
mod subscription_tests;

#[cfg(test)]
mod privacy_read_tests;

#[cfg(test)]
mod ipc_tests;

#[cfg(test)]
mod refusal_tests;

mod actions;
mod ipc_replies;
mod main_loop;
mod run;
mod runtime;
mod runtime_build;
use ipc_replies::*;
#[cfg(test)]
use run::run_with_helper_inner;
pub use run::{run, run_embedded, run_with_helper};
pub(crate) use runtime::MAX_PENDING_NATIVE;
use runtime::*;
mod agent_config;
mod assistant;
mod assistant_files;
mod attachments;
mod audio;
mod audio_sources;
mod bridge;
mod calendar;
mod call;
mod call_detection;
mod call_recovery;
mod capture_handle;
mod capture_lane;
mod capture_link;
mod capture_record;
mod commands;
mod computer_control;
mod cut_recovery;
mod finalize;
mod hub_account;
mod hub_events;
mod hub_link;
mod interactions;
mod kernel;
mod links;
mod loop_lag;
mod meeting_question;
mod meeting_runtime;
mod native_events;
mod native_host;
mod read_dispatch;
mod reads;
mod recording_notes;
mod retire;
mod sealing;
mod shutdown;
use bridge::{BridgeAsk, BridgeKind, BridgeReleaseReason, BridgeSlots};
use kernel::DurableSnapshot;
pub use shutdown::{SHUTDOWN_BUDGET, SNAPSHOT_RECEIPT_GRACE};
mod helper;
mod helper_waits;
mod sources;
mod speech;
mod startup;
mod stops;
mod store_actor;
pub(crate) mod ticketed;
mod timed_reader;
mod topics;
mod track_gaps;
mod track_signal;
mod transfers;
mod turn;
mod turn_watch;
mod upload_outcome;
mod voice_background;
mod voice_dispatch;
mod voice_notes;
mod window_attachment;
use turn::refusal_notice;
mod work;

#[cfg(test)]
mod agent_config_tests;
#[cfg(test)]
mod agents_tests;
#[cfg(test)]
mod attachments_tests;
#[cfg(test)]
mod capability_notice_tests;
#[cfg(test)]
mod commands_tests;
#[cfg(test)]
mod helper_liveness_tests;
#[cfg(test)]
mod pairing_session_tests;
#[cfg(test)]
mod state_observation_tests;
#[cfg(test)]
mod track_failure_tests;
#[cfg(test)]
mod voice_note_tests;

#[cfg(test)]
mod hub_backpressure_tests;

#[cfg(test)]
mod native_bridge_tests;

#[cfg(test)]
mod capture_isolation_tests;
#[cfg(test)]
mod chunk_delivery_tests;
#[cfg(test)]
mod meeting_continuity_tests;
#[cfg(test)]
mod offline_voice_note_tests;
#[cfg(test)]
mod restart_closure_tests;
#[cfg(test)]
mod seal_cause_tests;

#[cfg(test)]
mod privacy_capture_tests;

#[cfg(test)]
mod scheduling_tests;

#[cfg(test)]
mod error_report_tests;

#[cfg(test)]
pub(crate) mod authority_fixture;
#[cfg(test)]
mod authority_retirement_tests;
#[cfg(test)]
mod store_actor_tests;
#[cfg(test)]
mod turn_watch_tests;

#[cfg(test)]
mod test_store;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn account_summary(settings: &Settings) -> AccountSummary {
    AccountSummary {
        hub_url: settings.hub_url.clone().map(Into::into),
        device_name: Some(settings.device_name.clone().into()),
        device_id_short: settings
            .device_id
            .as_deref()
            .map(|device_id| device_id.chars().take(8).collect::<String>().into()),
    }
}

/// The applications the Mac attached to a meeting hotkey press.
///
/// Anything malformed is dropped rather than defaulted: a row with a missing
/// name or a zero PID would travel on as a real candidate and could end up
/// chosen as the thing to record.
fn meeting_applications(event: &Value) -> Vec<orbit_core::meeting_audio::RunningApplication> {
    event["applications"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let application_pid = i32::try_from(row["application_pid"].as_i64()?).ok()?;
                    let bundle_id = row["bundle_id"].as_str()?;
                    let name = row["name"].as_str()?;
                    if application_pid <= 0 || bundle_id.is_empty() || name.is_empty() {
                        return None;
                    }
                    Some(orbit_core::meeting_audio::RunningApplication {
                        application_pid,
                        bundle_id: bundle_id.to_owned(),
                        name: name.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn recover_store(store: &mut Store) -> Result<orbit_store::RecoveryReport> {
    let mut scan = store.begin_recovery()?;
    let mut total = orbit_store::RecoveryReport::default();
    loop {
        let batch = store.recover_next(&mut scan, 500)?;
        total.inspected += batch.inspected;
        total.recovered += batch.recovered;
        total.invalid += batch.invalid;
        total.missing += batch.missing;
        total.incomplete_temporary += batch.incomplete_temporary;
        if !batch.limit_reached {
            return Ok(total);
        }
        // No capture, transfer or position restoration before this reaches EOF.
        tokio::task::yield_now().await;
    }
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod meeting_application_tests;

#[cfg(test)]
#[allow(clippy::expect_used)]
fn topic(runtime: &Runtime, id: orbit_core::topics::TopicId) -> orbit_core::topics::TopicData {
    let mut board = topics::TopicBoard::new("fixture".into());
    let receiver = board
        .channels()
        .get(&id)
        .expect("registered topic")
        .subscribe();
    board.publish_command(&topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    });
    receiver.borrow().data.clone()
}

#[cfg(test)]
mod response_budget_tests;

#[cfg(test)]
mod conversation_switch_tests;
mod silence;
