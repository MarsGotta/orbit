//! Files the user attaches to a chat turn: what the Mac knows of
//! one, where its bytes come from, and the pure checks made before anything
//! is read or sent. The hub keeps the files and repeats every check.
use crate::refusal::{AttachmentCause, AttachmentCeiling, Refusal};
use serde::{Deserialize, Serialize};

/// What an attachment is to an agent, as the hub classified it by content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Pdf,
    Text,
}

/// One file the hub keeps for this device, as the hub describes it
/// (`POST /v1/attachments`, the messages page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub kind: AttachmentKind,
    pub mime: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// A PDF's page count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub pages: Option<u64>,
}

/// The hub's ceilings, from its `ready`. A hub that sends none (0.9.0) has
/// no attachments at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AttachmentLimits {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_file_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_message_files: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_message_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_session_bytes: u64,
}

/// Where a file for the composer comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "from", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttachmentSource {
    /// A file on this Mac, chosen in the dialog or dropped on the window:
    /// the daemon reads it by its path; its bytes never cross the interface.
    Path { path: String },
    /// Bytes the interface already holds: a pasted image, or a long pasted
    /// text («Texto pegado»).
    Bytes { name: String, data_base64: String },
    /// «Adjuntar ventana»: the frontmost window that is not Orbit's,
    /// captured as Orbit already captures it for the assistant.
    Window,
}

/// What «Adjuntar ventana» did with the window's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WindowText {
    /// Kept as its own `.txt` attachment, next to the capture.
    Attached { attachment: Attachment },
    /// Not read; the capture went alone. `cause` is `empty` for a window
    /// that shows no text Accessibility can read, or the code of what
    /// failed: the helper's (`accessibility_permission_required`…), the
    /// engine's (`engine_…`) or the wait's (`timeout`, `helper_restarted`).
    Missing { cause: String },
    /// Read, but the hub did not keep it: its refusal, names and numbers
    /// included. The capture went alone.
    Refused {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        refusal: Refusal,
    },
}

/// The answer to «Adjuntar»: the file the hub keeps and, for «Adjuntar
/// ventana», what became of the window's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AddedAttachment {
    pub attachment: Attachment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub window_text: Option<WindowText>,
}

/// Whether a file's name says it is a HEIF photo (an iPhone's `.HEIC`).
/// No agent reads one, so the Mac turns it into JPEG before uploading it.
pub fn is_heif_photo(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (extension.eq_ignore_ascii_case("heic") || extension.eq_ignore_ascii_case("heif"))
    })
}

/// The name of a photo once it is JPEG: the same name, with `.jpg`.
pub fn jpeg_name(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    format!("{stem}.jpg")
}

/// Whether one file of `bytes` may be uploaded: the hub must take
/// attachments, and the file must fit both the hub's ceiling and the Mac's
/// (`limits::MAX_ATTACHMENT_BYTES`, what the daemon reads whole).
pub fn admit_file(
    limits: Option<&AttachmentLimits>,
    name: &str,
    bytes: u64,
) -> Result<(), Refusal> {
    let limits = limits.ok_or(Refusal::Attachment {
        cause: AttachmentCause::Unsupported,
    })?;
    let max_bytes = limits
        .max_file_bytes
        .min(crate::limits::MAX_ATTACHMENT_BYTES as u64);
    if bytes > max_bytes {
        return Err(Refusal::Attachment {
            cause: AttachmentCause::TooLarge {
                ceiling: AttachmentCeiling::File,
                name: Some(name.to_owned()),
                files: None,
                bytes,
                session_bytes: None,
                max_bytes,
            },
        });
    }
    Ok(())
}

