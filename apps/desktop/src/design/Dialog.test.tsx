import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { Dialog } from "./Dialog";

afterEach(cleanup);

function Launcher({ onClose = () => undefined }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)} type="button">
        Nueva grabación
      </button>
      {open && (
        <Dialog
          onClose={() => {
            onClose();
            setOpen(false);
          }}
          subtitle="Elige qué audio grabar"
          title="Nueva grabación"
        >
          <label htmlFor="meeting-title">Título</label>
          <input id="meeting-title" />
        </Dialog>
      )}
    </>
  );
}

it("is a modal dialog named by its title, with focus on its first field", () => {
  render(<Launcher />);
  fireEvent.click(screen.getByRole("button", { name: "Nueva grabación" }));
  const dialog = screen.getByRole("dialog", { name: "Nueva grabación" });
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(document.activeElement).toBe(screen.getByLabelText("Título"));
});

it("closes with Esc and gives the focus back to the button that opened it", () => {
  const onClose = vi.fn();
  render(<Launcher onClose={onClose} />);
  const opener = screen.getByRole("button", { name: "Nueva grabación" });
  opener.focus();
  fireEvent.click(opener);
  fireEvent.keyDown(screen.getByLabelText("Título"), { key: "Escape" });
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(opener);
});

it("closes with a press outside it, not with one inside", () => {
  const onClose = vi.fn();
  render(<Launcher onClose={onClose} />);
  fireEvent.click(screen.getByRole("button", { name: "Nueva grabación" }));
  fireEvent.mouseDown(screen.getByLabelText("Título"));
  expect(onClose).not.toHaveBeenCalled();
  const dialog = screen.getByRole("dialog");
  if (!dialog.parentElement) throw new Error("The dialog sits on its scrim");
  fireEvent.mouseDown(dialog.parentElement);
  expect(onClose).toHaveBeenCalledOnce();
});

it("the × closes it too", () => {
  const onClose = vi.fn();
  render(<Launcher onClose={onClose} />);
  fireEvent.click(screen.getByRole("button", { name: "Nueva grabación" }));
  fireEvent.click(screen.getByRole("button", { name: "Cerrar" }));
  expect(onClose).toHaveBeenCalledOnce();
});
