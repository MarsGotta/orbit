import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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
        color: "var(--o-c2)",
        share_pct: 67,
      },
      {
        key: "remote_app",
        track: "remote_app",
        name: "Otra persona",
        color: "var(--o-c3)",
        share_pct: 33,
      },
    ],
  };
}

afterEach(cleanup);

const send0 = () => Promise.resolve();

describe("the transcript is what the hub computed", () => {
  it("shows times from the start and the speakers' names, an unnamed one asking for its name", () => {
    const { container } = render(<TranscriptTab note={note()} send={send0} />);
    const turns = Array.from(container.querySelectorAll("article"));
    expect(turns.map((turn) => turn.querySelector("time")?.textContent)).toEqual([
      "00:00",
      "00:12",
      "1:02:05",
    ]);
    expect(turns.map((turn) => within(turn).getByRole("button").textContent)).toEqual([
      "Alina",
      "Otra persona · ponle nombre",
      "Alina",
    ]);
    expect(screen.queryByText("Hablante sin identificar")).toBeNull();
    expect(turns.every((turn) => (turn.querySelector("p")?.textContent ?? "") !== "")).toBe(true);
    expect(
      screen.getByText(
        "Pulsa un nombre para cambiar quién habla. Selecciona una palabra para corregirla.",
      ),
    ).toBeTruthy();
  });

  it("does not jump to the end when it opens", () => {
    const scrolled = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrolled;
    try {
      render(<TranscriptTab note={note()} send={send0} />);
      expect(scrolled).not.toHaveBeenCalled();
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("renames any speaker, Alina's microphone too, by the hub's key", async () => {
    const send = vi.fn(send0);
    const { container } = render(<TranscriptTab note={note()} send={send} />);
    const first = container.querySelectorAll("article")[0];
    if (!first) throw new Error("no turn");
    fireEvent.click(within(first).getByRole("button", { name: "Alina" }));
    const menu = screen.getByRole("dialog", { name: "¿Quién es Alina?" });
    fireEvent.change(within(menu).getByRole("textbox", { name: es.notes.speakerSearch }), {
      target: { value: "Ada" },
    });
    fireEvent.click(within(menu).getByRole("button", { name: "+ Añadir «Ada»" }));
    expect(within(menu).getByRole("heading", { name: "¿Dónde pongo «Ada»?" })).toBeTruthy();
    expect(
      (within(menu).getByRole("radio", { name: "Solo en este turno (00:00)" }) as HTMLInputElement)
        .checked,
    ).toBe(true);
    await act(async () => {
      fireEvent.click(within(menu).getByRole("button", { name: "Aplicar" }));
    });
    expect(send).toHaveBeenCalledWith({
      command: "speaker_rename",
      speaker_key: "mic_local",
      name: "Ada",
      scope: { kind: "turn", at_ms: 0 },
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    // No note written yet: it will be written with the new name.
    expect(screen.getByRole("status").textContent).toBe("✓Cambiado en 1 turno.");
  });

  it("says a written note takes the change when its summary is redone", async () => {
    const send = vi.fn(send0);
    const written = note();
    written.summary = {
      short_summary: "Se cierra el comité.",
      sections: [],
      template_id: "review",
      template_name: "Revisión",
      generated_at_ms: 1,
      model: "hub",
      superseded_available: false,
    };
    render(<TranscriptTab note={written} send={send} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Alina" })[0]!);
    fireEvent.change(screen.getByRole("textbox", { name: es.notes.speakerSearch }), {
      target: { value: "Ada" },
    });
    fireEvent.click(screen.getByRole("button", { name: "+ Añadir «Ada»" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Aplicar" }));
    });
    const band = screen.getByRole("status");
    expect(band.textContent).toContain(
      "Cambiado en 1 turno. La nota usará el cambio cuando rehagas el resumen.",
    );
    fireEvent.click(within(band).getByRole("button", { name: "Rehacer resumen" }));
    expect(send).toHaveBeenLastCalledWith({
      command: "note_regenerate",
      recording_id: "meeting-transcript",
      template_id: "review",
    });
  });
});

it("opens a searchable speaker menu with the meeting's guests, excluding the owner", () => {
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
  render(<TranscriptTab note={value} send={vi.fn(send0)} />);
  fireEvent.click(screen.getByRole("button", { name: "Otra persona · ponle nombre" }));
  const picker = screen.getByRole("dialog", { name: "¿Quién es Otra persona?" });
  const search = within(picker).getByRole("textbox", { name: "Buscar o añadir un nombre" });
  expect(search.getAttribute("placeholder")).toBe("¿Quién es Otra persona?");
  expect(document.activeElement).toBe(search);
  expect(within(picker).getByText("Invitados de la reunión")).toBeTruthy();
  expect(within(picker).getByRole("button", { name: "Lucía" })).toBeTruthy();
  expect(within(picker).queryByText("alina@example.invalid")).toBeNull();
  expect(within(picker).getByRole("button", { name: "+ Añadir a alguien" })).toBeTruthy();
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
      color: "var(--o-c2)",
      share_pct: 67,
    },
    { key: "voice_2", track: "remote_app", name: "Lucía", color: "var(--o-c3)", share_pct: 33 },
  ];
  return value;
}

const unnamed = "Persona 1 · ponle nombre";

it.each([false, true])(
  "sends the selected scope (whole voice: %s) when assigning another voice's name",
  async (wholeVoice) => {
    const send = vi.fn(send0);
    const { rerender, container } = render(<TranscriptTab note={voicesNote()} send={send} />);
    fireEvent.click(screen.getAllByRole("button", { name: unnamed })[1]!);
    const menu = screen.getByRole("dialog", { name: "¿Quién es Persona 1?" });
    expect(within(menu).getByText("Ya en esta grabación")).toBeTruthy();
    fireEvent.click(within(menu).getByRole("button", { name: "Lucía" }));
    expect(send).not.toHaveBeenCalled();
    expect(
      (within(menu).getByRole("radio", { name: "Solo en este turno (00:21)" }) as HTMLInputElement)
        .checked,
    ).toBe(true);
    if (wholeVoice)
      fireEvent.click(within(menu).getByRole("radio", { name: "En los 2 turnos de Persona 1" }));
    await act(async () => {
      fireEvent.click(within(menu).getByRole("button", { name: "Aplicar" }));
    });
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "speaker_rename",
      speaker_key: "voice_1",
      name: "Lucía",
      scope: wholeVoice ? { kind: "voice" } : { kind: "turn", at_ms: 21000 },
    });
    // Until the hub answers, names remain the hub's facts.
    expect(screen.getAllByRole("button", { name: unnamed })).toHaveLength(2);
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
    ).toEqual(wholeVoice ? ["Lucía", "Lucía", "Lucía"] : [unnamed, "Lucía", "Lucía"]);
  },
);

it("says «Guardando…» on «Aplicar» until the order is taken", async () => {
  let resolve: () => void = () => undefined;
  const taken = new Promise<void>((done) => {
    resolve = done;
  });
  render(<TranscriptTab note={voicesNote()} send={() => taken} />);
  fireEvent.click(screen.getAllByRole("button", { name: unnamed })[0]!);
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Lucía" }));
  fireEvent.click(screen.getByRole("radio", { name: "En los 2 turnos de Persona 1" }));
  fireEvent.click(screen.getByRole("button", { name: "Aplicar" }));
  expect(screen.getByRole("button", { name: /Guardando…/ })).toBeTruthy();
  await act(async () => {
    resolve();
    await taken;
  });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByRole("status").textContent).toContain("Cambiado en 2 turnos.");
});

it("filters by name or email and closes without sending a rename", () => {
  const value = voicesNote();
  value.event = {
    title: "Comité",
    start_ms: 0,
    end_ms: 120000,
    attendees: [{ name: "Pedro", email: "pedro@example.invalid", is_self: false }],
  };
  const send = vi.fn(send0);
  render(<TranscriptTab note={value} send={send} />);
  const speaker = screen.getAllByRole("button", { name: unnamed })[0]!;
  fireEvent.click(speaker);
  const search = screen.getByRole("textbox", { name: es.notes.speakerSearch });
  fireEvent.change(search, { target: { value: "PEDRO@" } });
  const menu = screen.getByRole("dialog");
  expect(within(menu).getByRole("button", { name: "Pedro" })).toBeTruthy();
  expect(within(menu).queryByRole("button", { name: "Lucía" })).toBeNull();
  fireEvent.click(within(menu).getByRole("button", { name: "Pedro" }));
  fireEvent.click(screen.getByRole("button", { name: "Atrás" }));
  expect(screen.getByRole("textbox", { name: es.notes.speakerSearch })).toBeTruthy();
  fireEvent.keyDown(screen.getByRole("textbox", { name: es.notes.speakerSearch }), {
    key: "Escape",
  });
  expect(send).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(speaker);
  // Opening the same turn again starts with a fresh search.
  fireEvent.click(screen.getAllByRole("button", { name: unnamed })[0]!);
  expect(
    (screen.getByRole("textbox", { name: es.notes.speakerSearch }) as HTMLInputElement).value,
  ).toBe("");
});

it("supports choosing an existing voice by keyboard after searching", () => {
  const send = vi.fn(send0);
  render(<TranscriptTab note={voicesNote()} send={send} />);
  fireEvent.click(screen.getAllByRole("button", { name: unnamed })[0]!);
  const search = screen.getByRole("textbox", { name: es.notes.speakerSearch });
  fireEvent.change(search, { target: { value: "Lucía" } });
  fireEvent.keyDown(search, { key: "Enter" });
  expect(screen.getByRole("heading", { name: "¿Dónde pongo «Lucía»?" })).toBeTruthy();
  expect(send).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("radio", { name: "Solo en este turno (00:01)" }), {
    key: "Escape",
  });
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("closes the speaker menu when navigating to another note", () => {
  const send = vi.fn(send0);
  const first = voicesNote();
  const { rerender } = render(<NoteDetail note={first} send={send} error={null} />);
  fireEvent.click(screen.getByRole("tab", { name: "Transcripción" }));
  fireEvent.click(screen.getAllByRole("button", { name: unnamed })[0]!);
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Lucía" }));
  expect(screen.getByRole("heading", { name: "¿Dónde pongo «Lucía»?" })).toBeTruthy();
  rerender(
    <NoteDetail note={{ ...first, recording_id: "another-meeting" }} send={send} error={null} />,
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(send).not.toHaveBeenCalled();
});

it("adds someone only with a name typed", () => {
  render(<TranscriptTab note={voicesNote()} send={vi.fn(send0)} />);
  fireEvent.click(screen.getAllByRole("button", { name: unnamed })[0]!);
  const search = screen.getByRole("textbox", { name: es.notes.speakerSearch });
  fireEvent.click(screen.getByRole("button", { name: "+ Añadir a alguien" }));
  expect(document.activeElement).toBe(search);
  expect(screen.queryByRole("heading", { name: /¿Dónde pongo/ })).toBeNull();
  fireEvent.change(search, { target: { value: "Ramón" } });
  fireEvent.click(screen.getByRole("button", { name: "+ Añadir «Ramón»" }));
  expect(screen.getByRole("heading", { name: "¿Dónde pongo «Ramón»?" })).toBeTruthy();
});

it("offers a manually assigned name even when it resembles a numbered placeholder", () => {
  const value = voicesNote();
  value.speakers[0]!.name = "Persona 12";
  render(<TranscriptTab note={value} send={vi.fn(send0)} />);
  fireEvent.click(screen.getAllByRole("button", { name: "Persona 12" })[0]!);
  expect(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Persona 12" }),
  ).toBeTruthy();
});

it("finds words in the transcript, counts them and marks them", () => {
  const { container } = render(<TranscriptTab note={note()} send={send0} />);
  const search = screen.getByRole("searchbox", { name: "Buscar en la transcripción" });
  fireEvent.change(search, { target: { value: "amos" } });
  expect(screen.getByText("2 resultados")).toBeTruthy();
  expect(Array.from(container.querySelectorAll("mark")).map((mark) => mark.textContent)).toEqual([
    "amos",
    "amos",
  ]);
  fireEvent.change(search, { target: { value: "nadie" } });
  expect(screen.getByText("Sin resultados")).toBeTruthy();
});

it("requests a final pass for old and already separated closed meetings", () => {
  const separated = voicesNote();
  Object.assign(separated.transcript, { final_pass: { state: "completed", error: null } });
  for (const value of [note(), separated]) {
    const send = vi.fn();
    const view = render(<TranscriptTab note={value} send={send} />);
    fireEvent.click(screen.getByRole("button", { name: "Rehacer transcripción" }));
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "transcript_retranscribe",
      recording_id: value.recording_id,
    });
    view.unmount();
  }
});

it("keeps the old transcript with distinct queued and running words, without offering another pass", () => {
  const value = note();
  const { rerender } = render(<TranscriptTab note={value} send={vi.fn(send0)} />);
  for (const [state, words] of [
    ["queued", "En cola para rehacer la transcripción."],
    ["running", "Transcribiendo de nuevo…"],
  ]) {
    Object.assign(value.transcript, { final_pass: { state, error: null } });
    rerender(<TranscriptTab note={{ ...value }} send={vi.fn(send0)} />);
    expect(screen.getByText(words!)).toBeTruthy();
    expect(screen.getByText("Empezamos.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Rehacer transcripción" })).toBeNull();
  }
});

it("hides retranscription while capture is open or unknown", () => {
  const value = note();
  for (const capture of ["open", undefined]) {
    Object.assign(value, { capture });
    const view = render(<TranscriptTab note={value} send={vi.fn(send0)} />);
    expect(screen.queryByRole("button", { name: "Rehacer transcripción" })).toBeNull();
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
    render(<TranscriptTab note={value} send={vi.fn(send0)} />);
    expect(screen.getByRole("alert").textContent).toBe(text);
    expect(screen.getByText(/1 nombre.*no.*traslad/)).toBeTruthy();
    expect(screen.getByText("Empezamos.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Rehacer transcripción" })).toBeTruthy();
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
  fireEvent.click(screen.getByRole("button", { name: "Corregir «Empezamos.»" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
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
  expect((screen.getByRole("textbox", { name: "Debe decir" }) as HTMLInputElement).value).toBe(
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
  fireEvent.click(screen.getByRole("button", { name: "Corregir «Empezamos.»" }));
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
  fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
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
  fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
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
  const saving = screen.getByRole("button", { name: "Guardando en el hub…" });
  expect(saving.getAttribute("aria-busy")).toBe("true");
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
  expect((screen.getByRole("textbox", { name: "Debe decir" }) as HTMLInputElement).value).toBe(
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
  expect(screen.getByRole("status").textContent).toContain(
    "Corregido «Empezamos.» por «Empecemos.».",
  );
});

it("shows the correction window as the design draws it", () => {
  const current = {
    ...note(),
    dictionary: { dictionary_id: "vocabulary", source: "requested", series: null },
  };
  const words = {
    ...dictionaryLibrary(),
    dictionary_items: [
      {
        dictionary: {
          id: "vocabulary",
          name: "Proyecto",
          terms: [],
          series: [
            { calendar_item_id: "a", title: "Revisión semanal" },
            { calendar_item_id: "b", title: "Comité" },
          ],
        },
        pending: false,
      },
    ],
  };
  render(<TranscriptTab note={current} library={words} send={vi.fn(send0)} />);
  selectFirstTurn();
  const dialog = screen.getByRole("dialog", { name: "Corregir una palabra" });
  expect(
    within(dialog).getByText(
      "Orbit la corregirá en todas las reuniones que usan este diccionario.",
    ),
  ).toBeTruthy();
  expect(within(dialog).getByText("Se oyó")).toBeTruthy();
  expect(
    within(dialog).getByRole("option", { name: "Proyecto · lo usan Revisión semanal y Comité" }),
  ).toBeTruthy();
  expect(within(dialog).getByRole("option", { name: "Crear un diccionario nuevo" })).toBeTruthy();
  expect(
    within(dialog).getByText("La nota ya escrita cambiará cuando rehagas el resumen."),
  ).toBeTruthy();
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("does not turn an unread dictionary into a known empty one", () => {
  render(<TranscriptTab note={note()} library={dictionaryLibrary()} send={vi.fn(send0)} />);
  selectFirstTurn();
  expect(
    (screen.getByRole("button", { name: "Guardar corrección" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(
    screen.getAllByText("El diccionario aún no se ha podido leer del hub.").length,
  ).toBeGreaterThan(0);
});
