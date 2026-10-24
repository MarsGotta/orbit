import { useEffect, useRef, useState } from "react";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import { Button } from "../../design/Button";
import { Toast } from "../../design/Toast";
import { es } from "../../i18n/es";
import type { AppView } from "../../state/app-view";
import { openMeeting } from "./note-route";
import { noteRowState } from "./note-row";
import type { Send } from "./send";

/** What the notice says of one meeting, from its end to its note. */
type Shown =
  | { stage: "saved"; recordingId: string }
  | { stage: "ready"; recordingId: string; title: string; key: number }
  | { stage: "failed"; recordingId: string; title: string; text: string };

type Meeting = AppView["meeting"];
type Notes = AppView["notes"];

/** The note's failure as the list says it, if its row says one. */
function failure(row: NoteListItem | undefined): string | null {
  if (!row) return null;
  const state = noteRowState(row);
  return state.tone === "failed" ? state.text : null;
}

/** «Reunión guardada» when a meeting ends saved (one thrown away is not in `recent`). */
function useSavedMeeting(meeting: Meeting, show: (shown: Shown) => void) {
  const previous = useRef(meeting);
  useEffect(() => {
    const prior = previous.current;
    previous.current = meeting;
    if (prior.phase === "idle" || meeting.phase !== "idle") return;
    const saved = meeting.recent.find((recent) => recent.recording_id === prior.recording_id);
    if (saved) show({ stage: "saved", recordingId: saved.recording_id });
  }, [meeting, show]);
}

/** «Tu nota está lista» for each note the window saw become ready, once. */
function useReadyNote(notes: Notes, closed: { current: string | null }, show: (s: Shown) => void) {
  const seen = useRef(notes?.ready?.sequence ?? 0);
  const ready = notes?.ready;
  useEffect(() => {
    if (!ready || ready.sequence <= seen.current) return;
    seen.current = ready.sequence;
    if (ready.recording_id === closed.current) return;
    show({
      stage: "ready",
      recordingId: ready.recording_id,
      title: ready.title,
      key: ready.sequence,
    });
  }, [closed, ready, show]);
}

/** The saved meeting's note failed: its row in the list says so. */
function useFailedNote(shown: Shown | null, notes: Notes, show: (shown: Shown) => void) {
  const savedId = shown?.stage === "saved" ? shown.recordingId : null;
  const row = notes?.list.find((item) => item.recording_id === savedId);
  const failed = failure(row);
  useEffect(() => {
    if (savedId !== null && row && failed !== null) {
      show({ stage: "failed", recordingId: savedId, title: row.title, text: failed });
    }
  }, [failed, row, savedId, show]);
}

/**
 * One notice for each meeting, from its end to its note. «Reunión
 * guardada» stays while the note is written (about half a minute), then
 * becomes «Tu nota está lista · Abrir», or says the failure in the list's
 * words; closed by hand, it says nothing more of that meeting. A note
 * ready for another meeting, while this window looked, is said the same
 * way, once; one ready before it looked says nothing.
 */
export function MeetingNoteNotice({
  meeting,
  notes,
  send,
}: {
  meeting: Meeting;
  notes?: Notes;
  send: Send;
}) {
  const [shown, setShown] = useState<Shown | null>(null);
  const closed = useRef<string | null>(null);
  useSavedMeeting(meeting, setShown);
  useReadyNote(notes, closed, setShown);
  useFailedNote(shown, notes, setShown);
  if (shown === null) return null;
  const close = () => {
    closed.current = shown.recordingId;
    setShown(null);
  };
  const open = () => {
    setShown(null);
    openMeeting(shown.recordingId, send);
  };
  return <MeetingNoteToast close={close} open={open} shown={shown} />;
}

function MeetingNoteToast({
  shown,
  close,
  open,
}: {
  shown: Shown;
  close: () => void;
  open: () => void;
}) {
  if (shown.stage === "saved") {
    return (
      <Toast
        detail={es.meeting.savedDetail}
        key={`saved-${shown.recordingId}`}
        message={es.meeting.saved}
        onClose={close}
        persistent
      />
    );
  }
  const action = (
    <Button onClick={open} size={28} variant="outline">
      {es.noteScreen.open}
    </Button>
  );
  return (
    <Toast
      action={action}
      detail={shown.title}
      key={`${shown.stage}-${shown.stage === "ready" ? shown.key : shown.recordingId}`}
      kind={shown.stage === "failed" ? "failed" : "done"}
      message={shown.stage === "failed" ? shown.text : es.noteScreen.ready}
      onClose={close}
    />
  );
}
