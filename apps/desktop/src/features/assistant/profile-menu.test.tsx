import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { assistantSnapshot, DEIMOS, NOW, PHOBOS, renderAssistant } from "./test-assistants";

// The pencil (annex proposal 27): name, description, colour and primary,
// saved in the hub; Esc and a press outside save nothing; ↵ saves.

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  window.location.hash = "#/assistant";
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

async function openPencil(agent = PHOBOS, reply?: Parameters<typeof renderAssistant>[1]) {
  const rendered = renderAssistant(
    assistantSnapshot({ session_id: "s-1", agent_id: agent.id }),
    reply,
  );
  await userEvent.click(
    screen.getByRole("button", { name: `Editar nombre y descripción de ${agent.name}` }),
  );
  const menu = await screen.findByRole("dialog", { name: `Editar a ${agent.name}` });
  return { ...rendered, menu };
}

describe("the pencil", () => {
  it("opens with the assistant as it is, its service and how it is called", async () => {
    const { menu } = await openPencil();
    expect(
      within(menu).getByText("Funciona con Codex, con tu suscripción de ChatGPT"),
    ).toBeTruthy();
    expect((within(menu).getByRole("textbox", { name: "Nombre" }) as HTMLInputElement).value).toBe(
      "Phobos",
    );
    expect(within(menu).getByText("Le llamas con «Oye Phobos».")).toBeTruthy();
    expect(within(menu).getByText("23 / 90")).toBeTruthy();
    const colours = within(menu).getByRole("radiogroup", { name: "Color" });
    expect(within(colours).getByRole("radio", { name: "Azul" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    expect(within(menu).getByText("Ahora lo es Deimos.")).toBeTruthy();
    // Nothing changed yet: «Guardar» waits.
    expect(
      (within(menu).getByRole("button", { name: "Guardar" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("refuses an empty name and one another assistant has, ignoring case and accents", async () => {
    const { menu, calls } = await openPencil();
    const name = within(menu).getByRole("textbox", { name: "Nombre" });
    await userEvent.clear(name);
    expect(within(menu).getByText("Ponle un nombre: es como le llamas.")).toBeTruthy();
    await userEvent.type(name, "áres");
    expect(within(menu).getByText("Ya tienes otro asistente que se llama así.")).toBeTruthy();
    expect(name.getAttribute("aria-invalid")).toBe("true");
    await userEvent.type(name, "{Enter}");
    expect(
      (within(menu).getByRole("button", { name: "Guardar" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(calls).toEqual([]);
  });

  it("saves nothing on Esc nor on a press outside", async () => {
    const { menu, calls } = await openPencil();
    await userEvent.type(within(menu).getByRole("textbox", { name: "Nombre" }), "s");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(
      screen.getByRole("button", { name: "Editar nombre y descripción de Phobos" }),
    );
    const again = await screen.findByRole("dialog", { name: "Editar a Phobos" });
    expect((within(again).getByRole("textbox", { name: "Nombre" }) as HTMLInputElement).value).toBe(
      "Phobos",
    );
    await userEvent.type(within(again).getByRole("textbox", { name: "Nombre" }), "s");
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls).toEqual([]);
  });

  it("saves the four on ↵ in the name, says «✓ Guardado» for 2.5 s, and closes", async () => {
    const { menu, calls } = await openPencil();
    const name = within(menu).getByRole("textbox", { name: "Nombre" });
    await userEvent.clear(name);
    await userEvent.type(name, "Eris");
    await userEvent.click(within(menu).getByRole("radio", { name: "Ámbar" }));
    await userEvent.click(within(menu).getByRole("switch", { name: /Asistente principal/ }));
    expect(
      within(menu).getByText(
        "Responderá a «Oye…» sin nombre y en el panel flotante, en lugar de Deimos.",
      ),
    ).toBeTruthy();
    await userEvent.type(name, "{Enter}");
    expect(calls).toEqual([
      {
        call: "profile",
        agent_id: PHOBOS.id,
        name: "Eris",
        description: "Código, scripts y datos",
        color: "c4",
        primary: true,
      },
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const saved = within(screen.getByTestId("top-bar")).getByRole("status");
    expect(saved.textContent).toBe("✓ Guardado");
    // 2.5 s on the real clock, then it goes.
    await waitFor(() => expect(saved.textContent).toBe(""), { timeout: 4_000 });
  });

  it("says the hub's refusal and keeps what was written", async () => {
    const { menu } = await openPencil(PHOBOS, {
      agent: () => ({
        outcome: "refused",
        refusal: {
          code: "agent_name_taken",
          status: 409,
          numbers: { field: "name", agent_id: "deimos" },
        },
      }),
    });
    const name = within(menu).getByRole("textbox", { name: "Nombre" });
    await userEvent.clear(name);
    await userEvent.type(name, "Déimos 2{Enter}");
    expect(await within(menu).findByRole("alert")).toBeTruthy();
    expect((name as HTMLInputElement).value).toBe("Déimos 2");
  });

  it("on Deimos says it only changes how Orbit shows it, and a primary stays one", async () => {
    const { menu } = await openPencil(DEIMOS);
    expect(
      within(menu).getByText(
        "Esto solo cambia cómo lo ves en Orbit. Su personalidad y lo que puede usar los gestiona tu hub.",
      ),
    ).toBeTruthy();
    const primary = within(menu).getByRole("switch", { name: /Asistente principal/ });
    expect(primary.getAttribute("aria-checked")).toBe("true");
    expect((primary as HTMLButtonElement).disabled).toBe(true);
  });

  it("leads to the gear from «Más ajustes»", async () => {
    const { menu } = await openPencil();
    await userEvent.click(within(menu).getByRole("button", { name: "Más ajustes" }));
    expect(window.location.hash).toBe(`#/assistant/${PHOBOS.id}/settings`);
  });
});
