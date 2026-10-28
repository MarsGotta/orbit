//! The composer's files: read here by path or taken as bytes,
//! uploaded to the hub, read back for the chat's viewer and written to this
//! Mac's attachment folder for «Abrir». Each order runs off the loop, like a
//! hub read, and answers its own `AttachmentResponse`. A HEIF photo (an
//! iPhone's HEIC), which no agent reads, is known by its content once it is
//! read and is first written as JPEG by the helper.
use super::Runtime;
use super::helper_waits;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::attachments::{
    Attachment, AttachmentLimits, AttachmentSource, admit_file, is_heif_photo, jpeg_name,
};
use orbit_core::protocol::{AttachmentContent, AttachmentResponse, Command};
use orbit_core::refusal::{AttachmentCause, Operation, Refusal};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

/// How long a HEIF photo waits for the helper's JPEG.
///
/// Protects: the person waiting on the photo's chip, and the order's slot,
/// when the helper does not answer. Why 20 s: decoding and encoding a
/// 48-megapixel photo with ImageIO takes a few seconds on a Mac; not
/// measured. Past it the photo is refused with `timeout`; it does not limit
/// the upload that follows.
const PHOTO_WAIT: Duration = Duration::from_secs(20);

/// What a pasted photo is written as before the helper reads it: never
/// `.jpg`, so never the JPEG written next to it.
const PASTED_PHOTO: &str = "pasted.heic";

/// A folder of the daemon's removed when its order ends, a privacy stop
/// that drops the order included: the photo pasted and the JPEG in it are
/// plain copies of the user's photo.
struct RemovedOnDrop(PathBuf);

impl Drop for RemovedOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What an attachment order needs, taken from the runtime when it arrives.
pub(super) struct AttachmentContext {
    settings: crate::config::Settings,
    token: Option<String>,
    development: bool,
    privacy_stopped: bool,
    connection: orbit_core::Connection,
    pub(super) limits: Option<AttachmentLimits>,
    op: orbit_core::notice::Op,
    /// `<daemon root>/attachments`, where «Abrir» writes a file and a
    /// photo is written as JPEG.
    folder: PathBuf,
    /// The loop's work queue: only the loop sends the helper an order.
    work: tokio::sync::mpsc::Sender<super::WorkResult>,
}

/// What one order produced.
enum Answer {
    Added(Attachment),
    Fetched(AttachmentContent),
    Saved(PathBuf),
}

impl Runtime {
    /// What an attachment order needs, taken now: the order runs later.
    pub(super) fn attachment_context(&self) -> AttachmentContext {
        AttachmentContext {
            settings: self.settings.clone(),
            token: self.link.token.clone(),
            development: self.development,
            privacy_stopped: self.state.privacy_stopped() || self.closing,
            connection: self.state.connection(),
            limits: self.assistant.hub_attachments,
            op: self.turn.op.clone(),
            folder: self.paths.root.join("attachments"),
            work: self.work_events.clone(),
        }
    }

    /// Serve one attachment order off the loop; its answer goes to `reply`.
    pub(super) fn serve_attachment(
        &mut self,
        command: Command,
        request_id: String,
        reply: orbit_ipc::ReplySender,
    ) {
        let context = self.attachment_context();
        let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
        let started_at_epoch = *privacy_epoch.borrow_and_update();
        let log_root = self.paths.root.clone();
        self.reads.tasks.push(tokio::spawn(async move {
            let id = request_id.clone();
            let answer = async move { context.answer(command, id).await };
            // A privacy stop while it runs ends it: nothing more leaves.
            let response = tokio::select! {
                _ = privacy_epoch.changed() => {
                    refused(request_id, Refusal::PrivacyStopped { since_ms: None })
                }
                response = answer => response,
            };
            let response = if started_at_epoch != *privacy_epoch.borrow() && response.ok {
                refused(response.id, Refusal::PrivacyStopped { since_ms: None })
            } else {
                response
            };
            if reply.send(response).is_err() {
                crate::diagnostics::DiagnosticLog::in_root(&log_root)
                    .record("ipc_reply_receiver_gone", &[("command", "attachment")]);
            }
        }));
    }
}

/// The order's whole answer: what it produced, or its typed refusal.
fn respond(id: String, outcome: Result<Answer>) -> AttachmentResponse {
    match outcome {
        Ok(answer) => {
            let mut response = AttachmentResponse {
                v: 1,
                id,
                ok: true,
                error: None,
                refusal: None,
                attachment: None,
                content: None,
                path: None,
                window_text: None,
            };
            match answer {
                Answer::Added(attachment) => response.attachment = Some(attachment),
                Answer::Fetched(content) => response.content = Some(content),
                Answer::Saved(path) => {
                    response.path = Some(path.to_string_lossy().into_owned());
                }
            }
            response
        }
        Err(error) => refused(id, super::refusal_from_error(&error)),
    }
}

