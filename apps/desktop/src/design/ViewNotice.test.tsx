import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ViewNotice } from "./ViewNotice";

afterEach(cleanup);

it("says what is wrong, why, and offers the button that fixes it", () => {
  const onAction = vi.fn();
  render(
    <ViewNotice
      actionLabel="Iniciar sesión en Claude"
      onAction={onAction}
      title="Inicia sesión otra vez en Claude"
    >
      Tus conversaciones con Ares siguen aquí; solo falta tu sesión para que pueda responder.
    </ViewNotice>,
  );
  const notice = screen.getByRole("status");
  expect(notice.textContent).toContain("Inicia sesión otra vez en Claude");
  expect(notice.textContent).toContain("solo falta tu sesión");
  fireEvent.click(screen.getByRole("button", { name: "Iniciar sesión en Claude" }));
  expect(onAction).toHaveBeenCalledOnce();
});

it("while it waits, its button says «Esperando…» and cannot be pressed again", () => {
  const onAction = vi.fn();
  render(
    <ViewNotice
      actionLabel="Iniciar sesión en Claude"
      onAction={onAction}
      title="Esperando a que inicies sesión en el navegador…"
      waiting
    >
      Se ha abierto tu navegador. Inicia sesión con tu cuenta de Claude y vuelve aquí.
    </ViewNotice>,
  );
  const button = screen.getByRole("button", { name: "Esperando…" });
  expect(button.getAttribute("aria-busy")).toBe("true");
  fireEvent.click(button);
  expect(onAction).not.toHaveBeenCalled();
});
