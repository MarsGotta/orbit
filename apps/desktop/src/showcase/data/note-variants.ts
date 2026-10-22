import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { reviewNote } from "./note-open";
import { speakers, turns } from "./transcript";
import { at } from "./time";

const unwritten = {
  summary: null,
  note_revision: null,
  actions: [],
  decisions: [],
  open_topics: [],
  series: undefined,
};

/** Yesterday's call with Javier: uploaded, half transcribed, not written yet. */
export const transcribingNote: OpenNoteSnapshot = {
  ...reviewNote,
  ...unwritten,
  recording_id: "rec-api-20",
  title: "Llamada con Javier sobre la API de tarifas",
  started_at_ms: at(-1, 17, 10),
  ended_at_ms: at(-1, 17, 42),
  event: null,
  private_notes: "",
  progress: {
    upload: { state: "done", cause: null },
    transcript: { state: "active", cause: { kind: "transcribing", chunks: 4 } },
    summary: { state: "waiting", cause: { kind: "waiting_transcript" } },
  },
  transcript: {
    language: "es",
    turns: [
      {
        index: 0,
        at_ms: 4_000,
        speaker_key: "mic_local",
        text: "Javier, cuéntame cómo va la API de tarifas.",
      },
      {
        index: 1,
        at_ms: 9_000,
        speaker_key: "remote_3",
        text: "Ya devuelve precios por día. Me falta la caché y los descuentos de grupo.",
      },
      {
        index: 2,
        at_ms: 21_000,
        speaker_key: "mic_local",
        text: "¿Los descuentos entran en el piloto?",
      },
    ],
    pending_chunks: 4,
    failed_groups: 0,
  },
  speakers: [speakers[0], speakers[3]].filter((speaker) => speaker !== undefined),
};

/** The design review being recorded now, 18 minutes in. */
export const recordingNote: OpenNoteSnapshot = {
  ...reviewNote,
  ...unwritten,
  capture: "open",
  recording_id: "rec-review-21",
  started_at_ms: at(0, 10, 22),
  ended_at_ms: at(0, 10, 40),
  private_notes:
    "## Durante la reunión\n\n- Marta trae **dos versiones** del pago\n- [ ] Preguntar por el estado vacío del calendario\n- Javier: la disponibilidad tarda ~2 s",
  progress: {
    upload: { state: "active", cause: { kind: "recording" } },
    transcript: { state: "active", cause: { kind: "transcribing", chunks: 2 } },
    summary: { state: "waiting", cause: { kind: "after_recording" } },
  },
  transcript: { language: "es", turns: turns.slice(0, 6), pending_chunks: 2, failed_groups: 0 },
};

/** The interviews of two weeks ago: part of the audio could not be transcribed. */
export const failedNote: OpenNoteSnapshot = {
  ...reviewNote,
  ...unwritten,
  recording_id: "rec-users-06",
  title: "Entrevistas con recepcionistas",
  started_at_ms: at(-15, 10, 30),
  ended_at_ms: at(-15, 11, 10),
  event: null,
  private_notes: "",
  progress: {
    upload: { state: "done", cause: null },
    transcript: { state: "failed", cause: { kind: "transcript_failed", groups: 2 } },
    summary: { state: "waiting", cause: { kind: "waiting_transcript" } },
  },
  transcript: { language: "es", turns: turns.slice(0, 3), pending_chunks: 0, failed_groups: 2 },
};
