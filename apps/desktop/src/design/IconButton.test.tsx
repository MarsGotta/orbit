import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { useFeedback } from "./feedback";
import { IconButton } from "./IconButton";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function tip(text: string): HTMLElement | null {
  return screen.queryByText(text, { selector: "[data-tooltip]" });
}

function CopyAnswer({ copy }: { copy: () => Promise<void> }) {
  const { feedback, track } = useFeedback();
  return (
    <IconButton
      doneLabel="Copiado"
      feedback={feedback}
      icon="copy"
      label="Copiar"
      onClick={() => track(copy())}
      size={28}
    />
  );
}

function ListenAnswer() {
  const [playing, setPlaying] = useState(false);
  return (
    <IconButton
      active={playing ? { icon: "stop", label: "Parar" } : undefined}
      icon="listen"
      label="Escuchar"
      onClick={() => setPlaying(!playing)}
      size={28}
    />
  );
}

it("is named by its label, the same text its tooltip shows", () => {
  render(<IconButton icon="gear" label="Ajustes de Phobos" size={30} tip="below" />);
  const button = screen.getByRole("button", { name: "Ajustes de Phobos" });
  fireEvent.mouseEnter(button);
  expect(tip("Ajustes de Phobos")?.dataset.placement).toBe("below");
});

it("Copiar turns to ✓ and «Copiado» for 1.8 s once the copy is done, and says so", async () => {
  vi.useFakeTimers();
  render(<CopyAnswer copy={() => Promise.resolve()} />);
  const button = screen.getByRole("button", { name: "Copiar" });
  await act(async () => fireEvent.click(button));
  expect(screen.getByRole("button", { name: "Copiado" })).toBe(button);
  expect(button.dataset.echo).toBe("done");
  expect(screen.getByRole("status").textContent).toBe("Copiado");
  fireEvent.mouseEnter(button);
  expect(tip("Copiado")).toBeTruthy();
  act(() => vi.advanceTimersByTime(1_799));
  expect(screen.getByRole("button", { name: "Copiado" })).toBe(button);
  act(() => vi.advanceTimersByTime(1));
  expect(screen.getByRole("button", { name: "Copiar" })).toBe(button);
  expect(screen.getByRole("status").textContent).toBe("");
});

it("a copy that fails does not say «Copiado»", async () => {
  render(<CopyAnswer copy={() => Promise.reject(new Error("sin portapapeles"))} />);
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copiar" })));
  expect(screen.getByRole("button", { name: "Copiar" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Copiado" })).toBeNull();
});

it("Escuchar says «Parar» while it plays, and goes back when it stops", () => {
  render(<ListenAnswer />);
  fireEvent.click(screen.getByRole("button", { name: "Escuchar" }));
  const button = screen.getByRole("button", { name: "Parar" });
  expect(button.dataset.echo).toBe("active");
  fireEvent.mouseEnter(button);
  expect(tip("Parar")).toBeTruthy();
  fireEvent.click(button);
  expect(screen.getByRole("button", { name: "Escuchar" })).toBe(button);
});

it("shows no tooltip while the menu it opens is open", () => {
  render(
    <IconButton
      aria-expanded
      aria-haspopup="dialog"
      icon="edit"
      label="Editar nombre y descripción"
      size={30}
      tip="below"
    />,
  );
  fireEvent.mouseEnter(screen.getByRole("button", { name: "Editar nombre y descripción" }));
  expect(tip("Editar nombre y descripción")).toBeNull();
});

it("can be named apart from its tooltip, where the name needs more", () => {
  render(
    <IconButton
      aria-label="Editar nombre y descripción de Phobos"
      icon="edit"
      label="Editar nombre y descripción"
      size={30}
    />,
  );
  const button = screen.getByRole("button", { name: "Editar nombre y descripción de Phobos" });
  fireEvent.mouseEnter(button);
  expect(tip("Editar nombre y descripción")).toBeTruthy();
});
