import { getCopy } from "../../i18n";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";

/** How a row's line reads: red while recording or failed, muted while work goes on. */
export type RowTone = "live" | "working" | "failed" | "plain";

export interface RowState {
  text: string;
  tone: RowTone;
}

/** Where the recording stands when its note says nothing yet. */
function stateLine(note: NoteListItem): RowState {
  switch (note.state) {
    case "recording":
      return { text: getCopy().noteScreen.listRecording, tone: "live" };
    case "uploading":
      return { text: getCopy().noteScreen.listUploading, tone: "working" };
    case "transcribing":
      return { text: getCopy().noteScreen.listTranscribing, tone: "working" };
    case "ready":
      return { text: getCopy().noteScreen.listTranscribed, tone: "plain" };
    case "failed":
      return { text: getCopy().noteScreen.listTranscriptFailed, tone: "failed" };
  }
}

/**
 * The line under a note's title in the list: recording, where its
 * note stands, or how its short summary begins. A failure says so; its
 * cause stays in the log.
 */
export function noteRowState(note: NoteListItem): RowState {
  if (note.state === "recording" || note.state === "uploading") return stateLine(note);
  switch (note.headline?.phase) {
    case "transcribing":
      return { text: getCopy().noteScreen.listTranscribing, tone: "working" };
    case "writing":
      return { text: getCopy().noteScreen.listWriting, tone: "working" };
    case "failed":
    case "not_configured":
      return { text: getCopy().noteScreen.listFailed, tone: "failed" };
    case "ready":
      return { text: note.headline.preview, tone: "plain" };
    case undefined:
      return stateLine(note);
  }
}