/// Whether a turn may carry `attachments`: a hub that takes them, at most
/// its files and bytes per message. The conversation's own ceiling only the
/// hub can judge: it alone knows what the conversation already holds.
pub fn admit_message(
    limits: Option<&AttachmentLimits>,
    attachments: &[Attachment],
) -> Result<(), Refusal> {
    if attachments.is_empty() {
        return Ok(());
    }
    let limits = limits.ok_or(Refusal::Attachment {
        cause: AttachmentCause::Unsupported,
    })?;
    let files = attachments.len() as u64;
    if files > limits.max_message_files {
        return Err(Refusal::Attachment {
            cause: AttachmentCause::TooMany {
                files,
                max_files: limits.max_message_files,
            },
        });
    }
    let bytes = attachments
        .iter()
        .map(|attachment| attachment.bytes)
        .fold(0_u64, u64::saturating_add);
    if bytes > limits.max_message_bytes {
        return Err(Refusal::Attachment {
            cause: AttachmentCause::TooLarge {
                ceiling: AttachmentCeiling::Message,
                name: None,
                files: Some(files),
                bytes,
                session_bytes: None,
                max_bytes: limits.max_message_bytes,
            },
        });
    }
    Ok(())
}

/// The hub's attachment refusal (`refusal` of an HTTP error body or of a
/// chat `error` frame) as the Mac's, with every name and number it carried;
/// `None` for a code that is not an attachment's.
pub fn hub_refusal(fields: &serde_json::Value) -> Option<Refusal> {
    let text = |key: &str| fields.get(key).and_then(serde_json::Value::as_str);
    let owned = |key: &str| text(key).map(str::to_owned);
    let number = |key: &str| fields.get(key).and_then(serde_json::Value::as_u64);
    let cause = match text("code")? {
        "attachments_too_large" => AttachmentCause::TooLarge {
            ceiling: match text("ceiling")? {
                "file" => AttachmentCeiling::File,
                "message" => AttachmentCeiling::Message,
                "session" => AttachmentCeiling::Session,
                _ => return None,
            },
            name: owned("name"),
            files: number("files"),
            bytes: number("bytes")?,
            session_bytes: number("session_bytes"),
            max_bytes: number("max_bytes")?,
        },
        "attachments_too_many" => AttachmentCause::TooMany {
            files: number("files")?,
            max_files: number("max_files")?,
        },
        "attachment_type_unsupported" => AttachmentCause::TypeUnsupported {
            name: owned("name")?,
            mime: owned("mime")?,
        },
        "attachment_unreadable" => AttachmentCause::Unreadable {
            name: owned("name"),
            reason: owned("cause")?,
        },
        "attachment_refused" => AttachmentCause::Refused {
            name: owned("name"),
            upstream_code: fields
                .get("upstream_code")
                .and_then(serde_json::Value::as_i64),
        },
        "attachment_not_found" => AttachmentCause::NotFound {
            attachment_id: owned("attachment_id")?,
        },
        _ => return None,
    };
    Some(Refusal::Attachment { cause })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: AttachmentLimits = AttachmentLimits {
        max_file_bytes: 100,
        max_message_files: 2,
        max_message_bytes: 150,
        max_session_bytes: 1000,
    };

    fn file(name: &str, bytes: u64) -> Attachment {
        Attachment {
            id: format!("id-{name}"),
            name: name.into(),
            kind: AttachmentKind::Text,
            mime: "text/plain".into(),
            bytes,
            pages: None,
        }
    }

    #[test]
    fn a_hub_without_attachments_refuses_before_anything_is_read() {
        assert_eq!(
            admit_file(None, "nota.txt", 1),
            Err(Refusal::Attachment {
                cause: AttachmentCause::Unsupported
            })
        );
        assert_eq!(
            admit_message(None, &[file("nota.txt", 1)]),
            Err(Refusal::Attachment {
                cause: AttachmentCause::Unsupported
            })
        );
        assert_eq!(admit_message(None, &[]), Ok(()));
    }

    #[test]
    fn a_file_over_the_ceiling_is_named_with_its_size() {
        assert_eq!(admit_file(Some(&LIMITS), "a.txt", 100), Ok(()));
        assert_eq!(
            admit_file(Some(&LIMITS), "grande.pdf", 101),
            Err(Refusal::Attachment {
                cause: AttachmentCause::TooLarge {
                    ceiling: AttachmentCeiling::File,
                    name: Some("grande.pdf".into()),
                    files: None,
                    bytes: 101,
                    session_bytes: None,
                    max_bytes: 100,
                }
            })
        );
    }

    #[test]
    fn a_heic_photo_is_known_by_its_name_and_becomes_a_jpg() {
        for name in ["IMG_0001.HEIC", "foto.heic", "playa.Heif"] {
            assert!(is_heif_photo(name), "{name}");
        }
        for name in ["foto.jpg", "heic", ".heic", "notas.heic.txt"] {
            assert!(!is_heif_photo(name), "{name}");
        }
        assert_eq!(jpeg_name("IMG_0001.HEIC"), "IMG_0001.jpg");
        assert_eq!(jpeg_name("vacaciones.2026.heif"), "vacaciones.2026.jpg");
    }

    #[test]
    fn the_mac_never_reads_more_than_its_own_ceiling() {
        let generous = AttachmentLimits {
            max_file_bytes: u64::MAX,
            ..LIMITS
        };
        let over = crate::limits::MAX_ATTACHMENT_BYTES as u64 + 1;
        assert!(matches!(
            admit_file(Some(&generous), "enorme.bin", over),
            Err(Refusal::Attachment {
                cause: AttachmentCause::TooLarge { max_bytes, .. }
            }) if max_bytes == crate::limits::MAX_ATTACHMENT_BYTES as u64
        ));
    }

    /// The hub's refusals (server/tests/test_attachments.py pins their
    /// fields) keep every name and number on the Mac.
    #[test]
    fn the_hub_s_refusals_keep_their_names_and_numbers() {
        let read = |value: serde_json::Value| hub_refusal(&value);
        assert_eq!(
            read(
                serde_json::json!({"code":"attachments_too_large","ceiling":"session",
                "bytes":10,"session_bytes":250,"max_bytes":256,"retry":{"kind":"after_fix"}})
            ),
            Some(Refusal::Attachment {
                cause: AttachmentCause::TooLarge {
                    ceiling: AttachmentCeiling::Session,
                    name: None,
                    files: None,
                    bytes: 10,
                    session_bytes: Some(250),
                    max_bytes: 256,
                }
            })
        );
        assert_eq!(
            read(serde_json::json!({"code":"attachment_type_unsupported",
                "name":"fotos.zip","mime":"application/zip"})),
            Some(Refusal::Attachment {
                cause: AttachmentCause::TypeUnsupported {
                    name: "fotos.zip".into(),
                    mime: "application/zip".into()
                }
            })
        );
        assert_eq!(
            read(
                serde_json::json!({"code":"attachment_refused","name":"a.png",
                "upstream_code":4018})
            ),
            Some(Refusal::Attachment {
                cause: AttachmentCause::Refused {
                    name: Some("a.png".into()),
                    upstream_code: Some(4018)
                }
            })
        );
        assert_eq!(read(serde_json::json!({"code":"quota_exceeded"})), None);
        assert_eq!(
            read(
                serde_json::json!({"code":"attachments_too_large","ceiling":"planet",
                "bytes":1,"max_bytes":1})
            ),
            None
        );
    }

    #[test]
    fn a_message_is_held_to_its_files_and_bytes() {
        assert_eq!(
            admit_message(Some(&LIMITS), &[file("a", 75), file("b", 75)]),
            Ok(())
        );
        assert_eq!(
            admit_message(Some(&LIMITS), &[file("a", 1), file("b", 1), file("c", 1)]),
            Err(Refusal::Attachment {
                cause: AttachmentCause::TooMany {
                    files: 3,
                    max_files: 2
                }
            })
        );
        assert_eq!(
            admit_message(Some(&LIMITS), &[file("a", 100), file("b", 51)]),
            Err(Refusal::Attachment {
                cause: AttachmentCause::TooLarge {
                    ceiling: AttachmentCeiling::Message,
                    name: None,
                    files: Some(2),
                    bytes: 151,
                    session_bytes: None,
                    max_bytes: 150,
                }
            })
        );
    }
}
