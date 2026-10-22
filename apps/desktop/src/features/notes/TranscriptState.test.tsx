import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { TranscriptTab } from "./TranscriptTab";

function note(transcript: Partial<OpenNoteSnapshot["transcript"]>): OpenNoteSnapshot {
  return {
    recording_id: "meeting-transcript-state",
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

function renderTranscript(value: OpenNoteSnapshot, send = vi.fn(() => Promise.resolve())) {
  render(<TranscriptTab note={value} send={send} />);
  return send;
}

afterEach(cleanup);

describe("visible transcript state", () => {
  // Work on the hub is progress, said without its internal counts.
  it("shows chunks still to transcribe as more coming, not as a failure", () => {
    renderTranscript(note({ pending_chunks: 2 }));
    expect(screen.getByLabelText("Llegando más transcripción")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(es.notes.noTranscript)).toBeNull();
    expect(document.body.textContent).not.toMatch(/\b2\b/);
  });

  // A failed read says so and offers «Reintentar»; the daemon's
  // cause stays in the log, never on screen.
  it("shows the failure instead of the empty-state copy, without the daemon cause", () => {
    const send = renderTranscript(note({ error: "El hub no respondió a tiempo" }));
    const alert = screen.getByRole("alert").textContent;
    expect(alert).toContain(es.notes.transcriptFailed);
    expect(alert).not.toContain("El hub no respondió a tiempo");
    expect(screen.queryByText(es.notes.noTranscript)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(send).toHaveBeenCalledWith({
      command: "notes_open",
      recording_id: "meeting-transcript-state",
    });
  });

  it("says when there is nothing transcribed yet", () => {
    renderTranscript(note({}));
    expect(screen.getByText(es.notes.noTranscript)).toBeTruthy();
  });
});
