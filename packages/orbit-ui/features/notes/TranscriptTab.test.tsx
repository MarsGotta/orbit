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
        "Pulsa un nombre para cambiar quién habla. Pasa el ratón por una palabra y púlsala para corregirla: la corrección se guarda en los diccionarios de esta reunión.",
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

describe("«Rehacer transcripción» (annex, change 7)", () => {
  it("asks first, saying what it uses and what stays, and then asks the hub", () => {
    const separated = voicesNote();
    Object.assign(separated.transcript, { final_pass: { state: "completed", error: null } });
    for (const value of [note(), separated]) {
      const send = vi.fn(send0);
      const view = render(<TranscriptTab note={value} send={send} />);
      fireEvent.click(screen.getByRole("button", { name: "Rehacer transcripción" }));
      const question = screen.getByRole("dialog", { name: "¿Rehacer la transcripción?" });
      expect(question.textContent).toContain(
        "Orbit vuelve a escuchar la grabación (2 min) en español. Puedes seguir con lo tuyo mientras tanto.",
      );
      // A hub up to 0.10.1 writes the note again after the pass, and the question says so.
      expect(question.textContent).toContain("Después, Orbit reescribe la nota.");
      expect(send).not.toHaveBeenCalled();
      fireEvent.click(within(question).getByRole("button", { name: "Rehacer transcripción" }));
      expect(send).toHaveBeenCalledExactlyOnceWith({
        command: "transcript_retranscribe",
        recording_id: value.recording_id,
      });
      expect(screen.getByText("En cola para rehacer la transcripción.")).toBeTruthy();
      view.unmount();
    }
  });

  it("names the meeting's dictionaries, and keeps the note, with a hub that takes lists", () => {
    const value = {
      ...note(),
      dictionary: {
        dictionary_id: "dic-albar",
        dictionary_ids: ["dic-albar", "dic-lumo"],
        source: "requested",
        series: null,
      },
    };
    render(<TranscriptTab library={wordsLibrary()} note={value} send={vi.fn(send0)} />);
    fireEvent.click(screen.getByRole("button", { name: "Rehacer transcripción" }));
    const question = screen.getByRole("dialog", { name: "¿Rehacer la transcripción?" });
    expect(question.textContent).toContain("en español con Ferretería Albar y Lumo.");
    expect(question.textContent).toContain(
      "Se mantienen los nombres que pusiste a cada persona. El resumen no cambia hasta que lo rehagas.",
    );
    fireEvent.click(within(question).getByRole("button", { name: "Cancelar" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps the old transcript dimmed while queued and running, with the hub's progress", () => {
    const value = note();
    const { container, rerender } = render(<TranscriptTab note={value} send={vi.fn(send0)} />);
    for (const [final_pass, words] of [
      [{ state: "queued", error: null }, "En cola para rehacer la transcripción."],
      [
        {
          state: "running",
          error: null,
          progress: { done_ms: 30_000, total_ms: 120_000, remaining_ms: 90_000 },
        },
        "Rehaciendo la transcripción… · puedes seguir con lo tuyo",
      ],
    ] as const) {
      Object.assign(value.transcript, { final_pass });
      rerender(<TranscriptTab note={{ ...value }} send={vi.fn(send0)} />);
      expect(container.textContent).toContain(words);
      expect(screen.getByText("Empezamos.")).toBeTruthy();
      expect(container.querySelector("[aria-busy='true'] article")).toBeTruthy();
      expect(
        (screen.getByRole("button", { name: "Rehacer transcripción" }) as HTMLButtonElement)
          .disabled,
      ).toBe(true);
    }
    expect(screen.getByText("unos 2 min")).toBeTruthy();
    expect((container.querySelector("[data-progress]") as HTMLElement | null)?.style.width).toBe(
      "25%",
    );
  });

  it("offers «Rehacer resumen» once the pass it saw is done, and never for an old one", () => {
    const onRedoSummary = vi.fn();
    const value = { ...note(), summary: writtenSummary() };
    Object.assign(value.transcript, { final_pass: { state: "completed", error: null } });
    const { rerender } = render(
      <TranscriptTab note={value} onRedoSummary={onRedoSummary} send={vi.fn(send0)} />,
    );
    expect(screen.queryByText(/Transcripción rehecha/)).toBeNull();
    for (const state of ["running", "completed"]) {
      const next = {
        ...value,
        transcript: { ...value.transcript, final_pass: { state, error: null } },
      };
      rerender(
        <TranscriptTab
          note={next as OpenNoteSnapshot}
          onRedoSummary={onRedoSummary}
          send={vi.fn(send0)}
        />,
      );
    }
    const done = screen.getByRole("status");
    expect(done.textContent).toBe(
      "✓Transcripción rehecha. Rehaz el resumen para que la nota la use.Rehacer resumen",
    );
    fireEvent.click(within(done).getByRole("button", { name: "Rehacer resumen" }));
    expect(onRedoSummary).toHaveBeenCalledOnce();
    expect(
      (screen.getByRole("button", { name: "Rehacer transcripción" }) as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  it.each([
    [1, "En 1 tramo se mantuvo la transcripción en vivo, que era mejor."],
    [3, "En 3 tramos se mantuvo la transcripción en vivo, que era mejor."],
  ])("says quietly, not as a failure, that %s spans kept the live transcript", (spans, text) => {
    const value = note();
    Object.assign(value.transcript, {
      final_pass: {
        state: "completed",
        error: null,
        kept_draft: { spans, duration_ms: spans * 20_000 },
      },
    });
    render(<TranscriptTab note={value} send={vi.fn(send0)} />);
    expect(screen.getByRole("status").textContent).toBe(text);
    expect(screen.queryByRole("button", { name: "Cerrar" })).toBeNull();
    expect(screen.getByText("Empezamos.")).toBeTruthy();
  });

  it("says nothing of the live transcript when the pass kept none of it", () => {
    const value = note();
    Object.assign(value.transcript, { final_pass: { state: "completed", error: null } });
    render(<TranscriptTab note={value} send={vi.fn(send0)} />);
    expect(screen.queryByText(/transcripción en vivo/)).toBeNull();
  });

  it("is not offered while capture is open or unknown", () => {
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
    "explains failure %s with budget %s in the card, keeping the old transcript",
    (error, timeout_ms, text) => {
      const value = note();
      Object.assign(value.transcript, {
        final_pass: { state: "failed", error, timeout_ms },
        unmatched_speaker_names: 1,
      });
      render(<TranscriptTab note={value} send={vi.fn(send0)} />);
      expect(screen.getByText(text)).toBeTruthy();
      expect(screen.getByText(/1 nombre.*no.*traslad/)).toBeTruthy();
      expect(screen.getByText("Empezamos.")).toBeTruthy();
      expect(screen.getByRole("button", { name: "Rehacer transcripción" })).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "Cerrar" }));
      expect(screen.queryByText(text)).toBeNull();
      expect(screen.getByText("Empezamos.")).toBeTruthy();
    },
  );
});

function writtenSummary(): NonNullable<OpenNoteSnapshot["summary"]> {
  return {
    short_summary: "Se cierra el comité.",
    sections: [],
    template_id: "review",
    template_name: "Revisión",
    generated_at_ms: 1,
    model: "hub",
    superseded_available: false,
  };
}

/** A meeting that says «Orvit» three times, from a hub that takes several dictionaries. */
function wordsNote(dictionary_ids: string[] | null = ["dic-albar", "dic-lumo"]): OpenNoteSnapshot {
  const value = note();
  value.transcript.turns = [
    { index: 0, at_ms: 0, speaker_key: "mic_local", text: "Ya está en Orvit." },
    { index: 1, at_ms: 12_000, speaker_key: "remote_app", text: "Lo apunto en orvit, ¿vale?" },
    { index: 2, at_ms: 30_000, speaker_key: "mic_local", text: "Las notas de Orvit salen." },
  ];
  value.dictionary =
    dictionary_ids === null
      ? { dictionary_id: "dic-albar", source: "requested", series: null }
      : {
          dictionary_id: dictionary_ids[0] ?? null,
          dictionary_ids,
          source: "requested",
          series: null,
        };
  return value;
}

function wordsLibrary(terms: Record<string, { correct: string; wrong: string[] }[]> = {}) {
  return {
    ...dictionaryLibrary(),
    dictionary_items: [
      {
        id: "dic-albar",
        name: "Ferretería Albar",
        series: [{ calendar_item_id: "a", title: "Revisión semanal" }],
      },
      { id: "dic-lumo", name: "Lumo", series: [] },
      { id: "dic-otro", name: "Producto", series: [] },
    ].map((dictionary) => ({
      dictionary: {
        ...dictionary,
        terms: (terms[dictionary.id] ?? []).map((term, index) => ({ id: `t${index}`, ...term })),
      },
      pending: false,
    })),
  };
}

/** A press on the text, where it puts the caret: at `offset` of the turn's paragraph. */
function press(text: string, offset: number) {
  const paragraph = screen.getByText(text);
  const node = paragraph.firstChild;
  if (!node) throw new Error("no text");
  window.getSelection()?.collapse(node, offset);
  fireEvent.mouseUp(paragraph);
  fireEvent.click(paragraph);
}

const orbitTerm = { correct: "Orbit", wrong: ["orvit"] };

describe("correcting a word (annex, change 5)", () => {
  it("opens beside the word pressed, with how many times the meeting says it", () => {
    render(<TranscriptTab library={wordsLibrary()} note={wordsNote()} send={vi.fn(send0)} />);
    press("Ya está en Orvit.", 13);
    const menu = screen.getByRole("dialog", { name: "Corregir «Orvit»" });
    expect(within(menu).getByText("Sale 3 veces en esta reunión.")).toBeTruthy();
    const field = within(menu).getByRole("textbox", { name: "Debe decir" }) as HTMLInputElement;
    expect(field.value).toBe("Orvit");
    expect(document.activeElement).toBe(field);
    const every = within(menu).getByRole("checkbox", { name: "Corregir las 3 veces" });
    expect(every.getAttribute("aria-checked")).toBe("true");
    // A dictionary corrects every place: this cannot be unticked.
    expect(every.getAttribute("aria-disabled")).toBe("true");
    expect(
      within(menu).getByRole("button", { name: /^Se añade a · puedes elegir varios/ }).textContent,
    ).toBe("Ferretería AlbarLumo");
    expect(
      within(menu).getByText(
        "Orbit la corregirá en las reuniones que usen Ferretería Albar y Lumo.",
      ),
    ).toBeTruthy();
    // Nothing changed yet: nothing to save.
    expect(
      (within(menu).getByRole("button", { name: "Corregir y añadir" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(document.querySelector("[data-picked]")?.textContent).toBe("Orvit");
    fireEvent.keyDown(field, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("goes into every chosen dictionary at once, then shows the words corrected", () => {
    const send = vi.fn(send0);
    const value = { ...wordsNote(), summary: writtenSummary() };
    const { rerender, container } = render(
      <TranscriptTab library={wordsLibrary()} note={value} send={send} />,
    );
    press("Lo apunto en orvit, ¿vale?", 15);
    const menu = screen.getByRole("dialog", { name: "Corregir «orvit»" });
    fireEvent.change(within(menu).getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    fireEvent.click(within(menu).getByRole("button", { name: /^Se añade a/ }));
    fireEvent.click(within(menu).getByRole("checkbox", { name: /^Producto/ }));
    fireEvent.click(within(menu).getByRole("button", { name: "Corregir y añadir" }));
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "correct",
          recording_id: value.recording_id,
          dictionary_ids: ["dic-albar", "dic-lumo", "dic-otro"],
          wrong: "orvit",
          correct: "Orbit",
          name: null,
        },
      },
    });
    const pending = {
      ...wordsLibrary(),
      dictionary_pending: ["dictionaries:dic-albar,dic-lumo,dic-otro"],
    };
    rerender(<TranscriptTab library={pending} note={value} send={send} />);
    expect(screen.getByRole("button", { name: /Guardando/ }).getAttribute("aria-busy")).toBe(
      "true",
    );
    const saved = wordsLibrary({
      "dic-albar": [orbitTerm],
      "dic-lumo": [orbitTerm],
      "dic-otro": [orbitTerm],
    });
    rerender(<TranscriptTab library={saved} note={value} send={send} />);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("status").textContent).toContain(
      "«orvit» ahora es «Orbit» en 3 sitios y está en Ferretería Albar, Lumo y Producto. La nota usará el cambio cuando rehagas el resumen.",
    );
    // The words read corrected at once, and marked, before the hub's read brings them.
    expect(
      Array.from(container.querySelectorAll("[data-fixed]")).map((word) => word.textContent),
    ).toEqual(["Orbit", "Orbit", "Orbit"]);
    const read = {
      ...value,
      transcript: {
        ...value.transcript,
        turns: value.transcript.turns.map((turn) => ({
          ...turn,
          text: turn.text.replace(/orvit/i, "Orbit"),
        })),
      },
    };
    rerender(<TranscriptTab library={saved} note={read} send={send} />);
    expect(container.querySelectorAll("[data-fixed]")).toHaveLength(3);
    expect(screen.getByText(/Lo apunto en/).textContent).toBe("Lo apunto en Orbit, ¿vale?");
  });

  it("undoes it in every dictionary with «Deshacer», without asking", () => {
    const send = vi.fn(send0);
    const value = wordsNote(["dic-albar"]);
    const { rerender } = render(
      <TranscriptTab library={wordsLibrary()} note={value} send={send} />,
    );
    press("Ya está en Orvit.", 12);
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Corregir y añadir" }));
    const saved = wordsLibrary({ "dic-albar": [orbitTerm] });
    rerender(<TranscriptTab library={saved} note={value} send={send} />);
    fireEvent.click(within(screen.getByRole("status")).getByRole("button", { name: "Deshacer" }));
    expect(send).toHaveBeenLastCalledWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "remove_correction",
          dictionary_ids: ["dic-albar"],
          wrong: "Orvit",
          correct: "Orbit",
        },
      },
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    rerender(<TranscriptTab library={wordsLibrary()} note={value} send={send} />);
    expect(screen.getByRole("status").textContent).toBe(
      "Corrección deshecha. «Orvit» ya no está en Ferretería Albar.",
    );
    expect(document.querySelectorAll("[data-fixed]")).toHaveLength(0);
  });

  it("says when another dictionary of the meeting hears the word otherwise, and which wins", () => {
    const send = vi.fn(send0);
    const value = wordsNote(["dic-albar", "dic-lumo"]);
    const { rerender, container } = render(
      <TranscriptTab
        library={wordsLibrary({ "dic-albar": [{ correct: "Órbita", wrong: ["orvit"] }] })}
        note={value}
        send={send}
      />,
    );
    press("Ya está en Orvit.", 12);
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Se añade a/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /^Ferretería Albar/ }));
    fireEvent.click(screen.getByRole("button", { name: "Corregir y añadir" }));
    rerender(
      <TranscriptTab
        library={{
          ...wordsLibrary({
            "dic-albar": [{ correct: "Órbita", wrong: ["orvit"] }],
            "dic-lumo": [orbitTerm],
          }),
          dictionary_conflicts: [
            {
              target: "dictionaries:dic-lumo",
              conflicts: [
                {
                  wrong: "Orvit",
                  correct: "Orbit",
                  dictionary_id: "dic-lumo",
                  other_dictionary_id: "dic-albar",
                  other_name: "Ferretería Albar",
                  other_correct: "Órbita",
                  applied_dictionary_id: "dic-albar",
                },
              ],
            },
          ],
        }}
        note={value}
        send={send}
      />,
    );
    expect(screen.getByRole("status").textContent).toContain(
      "Ferretería Albar corrige «Orvit» por «Órbita». Cuando dos diccionarios no coinciden, gana el primero: en esta reunión, Ferretería Albar.",
    );
    // The meeting says the word the first dictionary gives.
    expect(
      Array.from(container.querySelectorAll("[data-fixed]")).map((word) => word.textContent),
    ).toEqual(["Órbita", "Órbita", "Órbita"]);
  });

  it("keeps what was typed when the hub refuses it, with the hub's numbers", () => {
    const send = vi.fn(send0);
    const value = wordsNote(["dic-albar"]);
    const { rerender } = render(
      <TranscriptTab library={wordsLibrary()} note={value} send={send} />,
    );
    press("Ya está en Orvit.", 12);
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Corregir y añadir" }));
    rerender(
      <TranscriptTab
        library={{
          ...wordsLibrary(),
          dictionary_failures: [
            {
              target: "dictionaries:dic-albar",
              code: "dictionary_full",
              detail: "HTTP 409",
              numbers: { terms: 128, max_terms: 128 },
            },
          ],
        }}
        note={value}
        send={send}
      />,
    );
    const menu = screen.getByRole("dialog", { name: "Corregir «Orvit»" });
    expect(within(menu).getByRole("alert").textContent).toBe(
      "El diccionario está lleno (términos: 128, máximo de términos: 128).",
    );
    expect(
      (within(menu).getByRole("textbox", { name: "Debe decir" }) as HTMLInputElement).value,
    ).toBe("Orbit");
    expect(
      (within(menu).getByRole("button", { name: "Corregir y añadir" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("can be pressed again when the daemon refuses the order at once", async () => {
    const send = vi.fn(async () => false);
    render(<TranscriptTab library={wordsLibrary()} note={wordsNote()} send={send} />);
    press("Ya está en Orvit.", 12);
    const menu = screen.getByRole("dialog", { name: "Corregir «Orvit»" });
    fireEvent.change(within(menu).getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    await act(async () => {
      fireEvent.click(within(menu).getByRole("button", { name: "Corregir y añadir" }));
    });
    expect(send).toHaveBeenCalledTimes(1);
    expect(within(menu).queryByRole("button", { name: /Guardando/ })).toBeNull();
    expect(
      (within(menu).getByRole("textbox", { name: "Debe decir" }) as HTMLInputElement).value,
    ).toBe("Orbit");
    expect(
      (within(menu).getByRole("button", { name: "Corregir y añadir" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("names one dictionary to a hub up to 0.10.1, and has no «Deshacer» there", () => {
    const send = vi.fn(send0);
    const value = wordsNote(null);
    const { rerender } = render(
      <TranscriptTab library={wordsLibrary()} note={value} send={send} />,
    );
    press("Ya está en Orvit.", 12);
    const menu = screen.getByRole("dialog", { name: "Corregir «Orvit»" });
    fireEvent.change(within(menu).getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    // Ticking another replaces the one chosen: that hub takes one.
    fireEvent.click(within(menu).getByRole("button", { name: /^Se añade a/ }));
    fireEvent.click(within(menu).getByRole("checkbox", { name: /^Lumo/ }));
    expect(within(menu).getByRole("button", { name: /^Se añade a/ }).textContent).toBe("Lumo");
    fireEvent.click(within(menu).getByRole("button", { name: "Corregir y añadir" }));
    expect(send).toHaveBeenCalledWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "correct",
          recording_id: value.recording_id,
          dictionary_id: "dic-lumo",
          wrong: "Orvit",
          correct: "Orbit",
          name: null,
        },
      },
    });
    rerender(
      <TranscriptTab
        library={wordsLibrary({ "dic-lumo": [orbitTerm] })}
        note={value}
        send={send}
      />,
    );
    expect(screen.getByRole("status").textContent).toContain("y está en Lumo.");
    expect(screen.queryByRole("button", { name: "Deshacer" })).toBeNull();
  });

  it("creates a dictionary with the first correction, proposing the series' title", () => {
    const send = vi.fn(send0);
    const value = {
      ...wordsNote([]),
      dictionary: {
        dictionary_id: null,
        dictionary_ids: [],
        source: null,
        series: { calendar_item_id: "daily", title: "Daily del proyecto" },
      },
    };
    render(<TranscriptTab library={dictionaryLibrary()} note={value} send={send} />);
    press("Ya está en Orvit.", 12);
    expect(
      (screen.getByRole("textbox", { name: "Nombre del diccionario" }) as HTMLInputElement).value,
    ).toBe("Daily del proyecto");
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    expect(
      screen.getByText("Orbit la corregirá en las reuniones que usen Daily del proyecto."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Corregir y añadir" }));
    expect(send).toHaveBeenCalledWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "correct",
          recording_id: value.recording_id,
          dictionary_ids: [expect.any(String)],
          wrong: "Orvit",
          correct: "Orbit",
          name: "Daily del proyecto",
        },
      },
    });
  });

  it("opens the same menu from a selection, the keyboard's way, under the turn", () => {
    const send = vi.fn(send0);
    const current = {
      ...note(),
      dictionary: {
        dictionary_id: "dic-albar",
        dictionary_ids: ["dic-albar"],
        source: "requested",
        series: null,
      },
    };
    render(<TranscriptTab library={wordsLibrary()} note={current} send={send} />);
    const paragraph = screen.getByText("Empezamos.");
    const range = document.createRange();
    range.selectNodeContents(paragraph);
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
    fireEvent.mouseUp(paragraph);
    fireEvent.click(screen.getByRole("button", { name: "Corregir «Empezamos.»" }));
    const menu = screen.getByRole("dialog", { name: "Corregir «Empezamos.»" });
    expect(within(menu).getByText("Sale 1 vez en esta reunión.")).toBeTruthy();
    expect(within(menu).queryByRole("checkbox", { name: /^Corregir las/ })).toBeNull();
    fireEvent.change(within(menu).getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Empecemos." },
    });
    fireEvent.keyDown(within(menu).getByRole("textbox", { name: "Debe decir" }), { key: "Enter" });
    expect(send).toHaveBeenCalledWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "correct",
          recording_id: current.recording_id,
          dictionary_ids: ["dic-albar"],
          wrong: "Empezamos.",
          correct: "Empecemos.",
          name: null,
        },
      },
    });
  });

  it("does not turn an unread dictionary into a known empty one", () => {
    render(<TranscriptTab library={dictionaryLibrary()} note={wordsNote()} send={vi.fn(send0)} />);
    press("Ya está en Orvit.", 12);
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    const menu = screen.getByRole("dialog", { name: "Corregir «Orvit»" });
    expect(
      (within(menu).getByRole("button", { name: "Corregir y añadir" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
    cleanup();
    const unread = { ...note(), transcript: wordsNote().transcript };
    render(<TranscriptTab library={dictionaryLibrary()} note={unread} send={vi.fn(send0)} />);
    press("Ya está en Orvit.", 12);
    fireEvent.change(screen.getByRole("textbox", { name: "Debe decir" }), {
      target: { value: "Orbit" },
    });
    expect(
      (screen.getByRole("button", { name: "Corregir y añadir" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(screen.getByRole("alert").textContent).toBe(
      "El diccionario aún no se ha podido leer del hub.",
    );
  });
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