fn refused(id: String, refusal: Refusal) -> AttachmentResponse {
    AttachmentResponse {
        v: 1,
        id,
        ok: false,
        error: Some(refusal.to_string()),
        refusal: Some(refusal),
        attachment: None,
        content: None,
        path: None,
        window_text: None,
    }
}

fn unreadable(name: Option<&str>, reason: &str) -> anyhow::Error {
    Refusal::Attachment {
        cause: AttachmentCause::Unreadable {
            name: name.map(str::to_owned),
            reason: reason.to_owned(),
        },
    }
    .into()
}

impl AttachmentContext {
    /// The order's whole answer: what it produced, or its typed refusal.
    pub(super) async fn answer(&self, command: Command, request_id: String) -> AttachmentResponse {
        // `if let`, not a match: every other order is not this module's, and
        // a wildcard arm would hide a new attachment order from the compiler.
        let outcome = if let Command::AttachmentAdd { source } = command {
            self.add(source).await.map(Answer::Added)
        } else if let Command::AttachmentFetch { attachment_id } = command {
            self.fetch(&attachment_id).await.map(Answer::Fetched)
        } else if let Command::AttachmentSave { attachment_id } = command {
            self.save(&attachment_id).await.map(Answer::Saved)
        } else {
            Err(anyhow::anyhow!("Unknown attachment command"))
        };
        respond(request_id, outcome)
    }

    /// A HEIF photo written as JPEG by the helper, then read and uploaded
    /// as any file, under the photo's name with `.jpg`; refused with the
    /// helper's cause when there is none. The helper reads a file: the
    /// photo's own (`path`), or the pasted `bytes` written first in the
    /// same private folder as the JPEG, which goes either way.
    async fn add_photo(
        &self,
        name: &str,
        path: Option<PathBuf>,
        bytes: Vec<u8>,
    ) -> Result<Attachment> {
        let folder = RemovedOnDrop(self.folder.join(format!("photo-{}", uuid::Uuid::new_v4())));
        let written = match path {
            Some(path) => private_folder(&folder.0)
                .map(|()| path)
                .map_err(anyhow::Error::from),
            None => write_private(&folder.0, PASTED_PHOTO, &bytes),
        };
        let source = written.map_err(|_| unreadable(Some(name), "io"))?;
        // Not held while the helper works: it reads the file.
        drop(bytes);
        let destination = folder.0.join(jpeg_name(file_name_in_folder(name)));
        let (asked, answer) = oneshot::channel();
        super::deliver_work(
            &self.work,
            super::WorkResult::PhotoJpeg {
                source_path: source,
                destination_path: destination.clone(),
                asked,
            },
        )
        .await;
        // The loop, and the helper with it, is gone only when the daemon closes.
        let answer = answer.await.map_err(|_| Refusal::HelperNotRunning)??;
        helper_waits::wait(answer, PHOTO_WAIT)
            .await
            .map_err(|cause| -> anyhow::Error {
                if cause == "timeout" {
                    let waited_ms = u64::try_from(PHOTO_WAIT.as_millis()).unwrap_or(u64::MAX);
                    Refusal::Timeout {
                        operation: Operation::Files,
                        waited_ms,
                        deadline_ms: waited_ms,
                    }
                    .into()
                } else {
                    unreadable(Some(name), &cause)
                }
            })?;
        let (name, bytes) = self.read_path(&destination).await?;
        self.upload(self.token()?, &name, bytes).await
    }

    /// Bytes the daemon itself made (the window's text), uploaded under
    /// `name` within the same ceilings as any file.
    pub(super) async fn add_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<Attachment> {
        let token = self.token()?;
        admit_file(self.limits.as_ref(), name, bytes.len() as u64)?;
        self.upload(token, name, bytes).await
    }

    fn token(&self) -> Result<&str> {
        Ok(crate::hub::access(
            &self.settings,
            self.token.as_deref(),
            self.privacy_stopped,
            self.connection.clone(),
        )?)
    }

