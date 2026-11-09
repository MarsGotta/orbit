import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MarkdownEditor, type MarkdownEditorProps } from "./MarkdownEditor";
import { editorNamed, focusAt, lines, paste, press, select, typeText } from "./test-editor";

// The one editor of Orbit, driven as Rita types in it. Invented notes and people.

const NOTES = [
  { title: "Comité de la biblioteca", detail: "14 oct" },
  { title: "Biblioteca · obras del verano", detail: "hoy, 16:00" },
  { title: "Huerto escolar", detail: "9 oct" },
];
const PEOPLE = [
  { name: "Bruno Lago", detail: "Invitado a esta reunión" },
  { name: "Celia Mar", detail: "De tu calendario" },
];

function renderEditor(props: Partial<MarkdownEditorProps> = {}) {
  const onChange = vi.fn();
  render(
    <MarkdownEditor
      label="Notas"
      notes={NOTES}
      onChange={onChange}
      people={PEOPLE}
      value=""
      {...props}
    />,
  );
  return { onChange, editor: editorNamed("Notas") };
}

function text(editor: ReturnType<typeof editorNamed>): string {
  return editor.state.doc.toString();
}

describe("what it holds", () => {
  it("is the Markdown exactly as typed and pasted, with nothing rewritten", () => {
    const { editor, onChange } = renderEditor();
    focusAt(editor);
    const pasted =
      "# Título\n\n*  lista con dos espacios\n\n| a | b |\n|---|---|\n<b>no es HTML</b>\n";
    paste(editor, pasted);
    typeText(editor, "__casi__ y 2*3");
    expect(text(editor)).toBe(`${pasted}__casi__ y 2*3`);
    expect(onChange).toHaveBeenLastCalledWith(`${pasted}__casi__ y 2*3`);
  });

  it("reports nothing until something is typed", () => {
    const { onChange } = renderEditor({ value: "## Plan\n- [ ] llamar" });
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("the formatted lines", () => {
  const NOTE = [
    "## Para el comité",
    "Confirmar con **Bruno** la fecha: el *lunes 26*.",
    "- [x] Repasar [[Comité de la biblioteca]]",
    "- [ ] Pedir a @Celia Mar la ==llave==",
    "> la sala nueva es más luminosa",
    "1. ¿Quién abre?",
    "---",
    "```bash",
    "ls salas",
    "```",
  ].join("\n");

  it("hide their marks on every line but the cursor's, which shows them", () => {
    const { editor } = renderEditor({ value: NOTE });
    const shown = lines(editor);
    expect(shown[0]).toMatchObject({ text: "Para el comité" });
    expect(shown[0]?.classes).toContain("cm-md-h2");
    expect(shown[1]?.text).toBe("Confirmar con Bruno la fecha: el lunes 26.");
    expect(shown[2]?.text).toBe("Repasar Comité de la biblioteca");
    expect(shown[2]?.classes).toContain("cm-md-done");
    expect(shown[3]?.text).toBe("Pedir a @Celia Mar la llave");
    expect(shown[4]?.text).toBe("la sala nueva es más luminosa");
    expect(shown[4]?.classes).toContain("cm-md-quote");
    expect(shown[5]?.text).toBe("1.¿Quién abre?");
    expect(shown[7]?.text).toBe("bash");
    expect(editor.contentDOM.querySelector(".cm-md-strong")?.textContent).toBe("Bruno");
    expect(editor.contentDOM.querySelector(".cm-md-link")?.textContent).toBe(
      "Comité de la biblioteca",
    );
    expect(editor.contentDOM.querySelector(".cm-md-mention")?.textContent).toBe("@Celia Mar");
    expect(editor.contentDOM.querySelector(".cm-md-highlight")?.textContent).toBe("llave");
    expect(editor.contentDOM.querySelector(".cm-md-rule")).not.toBeNull();

    // On the second line, its own Markdown, in grey; the first is formatted again.
    focusAt(editor, editor.state.doc.line(2).from);
    const writing = lines(editor);
    expect(writing[0]?.text).toBe("Para el comité");
    expect(writing[1]?.text).toBe("Confirmar con **Bruno** la fecha: el *lunes 26*.");
    expect(writing[1]?.classes).toContain("cm-md-active");
    expect(editor.contentDOM.querySelectorAll(".cm-md-active .cm-md-mark")).toHaveLength(4);

    // A selection keeps its line formatted, as the design draws the format bar.
    select(editor, editor.state.doc.line(2).from, editor.state.doc.line(2).from + 9);
    expect(lines(editor)[1]?.text).toBe("Confirmar con Bruno la fecha: el lunes 26.");
  });

  it("turn a task's box into a press that marks it, in the text", () => {
    const { editor, onChange } = renderEditor({ value: "- [ ] Pedir la llave" });
    const box = screen.getByRole("checkbox", { name: "Marcar tarea" });
    expect(box.getAttribute("aria-checked")).toBe("false");
    fireEvent.mouseDown(box);
    expect(text(editor)).toBe("- [x] Pedir la llave");
    expect(onChange).toHaveBeenLastCalledWith("- [x] Pedir la llave");
  });

  it("change a line's kind as its shortcut is typed at its start", async () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "### Dudas\n> cita");
    act(() => editor.contentDOM.blur());
    // The editor hears the focus leave a moment after the blur.
    await waitFor(() => expect(lines(editor)[1]?.text).toBe("cita"));
    const shown = lines(editor);
    expect(shown[0]).toMatchObject({ text: "Dudas" });
    expect(shown[0]?.classes).toContain("cm-md-h3");
    expect(shown[1]).toMatchObject({ text: "cita" });
    expect(shown[1]?.classes).toContain("cm-md-quote");
  });
});

describe("the keyboard", () => {
  it("↵ goes on with a list, and ↵ on an empty item ends it", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "- [ ] uno\ndos\n");
    expect(text(editor)).toBe("- [ ] uno\n- [ ] dos\n- [ ] ");
    press(editor, "Enter");
    expect(text(editor)).toBe("- [ ] uno\n- [ ] dos\n");
    typeText(editor, "1. a\n");
    expect(text(editor)).toBe("- [ ] uno\n- [ ] dos\n1. a\n2. ");
    typeText(editor, "\nfin");
    expect(text(editor)).toBe("- [ ] uno\n- [ ] dos\n1. a\nfin");
  });

  it("⌘B puts the chosen text in bold, and again takes it off", () => {
    const { editor } = renderEditor({ value: "la fecha de arranque" });
    select(editor, 3, 8);
    press(editor, "Mod-b");
    expect(text(editor)).toBe("la **fecha** de arranque");
    press(editor, "Mod-b");
    expect(text(editor)).toBe("la fecha de arranque");
  });

  it("⌘/ shows the raw Markdown, and the same text goes on in it", () => {
    const { editor } = renderEditor({ value: "## Plan" });
    focusAt(editor);
    press(editor, "Mod-/");
    expect(
      screen.getByRole("button", { name: /Volver al texto/ }).getAttribute("aria-pressed"),
    ).toBe("true");
    focusAt(editor, editor.state.doc.length);
    act(() => editor.contentDOM.blur());
    // Raw, the marks stay even off the cursor's line.
    expect(lines(editor)[0]?.text).toBe("## Plan");
    focusAt(editor);
    typeText(editor, " B");
    press(editor, "Mod-/");
    expect(screen.getByRole("button", { name: /Markdown/ }).getAttribute("aria-pressed")).toBe(
      "false",
    );
    expect(text(editor)).toBe("## Plan B");
  });
});

