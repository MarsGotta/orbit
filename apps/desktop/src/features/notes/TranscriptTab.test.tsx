import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { TranscriptTab } from "./TranscriptTab";
import { NoteDetail } from "./NoteDetail";

/** A note as the hub computed it: times from the start of the
 * recording, every speaker named, no empty turn, the language it heard. */
function note(): OpenNoteSnapshot {
  return {
    recording_id: "meeting-transcript",
    title: "Comité de dirección",
    capture: "closed",
    owner_name: "Alina",
    started_at_ms: 1_790_500_000_000,
    ended_at_ms: 1_790_500_120_000,
    event: null,
    summary: null,
    note_revision: null,
    actions: [],
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
      language: "es",
      turns: [
        { index: 0, at_ms: 0, speaker_key: "mic_local", text: "Empezamos." },
        { index: 1, at_ms: 12_000, speaker_key: "remote_app", text: "Vale." },
        { index: 2, at_ms: 3_725_000, speaker_key: "mic_local", text: "Cerramos." },
      ],
      pending_chunks: 0,
      failed_groups: 0,
    },
    speakers: [
      {
        key: "mic_local",
        track: "mic_local",
        name: "Alina",
        color: "var(--o-blue)",
        share_pct: 67,
      },
      {
        key: "remote_app",
        track: "remote_app",
        name: "Otra persona",
        color: "var(--o-green)",
        share_pct: 33,
      },
    ],
  };
}

afterEach(cleanup);

describe("the transcript is what the hub computed", () => {
  it("shows times from the start, the speakers' names and the language", () => {
    const { container } = render(<TranscriptTab note={note()} send={() => undefined} />);
    const turns = Array.from(container.querySelectorAll("article"));
    expect(turns.map((turn) => turn.querySelector("time")?.textContent)).toEqual([
      "00:00",
      "00:12",
      "1:02:05",
    ]);
    expect(turns.map((turn) => within(turn).getByRole("button").textContent)).toEqual([
      "Alina",
      "Otra persona",
      "Alina",
    ]);
    expect(screen.queryByText("Hablante sin identificar")).toBeNull();
    expect(turns.every((turn) => (turn.querySelector("p")?.textContent ?? "") !== "")).toBe(true);
    expect(
      screen.getByText(`${es.notes.transcriptLanguage}: ${es.notes.languages.es}`),
    ).toBeTruthy();
  });

  it("renames any speaker, Alina's microphone too, by the hub's key", () => {
    const send = vi.fn();
    const { container } = render(<TranscriptTab note={note()} send={send} />);
    const first = container.querySelectorAll("article")[0];
    if (!first) throw new Error("no turn");
    fireEvent.click(within(first).getByRole("button", { name: "Alina" }));
    fireEvent.change(screen.getByRole("textbox", { name: es.notes.speakerSearch }), {
      target: { value: "Ada" },
    });
    fireEvent.click(screen.getByRole("option", { name: es.notes.addSpeaker }));
    fireEvent.click(screen.getByRole("button", { name: es.notes.save }));
    expect(
      (screen.getByRole("radio", { name: es.notes.onlyThisTurn }) as HTMLInputElement).checked,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: es.notes.applySpeaker }));
    expect(send).toHaveBeenCalledWith({
      command: "speaker_rename",
      speaker_key: "mic_local",
      name: "Ada",
      scope: { kind: "turn", at_ms: 0 },
    });
  });
});

