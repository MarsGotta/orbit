import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { MultiSelect, type MultiSelectOption } from "./MultiSelect";

afterEach(cleanup);

const DICTIONARIES: MultiSelectOption[] = [
  { id: "nubaria", name: "Nubaria", detail: "Revisión semanal y Comité · 24 palabras" },
  { id: "ribera", name: "Hoteles Ribera", detail: "Seguimiento del piloto · 11 palabras" },
  { id: "producto", name: "Producto", detail: "Ninguna serie · 8 palabras" },
];

function Dictionaries(props: { onCreate?: () => void; onOuterKey?: () => void }) {
  const [chosen, setChosen] = useState(["nubaria", "ribera"]);
  return (
    <div onKeyDown={props.onOuterKey}>
      <MultiSelect
        createLabel="+ Crear un diccionario nuevo"
        label="Se añade a · puedes elegir varios"
        onChange={setChosen}
        onCreate={props.onCreate ?? (() => undefined)}
        options={DICTIONARIES.map((option) =>
          chosen.includes(option.id) ? { ...option, tag: "esta reunión" } : option,
        )}
        placeholder="Elige uno o varios"
        selected={chosen}
      />
    </div>
  );
}

function trigger() {
  return screen.getByRole("button", { name: /Se añade a/ });
}

it("closed, it shows the chosen ones as chips and no list", () => {
  render(<Dictionaries />);
  expect(trigger().getAttribute("aria-expanded")).toBe("false");
  expect(trigger().textContent).toContain("Nubaria");
  expect(trigger().textContent).toContain("Hoteles Ribera");
  expect(trigger().textContent).not.toContain("Producto");
  expect(screen.queryByRole("group")).toBeNull();
});

it("open, it is a group of checkboxes with each one's line, the creation at the end, and the focus on the first", () => {
  render(<Dictionaries />);
  fireEvent.click(trigger());
  expect(trigger().getAttribute("aria-expanded")).toBe("true");
  const group = screen.getByRole("group", { name: "Se añade a · puedes elegir varios" });
  const boxes = screen.getAllByRole("checkbox");
  expect(boxes.map((box) => box.getAttribute("aria-checked"))).toEqual(["true", "true", "false"]);
  expect(group.textContent).toContain("Ninguna serie · 8 palabras");
  expect(boxes[0]?.textContent).toContain("esta reunión");
  expect(boxes[2]?.textContent).not.toContain("esta reunión");
  const buttons = group.querySelectorAll("button");
  expect(buttons[buttons.length - 1]?.textContent).toBe("+ Crear un diccionario nuevo");
  expect(document.activeElement).toBe(boxes[0]);
});

it("↑ ↓ move between the options and space marks one", async () => {
  const user = userEvent.setup();
  render(<Dictionaries />);
  await user.click(trigger());
  await user.keyboard("{ArrowDown}{ArrowDown}");
  const producto = screen.getByRole("checkbox", { name: /Producto/ });
  expect(document.activeElement).toBe(producto);
  await user.keyboard(" ");
  expect(producto.getAttribute("aria-checked")).toBe("true");
  expect(trigger().textContent).toContain("Producto");
  await user.keyboard("{ArrowDown}");
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "+ Crear un diccionario nuevo" }),
  );
  await user.keyboard("{ArrowUp}{ArrowUp}");
  await user.keyboard(" ");
  expect(
    screen.getByRole("checkbox", { name: /Hoteles Ribera/ }).getAttribute("aria-checked"),
  ).toBe("false");
});

it("with none chosen, it says what to do", () => {
  render(<Dictionaries />);
  fireEvent.click(trigger());
  fireEvent.click(screen.getByRole("checkbox", { name: /Nubaria/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /Hoteles Ribera/ }));
  expect(trigger().textContent).toContain("Elige uno o varios");
});

it("Esc closes the list only, and the focus goes back to it", () => {
  const onOuterKey = vi.fn();
  render(<Dictionaries onOuterKey={onOuterKey} />);
  fireEvent.click(trigger());
  fireEvent.keyDown(screen.getByRole("checkbox", { name: /Nubaria/ }), { key: "Escape" });
  expect(screen.queryByRole("group")).toBeNull();
  expect(document.activeElement).toBe(trigger());
  // The menu it sits in stays open.
  expect(onOuterKey).not.toHaveBeenCalled();
});

it("«+ Crear …» asks for a new one", () => {
  const onCreate = vi.fn();
  render(<Dictionaries onCreate={onCreate} />);
  fireEvent.click(trigger());
  fireEvent.click(screen.getByRole("button", { name: "+ Crear un diccionario nuevo" }));
  expect(onCreate).toHaveBeenCalledOnce();
});
