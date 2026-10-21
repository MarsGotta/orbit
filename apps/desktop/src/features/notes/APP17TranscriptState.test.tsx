import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { TranscriptTab, TranscriptionProgress } from "./TranscriptTab";

function note(transcript: Partial<OpenNoteSnapshot["transcript"]>): OpenNoteSnapshot {
  return {
    recording_id: "meeting-app17",
    title: "Revisión semanal",
    owner_name: "Alina",
    started_at_ms: 1,
    ended_at_ms: 2,
    event: null,
    actions: [],
    summary: null,
    note_revision: null,
    decisions: [],
    open_topics: [],
    private_notes: "",
    private_notes_revision: 0,
    progress: {
      upload: { state: "done", cause: null },
      transcript: { state: "done", cause: null },
      summary: { state: "waiting", cause: { kind: "not_requested" } },
    },
    transcript: {
      language: null,
      turns: [],
      pending_chunks: 0,
      failed_groups: 0,
      ...transcript,
    },
    speakers: [],
  };
}

function renderTranscript(value: OpenNoteSnapshot) {
  render(
    <>
      <TranscriptionProgress note={value} send={() => undefined} />
      <TranscriptTab note={value} send={() => undefined} />
    </>,
  );
}

afterEach(cleanup);

describe("APP-17 A1 visible transcript state", () => {
  // SIM-24: work on the hub is progress, said without its internal counts.
  it("shows chunks still to transcribe as progress, not as a failure", () => {
    renderTranscript(note({ pending_chunks: 2 }));
    const status = screen.getByRole("status").textContent;
    expect(status).toBe(es.notes.transcribing);
    expect(status).not.toMatch(/\d/);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(es.notes.noTranscript)).toBeNull();
  });

  // SIM-24: a failed read says so and offers «Reintentar»; the daemon's
  // cause stays in the log, never on screen.
  it("shows the failure instead of the empty-state copy, without the daemon cause", () => {
    renderTranscript(note({ error: "El hub no respondió a tiempo" }));
    const alert = screen.getByRole("alert").textContent;
    expect(alert).toContain(es.notes.transcriptFailed);
    expect(alert).not.toContain("El hub no respondió a tiempo");
    expect(screen.queryByText(es.notes.noTranscript)).toBeNull();
  });

  it("offers «Reintentar» when the hub gave up on part of the transcript", () => {
    renderTranscript(note({ failed_groups: 1 }));
    expect(screen.getByRole("status").textContent).toContain(es.notes.failedGroups);
    expect(screen.getByRole("button", { name: es.notes.retry })).toBeTruthy();
  });
});