it("opens a searchable speaker picker with event guests, excluding the owner", () => {
  const value = note();
  value.event = {
    title: "Comité",
    start_ms: 0,
    end_ms: 120000,
    ...{
      attendees: [
        { name: "Alina", email: "alina@example.invalid", is_self: true },
        { name: "Lucía", email: "lucia@example.invalid", is_self: false },
      ],
    },
  };
  render(<TranscriptTab note={value} send={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Otra persona" }));
  const picker = screen.getByRole("dialog", { name: "Cambiar hablante" });
  expect(within(picker).getByRole("textbox", { name: "Buscar o añadir un nombre" })).toBeTruthy();
  expect(within(picker).getByRole("option", { name: "Lucía" })).toBeTruthy();
  expect(within(picker).queryByText("alina@example.invalid")).toBeNull();
});

function voicesNote(): OpenNoteSnapshot {
  const value = note();
  value.transcript.turns = [
    { index: 0, at_ms: 1000, speaker_key: "voice_1", text: "Uno." },
    { index: 1, at_ms: 11000, speaker_key: "voice_2", text: "Dos." },
    { index: 2, at_ms: 21000, speaker_key: "voice_1", text: "Tres." },
  ];
  value.speakers = [
    {
      key: "voice_1",
      track: "remote_app",
      name: "Persona 1",
      color: "var(--o-blue)",
      share_pct: 67,
    },
    { key: "voice_2", track: "remote_app", name: "Lucía", color: "var(--o-green)", share_pct: 33 },
  ];
  return value;
}

it.each([false, true])(
  "sends the selected scope (whole voice: %s) when assigning another voice's name",
  (wholeVoice) => {
    const send = vi.fn();
    const { rerender, container } = render(<TranscriptTab note={voicesNote()} send={send} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[1]!);
    fireEvent.click(screen.getByRole("option", { name: "Lucía" }));
    expect(send).not.toHaveBeenCalled();
    expect(
      (screen.getByRole("radio", { name: es.notes.onlyThisTurn }) as HTMLInputElement).checked,
    ).toBe(true);
    if (wholeVoice)
      fireEvent.click(screen.getByRole("radio", { name: "Todos los turnos de Persona 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Aplicar" }));
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "speaker_rename",
      speaker_key: "voice_1",
      name: "Lucía",
      scope: wholeVoice ? { kind: "voice" } : { kind: "turn", at_ms: 21000 },
    });
    // Until the hub answers, names remain the hub's facts.
    expect(screen.getAllByRole("button", { name: "Persona 1" })).toHaveLength(2);
    const updated = voicesNote();
    updated.transcript.turns = updated.transcript.turns.map((turn) =>
      turn.speaker_key === "voice_1" && (wholeVoice || turn.at_ms === 21000)
        ? { ...turn, speaker_key: "voice_2" }
        : turn,
    );
    if (wholeVoice)
      updated.speakers = updated.speakers.filter((speaker) => speaker.key === "voice_2");
    rerender(<TranscriptTab note={updated} send={send} />);
    expect(
      Array.from(container.querySelectorAll("article")).map(
        (turn) => within(turn).getByRole("button").textContent,
      ),
    ).toEqual(wholeVoice ? ["Lucía", "Lucía", "Lucía"] : ["Persona 1", "Lucía", "Lucía"]);
  },
);

