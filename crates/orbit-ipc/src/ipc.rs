//! Owner-only Unix IPC; bounded frames and one request per connection.
//!
//! This file holds what both ends share: a call and its reply, the lock and the
//! socket path, and the bounded frame. `server` is the daemon's listener,
//! `client` a process that reaches it over the socket, and `local_client` the
//! app's typed access to the daemon on its own thread.
use crate::Paths;
use anyhow::{Context, Result, bail};
use orbit_core::{
    PROTOCOL_VERSION,
    limits::MAX_IPC_LINE_BYTES,
    protocol::{
        AgentResponse, AttachmentContent, AttachmentResponse, Command, Request, Response,
        TopicEvent, TopicResponse, VoiceNoteAudio, VoiceNoteAudioResponse,
    },
    reads::ReadRequest,
    refusal::{Refusal, Resource},
    topics::{TopicFrame, TopicId, read_transport_max_bytes, transport_max_bytes},
};
use std::{
    collections::BTreeMap,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream, unix::OwnedReadHalf},
    sync::{Semaphore, mpsc, oneshot, watch},
};

mod budget;
mod client;
mod local_client;
mod server;
pub use budget::{bound_topic_frame, bound_topic_response};
pub use client::{
    KEYED_READ_TIMEOUT, Subscription, new_op, read_topic, request, request_command,
    request_command_as, subscribe_topics, voice_note_audio,
};
pub use local_client::LocalClient;
pub use server::listen;
#[cfg(test)]
mod tests;

pub struct Call {
    pub request: Request,
    pub reply: ReplySender,
}

#[derive(Debug)]
pub enum Reply {
    Command(Response),
    VoiceNoteAudio(VoiceNoteAudioResponse),
    Topics(TopicResponse),
    Agent(AgentResponse),
    Vault(orbit_core::vault::VaultResponse),
    Attachment(AttachmentResponse),
}

impl From<orbit_core::vault::VaultResponse> for Reply {
    fn from(response: orbit_core::vault::VaultResponse) -> Self {
        Self::Vault(response)
    }
}

impl From<AttachmentResponse> for Reply {
    fn from(response: AttachmentResponse) -> Self {
        Self::Attachment(response)
    }
}

/// How long the app waits for an attachment order: the hub request's own
/// 90 s (`HUB_HTTP_TIMEOUT` in orbit-daemon's hub.rs) and 10 s for the
/// daemon to take it and answer. It bounds one upload or read of up to
/// 20 MiB, which the 10 s of other orders could not hold on a slow link;
/// it is not a retry, and a lost answer is never sent again.
const ATTACHMENT_CALL_TIMEOUT: Duration = Duration::from_secs(ATTACHMENT_CALL_TIMEOUT_SECONDS);
/// In seconds, as the interface reads it (`contracts/constants.json`).
pub const ATTACHMENT_CALL_TIMEOUT_SECONDS: u64 = 100;

/// How long the app waits for one call to the daemon, its queue included: the
/// ten-second confirmation window every order and read has. The interface
/// waits this, then the frames the reply published, before it calls the wait
/// a failure (`apps/desktop/src/state/ceilings.ts`). Never a retry.
pub const CALL_TIMEOUT_SECONDS: u64 = 10;

impl From<Response> for Reply {
    fn from(response: Response) -> Self {
        Self::Command(response)
    }
}

impl From<VoiceNoteAudioResponse> for Reply {
    fn from(response: VoiceNoteAudioResponse) -> Self {
        Self::VoiceNoteAudio(response)
    }
}

impl From<AgentResponse> for Reply {
    fn from(response: AgentResponse) -> Self {
        Self::Agent(response)
    }
}

impl From<TopicResponse> for Reply {
    fn from(response: TopicResponse) -> Self {
        Self::Topics(response)
    }
}

pub type TopicChannels = BTreeMap<TopicId, watch::Sender<TopicFrame>>;

pub struct ReplySender(oneshot::Sender<Reply>);

#[derive(Debug)]
pub struct ReplySendError;

impl ReplySender {
    pub async fn closed(&mut self) {
        self.0.closed().await;
    }

    pub fn send(self, response: impl Into<Reply>) -> std::result::Result<(), ReplySendError> {
        self.0.send(response.into()).map_err(|_| ReplySendError)
    }
}

/// How the daemon's thread ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonEnded {
    /// Its closure ran to the end: a `Shutdown`, a signal or its own decision.
    Closed,
    /// It returned an error; the daemon's own words.
    Failed(String),
    /// The thread ended without a word: it panicked, and the panic was
    /// printed (and reported, if the user allows it) where it happened.
    Vanished,
}

impl DaemonEnded {
    pub fn describe(&self) -> String {
        match self {
            Self::Closed => "closed in order".into(),
            Self::Failed(error) => format!("ended with an error: {error}"),
            Self::Vanished => "its thread ended without a result (a panic)".into(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Closed => 0,
            Self::Failed(_) | Self::Vanished => 1,
        }
    }
}

/// Hold this file for the complete daemon lifetime. The lock is acquired before
/// opening storage, so a competing process cannot recover or rewrite live state.
pub fn lock_instance(paths: &Paths) -> Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())?)
        .open(paths.root.join("daemon.lock"))?;
    let metadata = lock.metadata()?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() {
        bail!("The daemon lock does not belong to the current user");
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .context("An Orbit service is already active or this directory cannot be locked")?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(lock)
}

/// Retire an owner-local socket path. The daemon calls this only while holding
/// daemon.lock; the wire listener calls it after proving the old peer is gone.
/// A symlink or a regular file is never removed as though it were a socket.
pub fn remove_owned_socket(paths: &Paths) -> Result<()> {
    let meta = match std::fs::symlink_metadata(&paths.socket) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("Could not inspect the retired IPC path"),
    };
    if !meta.file_type().is_socket() {
        bail!(
            "IPC path occupied at {}: it is not a socket; it will not be removed",
            paths.socket.display()
        );
    }
    let owner = rustix::process::geteuid().as_raw();
    if meta.uid() != owner {
        bail!(
            "IPC path occupied at {}: uid {}, expected {}; it will not be removed",
            paths.socket.display(),
            meta.uid(),
            owner
        );
    }
    std::fs::remove_file(&paths.socket)?;
    Ok(())
}

pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: R) -> Result<Vec<u8>> {
    read_frame_with_limit(reader, orbit_core::limits::MAX_COMMAND_RESPONSE_BYTES).await
}

pub async fn read_frame_with_limit<R: tokio::io::AsyncBufRead + Unpin>(
    reader: R,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    read_frame_into(reader, limit, Resource::IpcResponse, &mut frame).await?;
    Ok(frame)
}

async fn read_frame_into<R: tokio::io::AsyncBufRead + Unpin>(
    mut reader: R,
    limit: usize,
    resource: Resource,
    frame: &mut Vec<u8>,
) -> Result<()> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            bail!("Connection closed before end of message");
        }
        let end = available.iter().position(|b| *b == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        let observed = frame.len() + count;
        if observed > limit {
            frame.extend_from_slice(&available[..limit.saturating_sub(frame.len())]);
            return Err(Refusal::TooLarge {
                resource,
                bytes: observed as u64,
                max_bytes: limit as u64,
            }
            .into());
        }
        frame.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(());
        }
    }
}
