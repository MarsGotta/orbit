// Invented meetings for the notes tests: one written note and the list.
// Each test changes the copy it takes.
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NotesSnapshot } from "../../bridge/generated/NotesSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { AppView } from "../../state/app-view";

export { library } from "./test-library";

const MINUTE = 60_000;
/** Wednesday 14 October 2026, 11:00 UTC: the suite runs in UTC. */
export const STARTED = Date.UTC(2026, 9, 14, 11, 0);

function event(): OpenNoteSnapshot["event"] {
  return {
    title: "Comité de la biblioteca",
    start_ms: STARTED,
    end_ms: STARTED + 45 * MINUTE,
    attendees: [
      { name: "Alina Prado", email: "alina@example.test", is_self: true },
      { name: "Bruno Lago", email: "bruno@example.test", is_self: false },
      { name: "Celia Mar", email: "celia@example.test", is_self: false },
    ],
  };
}

function transcript(): OpenNoteSnapshot["transcript"] {
  const turn = (index: number, at_ms: number, speaker_key: string, text: string) => ({
    index,
    at_ms,
    speaker_key,
    text,
  });
  return {
    final_pass: { state: "completed", error: null },
    language: "es",
    turns: [
      turn(0, 8_000, "mic_local", "Empezamos con la sala."),
      turn(1, 21_000, "voice_1", "La sala está lista."),
      turn(2, 47_000, "voice_2", "¿Y las estanterías de la sala?"),
      turn(3, 65_000, "voice_2", "Las pido el jueves."),
    ],
    pending_chunks: 0,
    failed_groups: 0,
  };
}

function speakers(): OpenNoteSnapshot["speakers"] {
  return [
    {
      key: "mic_local",
      track: "mic_local",
      name: "Alina Prado",
      color: "var(--o-c1)",
      share_pct: 25,
    },
    {
      key: "voice_1",
      track: "remote_app",
      name: "Bruno Lago",
      color: "var(--o-c2)",
      share_pct: 25,
    },
    { key: "voice_2", track: "remote_app", name: "Persona 2", color: "var(--o-c3)", share_pct: 50 },
  ];
}

function summary(): OpenNoteSnapshot["summary"] {
  return {
    short_summary: "Se aprueba la sala de lectura nueva y se reparten las compras.",
    sections: [
      { name: "Contexto", markdown: "La sala abre en noviembre.", edited: false },
      { name: "Próximos pasos", markdown: "1. Pedir las estanterías.", edited: false },
    ],
    template_id: "review",
    template_name: "Revisión",
    generated_at_ms: STARTED + 50 * MINUTE,
    model: "hub",
    superseded_available: false,
  };
}

export function writtenNote(): OpenNoteSnapshot {
  return {
    capture: "closed",
    recording_id: "rec-comite",
    title: "Comité de la biblioteca",
    owner_name: "Alina Prado",
    started_at_ms: STARTED,
    ended_at_ms: STARTED + 47 * MINUTE,
    event: event(),
    summary: summary(),
    note_revision: 3,
    actions: [
      {
        id: "a1",
        owner: "Bruno Lago",
        text: "Pedir tres presupuestos",
        due: "jueves",
        done: false,
      },
      { id: "a2", owner: "Alina Prado", text: "Reservar la sala", due: "viernes", done: false },
      { id: "a3", owner: "Alina Prado", text: "Enviar el acta", due: null, done: true },
      { id: "a4", owner: null, text: "Revisar el cartel", due: null, done: false },
    ],
    decisions: ["La sala abre en noviembre."],
    open_topics: ["El horario de los sábados."],
    private_notes: "",
    private_notes_revision: 0,
    progress: {
      upload: { state: "done", cause: null },
      transcript: { state: "done", cause: null },
      summary: { state: "done", cause: null },
    },
    transcript: transcript(),
    speakers: speakers(),
    series: {
      calendar_item_id: "serie-comite",
      assigned: { state: "known", template_id: "review" },
    },
    dictionary: { dictionary_id: "dic-biblio", source: "series", series: null },
  };
}

/** A note uploaded and transcribing: nothing written yet. */
export function writingNote(): OpenNoteSnapshot {
  return {
    ...writtenNote(),
    summary: null,
    note_revision: null,
    actions: [],
    decisions: [],
    open_topics: [],
    progress: {
      upload: { state: "done", cause: null },
      transcript: { state: "active", cause: { kind: "transcribing", chunks: 3 } },
      summary: { state: "waiting", cause: { kind: "waiting_transcript" } },
    },
    transcript: { ...transcript(), pending_chunks: 3 },
  };
}

function row(
  recording_id: string,
  title: string,
  started_at_ms: number,
  group: NoteListItem["group"],
  headline: NoteListItem["headline"],
): NoteListItem {
  return {
    recording_id,
    title,
    started_at_ms,
    group,
    headline,
    duration_ms: 30 * MINUTE,
    state: "ready",
  };
}

export function meetingList(): NoteListItem[] {
  return [
    row("rec-hoy", "Reunión de equipo", Date.UTC(2026, 9, 21, 9, 0), "today", {
      phase: "ready",
      preview: "Bruno termina el cartel",
    }),
    {
      ...row("rec-ayer", "Llamada con Celia", Date.UTC(2026, 9, 20, 17, 10), "yesterday", {
        phase: "transcribing",
      }),
      state: "transcribing",
    },
    row("rec-comite", "Comité de la biblioteca", STARTED, "last_30_days", {
      phase: "ready",
      preview: "Se aprueba la sala de lectura",
    }),
    row("rec-fallo", "Entrevistas a usuarios", Date.UTC(2026, 9, 6, 10, 0), "last_30_days", {
      phase: "failed",
    }),
    row("rec-antes", "Jornada de septiembre", Date.UTC(2026, 8, 17, 9, 0), "before", {
      phase: "ready",
      preview: "Valores del equipo",
    }),
  ];
}

export function notesSnapshot(change: Partial<NotesSnapshot> = {}): NotesSnapshot {
  return {
    list: meetingList(),
    index: { state: "available", more: false, detail: null },
    open: null,
    ...change,
  };
}

/**
 * The app's view with only what the notes screen reads: the notes, and for
 * the editors the agents (none) and the calendar's events (none).
 */
export function appView(notes: NotesSnapshot): AppView {
  return {
    notes,
    settings: { agents: [], assistant_name: "Deimos", default_agent_id: null },
    calendar: { events: [] },
  } as unknown as AppView;
}