it("filters by name or email and cancels a selection without sending a rename", () => {
  const value = voicesNote();
  value.event = {
    title: "Comité",
    start_ms: 0,
    end_ms: 120000,
    attendees: [{ name: "Pedro", email: "pedro@example.invalid", is_self: false }],
  };
  const send = vi.fn();
  render(<TranscriptTab note={value} send={send} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  const search = screen.getByRole("textbox", { name: es.notes.speakerSearch });
  fireEvent.change(search, { target: { value: "PEDRO@" } });
  expect(screen.getByRole("option", { name: "Pedro" })).toBeTruthy();
  expect(screen.queryByRole("option", { name: "Lucía" })).toBeNull();
  fireEvent.click(screen.getByRole("option", { name: "Pedro" }));
  expect(screen.getByText(es.notes.speakerNoteHelp)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(send).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).toBeNull();
  // Opening the same turn again starts with a fresh picker.
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  expect(screen.getByRole("textbox", { name: es.notes.speakerSearch })).toBeTruthy();
});

it("supports choosing an existing voice by keyboard after searching", () => {
  const send = vi.fn();
  render(<TranscriptTab note={voicesNote()} send={send} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  const search = screen.getByRole("textbox", { name: es.notes.speakerSearch });
  fireEvent.change(search, { target: { value: "Lucía" } });
  fireEvent.keyDown(search, { key: "Enter" });
  expect(screen.getByRole("dialog", { name: "Asignar «Lucía»" })).toBeTruthy();
  expect(send).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("radio", { name: es.notes.onlyThisTurn }), { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("closes the speaker dialog when navigating to another note", () => {
  const send = vi.fn();
  const first = voicesNote();
  const { rerender } = render(<NoteDetail note={first} send={send} series="Comité" error={null} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  fireEvent.click(screen.getByRole("option", { name: "Lucía" }));
  expect(screen.getByRole("dialog", { name: "Asignar «Lucía»" })).toBeTruthy();
  rerender(
    <NoteDetail
      note={{ ...first, recording_id: "another-meeting" }}
      send={send}
      series="Comité"
      error={null}
    />,
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(send).not.toHaveBeenCalled();
});

it("keeps focus in the dialog after choosing the second radio and skips disabled buttons", () => {
  render(<TranscriptTab note={voicesNote()} send={vi.fn()} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  fireEvent.click(screen.getByRole("option", { name: "Lucía" }));
  const all = screen.getByRole("radio", { name: "Todos los turnos de Persona 1" });
  fireEvent.click(all);
  all.focus();
  fireEvent.keyDown(all, { key: "Tab", shiftKey: true });
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Aplicar" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 1" })[0]!);
  fireEvent.click(screen.getByRole("option", { name: es.notes.addSpeaker }));
  const cancel = screen.getByRole("button", { name: "Cancelar" });
  expect((screen.getByRole("button", { name: "Guardar" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  cancel.focus();
  fireEvent.keyDown(cancel, { key: "Tab" });
  expect(document.activeElement).toBe(screen.getByRole("textbox", { name: es.notes.speakerName }));
});

it("offers a manually assigned name even when it resembles a numbered placeholder", () => {
  const value = voicesNote();
  value.speakers[0]!.name = "Persona 12";
  render(<TranscriptTab note={value} send={vi.fn()} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 12" })[0]!);
  expect(screen.getByRole("option", { name: "Persona 12" })).toBeTruthy();
});

it("requests a final pass for old and already separated closed meetings", () => {
  const separated = voicesNote();
  Object.assign(separated.transcript, { final_pass: { state: "completed", error: null } });
  for (const value of [note(), separated]) {
    const send = vi.fn();
    const view = render(<TranscriptTab note={value} send={send} />);
    fireEvent.click(screen.getByRole("button", { name: "Volver a transcribir" }));
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "transcript_retranscribe",
      recording_id: value.recording_id,
    });
    view.unmount();
  }
});

it("keeps the old transcript with distinct queued and running words, without offering another pass", () => {
  const value = note();
  const { rerender } = render(<TranscriptTab note={value} send={vi.fn()} />);
  for (const [state, words] of [
    ["queued", "En cola para volver a transcribir."],
    ["running", "Transcribiendo de nuevo…"],
  ]) {
    Object.assign(value.transcript, { final_pass: { state, error: null } });
    rerender(<TranscriptTab note={{ ...value }} send={vi.fn()} />);
    expect(screen.getByText(words!)).toBeTruthy();
    expect(screen.getByText("Empezamos.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Volver a transcribir" })).toBeNull();
  }
});

it("hides retranscription while capture is open or unknown", () => {
  const value = note();
  for (const capture of ["open", undefined]) {
    Object.assign(value, { capture });
    const view = render(<TranscriptTab note={value} send={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Volver a transcribir" })).toBeNull();
    view.unmount();
  }
});

it.each([
  [
    "final_pass_incomplete",
    undefined,
    "La transcripción nueva perdía partes de la anterior, así que se conserva la anterior.",
  ],
  [
    "final_pass_timeout",
    90000,
    "La transcripción nueva tardó más de los 1,5 minutos que permite el hub; se conserva la anterior.",
  ],
  [
    "final_pass_timeout",
    undefined,
    "La transcripción nueva tardó más de lo que permite el hub; se conserva la anterior.",
  ],
  [
    "final_audio_invalid",
    undefined,
    "No se pudo completar la nueva transcripción; se conserva la anterior. (final_audio_invalid)",
  ],
])(
  "explains failure %s with budget %s without displaying diagnostic JSON",
  (error, timeout_ms, text) => {
    const value = note();
    Object.assign(value.transcript, {
      final_pass: { state: "failed", error, timeout_ms },
      unmatched_speaker_names: 1,
    });
    render(<TranscriptTab note={value} send={vi.fn()} />);
    expect(screen.getByRole("alert").textContent).toBe(text);
    expect(screen.getByText(/1 nombre.*no.*traslad/)).toBeTruthy();
    expect(screen.getByText("Empezamos.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Volver a transcribir" })).toBeTruthy();
  },
);

it("corrects selected transcript text and keeps the form until the hub answers", () => {
  const send = vi.fn();
  const current = {
    ...note(),
    dictionary: { dictionary_id: "vocabulary", source: "requested", series: null },
  };
  render(<TranscriptTab note={current} send={send} />);
  const paragraph = screen.getByText("Empezamos.");
  const range = document.createRange();
  range.selectNodeContents(paragraph);
  window.getSelection()?.removeAllRanges();
  window.getSelection()?.addRange(range);
  fireEvent.mouseUp(paragraph);
  fireEvent.click(screen.getByRole("button", { name: "Corregir texto" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Forma correcta" }), {
    target: { value: "Empecemos." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Guardar corrección" }));
  expect(send).toHaveBeenCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: {
        action: "correct",
        recording_id: current.recording_id,
        dictionary_id: "vocabulary",
        wrong: "Empezamos.",
        correct: "Empecemos.",
        name: null,
      },
    },
  });
  expect((screen.getByRole("textbox", { name: "Forma correcta" }) as HTMLInputElement).value).toBe(
    "Empecemos.",
  );
});

function dictionaryLibrary() {
  return {
    read: "available" as const,
    read_detail: null,
    dictionaries: { state: "available" as const, detail: null },
    dictionary_items: [],
    dictionary_pending: [] as string[],
    dictionary_failures: [],
    templates: { state: "available" as const, detail: null },
    items: [],
    links: { state: "available" as const, detail: null },
    assignments: [],
    refusals: [],
    restores: [],
    bounds: {
      max_sections: 12,
      max_name_chars: 80,
      max_context_chars: 4000,
      max_instructions_chars: 2000,
      max_custom_templates: 32,
    },
  };
}

function selectFirstTurn() {
  const paragraph = screen.getByText("Empezamos.");
  const range = document.createRange();
  range.selectNodeContents(paragraph);
  window.getSelection()?.removeAllRanges();
  window.getSelection()?.addRange(range);
  fireEvent.mouseUp(paragraph);
  fireEvent.click(screen.getByRole("button", { name: "Corregir texto" }));
}

it("creates the named dictionary with the first correction, proposing the series title", () => {
  const send = vi.fn();
  const current = {
    ...note(),
    dictionary: {
      dictionary_id: null,
      source: null,
      series: { calendar_item_id: "daily", title: "Daily del proyecto" },
    },
  };
  render(<TranscriptTab note={current} library={dictionaryLibrary()} send={send} />);
  selectFirstTurn();
  expect(
    (screen.getByRole("textbox", { name: "Nombre del diccionario" }) as HTMLInputElement).value,
  ).toBe("Daily del proyecto");
  fireEvent.change(screen.getByRole("textbox", { name: "Forma correcta" }), {
    target: { value: "Empecemos." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Guardar corrección" }));
  expect(send).toHaveBeenCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: {
        action: "correct",
        recording_id: current.recording_id,
        dictionary_id: expect.any(String),
        wrong: "Empezamos.",
        correct: "Empecemos.",
        name: "Daily del proyecto",
      },
    },
  });
});

it("keeps the correction while saving or refused, with the cause and numbers visible", () => {
  const current = {
    ...note(),
    dictionary: { dictionary_id: "vocabulary", source: "requested", series: null },
  };
  const send = vi.fn();
  const { rerender } = render(
    <TranscriptTab note={current} library={dictionaryLibrary()} send={send} />,
  );
  selectFirstTurn();
  fireEvent.change(screen.getByRole("textbox", { name: "Forma correcta" }), {
    target: { value: "Empecemos." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Guardar corrección" }));
  rerender(
    <TranscriptTab
      note={current}
      library={{ ...dictionaryLibrary(), dictionary_pending: ["dictionary:vocabulary"] }}
      send={send}
    />,
  );
  expect(screen.getByText("Guardando en el hub…")).toBeTruthy();
  expect(
    (screen.getByRole("button", { name: "Guardar corrección" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  rerender(
    <TranscriptTab
      note={current}
      library={{
        ...dictionaryLibrary(),
        dictionary_failures: [
          {
            target: "dictionary:vocabulary",
            code: "dictionary_text_too_long",
            detail: "HTTP 413",
            numbers: { characters: 129, max_characters: 128 },
          },
        ],
      }}
      send={send}
    />,
  );
  expect(screen.getByRole("alert").textContent).toContain("demasiado largo");
  expect(screen.getByRole("alert").textContent).toContain("129");
  expect(screen.getByRole("alert").textContent).toContain("128");
  expect((screen.getByRole("textbox", { name: "Forma correcta" }) as HTMLInputElement).value).toBe(
    "Empecemos.",
  );
  rerender(
    <TranscriptTab
      note={current}
      library={{
        ...dictionaryLibrary(),
        dictionary_items: [
          {
            dictionary: {
              id: "vocabulary",
              name: "Proyecto",
              terms: [{ id: "entry", correct: "Empecemos.", wrong: ["Empezamos."] }],
              series: [],
            },
            pending: false,
          },
        ],
      }}
      send={send}
    />,
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("does not turn an unread dictionary into a known empty one", () => {
  render(<TranscriptTab note={note()} library={dictionaryLibrary()} send={vi.fn()} />);
  selectFirstTurn();
  expect(
    (screen.getByRole("button", { name: "Guardar corrección" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(
    screen.getAllByText("El diccionario aún no se ha podido leer del hub.").length,
  ).toBeGreaterThan(0);
});

it("selects the meeting dictionary in the header and assigns it to the series", () => {
  const send = vi.fn();
  const first = { id: "one", name: "Proyecto", terms: [], series: [] };
  const second = { id: "two", name: "Otro", terms: [], series: [] };
  const library = {
    ...dictionaryLibrary(),
    dictionary_items: [
      { dictionary: first, pending: false },
      { dictionary: second, pending: false },
    ],
  };
  const current = {
    ...note(),
    dictionary: {
      dictionary_id: "one",
      source: "series",
      series: { calendar_item_id: "daily", title: "Daily" },
    },
  };
  const { rerender } = render(
    <NoteDetail note={current} library={library} send={send} error={null} series="Daily" />,
  );
  fireEvent.change(screen.getByRole("combobox", { name: "Diccionario" }), {
    target: { value: "two" },
  });
  expect(send).toHaveBeenLastCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: {
        action: "select",
        recording_id: current.recording_id,
        dictionary_id: "two",
        use_for_series: false,
      },
    },
  });
  rerender(
    <NoteDetail
      note={{ ...current, dictionary: { ...current.dictionary, dictionary_id: "two" } }}
      library={library}
      send={send}
      error={null}
      series="Daily"
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: es.library.useAlways }));
  expect(send).toHaveBeenLastCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: {
        action: "select",
        recording_id: current.recording_id,
        dictionary_id: "two",
        use_for_series: true,
      },
    },
  });
});
