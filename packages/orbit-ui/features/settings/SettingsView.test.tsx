import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { CopyProvider } from "../../i18n/CopyProvider";
import { TEST_NOW_MS } from "../../test-clock";
import SettingsView, { type SettingsViewProps } from "./SettingsView";
import type { SettingsSection } from "../../state/ui";

// Settings in five sections. Only the view fields Settings reads; names,
// hosts and contents are invented.

const TODAY_MS = TEST_NOW_MS - (14 * 60 + 30) * 60_000;

type Overrides = {
  settings?: Record<string, unknown>;
  permissions?: Record<string, unknown>;
  meeting?: Record<string, unknown>;
  control?: unknown;
  notes?: unknown;
  status?: "current" | "unknown";
};

function snapshot(overrides: Overrides = {}): AppView {
  return {
    connection: { daemon: "ready" },
    status_availability: { state: overrides.status ?? "current" },
    meeting: { call_preference: "notify", limit_minutes: 60, ...overrides.meeting },
    settings: {
      agents: [],
      permissions: {
        microphone: "granted",
        screen: "granted",
        accessibility: "granted",
        calendars: "granted",
        availability: { state: "current" },
        ...overrides.permissions,
      },
      hub: {
        url: "https://hub.example",
        paired: true,
        device_name: "Mac de Prueba",
        device_id_short: "abc123",
        connection: "online",
      },
      helper_connected: true,
      versions: { app: "0.10.0" },
      update: { state: "idle", version: null, message: null },
      privacy_stopped: false,
      login_item: "on",
      profile: { name: "Noa" },
      ...overrides.settings,
    },
    computer_control: overrides.control,
    notes: overrides.notes,
  } as unknown as AppView;
}

function renderSettings(props: Partial<SettingsViewProps> & { overrides?: Overrides } = {}) {
  const send = vi.fn<(command: unknown) => Promise<void>>(() => Promise.resolve());
  const onReopenOnboarding = vi.fn();
  const view = render(
    <SettingsView
      error={null}
      onReopenOnboarding={onReopenOnboarding}
      send={send}
      snapshot={snapshot(props.overrides)}
      {...props}
    />,
  );
  return { send, onReopenOnboarding, ...view };
}

function open(section: string) {
  fireEvent.click(
    within(screen.getByRole("navigation", { name: "Secciones de ajustes" })).getByRole("button", {
      name: new RegExp(`^${section}`),
    }),
  );
}

afterEach(cleanup);

describe("the index", () => {
  it("lists the seven sections and opens General without a section", () => {
    renderSettings();
    const index = screen.getByRole("navigation", { name: "Secciones de ajustes" });
    expect(
      within(index)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual([
      "General",
      "Privacidad y permisos",
      "Grabación y llamadas",
      "Asistentes",
      "IA",
      "Reuniones",
      "Notas",
    ]);
    expect(
      within(index).getByRole("button", { name: "General" }).getAttribute("aria-current"),
    ).toBe("page");
    expect(screen.getByRole("heading", { level: 2, name: "General" })).toBeTruthy();
  });

  it("opens the section the route names and asks the router for another", () => {
    const onSection = vi.fn();
    renderSettings({ section: "privacidad" satisfies SettingsSection, onSection });
    expect(screen.getByRole("heading", { level: 2, name: "Privacidad y permisos" })).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /^Privacidad y permisos/ }).getAttribute("aria-current"),
    ).toBe("page");
    open("Reuniones");
    expect(onSection).toHaveBeenCalledWith("reuniones");
  });

  it("moves between sections by itself when no router drives it", () => {
    renderSettings();
    open("Grabación y llamadas");
    expect(screen.getByRole("heading", { level: 2, name: "Grabación y llamadas" })).toBeTruthy();
  });

  it("says «● Falta 1» on Privacidad y permisos when macOS has not given one", () => {
    renderSettings({ overrides: { permissions: { screen: "denied" } } });
    const privacy = screen.getByRole("button", { name: /^Privacidad y permisos/ });
    expect(privacy.textContent).toBe("Privacidad y permisosFalta 1");
  });

  it("says nothing missing while macOS has not said anything", () => {
    renderSettings({
      overrides: { permissions: { screen: "unknown", availability: { state: "unknown" } } },
    });
    expect(screen.getByRole("button", { name: /^Privacidad y permisos/ }).textContent).toBe(
      "Privacidad y permisos",
    );
  });

  it("«Salir de Orbit» quits, and the version is under it", () => {
    const { send } = renderSettings();
    fireEvent.click(screen.getByRole("button", { name: "Salir de Orbit" }));
    expect(send).toHaveBeenCalledWith({ command: "quit" });
    expect(screen.getByText("Orbit 0.10.0")).toBeTruthy();
  });
});

