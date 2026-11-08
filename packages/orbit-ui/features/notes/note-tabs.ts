import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { isRecording } from "./note-progress";
import type { NoteTab } from "../../state/ui";

export type { NoteTab };

/**
 * The tab a note opens on: while recording, the private notes; once
 * written, the summary; while it is being written, the transcript as it fills.
 */
export function defaultTab(note: OpenNoteSnapshot): NoteTab {
  if (isRecording(note.progress)) return "private";
  if (note.summary) return "summary";
  return "transcript";
}
