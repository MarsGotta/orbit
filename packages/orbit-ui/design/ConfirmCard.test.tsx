import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { ConfirmCard } from "./ConfirmCard";

afterEach(cleanup);

const REDO = "Rehacer transcripción";

function Redo({ onConfirm }: { onConfirm: () => void }) {
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button aria-expanded={open} onClick={() => setOpen(true)} ref={setAnchor} type="button">
        {REDO}
      </button>
      {open && (
        <ConfirmCard
          anchor={anchor}
          confirmLabel={REDO}
          onClose={() => setOpen(false)}
          onConfirm={() => {
            onConfirm();
            setOpen(false);
          }}
          title="¿Rehacer la transcripción?"
        >
          Tarda unos 3 minutos y puedes seguir con lo tuyo.
        </ConfirmCard>
      )}
    </>
  );
}

it("asks beside its control, in a window named by its question, with the focus on «Cancelar»", () => {
  render(<Redo onConfirm={() => undefined} />);
  fireEvent.click(screen.getByRole("button", { name: REDO }));
  const card = screen.getByRole("dialog", { name: "¿Rehacer la transcripción?" });
  expect(card.textContent).toContain("Tarda unos 3 minutos");
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancelar" }));
});

it("its main button repeats the verb that opened it, and only that button acts", () => {
  const onConfirm = vi.fn();
  render(<Redo onConfirm={onConfirm} />);
  fireEvent.click(screen.getByRole("button", { name: REDO }));
  const buttons = screen.getAllByRole("button", { name: REDO });
  expect(buttons).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(onConfirm).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: REDO }));
  const main = screen.getAllByRole("button", { name: REDO })[1];
  if (main) fireEvent.click(main);
  expect(onConfirm).toHaveBeenCalledOnce();
});

it("Esc closes it without acting and the focus goes back to its control", () => {
  const onConfirm = vi.fn();
  render(<Redo onConfirm={onConfirm} />);
  const opener = screen.getByRole("button", { name: REDO });
  fireEvent.click(opener);
  fireEvent.keyDown(screen.getByRole("button", { name: "Cancelar" }), { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(onConfirm).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(opener);
});

it("is 350 points wide, within the design's 340 to 380", () => {
  render(<Redo onConfirm={() => undefined} />);
  fireEvent.click(screen.getByRole("button", { name: REDO }));
  expect(screen.getByRole("dialog").style.width).toBe("350px");
});
