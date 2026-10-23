import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import type { AppView } from "../../state/app-view";
import { OnboardingView } from "./OnboardingView";

afterEach(cleanup);

const unasked: PermissionSnapshot = {
  microphone: "not_determined",
  screen: "not_determined",
  accessibility: "not_granted",
  calendars: "not_determined",
};

function snapshot(paired = false, permissions = unasked): AppView {
  return {
    settings: { hub: { url: null, paired, device_name: null }, permissions },
  } as unknown as AppView;
}

function show(props: Partial<Parameters<typeof OnboardingView>[0]> = {}) {
  const sent: AppCommand[] = [];
  const onComplete = vi.fn();
  const send = vi.fn(async (command: AppCommand) => {
    sent.push(command);
  });
  const view = render(
    <OnboardingView
      error={null}
      onComplete={onComplete}
      send={send}
      snapshot={snapshot()}
      {...props}
    />,
  );
  return { ...view, sent, send, onComplete };
}

const press = (name: string) => fireEvent.click(screen.getByRole("button", { name }));
const progress = () => screen.getByText(/^Paso \d de 5 · /).textContent;

describe("the first run says where it is", () => {
  it("«Paso N de 5 · {paso}» and five bars, the current one marked and the future ones closed", () => {
    show();
    expect(progress()).toBe("Paso 1 de 5 · Bienvenida");
    const bars = within(screen.getByRole("list", { name: "Pasos" })).getAllByRole("button");
    expect(bars.map((bar) => bar.getAttribute("aria-label"))).toEqual([
      "Paso 1: Bienvenida",
      "Paso 2: Tu hub",
      "Paso 3: Permisos",
      "Paso 4: Asistentes",
      "Paso 5: Listo",
    ]);
    expect(bars[0]?.getAttribute("aria-current")).toBe("step");
    expect(bars.slice(1).every((bar) => (bar as HTMLButtonElement).disabled)).toBe(true);
  });

  it("the welcome says what Orbit does, the steps to come and that everything stops with ⌃⌥Esc", () => {
    show();
    expect(
      screen.getByText(
        "En un par de minutos lo dejas listo: conectas este Mac con tu hub, eliges qué puede usar Orbit y con qué asistentes vas a hablar.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("Graba tus reuniones y olvídate de tomar notas")).toBeTruthy();
    expect(screen.getByText("Tu día de un vistazo")).toBeTruthy();
    expect(screen.getByText("Asistentes que te echan una mano")).toBeTruthy();
    const plan = screen.getByRole("list", { name: "Los pasos que vienen" });
    expect(
      within(plan)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual([
      "2Tu hubConecta este Mac con el hub donde se guardan tus reuniones.",
      "3PermisosMicrófono, pantalla, accesibilidad y calendarios.",
      "4AsistentesDeimos viene con tu hub; los demás, con tu suscripción.",
      "5ListoTres atajos para empezar.",
    ]);
    expect(
      screen.getByText("Tú decides qué ve y oye Orbit. Lo paras todo al instante con ⌃⌥Esc."),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Atrás" })).toBeNull();
  });

  it("«Empezar» goes on, «Atrás» comes back, and a bar returns to a step already seen", () => {
    show();
    press("Empezar");
    expect(progress()).toBe("Paso 2 de 5 · Tu hub");
    press("Atrás");
    expect(progress()).toBe("Paso 1 de 5 · Bienvenida");
    press("Empezar");
    press("Emparejar después");
    expect(progress()).toBe("Paso 3 de 5 · Permisos");
    press("Continuar");
    expect(progress()).toBe("Paso 4 de 5 · Asistentes");
    // A step not reached yet cannot be jumped to.
    expect(
      (screen.getByRole("button", { name: "Paso 5: Listo" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    press("Paso 1: Bienvenida");
    expect(progress()).toBe("Paso 1 de 5 · Bienvenida");
  });

  it("«Atrás» on every step after the first, the last one too", () => {
    show();
    press("Empezar");
    press("Emparejar después");
    press("Continuar");
    press("Continuar");
    expect(progress()).toBe("Paso 5 de 5 · Listo");
    press("Atrás");
    expect(progress()).toBe("Paso 4 de 5 · Asistentes");
  });

  it("«Configurarlo más tarde» leaves the whole first run from any step before the last", () => {
    const { sent, onComplete } = show({ startAtPair: true });
    press("Configurarlo más tarde");
    expect(sent).toEqual([{ command: "onboarding_done" }]);
    expect(onComplete).toHaveBeenCalledOnce();
  });
});

describe("pairing with the hub", () => {
  it("asks for the address, the six-digit code and this Mac's name", () => {
    show({ startAtPair: true });
    expect(screen.getByRole("heading", { name: "Conecta este Mac con tu hub" })).toBeTruthy();
    expect(
      screen.getByText(
        "El hub es donde se guardan tus reuniones y notas, y donde vive Deimos. Quien lo gestiona te da la dirección y un código.",
      ),
    ).toBeTruthy();
    const code = screen.getByLabelText<HTMLInputElement>("Código de emparejamiento");
    expect(code.getAttribute("inputmode")).toBe("numeric");
    expect(code.maxLength).toBe(6);
    expect(screen.getByText("Sirve una sola vez y caduca a los 5 minutos.").id).toBe(
      code.getAttribute("aria-describedby"),
    );
    fireEvent.change(code, { target: { value: "48a-29 13" } });
    expect(code.value).toBe("482913");
    expect(screen.getByLabelText("Nombre de este Mac")).toBeTruthy();
  });

  it("«Emparejar» says «Emparejando…» and then which hub this Mac is paired with", async () => {
    let settle: () => void = () => undefined;
    const send = vi.fn(
      (command: AppCommand) =>
        new Promise<void>((resolve) => {
          if (command.command === "pair") settle = resolve;
          else resolve();
        }),
    );
    show({ send, startAtPair: true });
    fireEvent.change(screen.getByLabelText("Dirección del hub"), {
      target: { value: "https://hub.nubaria.example/" },
    });
    fireEvent.change(screen.getByLabelText("Código de emparejamiento"), {
      target: { value: "482913" },
    });
    fireEvent.change(screen.getByLabelText("Nombre de este Mac"), {
      target: { value: "Mac de Lucía" },
    });
    press("Emparejar");
    expect(send).toHaveBeenCalledWith({
      command: "pair",
      hub_url: "https://hub.nubaria.example/",
      code: "482913",
      device_name: "Mac de Lucía",
    });
    expect(screen.getByRole("button", { name: "Emparejando…" })).toBeTruthy();
    await act(async () => settle());
    expect(screen.getByRole("status").textContent).toBe(
      "✓ Mac emparejado con hub.nubaria.example. Deimos ya está listo.",
    );
    expect(screen.queryByRole("button", { name: "Emparejar después" })).toBeNull();
    press("Continuar");
    expect(progress()).toBe("Paso 3 de 5 · Permisos");
  });

  it("a code without six digits is not sent: the field says how many it has", () => {
    const { sent } = show({ startAtPair: true });
    const code = screen.getByLabelText<HTMLInputElement>("Código de emparejamiento");
    fireEvent.change(code, { target: { value: "48291" } });
    press("Emparejar");
    expect(sent.some((command) => command.command === "pair")).toBe(false);
    expect(code.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByText("Tiene 5 cifras; deben ser 6.").id).toBe(
      code.getAttribute("aria-describedby"),
    );
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toBe(
      "No se pudo emparejarEl código tiene 6 cifras y sirve una sola vez. Comprueba también la dirección del hub o pide un código nuevo a quien lo gestiona.",
    );
    // Typing again starts over.
    fireEvent.change(code, { target: { value: "482913" } });
    expect(code.getAttribute("aria-invalid")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("a refusal says why, stays on the step and lets the person try again", async () => {
    // The store's send answers whether the order was accepted.
    const view = show({ send: vi.fn(async () => false), startAtPair: true });
    fireEvent.change(screen.getByLabelText("Código de emparejamiento"), {
      target: { value: "482913" },
    });
    press("Emparejar");
    await act(async () => undefined);
    const failure =
      "No se pudo emparejar este Mac. Vuelve a intentarlo. Si se repite, comparte el código A1B2C3 con quien gestiona tu hub.";
    view.rerender(
      <OnboardingView
        error={failure}
        onComplete={view.onComplete}
        send={view.send}
        snapshot={snapshot()}
        startAtPair
      />,
    );
    expect(screen.getByRole("alert").textContent).toBe(`No se pudo emparejar${failure}`);
    expect(progress()).toBe("Paso 2 de 5 · Tu hub");
    expect(screen.getByRole("button", { name: "Emparejar" })).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("a Mac already paired says so and goes on", () => {
    show({
      snapshot: {
        settings: {
          hub: { url: "https://hub.example", paired: true, device_name: "Mac de Lucía" },
          permissions: unasked,
        },
      } as unknown as AppView,
      startAtPair: false,
    });
    press("Empezar");
    expect(screen.getByRole("status").textContent).toBe(
      "✓ Mac emparejado con hub.example. Deimos ya está listo.",
    );
    expect(screen.queryByRole("button", { name: "Emparejar" })).toBeNull();
    press("Continuar");
    expect(progress()).toBe("Paso 3 de 5 · Permisos");
  });
});

describe("the permissions", () => {
  it("«Pedir» waits for macOS and then says «✓ Concedido»", () => {
    const view = show();
    press("Empezar");
    press("Emparejar después");
    const microphone = () => screen.getByText("Micrófono").closest("li") as HTMLElement;
    expect(within(microphone()).getByText(/hablar con tus asistentes/)).toBeTruthy();
    fireEvent.click(within(microphone()).getByRole("button", { name: "Pedir" }));
    expect(view.sent).toContainEqual({ command: "permissions", request: ["microphone"] });
    expect(within(microphone()).getByText("Esperando a macOS…")).toBeTruthy();
    expect(within(microphone()).queryByRole("button")).toBeNull();
    view.rerender(
      <OnboardingView
        error={null}
        onComplete={view.onComplete}
        send={view.send}
        snapshot={snapshot(false, { ...unasked, microphone: "granted" })}
      />,
    );
    expect(within(microphone()).getByText("✓ Concedido")).toBeTruthy();
    expect(
      screen.getByText(
        "Puedes seguir sin alguno: Orbit te dirá qué falta justo cuando lo necesites.",
      ),
    ).toBeTruthy();
  });

  it("accessibility opens System Settings, since macOS does not ask for it", () => {
    const { sent } = show();
    press("Empezar");
    press("Emparejar después");
    const row = screen.getByText("Accesibilidad").closest("li") as HTMLElement;
    fireEvent.click(within(row).getByRole("button", { name: "Abrir Ajustes" }));
    expect(sent).toContainEqual({ command: "open_system_privacy", pane: "Accessibility" });
  });

  it("one refused says «Sin conceder» and waits for System Settings once opened", () => {
    const { sent } = show({ snapshot: snapshot(false, { ...unasked, calendars: "denied" }) });
    press("Empezar");
    press("Emparejar después");
    const row = () => screen.getByText("Calendarios").closest("li") as HTMLElement;
    expect(within(row()).getByText("Sin conceder")).toBeTruthy();
    fireEvent.click(within(row()).getByRole("button", { name: "Abrir Ajustes del Sistema" }));
    expect(sent).toContainEqual({ command: "open_system_privacy", pane: "Calendars" });
    expect(within(row()).getByText("Esperando a Ajustes del Sistema…")).toBeTruthy();
  });
});

describe("back from System Settings", () => {
  // The person opens System Settings for a permission and comes back to
  // Orbit without granting it: the row never waits for ever.
  const returnToOrbit = () =>
    act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });

  it("reads the permissions again and, still refused, offers its button again", async () => {
    const { sent } = show({ snapshot: snapshot(false, { ...unasked, calendars: "denied" }) });
    press("Empezar");
    press("Emparejar después");
    const row = () => screen.getByText("Calendarios").closest("li") as HTMLElement;
    fireEvent.click(within(row()).getByRole("button", { name: "Abrir Ajustes del Sistema" }));
    expect(within(row()).getByText("Esperando a Ajustes del Sistema…")).toBeTruthy();
    await returnToOrbit();
    expect(sent).toContainEqual({ command: "permissions", request: [] });
    expect(within(row()).queryByText("Esperando a Ajustes del Sistema…")).toBeNull();
    expect(within(row()).getByText("Sin conceder")).toBeTruthy();
    expect(within(row()).getByRole("button", { name: "Abrir Ajustes del Sistema" })).toBeTruthy();
  });

  it("accessibility not switched on offers «Abrir Ajustes» again", async () => {
    show();
    press("Empezar");
    press("Emparejar después");
    const row = () => screen.getByText("Accesibilidad").closest("li") as HTMLElement;
    fireEvent.click(within(row()).getByRole("button", { name: "Abrir Ajustes" }));
    expect(within(row()).getByText("Esperando a Ajustes del Sistema…")).toBeTruthy();
    await returnToOrbit();
    expect(within(row()).getByRole("button", { name: "Abrir Ajustes" })).toBeTruthy();
  });

  it("nothing is read again while no row waits for System Settings", async () => {
    const { sent } = show();
    press("Empezar");
    press("Emparejar después");
    await returnToOrbit();
    expect(sent).not.toContainEqual({ command: "permissions", request: [] });
  });
});

describe("all set", () => {
  it("shows the three shortcuts and starts using Orbit", () => {
    const { sent, onComplete } = show();
    press("Empezar");
    press("Emparejar después");
    press("Continuar");
    press("Continuar");
    expect(progress()).toBe("Paso 5 de 5 · Listo");
    expect(screen.getByRole("heading", { name: "Todo listo" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Configurarlo más tarde" })).toBeNull();
    expect(
      screen.getByRole("heading", { name: "Tres atajos que funcionan desde cualquier app" }),
    ).toBeTruthy();
    for (const keys of ["⌃⌥M", "⌃⌥␣", "⌃⌥Esc"]) expect(screen.getByText(keys)).toBeTruthy();
    expect(screen.getByText("Hablar con Deimos")).toBeTruthy();
    press("Empezar a usar Orbit");
    expect(sent).toContainEqual({ command: "onboarding_done" });
    expect(onComplete).toHaveBeenCalledOnce();
  });
});
