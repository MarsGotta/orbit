import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { isRecording } from "./note-progress";

export type NoteTab = "actions" | "private" | "summary" | "transcript";

/**
 * The tab a note opens on (P9-04): while recording, the private notes;
 * once written, the summary; otherwise the transcript.
 */
export function defaultTab(note: OpenNoteSnapshot): NoteTab {
  if (isRecording(note.progress)) return "private";
  if (note.summary) return "summary";
  return "transcript";
}
