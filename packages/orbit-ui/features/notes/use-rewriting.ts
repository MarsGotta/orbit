import { useState } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { isRecording, templateOf } from "./note-progress";
import { sent, type Send } from "./send";

/** Writing the note again, wherever it was asked from: the header's menu, the Resumen's toolbar or a band. */
export interface Rewriting {
  /** The template the note is being written with now, or `null`. */
  template: string | null;
  /** The menu's own orders, with the name of the template they write it with. */
  start: (template: string, work: Promise<unknown>) => void;
  /** «Rehacer resumen»: the note written again with its own template. */
  redo: () => void;
}

/**
 * «Reescribiendo la nota…» from the press until the hub has taken it, then
 * while the hub writes it.
 */
export function useRewriting(note: OpenNoteSnapshot, send: Send): Rewriting {
  const [asked, setAsked] = useState<{ id: string; template: string } | null>(null);
  const start = (template: string, work: Promise<unknown>) => {
    const request = { id: note.recording_id, template };
    setAsked(request);
    const done = () => setAsked((value) => (value === request ? null : value));
    work.then(done, done);
  };
  const redo = () => {
    const template = templateOf(note);
    start(
      note.summary?.template_name ?? template,
      sent(send, {
        command: "note_regenerate",
        recording_id: note.recording_id,
        template_id: template,
      }),
    );
  };
  const writing =
    note.summary !== null &&
    note.progress.summary.state === "active" &&
    !isRecording(note.progress);
  const template =
    asked?.id === note.recording_id
      ? asked.template
      : writing
        ? (note.summary?.template_name ?? null)
        : null;
  return { template, start, redo };
}