    async fn add(&self, source: AttachmentSource) -> Result<Attachment> {
        let token = self.token()?;
        let (name, bytes, path) = match source {
            AttachmentSource::Path { path } => {
                let (name, bytes) = self.read_path(Path::new(&path)).await?;
                (name, bytes, Some(PathBuf::from(path)))
            }
            AttachmentSource::Bytes { name, data_base64 } => {
                // The decoded size from the base64 length, before decoding it.
                let decoded = (data_base64.len() / 4).saturating_mul(3) as u64;
                admit_file(self.limits.as_ref(), &name, decoded)?;
                let bytes = STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| unreadable(Some(&name), "invalid_bytes"))?;
                admit_file(self.limits.as_ref(), &name, bytes.len() as u64)?;
                (name, bytes, None)
            }
            // The window is captured on the loop (`attach_window`), which
            // then uploads it here with its name and bytes.
            AttachmentSource::Window => {
                return Err(anyhow::anyhow!(
                    "The window is captured in the daemon loop, not here"
                ));
            }
        };
        // Known by its content, whatever its name: a pasted photo, or one
        // called `.jpg`, is a HEIC all the same.
        if is_heif_photo(&bytes) {
            return self.add_photo(&name, path, bytes).await;
        }
        self.upload(token, &name, bytes).await
    }

    /// A capture already on the hub (`PUT /v1/captures`), kept as an attachment.
    pub(super) async fn attachment_from_capture(
        &self,
        capture_id: &str,
        request_id: &str,
        name: &str,
    ) -> Result<Attachment> {
        let token = self.token()?;
        crate::hub::attachment_from_capture(
            &self.settings,
            token,
            self.development,
            capture_id,
            request_id,
            name,
            &self.op,
        )
        .await
    }

    async fn upload(&self, token: &str, name: &str, bytes: Vec<u8>) -> Result<Attachment> {
        crate::hub::upload_attachment(
            &self.settings,
            token,
            self.development,
            name,
            bytes,
            &self.op,
        )
        .await
    }

    /// The file at `path`, refused before reading it when the hub takes no
    /// attachments or the file is over the ceiling; read whole, at most
    /// one byte past the ceiling so a file that grows is still refused.
    async fn read_path(&self, path: &Path) -> Result<(String, Vec<u8>)> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| unreadable(None, "not_a_file"))?;
        if self.limits.is_none() {
            admit_file(None, &name, 0)?;
        }
        let metadata = tokio::fs::metadata(path).await.map_err(|error| {
            let reason = if error.kind() == std::io::ErrorKind::NotFound {
                "missing"
            } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                "permission"
            } else {
                "io"
            };
            unreadable(Some(&name), reason)
        })?;
        if !metadata.is_file() {
            return Err(unreadable(Some(&name), "not_a_file"));
        }
        admit_file(self.limits.as_ref(), &name, metadata.len())?;
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            unreadable(
                Some(&name),
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    "permission"
                } else {
                    "io"
                },
            )
        })?;
        let ceiling = orbit_core::limits::MAX_ATTACHMENT_BYTES as u64;
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.take(ceiling + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| unreadable(Some(&name), "io"))?;
        admit_file(self.limits.as_ref(), &name, bytes.len() as u64)?;
        Ok((name, bytes))
    }

    async fn fetch(&self, attachment_id: &str) -> Result<AttachmentContent> {
        let token = self.token()?;
        if !orbit_core::wire::canonical_uuid(attachment_id) {
            return Err(Refusal::Attachment {
                cause: AttachmentCause::NotFound {
                    attachment_id: attachment_id.chars().take(80).collect(),
                },
            }
            .into());
        }
        crate::hub::fetch_attachment(
            &self.settings,
            token,
            self.development,
            attachment_id,
            &self.op,
        )
        .await
    }

    /// The attachment written as `<folder>/<id>/<name>`, readable only by
    /// this user, so that «Abrir» hands macOS a real file. The name is the
    /// one the hub keeps; it was refused there if it held a `/`.
    async fn save(&self, attachment_id: &str) -> Result<PathBuf> {
        let content = self.fetch(attachment_id).await?;
        retire_saved(&self.folder, std::time::SystemTime::now());
        let directory = self.folder.join(attachment_id);
        write_private(&directory, &content.name, &content.bytes)
            .with_context(|| format!("Could not write the attachment to {}", directory.display()))
    }
}

/// How long a file written for «Abrir» stays in the attachment folder: it is
/// a plain copy of something the hub keeps encrypted, there only so another
/// app can open it. A day outlasts reading it in that app; past it the copy is
/// deleted when the next one is written. It does not touch the hub's copy.
const SAVED_ATTACHMENT_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Delete the copies written for «Abrir» more than `SAVED_ATTACHMENT_TTL`
/// ago. Best effort: a copy that cannot be removed now is tried next time.
fn retire_saved(folder: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > SAVED_ATTACHMENT_TTL);
        if old && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `directory` and its parent (the attachment folder), both private to this
/// user.
fn private_folder(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if let Some(parent) = directory.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
}

/// `name` as a file inside a folder: a name that would leave it, or that
/// names no file, is written under a plain one.
fn file_name_in_folder(name: &str) -> &str {
    match name {
        "" | "." | ".." => "adjunto",
        name if name.contains('/') => "adjunto",
        name => name,
    }
}

/// Write `bytes` as `directory/name`, both private to this user.
fn write_private(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    private_folder(directory)?;
    let path = directory.join(file_name_in_folder(name));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        // A link planted where the file goes is refused, not followed.
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(path)
}

#[cfg(test)]
#[path = "attachments_saved_tests.rs"]
mod tests;
