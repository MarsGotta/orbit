import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { isRecording } from "./note-progress";

export type NoteTab = "actions" | "private" | "summary" | "transcript";

/**
 * The tab a note opens on: while recording, the private notes; once
 * written, the summary; while it is being written, the transcript as it fills.
 */
export function defaultTab(note: OpenNoteSnapshot): NoteTab {
  if (isRecording(note.progress)) return "private";
  if (note.summary) return "summary";
  return "transcript";
}
