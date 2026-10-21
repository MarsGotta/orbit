import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { NoteHeader } from "./NoteHeader";
import { summaryMarkdown } from "./notes-format";

// Found on 2026-10-17: the list of Notes names a note by its meeting's title,
// and the opened note showed the title the note model wrote. Both say the
// meeting's title (the hub's document carries only that one).

function written(): OpenNoteSnapshot {
  return {
    recording_id: "meeting-title",
    title: "Comité de presupuesto",
    owner_name: "Alina",
    started_at_ms: 1,
    ended_at_ms: 2,
    event: null,
    actions: [],
    summary: {
      short_summary: "Se repasó el presupuesto.",
      sections: [],
      template_id: "general",
      template_name: "General",
      generated_at_ms: 3,
      model: "qwen",
      superseded_available: false,
    },
    note_revision: 1,
    decisions: [],
    open_topics: [],
    private_notes: "",
    private_notes_revision: 0,
    progress: {
      upload: { state: "done", cause: null },
      transcript: { state: "done", cause: null },
      summary: { state: "done", cause: null },
    },
    transcript: { language: "es", turns: [], pending_chunks: 0, failed_groups: 0 },
    speakers: [],
  };
}

afterEach(cleanup);

function title(): HTMLInputElement {
  return screen.getByRole("textbox", { name: es.noteScreen.titleLabel }) as HTMLInputElement;
}

describe("the opened note's title", () => {
  it("is the meeting's title, not the one the note was written with", () => {
    render(<NoteHeader note={written()} onAsk={() => undefined} send={() => undefined} />);
    expect(screen.getByRole("heading", { level: 1 })).toBeTruthy();
    expect(title().value).toBe("Comité de presupuesto");
  });

  it("heads the copied summary too", () => {
    expect(summaryMarkdown(written()).split("\n")[0]).toBe("# Comité de presupuesto");
  });
});

// The title renames the meeting everywhere; the language picker sits
// next to «Plantilla» and shows the meeting's language and its repetition.

function header(note: OpenNoteSnapshot, send = vi.fn()) {
  render(<NoteHeader note={note} onAsk={() => undefined} send={send} />);
  return send;
}

describe("renaming the meeting", () => {
  it("sends Alina's name, trimmed, when she leaves the field or presses Enter", () => {
    const send = header(written());
    fireEvent.change(title(), { target: { value: "  Presupuesto 2027 " } });
    fireEvent.keyDown(title(), { key: "Enter" });
    expect(send).toHaveBeenCalledWith({
      command: "meeting_rename",
      recording_id: "meeting-title",
      title: "Presupuesto 2027",
    });
    expect(send).toHaveBeenCalledTimes(1);
    fireEvent.blur(title());
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("gives the meeting back its own title when the field is left empty", () => {
    const send = header(written());
    fireEvent.change(title(), { target: { value: "   " } });
    fireEvent.blur(title());
    expect(send).toHaveBeenCalledWith({
      command: "meeting_rename",
      recording_id: "meeting-title",
      title: "",
    });
  });

  it("sends nothing when the title did not change, and Escape puts it back", () => {
    const send = header(written());
    fireEvent.blur(title());
    fireEvent.change(title(), { target: { value: "Otro" } });
    fireEvent.keyDown(title(), { key: "Escape" });
    expect(title().value).toBe("Comité de presupuesto");
    fireEvent.blur(title());
    expect(send).not.toHaveBeenCalled();
  });

  it("says why a recording the hub does not have yet cannot be renamed", () => {
    const note = written();
    note.progress.upload = { state: "active", cause: { kind: "uploading", chunks: 3 } };
    header(note);
    expect(title().disabled).toBe(true);
    expect(title().title).toBe(es.noteScreen.titleNotOnHub);
    expect(screen.getByText(es.noteScreen.titleNotOnHub)).toBeTruthy();
  });
});

function picker(): HTMLSelectElement {
  return screen.getByRole("combobox", { name: es.noteScreen.language }) as HTMLSelectElement;
}

describe("the meeting's language", () => {
  it("shows the language the hub says, among seven named in their own language", () => {
    header(written());
    expect(picker().value).toBe("es");
    expect(Array.from(picker().options).map((option) => option.textContent)).toEqual([
      "Español",
      "English",
      "Català",
      "Français",
      "Português",
      "Deutsch",
      "Italiano",
    ]);
  });

  it("asks the hub for another language", () => {
    const send = header(written());
    fireEvent.change(picker(), { target: { value: "en" } });
    expect(send).toHaveBeenCalledWith({
      command: "meeting_language",
      recording_id: "meeting-title",
      language: "en",
    });
  });

  it("names a language the hub heard that is not among the seven by its code", () => {
    const note = written();
    note.transcript.language = "nl";
    header(note);
    expect(picker().value).toBe("nl");
  });

  it("says how far transcribing it again got", () => {
    const note = written();
    note.transcript.language = "en";
    note.transcript.retranscribing = { language: "en", done: 1, total: 4 };
    header(note);
    expect(screen.getByRole("status").textContent).toBe(es.noteScreen.retranscribing(1, 4));
    expect(es.noteScreen.retranscribing(1, 4)).toBe("Transcribiendo de nuevo… 1 de 4");
  });

  it("is not offered before the hub has the meeting", () => {
    const note = written();
    note.transcript.language = null;
    header(note);
    expect(screen.queryByRole("combobox", { name: es.noteScreen.language })).toBeNull();
  });
});