describe("the «/» menu", () => {
  it("opens at the start of an empty line, filters without accents and inserts with ↵", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "/");
    const menu = screen.getByRole("listbox", { name: "Insertar" });
    expect(
      within(menu)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual([
      "TTítulo##",
      "tSubtítulo###",
      "•Lista-",
      "1.Lista numerada1.",
      "☐Tarea- [ ]",
      "❝Cita>",
      "{}Código```",
      "—Separador---",
      "[[Enlazar una nota[[",
      "@Mencionar a alguien@",
    ]);
    typeText(editor, "titulo");
    const filtered = screen.getAllByRole("option");
    expect(filtered.map((option) => option.textContent)).toEqual(["TTítulo##", "tSubtítulo###"]);
    press(editor, "ArrowDown");
    expect(filtered[1]?.getAttribute("aria-selected")).toBe("true");
    press(editor, "Enter");
    expect(text(editor)).toBe("### ");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("says when nothing is called so, and Esc closes it leaving the text", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "/zz");
    expect(
      screen.getByText("Nada se llama así. Sigue escribiendo para dejarlo como texto."),
    ).toBeTruthy();
    press(editor, "Escape");
    expect(screen.queryByText(/Nada se llama así/)).toBeNull();
    expect(text(editor)).toBe("/zz");
  });

  it("offers the principal to go on writing, with what is written so far", () => {
    const keepWriting = vi.fn();
    const { editor } = renderEditor({
      assistant: { name: "Vega", ask: vi.fn(), keepWriting },
      value: "Primera idea",
    });
    focusAt(editor);
    typeText(editor, "\n/pedir");
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([
      "✦Pedir a Vega que siga escribiendo",
    ]);
    press(editor, "Enter");
    expect(keepWriting).toHaveBeenCalledWith("Primera idea\n");
    expect(text(editor)).toBe("Primera idea\n");
  });
});

