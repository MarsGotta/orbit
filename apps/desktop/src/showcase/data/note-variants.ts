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

const lucia = speakers[0];
const marta = speakers[1];
const javier = speakers[3];
/** A voice nobody named yet: the hub calls it «Persona 2». */
const persona2 = {
  key: "voice_2",
  track: "remote_app",
  name: "Persona 2",
  color: "var(--o-c2)",
  share_pct: 34,
};
const named = [lucia, marta, javier].filter((speaker) => speaker !== undefined);

const SECOND = 1_000;
const hablante = [
  [8, "mic_local", "Empezamos. Marta, ¿cómo va el prototipo del pago en una sola pantalla?"],
  [
    21,
    "remote_1",
    "Ya está. Lo probamos ayer con tres recepcionistas y lo terminaron todas sin ayuda.",
  ],
  [47, "voice_2", "¿Y la disponibilidad? La semana pasada tardaba unos dos segundos."],
  [
    65,
    "remote_3",
    "Con la caché por día baja a unos trescientos milisegundos con los cinco hoteles.",
  ],
  [92, "mic_local", "Orvit ya lo tiene apuntado para el lunes."],
  [110, "voice_2", "Yo preparo la demo con Ribera Aeropuerto si llega la caché."],
  [130, "voice_2", "Y reviso los textos del paso de pago."],
] as const;

/** Today's design review just recorded: the audio saved, the transcript filling. */
export const writingNote: OpenNoteSnapshot = {
  ...reviewNote,
  ...unwritten,
  recording_id: "rec-review-today",
  started_at_ms: at(0, 11, 0),
  ended_at_ms: at(0, 11, 47),
  progress: {
    upload: { state: "done", cause: null },
    transcript: { state: "active", cause: { kind: "transcribing", chunks: 5 } },
    summary: { state: "waiting", cause: { kind: "waiting_transcript" } },
  },
  transcript: {
    language: "es",
    turns: hablante.slice(0, 4).map(([seconds, speaker_key, text], index) => ({
      index,
      at_ms: seconds * SECOND,
      speaker_key,
      text,
    })),
    pending_chunks: 5,
    failed_groups: 0,
  },
  speakers: [...named, persona2],
  series: reviewNote.series,
};

/** The same review once written, with one voice still to name and a word to correct. */
export const speakerNote: OpenNoteSnapshot = {
  ...reviewNote,
  recording_id: "rec-review-today",
  started_at_ms: at(0, 11, 0),
  ended_at_ms: at(0, 11, 47),
  transcript: {
    ...reviewNote.transcript,
    turns: hablante.map(([seconds, speaker_key, text], index) => ({
      index,
      at_ms: seconds * SECOND,
      speaker_key,
      text,
    })),
  },
  speakers: [...named, persona2],
};

/** Two weeks ago: transcribed, but the hub could not write its note. */
export const unwrittenNote: OpenNoteSnapshot = {
  ...failedNote,
  progress: {
    upload: { state: "done", cause: null },
    transcript: { state: "done", cause: null },
    summary: { state: "failed", cause: { kind: "failed" } },
  },
  transcript: { language: "es", turns: turns.slice(0, 6), pending_chunks: 0, failed_groups: 0 },
};
