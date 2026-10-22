//! The composer's files: read here by path or taken as bytes,
//! uploaded to the hub, read back for the chat's viewer and written to this
//! Mac's attachment folder for «Abrir». Each order runs off the loop, like a
//! hub read, and answers its own `AttachmentResponse`. A HEIC photo, which
//! no agent reads, is first written as JPEG by the helper.
use super::Runtime;
use super::helper_waits::{self, HelperAnswer};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::attachments::{
    Attachment, AttachmentLimits, AttachmentSource, admit_file, is_heif_photo, jpeg_name,
};
use orbit_core::protocol::{AttachmentContent, AttachmentResponse, Command};
use orbit_core::refusal::{AttachmentCause, Operation, Refusal};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

/// How long a HEIC photo waits for the helper's JPEG.
///
/// Protects: the person waiting on the photo's chip, and the order's slot,
/// when the helper does not answer. Why 20 s: decoding and encoding a
/// 48-megapixel photo with ImageIO takes a few seconds on a Mac; not
/// measured. Past it the photo is refused with `timeout`; it does not limit
/// the upload that follows.
const PHOTO_WAIT: Duration = Duration::from_secs(20);

/// A HEIC photo on its way to JPEG: the helper writes `destination`, inside
/// `folder`, which goes with the order however it ends.
struct PhotoConversion {
    name: String,
    folder: RemovedOnDrop,
    destination: PathBuf,
    answer: Result<oneshot::Receiver<HelperAnswer>>,
}

/// A folder of the daemon's removed when its order ends, a privacy stop
/// that drops the order included: the JPEG in it is a plain copy of the
/// user's photo.
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
    /// `<daemon root>/attachments`, where «Abrir» writes a file.
    folder: PathBuf,
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
            token: self.token.clone(),
            development: self.development,
            privacy_stopped: self.state.privacy_stopped() || self.closing,
            connection: self.state.connection(),
            limits: self.conversation.hub_attachments,
            op: self.turn.op.clone(),
            folder: self.paths.root.join("attachments"),
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
        let photo = self.convert_photo(&command);
        let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
        let started_at_epoch = *privacy_epoch.borrow_and_update();
        let log_root = self.paths.root.clone();
        self.reads.tasks.push(tokio::spawn(async move {
            let id = request_id.clone();
            let answer = async move {
                match photo {
                    Some(photo) => respond(id, context.add_photo(photo).await.map(Answer::Added)),
                    None => context.answer(command, id).await,
                }
            };
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

    /// A HEIC photo chosen by its path is first written as JPEG by the
    /// helper, under the photo's name with `.jpg`, in a private folder of
    /// the daemon's; `None` for any other order, and for a hub that takes
    /// no attachments (the order is refused for that).
    fn convert_photo(&mut self, command: &Command) -> Option<PhotoConversion> {
        let Command::AttachmentAdd {
            source: AttachmentSource::Path { path },
        } = command
        else {
            return None;
        };
        let name = Path::new(path).file_name()?.to_string_lossy().into_owned();
        if !is_heif_photo(&name) || self.conversation.hub_attachments.is_none() {
            return None;
        }
        let folder = self
            .paths
            .root
            .join("attachments")
            .join(format!("photo-{}", uuid::Uuid::new_v4()));
        let destination = folder.join(jpeg_name(&name));
        let answer = private_folder(&folder)
            .map_err(|_| unreadable(Some(&name), "io"))
            .and_then(|()| {
                self.ask_helper(
                    "photo_jpeg",
                    json!({"source_path": path, "destination_path": destination}),
                    Operation::Files,
                )
                .map_err(anyhow::Error::from)
            });
        Some(PhotoConversion {
            name,
            folder: RemovedOnDrop(folder),
            destination,
            answer,
        })
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
            Err(anyhow::anyhow!("Orden de adjunto desconocida"))
        };
        respond(request_id, outcome)
    }

    /// The JPEG the helper wrote for a HEIC photo, read and uploaded as any
    /// file; the photo is refused with the helper's cause when there is
    /// none. The folder goes either way.
    async fn add_photo(&self, photo: PhotoConversion) -> Result<Attachment> {
        let PhotoConversion {
            name,
            folder: _folder,
            destination,
            answer,
        } = photo;
        helper_waits::wait(answer?, PHOTO_WAIT)
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
                    unreadable(Some(&name), &cause)
                }
            })?;
        self.add(AttachmentSource::Path {
            path: destination.to_string_lossy().into_owned(),
        })
        .await
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
        let (name, bytes) = match source {
            AttachmentSource::Path { path } => self.read_path(Path::new(&path)).await?,
            AttachmentSource::Bytes { name, data_base64 } => {
                // The decoded size from the base64 length, before decoding it.
                let decoded = (data_base64.len() / 4).saturating_mul(3) as u64;
                admit_file(self.limits.as_ref(), &name, decoded)?;
                let bytes = STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| unreadable(Some(&name), "invalid_bytes"))?;
                admit_file(self.limits.as_ref(), &name, bytes.len() as u64)?;
                (name, bytes)
            }
            // The window is captured on the loop (`attach_window`), which
            // then uploads it here with its name and bytes.
            AttachmentSource::Window => {
                return Err(anyhow::anyhow!(
                    "La ventana se captura en el bucle del daemon, no aquí"
                ));
            }
        };
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
            .with_context(|| format!("No se pudo escribir el adjunto en {}", directory.display()))
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

/// Write `bytes` as `directory/name`, both private to this user.
fn write_private(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let name = match name {
        "" | "." | ".." => "adjunto",
        name if name.contains('/') => "adjunto",
        name => name,
    };
    private_folder(directory)?;
    let path = directory.join(name);
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
mod tests {
    use super::*;

    #[test]
    fn a_copy_for_opening_is_deleted_after_a_day() -> Result<()> {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("attachments");
        let old = write_private(&folder.join("id-viejo"), "viejo.pdf", b"%PDF")?;
        let fresh = write_private(&folder.join("id-nuevo"), "nuevo.pdf", b"%PDF")?;
        let now = std::time::SystemTime::now();
        std::fs::File::open(folder.join("id-viejo"))?
            .set_modified(now - SAVED_ATTACHMENT_TTL - std::time::Duration::from_secs(60))?;
        retire_saved(&folder, now);
        assert!(!old.exists() && !folder.join("id-viejo").exists());
        assert!(fresh.exists());
        Ok(())
    }

    #[test]
    fn a_saved_attachment_is_private_and_never_follows_a_link() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let directory = root.path().join("attachments").join("id-1");
        let path = write_private(&directory, "informe.pdf", b"%PDF-1.4")?;
        assert_eq!(std::fs::read(&path)?, b"%PDF-1.4");
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory)?.permissions().mode() & 0o777,
            0o700
        );
        // A name that would leave the folder is written under a plain one.
        let safe = write_private(&directory, "..", b"x")?;
        assert_eq!(safe, directory.join("adjunto"));
        // A link where the file goes is refused.
        std::os::unix::fs::symlink(root.path().join("fuera"), directory.join("enlace.txt"))?;
        assert!(write_private(&directory, "enlace.txt", b"x").is_err());
        assert!(!root.path().join("fuera").exists());
        Ok(())
    }
}
