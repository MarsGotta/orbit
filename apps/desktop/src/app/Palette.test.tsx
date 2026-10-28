import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { ErrorOrigin } from "../state/orbit-store";
import type { ViewId } from "../state/ui";
import {
  askAssistant,
  matchCommands,
  paletteCommands,
  type PaletteActions,
} from "./palette-commands";
import { Palette } from "./Palette";

afterEach(cleanup);

function actions(nextMeeting: string | null = "Revisión del plan de piloto"): PaletteActions & {
  sent: [AppCommand, ErrorOrigin | undefined][];
  went: ViewId[];
  meetings: number;
} {
  const record = {
    sent: [] as [AppCommand, ErrorOrigin | undefined][],
    went: [] as ViewId[],
    meetings: 0,
    navigate: (view: ViewId) => record.went.push(view),
    send: (command: AppCommand, origin?: ErrorOrigin) => record.sent.push([command, origin]),
    openMeeting: () => {
      record.meetings += 1;
    },
    nextMeeting,
  };
  return record;
}

function Opened({ deps }: { deps: PaletteActions }) {
  const [open, setOpen] = useState(true);
  return (
    <Palette
      commands={paletteCommands(deps)}
      onAsk={(text) => askAssistant(deps, text)}
      onOpenChange={setOpen}
      open={open}
    />
  );
}

const titles = (options: HTMLElement[]) =>
  options.map((option) => option.querySelector("[data-title]")?.textContent);

describe("what the palette offers", () => {
  it("groups the actions, the views and privacy, each with its subtitle and shortcut", () => {
    render(<Opened deps={actions()} />);
    const list = screen.getByRole("listbox");
    expect(
      within(list)
        .getAllByRole("group")
        .map((group) => group.getAttribute("aria-label")),
    ).toEqual(["Acciones", "Ir a", "Privacidad"]);
    expect(titles(within(list).getAllByRole("option"))).toEqual([
      "Grabar una reunión",
      "Llamar a Deimos",
      "Mandar una nota de voz",
      "Preparar la siguiente reunión",
      "Hoy",
      "Calendario",
      "Reuniones",
      "Notas",
      "Asistente",
      "Ajustes",
      "Parar todo",
    ]);
    const record = screen.getByRole("option", { name: /Grabar una reunión/ });
    expect(record.textContent).toContain("Elige qué audio y empieza");
    expect(record.textContent).toContain("⌃⌥M");
    expect(screen.getByRole("option", { name: /Preparar la siguiente/ }).textContent).toContain(
      "Revisión del plan de piloto",
    );
    expect(screen.getByRole("option", { name: /Parar todo/ }).textContent).toContain("⌃⌥Esc");
    const footer = screen.getByText("↑↓ moverte").parentElement;
    expect(footer?.textContent).toBe("↑↓ moverte↵ abriresc cerrar");
  });

  it("offers to prepare the next meeting only when there is one", () => {
    render(<Opened deps={actions(null)} />);
    expect(screen.queryByRole("option", { name: /Preparar la siguiente/ })).toBeNull();
  });

  it("each command does what it says", () => {
    const deps = actions();
    const byTitle = Object.fromEntries(
      paletteCommands(deps).map((command) => [command.title, command]),
    );
    byTitle["Grabar una reunión"]?.run();
    byTitle["Llamar a Deimos"]?.run();
    byTitle["Reuniones"]?.run();
    byTitle["Parar todo"]?.run();
    expect(deps.meetings).toBe(1);
    expect(deps.went).toEqual(["assistant", "notes"]);
    expect(deps.sent).toEqual([
      [{ command: "call_start" }, "assistant"],
      [{ command: "privacy_stop" }, undefined],
    ]);
  });
});

describe("writing filters", () => {
  it("without accents or capitals", () => {
    const commands = paletteCommands(actions());
    expect(matchCommands(commands, "REUNION").map((command) => command.title)).toEqual([
      "Grabar una reunión",
      "Preparar la siguiente reunión",
      "Reuniones",
    ]);
    expect(matchCommands(commands, "  calendário ").map((command) => command.title)).toEqual([
      "Calendario",
    ]);
    expect(matchCommands(commands, "pantalla").map((command) => command.title)).toEqual([
      "Parar todo",
    ]);
  });

  it("keeps the matches selectable, with asking last", () => {
    render(<Opened deps={actions()} />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "ajus" } });
    const options = screen.getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual([
      "Ajustes",
      "Preguntar a Deimos: «ajus»↵",
    ]);
    expect(options[0]?.getAttribute("aria-selected")).toBe("true");
  });
});

describe("what matches nothing goes to Deimos", () => {
  it("the first row asks it, and ↵ sends the text as a new message", () => {
    const deps = actions();
    render(<Opened deps={deps} />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "ribera" } });
    const [ask] = screen.getAllByRole("option");
    expect(ask?.textContent).toBe("Preguntar a Deimos: «ribera»↵");
    expect(ask?.getAttribute("aria-selected")).toBe("true");
    expect(
      screen.getByText("Ninguna orden se llama así. Pulsa ↵ y se lo preguntamos a Deimos."),
    ).toBeTruthy();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(deps.went).toEqual(["assistant"]);
    expect(deps.sent).toEqual([[{ command: "chat", text: "ribera", primary: true }, "assistant"]]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("the keyboard and the pointer", () => {
  it("↑ ↓ move one selection, ↵ runs it and closes", () => {
    const deps = actions();
    render(<Opened deps={deps} />);
    const input = screen.getByRole("combobox");
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(screen.getByRole("option", { selected: true }).textContent).toContain(
      "Grabar una reunión",
    );
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(screen.getAllByRole("option", { selected: true })).toHaveLength(1);
    expect(input.getAttribute("aria-activedescendant")).toBe(
      screen.getByRole("option", { selected: true }).id,
    );
    fireEvent.keyDown(input, { key: "Enter" });
    expect(deps.went).toEqual(["assistant"]);
    expect(deps.sent).toEqual([]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Esc closes it, and so does a press outside it", () => {
    const { unmount } = render(<Opened deps={actions()} />);
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    unmount();
    render(<Opened deps={actions()} />);
    const dialog = screen.getByRole("dialog");
    const backdrop = dialog.parentElement;
    if (!backdrop) throw new Error("the palette must sit on its backdrop");
    fireEvent.mouseDown(backdrop);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("⌘⇧␣ opens and closes it from anywhere in the window", () => {
    const onOpenChange = vi.fn();
    const view = render(
      <Palette commands={[]} onAsk={vi.fn()} onOpenChange={onOpenChange} open={false} />,
    );
    fireEvent.keyDown(window, { code: "Space", metaKey: true, shiftKey: true });
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
    view.rerender(<Palette commands={[]} onAsk={vi.fn()} onOpenChange={onOpenChange} open />);
    fireEvent.keyDown(window, { code: "Space", metaKey: true, shiftKey: true });
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
  });
});
