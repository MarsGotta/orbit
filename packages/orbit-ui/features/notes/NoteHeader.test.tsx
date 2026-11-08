import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { NoteHeader } from "./NoteHeader";
import type { Send } from "./send";
import { useRewriting } from "./use-rewriting";
import { summaryMarkdown } from "./notes-format";
import { library, writtenNote } from "./test-notes";

// Found on 2026-10-17: the list names a meeting by its title, and the opened
// note showed the title the note model wrote. Both say the meeting's title
// (the hub's document carries only that one).

afterEach(cleanup);

function written(): OpenNoteSnapshot {
  return { ...writtenNote(), recording_id: "meeting-title", title: "Comité de presupuesto" };
}

function title(): HTMLInputElement {
  return screen.getByRole("textbox", { name: es.noteScreen.titleLabel }) as HTMLInputElement;
}

/** The header as the note shows it: «Reescribiendo…» is the note's, shared with the Resumen. */
function Header({
  note,
  notes,
  send,
}: {
  note: OpenNoteSnapshot;
  notes: NoteLibrarySnapshot | null;
  send: Send;
}) {
  const rewriting = useRewriting(note, send);
  return (
    <NoteHeader
      library={notes}
      note={note}
      onAsk={() => undefined}
      rewriting={rewriting}
      send={send}
    />
  );
}

function header(
  note: OpenNoteSnapshot,
  send = vi.fn<(command: unknown) => Promise<void>>(() => Promise.resolve()),
  notes: NoteLibrarySnapshot | null = library(),
) {
  const view = render(<Header note={note} notes={notes} send={send} />);
  return { send, view };
}

/** A hub that takes several dictionaries: its documents say `dictionary_ids`. */
function withLists(note: OpenNoteSnapshot, ids: string[]): OpenNoteSnapshot {
  return {
    ...note,
    dictionary: {
      dictionary_id: ids[0] ?? null,
      dictionary_ids: ids,
      source: "requested",
      series: null,
    },
  };
}

describe("the opened note's title", () => {
  it("is the meeting's title, not the one the note was written with", () => {
    header(written());
    expect(screen.getByRole("heading", { level: 1 })).toBeTruthy();
    expect(title().value).toBe("Comité de presupuesto");
  });

  it("heads the copied summary too", () => {
    expect(summaryMarkdown(written()).split("\n")[0]).toBe("# Comité de presupuesto");
  });

  it("is followed by the short summary and the meeting's day, hours, length and people", () => {
    header(written());
    expect(
      screen.getByText("Se aprueba la sala de lectura nueva y se reparten las compras."),
    ).toBeTruthy();
    expect(
      screen.getByText("Mié 14 oct · 11:00 – 11:47 · 47 min · Bruno, Celia y tú"),
    ).toBeTruthy();
  });
});

describe("what this Mac changed and the hub does not have yet", () => {
  it("is said under the title, and only while there is some", () => {
    const { view } = header({ ...written(), pending_changes: ["title"] });
    expect(screen.getByText(es.noteScreen.changesOnMac)).toBeTruthy();
    view.unmount();
    header(written());
    expect(screen.queryByText(es.noteScreen.changesOnMac)).toBeNull();
  });
});

