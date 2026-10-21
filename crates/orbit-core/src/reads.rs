//! Keyed, paged reads. These heads reserve the protocol for feature migrations.
use crate::topics::Cursor;
use serde::{Deserialize, Serialize};

macro_rules! reads {
    ($($variant:ident $({ $($field:ident: $ty:ty),* $(,)? })? => $max:expr;)+) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum ReadRequest {
            $($variant { $($($field: $ty,)*)? cursor: Option<Cursor> },)+
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum ReadId { $($variant,)+ }

        impl ReadRequest {
            pub fn id(&self) -> ReadId {
                match self { $(Self::$variant { .. } => ReadId::$variant,)+ }
            }
        }

        impl ReadId {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];
            pub const fn max_bytes(self) -> usize {
                match self { $(Self::$variant => $max,)+ }
            }
        }
    };
}

// Each bound protects one read response allocation and serialization time;
// none limits a collection's length.
// AgendaRange: 128 KiB holds a loaded week (112 KiB); months continue by cursor.
// AgendaCalendars: 64 KiB holds about 330 calendars, without a calendar limit.
// NotesPage (SIM-41): one page of the hub's list, at most fifty meetings
// (`notes::LIST_PAGE`), plus on the first page this Mac's recordings the hub
// does not have whole (the one being recorded and up to a hundred still
// uploading). At every bound — a 256-character title and a 1,000-character
// short summary per hub row, a 256-character title per local one, every
// character a control character escaped as six bytes — the page measured
// 562,524 bytes (`sim41_a_first_page_at_every_bound_fits_its_read`); 640 KiB
// holds it, the largest read there already is. A larger page is refused with
// its size.
// NoteDetail: the hub's document of one meeting with its note as Mars sees it
// and her private notes, not the turns. Measured on 2026-10-14 (P9-03): a
// four-hour note is ~19 KB, 52,933 bytes at the hub's default 8,192-token
// completion ceiling; private notes go up to 65,536 characters, 131,072 bytes
// of accented Spanish. 256 KiB holds both; it bounds one read.
// NoteTranscript: one page of the hub's turns (`/turns?offset=`), which the
// hub bounds at `notes::TURN_PAGE_BYTES` of turns (256 KiB, about two hours);
// passed through, the largest measured 261,814 bytes with its fields. 4 KiB
// above the hub's bound holds the page's own fields. A single turn larger
// than that is refused with its size, never cut.
// NoteLibrary (P9-05): templates and assignments, one per item, paged by
// index. A template written to every hub bound is ~116 KiB of four-byte text,
// so a 256 KiB page always holds one.
// Sessions: one hub page of 128 rows (`SESSION_LIST_LIMIT`), each an id and
// three numbers, measured at 19,637 bytes with 36-byte ids and the largest
// numbers (`sim30_a_full_sessions_page_fits_its_read`); 32 KiB holds it, not a
// history limit.
// SessionMessages (SIM-30): one hub page of a conversation, at most 512 KiB of
// JSON (`SESSION_MESSAGES_PAGE_BYTES` in server/orbit_hub/storage.py), with
// what the daemon adds to each message (`audio_available`, the Mac's id): a
// full page measured 516,314 bytes (`sim30_a_full_messages_page_fits_its_read`).
// 640 KiB holds it below the 1 MiB IPC frame. This bounds one read, not a
// conversation.
// ControlHistory and EngineTools: 96 KiB pages the 148 KiB measured catalog.
reads! {
    AgendaRange { start_ms: i64, end_ms: i64 } => 128 * 1024;
    AgendaCalendars => 64 * 1024;
    NotesPage => 640 * 1024;
    NoteDetail { id: String } => 256 * 1024;
    NoteTranscript { id: String } => crate::notes::TURN_PAGE_BYTES + 4 * 1024;
    NoteLibrary => 256 * 1024;
    Sessions => 32 * 1024;
    SessionMessages { session_id: String } => 640 * 1024;
    ControlHistory => 96 * 1024;
    EngineTools => 96 * 1024;
}

pub const fn max_page_bytes() -> usize {
    let mut max = 0;
    let mut index = 0;
    while index < ReadId::ALL.len() {
        let bytes = ReadId::ALL[index].max_bytes();
        if bytes > max {
            max = bytes;
        }
        index += 1;
    }
    max
}
