import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EditMenu } from "./EditMenu";

const WINDOW_HEIGHT = 900;
let anchorTop = 40;
const realHeight = window.innerHeight;

function rect(top: number, height: number, left = 300, width = 30): DOMRect {
  return DOMRect.fromRect({ x: left, y: top, width, height });
}

beforeEach(() => {
  anchorTop = 40;
  window.innerHeight = WINDOW_HEIGHT;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("role") === "dialog") return rect(0, 420, 0, 400);
    if (this.dataset.anchor !== undefined) return rect(anchorTop, 30);
    return rect(0, 0);
  });
});

afterEach(() => {
  cleanup();
  window.innerHeight = realHeight;
});

function Rename({ onSave = () => undefined }: { onSave?: (name: string) => void }) {
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("Phobos");
  const [about, setAbout] = useState("Código, scripts y datos");
  const valid = name.trim() !== "" && name !== "Phobos";
  return (
    <>
      <p>Fuera del menú</p>
      <button
        aria-expanded={open}
        data-anchor=""
        onClick={() => setOpen(!open)}
        ref={setAnchor}
        type="button"
      >
        Editar a Phobos
      </button>
      {open && (
        <EditMenu
          anchor={anchor}
          canSave={valid}
          error={name === "Deimos" ? "Ya tienes otro asistente que se llama así." : undefined}
          onClose={() => setOpen(false)}
          onSave={() => {
            onSave(name);
            setOpen(false);
          }}
          title="Editar a Phobos"
          width={400}
        >
          <label>
            Nombre
            <input onChange={(event) => setName(event.target.value)} value={name} />
          </label>
          <label>
            Descripción
            <textarea onChange={(event) => setAbout(event.target.value)} value={about} />
          </label>
        </EditMenu>
      )}
    </>
  );
}

function openMenu() {
  fireEvent.click(screen.getByRole("button", { name: "Editar a Phobos" }));
  return screen.getByRole("dialog", { name: "Editar a Phobos" });
}

it("is a window named by its title, 400 points wide, with the focus on its first field", () => {
  render(<Rename />);
  const menu = openMenu();
  expect(menu.getAttribute("aria-labelledby")).toBeTruthy();
  expect(menu.style.width).toBe("400px");
  expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Nombre" }));
});

it("keeps its main button off until there is a valid change", () => {
  render(<Rename />);
  openMenu();
  const save = screen.getByRole("button", { name: "Guardar" }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  fireEvent.change(screen.getByRole("textbox", { name: "Nombre" }), { target: { value: "" } });
  expect(save.disabled).toBe(true);
  fireEvent.change(screen.getByRole("textbox", { name: "Nombre" }), { target: { value: "Ío" } });
  expect(save.disabled).toBe(false);
});

it("↵ in a one-line field saves, but not without a valid change, nor in a text of several lines", () => {
  const onSave = vi.fn();
  render(<Rename onSave={onSave} />);
  openMenu();
  const name = screen.getByRole("textbox", { name: "Nombre" });
  fireEvent.keyDown(name, { key: "Enter" });
  expect(onSave).not.toHaveBeenCalled();
  fireEvent.change(name, { target: { value: "Ío" } });
  fireEvent.keyDown(screen.getByRole("textbox", { name: "Descripción" }), { key: "Enter" });
  expect(onSave).not.toHaveBeenCalled();
  fireEvent.keyDown(name, { key: "Enter" });
  expect(onSave).toHaveBeenCalledWith("Ío");
});

it("Esc closes it without saving and gives the focus back to the control that opened it", () => {
  const onSave = vi.fn();
  render(<Rename onSave={onSave} />);
  openMenu();
  const name = screen.getByRole("textbox", { name: "Nombre" });
  fireEvent.change(name, { target: { value: "Ío" } });
  fireEvent.keyDown(name, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(onSave).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Editar a Phobos" }));
});

it("a press outside closes it without saving; one inside, or on its control, does not", () => {
  const onSave = vi.fn();
  render(<Rename onSave={onSave} />);
  openMenu();
  fireEvent.mouseDown(screen.getByRole("textbox", { name: "Nombre" }));
  fireEvent.mouseDown(screen.getByRole("button", { name: "Editar a Phobos" }));
  expect(screen.getByRole("dialog")).toBeTruthy();
  fireEvent.mouseDown(screen.getByText("Fuera del menú"));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(onSave).not.toHaveBeenCalled();
});

it("says what stops the change above its buttons", () => {
  render(<Rename />);
  openMenu();
  fireEvent.change(screen.getByRole("textbox", { name: "Nombre" }), {
    target: { value: "Deimos" },
  });
  expect(screen.getByRole("alert").textContent).toBe("Ya tienes otro asistente que se llama así.");
});

it("opens 6 points below its control when it fits", () => {
  render(<Rename />);
  const menu = openMenu();
  expect(menu.dataset.placement).toBe("below");
  expect(menu.style.top).toBe("76px");
});

it("opens above its control when there is no room below", () => {
  anchorTop = 600;
  render(<Rename />);
  const menu = openMenu();
  expect(menu.dataset.placement).toBe("above");
  // Its bottom edge 6 points above the control's top, so it grows upwards.
  expect(menu.style.bottom).toBe(`${WINDOW_HEIGHT - 600 + 6}px`);
});