describe("renaming the meeting", () => {
  it("sends Alina's name, trimmed, when she leaves the field or presses Enter", () => {
    const { send } = header(written());
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
    const { send } = header(written());
    fireEvent.change(title(), { target: { value: "   " } });
    fireEvent.blur(title());
    expect(send).toHaveBeenCalledWith({
      command: "meeting_rename",
      recording_id: "meeting-title",
      title: "",
    });
  });

  it("sends nothing when the title did not change, and Escape puts it back", () => {
    const { send } = header(written());
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

function menu() {
  fireEvent.click(screen.getByRole("button", { name: /^Plantilla: Revisión · Español/ }));
  return screen.getByRole("dialog", { name: "Cómo escribe Orbit esta nota" });
}

/** Ticks a dictionary in the menu's multi-select. */
function tick(dialog: HTMLElement, name: string) {
  const trigger = within(dialog).getByRole("button", { name: /^Diccionarios de palabras/ });
  if (trigger.getAttribute("aria-expanded") !== "true") fireEvent.click(trigger);
  fireEvent.click(within(dialog).getByRole("checkbox", { name: new RegExp(`^${name}`) }));
}

function choose(dialog: HTMLElement, label: string, value: string) {
  fireEvent.change(within(dialog).getByRole("combobox", { name: label }), {
    target: { value },
  });
}

describe("how Orbit writes the note", () => {
  it("names the template, the language and how many dictionaries on its button", () => {
    const { view } = header(written(), undefined, null);
    expect(
      screen.getByRole("button", { name: "Plantilla: Revisión · Español · 1 diccionario" }),
    ).toBeTruthy();
    view.rerender(
      <Header
        note={withLists(written(), ["dic-biblio", "dic-otro"])}
        notes={null}
        send={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Plantilla: Revisión · Español · 2 diccionarios" }),
    ).toBeTruthy();
    view.rerender(<Header note={withLists(written(), [])} notes={null} send={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Plantilla: Revisión · Español" })).toBeTruthy();
  });

  it("offers the template, the meeting's language, its dictionaries and one switch for the series", () => {
    header(written());
    const dialog = menu();
    expect(
      (within(dialog).getByRole("combobox", { name: "Plantilla" }) as HTMLSelectElement).value,
    ).toBe("review");
    const language = within(dialog).getByRole("combobox", {
      name: "Idioma de la reunión",
    }) as HTMLSelectElement;
    expect(language.value).toBe("es");
    expect(Array.from(language.options).map((option) => option.textContent)).toEqual([
      "Español",
      "English",
      "Català",
      "Français",
      "Português",
      "Deutsch",
      "Italiano",
    ]);
    const dictionaries = within(dialog).getByRole("button", { name: /^Diccionarios de palabras/ });
    expect(dictionaries.textContent).toBe("Biblioteca");
    fireEvent.click(dictionaries);
    const listed = within(dialog).getByRole("group", { name: "Diccionarios de palabras" });
    expect(
      within(listed)
        .getAllByRole("checkbox")
        .map((box) => [box.textContent, box.getAttribute("aria-checked")]),
    ).toEqual([
      ["Bibliotecaesta reuniónComité y Obras · 0 palabras", "true"],
      ["OtroNinguna serie · 0 palabras", "false"],
    ]);
    const series = within(dialog).getByRole("switch", { name: "Usar lo mismo en toda la serie" });
    expect(series.getAttribute("aria-checked")).toBe("true");
    expect(within(dialog).queryByRole("button", { name: "Reescribir la nota" })).toBeNull();
  });

  it("does not write the note again on a mere change of template", () => {
    const { send } = header(written());
    const dialog = menu();
    choose(dialog, "Plantilla", "interview");
    expect(send).not.toHaveBeenCalled();
    expect(
      within(dialog).getByText(
        "Orbit volverá a escribir la nota. Si cambiaste el resumen, tu versión queda guardada.",
      ),
    ).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reescribir la nota" }));
    // The switch stays on: the series follows the new template.
    expect(send.mock.calls.map(([command]) => command)).toEqual([
      {
        command: "note_library",
        request: {
          action: "use_for_series",
          recording_id: "meeting-title",
          template_id: "interview",
          dictionary_id: "dic-biblio",
        },
      },
      { command: "note_regenerate", recording_id: "meeting-title", template_id: "interview" },
    ]);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("status").textContent).toBe(
      "Reescribiendo la nota con «Entrevista». Tu versión anterior queda guardada.",
    );
  });

  it("transcribes the meeting in another language only when asked to write it again", () => {
    const { send } = header(written());
    const dialog = menu();
    choose(dialog, "Idioma de la reunión", "en");
    expect(send).not.toHaveBeenCalled();
    expect(
      within(dialog).getByText(
        "Orbit volverá a escribir la nota y a transcribir la reunión en English. Si cambiaste el resumen, tu versión queda guardada.",
      ),
    ).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reescribir la nota" }));
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "meeting_language",
      recording_id: "meeting-title",
      language: "en",
    });
  });

  it("saves the template and the dictionary for the series in one order, and writes the note again", () => {
    const note = written();
    note.series = {
      calendar_item_id: "serie-comite",
      assigned: { state: "known", template_id: null },
    };
    const { send } = header(note);
    const dialog = menu();
    const series = within(dialog).getByRole("switch", { name: "Usar lo mismo en toda la serie" });
    expect(series.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(series);
    // A hub up to 0.10.1 takes one dictionary: ticking another replaces it.
    tick(dialog, "Otro");
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reescribir la nota" }));
    expect(send.mock.calls.map(([command]) => command)).toEqual([
      {
        command: "note_library",
        request: {
          action: "use_for_series",
          recording_id: "meeting-title",
          template_id: "review",
          dictionary_id: "dic-otro",
        },
      },
      { command: "note_regenerate", recording_id: "meeting-title", template_id: "review" },
    ]);
  });

  it("asks to write the note again when its dictionaries change, as for the template", () => {
    const { send } = header(withLists(written(), ["dic-biblio"]));
    const dialog = menu();
    tick(dialog, "Otro");
    expect(within(dialog).queryByRole("button", { name: "Guardar" })).toBeNull();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reescribir la nota" }));
    // The switch is on: the series takes the whole list with the template.
    expect(send.mock.calls.map(([command]) => command)).toEqual([
      {
        command: "note_library",
        request: {
          action: "use_for_series",
          recording_id: "meeting-title",
          template_id: "review",
          dictionary_id: null,
          dictionary_ids: ["dic-biblio", "dic-otro"],
        },
      },
      { command: "note_regenerate", recording_id: "meeting-title", template_id: "review" },
    ]);
    expect(screen.getByRole("status").textContent).toBe(
      "Reescribiendo la nota con «Revisión». Tu versión anterior queda guardada.",
    );
  });

  it.each([
    [
      "a hub that takes lists",
      ["dic-biblio"],
      { dictionary_id: null, dictionary_ids: ["dic-biblio", "dic-otro"] },
    ],
    ["a hub up to 0.10.1", null, { dictionary_id: "dic-otro" }],
  ])("chooses dictionaries for this meeting alone with %s", (_, ids, chosenOnes) => {
    const { send } = header(ids === null ? written() : withLists(written(), ids));
    const dialog = menu();
    fireEvent.click(within(dialog).getByRole("switch", { name: "Usar lo mismo en toda la serie" }));
    tick(dialog, "Otro");
    fireEvent.click(within(dialog).getByRole("button", { name: "Reescribir la nota" }));
    expect(send).toHaveBeenCalledWith({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "select",
          recording_id: "meeting-title",
          ...chosenOnes,
          use_for_series: false,
        },
      },
    });
    expect(send).toHaveBeenLastCalledWith({
      command: "note_regenerate",
      recording_id: "meeting-title",
      template_id: "review",
    });
  });

  it("says why the series could not take them, in the menu", () => {
    const refused = library();
    refused.refusals = [
      {
        target_kind: "meeting_series",
        target_id: "meeting-title",
        action: "use_for_series",
        status: 409,
        code: "meeting_series_missing",
      },
    ];
    header(written(), undefined, refused);
    const dialog = menu();
    expect(within(dialog).getByRole("alert").textContent).toBe(
      "Esta reunión no pertenece a una serie del calendario.",
    );
  });

  it("does not offer a language before the hub has the meeting", () => {
    const note = written();
    note.transcript.language = null;
    header(note);
    fireEvent.click(screen.getByRole("button", { name: /^Plantilla: Revisión/ }));
    expect(screen.queryByRole("combobox", { name: "Idioma de la reunión" })).toBeNull();
  });

  it("says how far transcribing it again got", () => {
    const note = written();
    note.transcript.language = "en";
    note.transcript.retranscribing = { language: "en", done: 1, total: 4 };
    header(note);
    expect(screen.getByRole("status").textContent).toBe("Transcribiendo de nuevo… 1 de 4");
  });
});

describe("«Rehacer resumen»", () => {
  it("is no longer in the header: it is in the Resumen's toolbar", () => {
    header(written());
    expect(screen.queryByRole("button", { name: "Rehacer resumen" })).toBeNull();
  });
});

describe("the meeting's menu", () => {
  it("offers «Borrar reunión» on a meeting, never on the one being recorded", () => {
    const { view } = header(written());
    expect(screen.getByRole("button", { name: "Más" })).toBeTruthy();
    view.unmount();
    const recording = written();
    recording.progress.upload = { state: "active", cause: { kind: "recording" } };
    header(recording);
    expect(screen.queryByRole("button", { name: "Más" })).toBeNull();
  });
});