describe("General", () => {
  it("saves the name on leaving the field and says «✓ Guardado»", async () => {
    const { send } = renderSettings();
    const name = screen.getByRole("textbox", { name: "Tu nombre" });
    fireEvent.blur(name);
    expect(send).not.toHaveBeenCalledWith(expect.objectContaining({ command: "profile_set" }));
    fireEvent.change(name, { target: { value: "Noa Prado " } });
    fireEvent.blur(name);
    expect(send).toHaveBeenCalledWith({ command: "profile_set", name: "Noa Prado" });
    expect(await screen.findByText("✓ Guardado")).toBeTruthy();
  });

  it("shows this Mac and its hub, and asks inside the page before pairing again", () => {
    const { onReopenOnboarding } = renderSettings();
    expect(screen.getByText("Mac de Prueba")).toBeTruthy();
    expect(screen.getByText("Emparejado con hub.example")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Emparejar de nuevo" }));
    expect(onReopenOnboarding).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("¿Emparejar este Mac otra vez?");
    fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Emparejar de nuevo" }));
    fireEvent.click(screen.getByRole("button", { name: "Sí, emparejar de nuevo" }));
    expect(onReopenOnboarding).toHaveBeenCalledWith("pair");
  });

  it("pairs an unpaired Mac without asking, and reopens the welcome", () => {
    const { onReopenOnboarding } = renderSettings({
      overrides: {
        settings: { hub: { url: null, paired: false, device_name: null, device_id_short: null } },
      },
    });
    expect(screen.getByText("Este Mac no está emparejado.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Emparejar" }));
    expect(onReopenOnboarding).toHaveBeenCalledWith("pair");
    fireEvent.click(screen.getByRole("button", { name: "Volver a ver la bienvenida" }));
    expect(onReopenOnboarding).toHaveBeenLastCalledWith();
  });

  it("fixes the theme from Aspecto", () => {
    renderSettings();
    const themes = screen.getByRole("radiogroup", { name: "Tema" });
    const light = within(themes).getByRole("radio", { name: "Claro" });
    fireEvent.click(light);
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(light.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(within(themes).getByRole("radio", { name: "Oscuro" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("switches «Abrir Orbit al iniciar sesión»", () => {
    const { send } = renderSettings();
    fireEvent.click(screen.getByRole("switch", { name: "Abrir Orbit al iniciar sesión" }));
    expect(send).toHaveBeenCalledWith({ command: "login_item", on: false });
  });

  it("searches for an update and says where it stands", () => {
    const { send, rerender } = renderSettings();
    expect(screen.getByText("Orbit busca versiones nuevas al abrirse y cada 6 horas")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Buscar actualización" }));
    expect(send).toHaveBeenCalledWith({ command: "update_check" });
    const props = { error: null, onReopenOnboarding: vi.fn(), send };
    rerender(
      <SettingsView
        {...props}
        snapshot={snapshot({ settings: { update: { state: "checking", version: null } } })}
      />,
    );
    expect(screen.getByText("Buscando una versión nueva…")).toBeTruthy();
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "Buscar actualización" }).disabled,
    ).toBe(true);
    rerender(
      <SettingsView
        {...props}
        snapshot={snapshot({ settings: { update: { state: "up_to_date", version: "0.10.0" } } })}
      />,
    );
    expect(screen.getByText("Orbit está al día")).toBeTruthy();
  });

  it("explains why this build cannot check updates and disables the search", () => {
    renderSettings({ overrides: { settings: { update: { state: "disabled", version: null } } } });
    expect(screen.getByText(/ORBIT_UPDATE_BASE_URL/)).toBeTruthy();
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "Buscar actualización" }).disabled,
    ).toBe(true);
  });
});

describe("Privacidad y permisos", () => {
  it("stops everything, and once stopped offers «Reactivar Orbit», saying «Reactivando…»", async () => {
    let settle: () => void = () => undefined;
    const send = vi.fn(
      (command: { command: string }) =>
        new Promise<void>((resolve) => {
          if (command.command === "rearm") settle = resolve;
          else resolve();
        }),
    );
    const { rerender } = render(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="privacidad"
        snapshot={snapshot()}
      />,
    );
    expect(screen.getByRole("heading", { level: 3, name: "Parar todo" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Parar todo ahora" }));
    expect(send).toHaveBeenCalledWith({ command: "privacy_stop" });
    rerender(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="privacidad"
        snapshot={snapshot({ settings: { privacy_stopped: true } })}
      />,
    );
    expect(screen.getByRole("heading", { level: 3, name: "Todo está parado" })).toBeTruthy();
    const rearm = screen.getByRole("button", { name: /Reactivar Orbit/ });
    fireEvent.click(rearm);
    expect(send).toHaveBeenLastCalledWith({ command: "rearm" });
    expect(rearm.getAttribute("aria-busy")).toBe("true");
    expect(within(rearm).getByText("Reactivando…").getAttribute("aria-hidden")).toBe("false");
    await act(async () => settle());
    expect(rearm.getAttribute("aria-busy")).toBeNull();
  });

  it("shows each permission: «Permitido», or what is missing with the button that fixes it", async () => {
    const { send } = renderSettings({
      section: "privacidad",
      overrides: { permissions: { screen: "denied", calendars: "not_determined" } },
    });
    const permissions = screen.getByRole("region", { name: "Permisos de macOS" });
    expect(within(permissions).getAllByText("Permitido")).toHaveLength(2);
    expect(within(permissions).getAllByText("Sin permiso")).toHaveLength(2);
    fireEvent.click(
      screen.getByRole("button", {
        name: "Pedir permiso: Pantalla y audio del sistema",
      }),
    );
    expect(send).toHaveBeenLastCalledWith({ command: "permissions", request: ["screen"] });
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Abrir Ajustes del Sistema: Pantalla y audio del sistema",
      }),
    );
    expect(send).toHaveBeenLastCalledWith({
      command: "open_system_privacy",
      pane: "ScreenCapture",
    });
    fireEvent.click(screen.getByRole("button", { name: "Pedir permiso: Calendarios" }));
    expect(send).toHaveBeenLastCalledWith({ command: "permissions", request: ["calendars"] });
  });

  it("back from System Settings, reads the permissions again while one is missing", async () => {
    const { send, rerender } = renderSettings({
      section: "privacidad",
      overrides: { permissions: { microphone: "denied" } },
    });
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });
    expect(send).toHaveBeenCalledWith({ command: "permissions", request: [] });
    send.mockClear();
    rerender(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="privacidad"
        snapshot={snapshot()}
      />,
    );
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });
    expect(send).not.toHaveBeenCalledWith({ command: "permissions", request: [] });
  });

  it("without the Mac's helper, «Reiniciar Orbit» restarts Orbit, which starts it again", () => {
    const { send } = renderSettings({
      section: "privacidad",
      overrides: { settings: { helper_connected: false } },
    });
    expect(
      screen.getByText("Orbit.app no está en marcha. Ábrela para ver los permisos."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reiniciar Orbit" }));
    expect(send).toHaveBeenCalledWith({ command: "restart" });
    expect(send).not.toHaveBeenCalledWith({ command: "quit" });
  });

  it("does not paint a permission macOS has not reported as missing", () => {
    renderSettings({
      section: "privacidad",
      overrides: { permissions: { availability: { state: "unknown" } } },
    });
    const permissions = screen.getByRole("region", { name: "Permisos de macOS" });
    expect(within(permissions).getAllByText("Comprobando…")).toHaveLength(5);
    expect(within(permissions).queryByText("Sin permiso")).toBeNull();
    expect(within(permissions).queryByText("Permitido")).toBeNull();
  });

  it("puts «Micrófono siempre listo» in the microphone's row, waiting for macOS until it answers", () => {
    const { send, rerender } = renderSettings({
      section: "privacidad",
      overrides: { permissions: { microphone: "not_determined" } },
    });
    const toggle = screen.getByRole("switch", { name: /Micrófono siempre listo/ });
    expect(toggle.closest("[data-warn]")?.textContent).toContain(
      "Para grabar reuniones, mandar notas de voz",
    );
    fireEvent.click(toggle);
    expect(send).toHaveBeenCalledWith({ command: "permissions", request: ["microphone"] });
    expect(screen.getByText("Esperando a macOS…")).toBeTruthy();
    rerender(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="privacidad"
        snapshot={snapshot({ permissions: { microphone: "denied" } })}
      />,
    );
    return vi.waitFor(() => expect(screen.queryByText("Esperando a macOS…")).toBeNull());
  });

  it("offers the error reports only when the build can send them", () => {
    const { send, rerender } = renderSettings({ section: "privacidad" });
    expect(
      screen.queryByRole("switch", { name: "Enviar informes de errores anónimos" }),
    ).toBeNull();
    rerender(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="privacidad"
        snapshot={snapshot({ settings: { error_reports: false } })}
      />,
    );
    const toggle = screen.getByRole("switch", { name: "Enviar informes de errores anónimos" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(send).toHaveBeenCalledWith({ command: "error_reports", enabled: true });
  });
});

describe("Grabación y llamadas", () => {
  it("chooses what a video call does, with nothing chosen while it is unknown", () => {
    const { send, rerender } = renderSettings({
      section: "grabacion",
      overrides: { meeting: { call_preference: null } },
    });
    const choices = screen.getByRole("radiogroup", { name: "Al detectar una llamada" });
    expect(
      within(choices)
        .getAllByRole<HTMLInputElement>("radio")
        .map((radio) => radio.checked),
    ).toEqual([false, false, false]);
    fireEvent.click(within(choices).getByRole("radio", { name: /Grabar sin preguntar/ }));
    expect(send).toHaveBeenCalledWith({ command: "call_preference", preference: "record" });
    rerender(
      <SettingsView
        error={null}
        onReopenOnboarding={vi.fn()}
        send={send}
        section="grabacion"
        snapshot={snapshot()}
      />,
    );
    expect(
      screen.getByRole<HTMLInputElement>("radio", { name: /Avisarme \(recomendado\)/ }).checked,
    ).toBe(true);
  });

  it.each([
    ["es", -9878, "No disponible: otra app está usando este atajo (OSStatus -9878)."],
    ["en", -9878, "Unavailable: another app is using this shortcut (OSStatus -9878)."],
    ["es", -50, "No disponible: macOS rechazó el atajo (OSStatus -50)."],
    ["en", -50, "Unavailable: macOS rejected the shortcut (OSStatus -50)."],
  ] as const)("names the failed shortcut and OSStatus in %s (%s)", (language, status, message) => {
    render(
      <CopyProvider choice={language}>
        <SettingsView
          error={null}
          onReopenOnboarding={vi.fn()}
          send={vi.fn()}
          section="grabacion"
          snapshot={snapshot({
            settings: {
              shortcut_failures: [{ combination: "⌃⌥D", os_status: status }],
            },
          })}
        />
      </CopyProvider>,
    );
    const row = screen.getByText("⌃⌥D").parentElement!;
    expect(within(row).getByText(message)).not.toBeNull();
    expect(screen.queryAllByText(/OSStatus/)).toHaveLength(1);
  });

  it("shows shortcuts without a retired duration ceiling", () => {
    renderSettings({ section: "grabacion", overrides: { meeting: { limit_minutes: 45 } } });
    expect(screen.queryByText("hasta 45 min")).toBeNull();
    const shortcuts = screen.getByRole("region", { name: "Atajos desde cualquier app" });
    expect(
      within(shortcuts)
        .getAllByText(/^⌃⌥/)
        .map((key) => key.textContent),
    ).toEqual(["⌃⌥M", "⌃⌥␣", "⌃⌥D", "⌃⌥Esc"]);
  });
});

describe("Asistentes", () => {
  it("lists what the assistants did on this Mac, by day", () => {
    renderSettings({
      section: "asistentes",
      overrides: {
        control: {
          engine: { state: "ready", tools: 3 },
          history: [
            {
              operation_id: "op-2",
              requested: "Abrir el informe",
              application: "Numbers",
              channel: "chat",
              at_ms: TODAY_MS + (9 * 60 + 52) * 60_000,
              outcome: "performed",
            },
            {
              operation_id: "op-1",
              requested: "Copiar la tabla",
              application: "Safari",
              channel: "chat",
              at_ms: TODAY_MS - 86_400_000 + (18 * 60 + 5) * 60_000,
              outcome: "failed",
            },
          ],
          folders: { state: "known", max: 8, folders: [] },
        },
      },
    });
    const history = screen.getByRole("region", { name: "Lo que han hecho en este Mac" });
    expect(history.textContent).toContain("NumbersAbrir el informeHoy, 9:52Hecho");
    expect(history.textContent).toContain("SafariCopiar la tablaAyer, 18:05Falló");
  });

  it("shows the history's shape while the daemon has not said it", () => {
    renderSettings({ section: "asistentes" });
    const history = screen.getByRole("region", { name: "Lo que han hecho en este Mac" });
    expect(history.querySelector("[aria-busy='true']")).toBeTruthy();
  });
});