describe("«[[» and «@»", () => {
  it("«[[» lists the notes, filters as it is typed and leaves the link by title", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "Revisar [[bibli");
    const menu = screen.getByRole("listbox", { name: "Enlazar una nota" });
    expect(
      within(menu)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["[[Comité de la biblioteca14 oct", "[[Biblioteca · obras del veranohoy, 16:00"]);
    press(editor, "ArrowDown");
    press(editor, "Enter");
    expect(text(editor)).toBe("Revisar [[Biblioteca · obras del verano]] ");
  });

  it("«@» lists the meeting's guests and the people Orbit knows", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "Pedir a @");
    const menu = screen.getByRole("listbox", { name: "Mencionar" });
    expect(
      within(menu)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["BBruno LagoInvitado a esta reunión", "CCelia MarDe tu calendario"]);
    typeText(editor, "cel");
    press(editor, "Tab");
    expect(text(editor)).toBe("Pedir a @Celia Mar ");
  });

  it("an «@» inside a word, as in an address, opens nothing", () => {
    const { editor } = renderEditor();
    focusAt(editor);
    typeText(editor, "rita@");
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});

describe("a selection", () => {
  it("opens the format bar, whose marks go around it, and «Preguntar» carries it", () => {
    const ask = vi.fn();
    const { editor } = renderEditor({
      assistant: { name: "Vega", ask, keepWriting: vi.fn() },
      value: "la fecha de arranque",
    });
    select(editor, 3, 20);
    const bar = screen.getByRole("toolbar", { name: "Dar formato al texto elegido" });
    fireEvent.click(within(bar).getByRole("button", { name: "Resaltar · ⌘⇧H" }));
    expect(text(editor)).toBe("la ==fecha de arranque==");
    fireEvent.click(within(bar).getByRole("button", { name: "Preguntar a Vega" }));
    expect(ask).toHaveBeenCalledWith("fecha de arranque");
  });
});

describe("an empty text", () => {
  it("says «Escribe o pulsa / para insertar…»", () => {
    renderEditor();
    expect(screen.getByText("Escribe o pulsa / para insertar…")).toBeTruthy();
  });
});

describe("what an assistant wrote", () => {
  it("shows marked as its, is not part of the text until accepted, and is put in when it is", () => {
    const onSettled = vi.fn();
    const { editor, onChange } = renderEditor({
      onSuggestionSettled: onSettled,
      suggestion: { author: "Vega", text: "Llevar la llave el martes." },
      value: "Para el comité",
    });
    const block = screen.getByRole("group", { name: "Escrito por Vega" });
    expect(block.textContent).toContain("Llevar la llave el martes.");
    expect(text(editor)).toBe("Para el comité");
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(within(block).getByRole("button", { name: "Aceptar" }));
    expect(text(editor)).toBe("Para el comité\nLlevar la llave el martes.");
    expect(onChange).toHaveBeenLastCalledWith("Para el comité\nLlevar la llave el martes.");
    expect(onSettled).toHaveBeenCalledWith(true);
    expect(screen.queryByRole("group", { name: "Escrito por Vega" })).toBeNull();
  });

  it("is gone with «Descartar», and nothing of it reaches the text", () => {
    const onSettled = vi.fn();
    const { editor, onChange } = renderEditor({
      onSuggestionSettled: onSettled,
      suggestion: { author: "Vega", text: "Otra idea." },
      value: "Para el comité",
    });
    fireEvent.click(screen.getByRole("button", { name: "Descartar" }));
    expect(screen.queryByRole("group", { name: "Escrito por Vega" })).toBeNull();
    expect(text(editor)).toBe("Para el comité");
    expect(onChange).not.toHaveBeenCalled();
    expect(onSettled).toHaveBeenCalledWith(false);
  });
});

describe("the ceiling", () => {
  it("takes nothing past it, and says so", () => {
    const { editor } = renderEditor({ maxChars: 10, value: "123456789" });
    focusAt(editor);
    typeText(editor, "ñ");
    typeText(editor, "x");
    expect(text(editor)).toBe("123456789ñ");
    expect(
      screen.getByText("Has llegado al máximo de 10 caracteres: lo que sigue no se escribe."),
    ).toBeTruthy();
  });
});

describe("read only", () => {
  it("cannot be changed", () => {
    const { editor } = renderEditor({ readOnly: true, value: "Soy **Atlas**." });
    expect(editor.contentDOM.getAttribute("contenteditable")).toBe("false");
    expect(editor.state.readOnly).toBe(true);
  });
});
