import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { ChatSnapshot } from "../bridge/generated/ChatSnapshot";
import { AssistantComposer } from "../features/assistant/AssistantComposer";
import { UnknownAssistant } from "../features/assistant/AssistantAvailability";
import { briefingAgendaStatus } from "../features/briefing/BriefingSections";
import { CalendarFeedback } from "../features/calendar/CalendarFeedback";
import { MeetingEntryPoints } from "../features/meetings/MeetingEntryPoints";
import type { AppView as AppSnapshot } from "../state/app-view";
import type { ViewId } from "../state/ui";
import { AppTopBar } from "./AppTopBar";
import { NoticeBanner } from "./NoticeBanner";

// SIM-24: when everything is fine the screen says nothing; a problem says
// what to do; an unknown is not a failure and is not painted.

type Overrides = {
  connection?: Partial<AppSnapshot["connection"]>;
  hub?: Partial<AppSnapshot["settings"]["hub"]>;
  permissions?: Partial<AppSnapshot["permissions"]>;
  privacy_stopped?: boolean;
  status_availability?: AppSnapshot["status_availability"];
  recording_availability?: AppSnapshot["recording_availability"];
};

function snapshot(overrides: Overrides = {}): AppSnapshot {
  const permissions = {
    microphone: "granted",
    screen: "granted",
    accessibility: "granted",
    calendars: "granted",
    ...overrides.permissions,
  } as AppSnapshot["permissions"];
  return {
    connection: { daemon: "ready", hub: "paired", ...overrides.connection },
    status_availability: overrides.status_availability ?? { state: "current" },
    recording_availability: overrides.recording_availability ?? { state: "current" },
    privacy_stopped: overrides.privacy_stopped ?? false,
    permissions,
    settings: {
      hub: {
        url: "https://hub.example",
        paired: true,
        device_name: "Sample Mac",
        device_id_short: "a1b2c3",
        connection: "online",
        ...overrides.hub,
      },
      permissions,
    },
    meeting: { phase: "idle", recent: [], tracks: [] },
  } as unknown as AppSnapshot;
}

function renderTopBar(value: AppSnapshot, view: ViewId = "briefing") {
  const sent: AppCommand[] = [];
  const opened: ViewId[] = [];
  const view_ = render(
    <AppTopBar
      dispatch={(command) => sent.push(command)}
      navigate={(target) => opened.push(target)}
      onOpenPalette={() => undefined}
      onRecord={() => undefined}
      onToggleTheme={() => undefined}
      snapshot={value}
      view={view}
    />,
  );
  return { sent, opened, view: view_ };
}

const NOISE = [
  /Listo/,
  /Emparejado con/,
  /orbit-hub/,
  /Sample Mac/,
  /\d\/\d permisos/,
  /Privacidad: normal/,
  /en línea/,
  /Autonomía total/,
  /todavía no se conoce/,
  /aún no se ha observado/,
  /Estado del daemon/,
];

function expectQuiet(text: string | null) {
  for (const pattern of NOISE) expect(text ?? "").not.toMatch(pattern);
}

afterEach(cleanup);

describe("SIM-24 la píldora de estado", () => {
  it("con todo en orden no hay píldora", () => {
    const { view } = renderTopBar(snapshot());
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
  });

  it("sin conexión, la píldora dice qué hacer", () => {
    const { view } = renderTopBar(
      snapshot({ connection: { hub: "offline" }, hub: { connection: "offline" } }),
    );
    const pill = screen.getByRole("status");
    expect(pill.textContent).toMatch(/Sin conexión/);
    expect(pill.textContent).toMatch(/Comprueba/);
    expectQuiet(view.container.textContent);
  });

  it("una reconexión no hace parpadear la píldora: conectando sigue diciendo sin conexión", () => {
    const offline = snapshot({ connection: { hub: "offline" }, hub: { connection: "offline" } });
    const { view } = renderTopBar(offline);
    view.rerender(
      <AppTopBar
        dispatch={() => undefined}
        navigate={() => undefined}
        onOpenPalette={() => undefined}
        onRecord={() => undefined}
        onToggleTheme={() => undefined}
        snapshot={snapshot({
          connection: { hub: "connecting" },
          hub: { connection: "connecting" },
        })}
        view="briefing"
      />,
    );
    expect(screen.getByRole("status").textContent).toMatch(/Sin conexión/);
  });

  it("al arrancar, mientras conecta por primera vez, no dice nada", () => {
    renderTopBar(
      snapshot({ connection: { hub: "connecting" }, hub: { connection: "connecting" } }),
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("si falta un permiso, la píldora lo dice y lleva a Ajustes", async () => {
    const { opened } = renderTopBar(snapshot({ permissions: { microphone: "denied" } }));
    const pill = screen.getByRole("status");
    expect(pill.textContent).toMatch(/Falta el permiso de micrófono/);
    await userEvent.click(screen.getByRole("button", { name: "Abrir Ajustes" }));
    expect(opened).toEqual(["settings"]);
  });

  it("un permiso desconocido no es un permiso que falte", () => {
    renderTopBar(snapshot({ permissions: { accessibility: "unknown" } }));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("con la privacidad detenida, la píldora ofrece rearmar", async () => {
    const { sent } = renderTopBar(snapshot({ privacy_stopped: true }));
    expect(screen.getByRole("status").textContent).toMatch(/Privacidad detenida/);
    await userEvent.click(screen.getByRole("button", { name: "Rearmar" }));
    expect(sent).toEqual([{ command: "rearm" }]);
  });

  it("sin emparejar, la píldora lleva a emparejar", async () => {
    const { opened } = renderTopBar(
      snapshot({ connection: { hub: "unpaired" }, hub: { paired: false, connection: "offline" } }),
    );
    expect(screen.getByRole("status").textContent).toMatch(/no está emparejado/);
    await userEvent.click(screen.getByRole("button", { name: "Emparejar" }));
    expect(opened).toEqual(["settings"]);
  });

  it("un estado desconocido no pinta nada", () => {
    const { view } = renderTopBar(
      snapshot({
        status_availability: { state: "unknown", cause: "pending:not_observed_yet" },
        recording_availability: { state: "unknown", cause: "pending:not_observed_yet" },
      }),
    );
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
  });
});

const chat: ChatSnapshot = {
  turn: [],
  pending: null,
  session_id: null,
  voice: { level: 0, state: "idle" },
};

describe("SIM-24 una disponibilidad desconocida no pinta nada", () => {
  it("la nota de voz y la llamada desconocidas no dicen nada en el asistente", () => {
    const view = render(
      <AssistantComposer
        callActive={false}
        callPending={false}
        callUnknown
        chat={chat}
        daemonReady
        hub="online"
        onCallToggle={() => undefined}
        paired
        send={() => undefined}
        voiceUnknown
      />,
    );
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
    expect(view.container.textContent).not.toMatch(/Nota de voz:|Contexto:/);
  });

  it("un asistente desconocido es una vista vacía", () => {
    const value = {
      ...snapshot(),
      chat: { ...chat, availability: { state: "unknown", cause: "pending:not_observed_yet" } },
      call_availability: { state: "unknown", cause: "pending:not_observed_yet" },
    } as unknown as AppSnapshot;
    const view = render(<UnknownAssistant snapshot={value} />);
    expect(view.container.textContent).toBe("");
  });

  it("una grabación desconocida no pinta nada junto a la barra", () => {
    const view = render(
      <MeetingEntryPoints
        onRecord={() => undefined}
        snapshot={snapshot({
          recording_availability: { state: "unknown", cause: "pending:helper_starting" },
        })}
        view="briefing"
      />,
    );
    expect(view.container.textContent).toBe("");
  });

  it("sin avisos, la franja de avisos no pinta nada", () => {
    const view = render(
      <NoticeBanner messages={[]} onAction={() => undefined} onDismiss={() => undefined} />,
    );
    expect(view.container.textContent).toBe("");
  });
});

describe("SIM-24 el calendario no explica lo que va bien", () => {
  it("con la agenda leída no hay bloque «Deimos la lee»", () => {
    const calendar = {
      authorization: "granted",
      reading: { state: "available", blocker: null },
      stale: false,
    } as unknown as AppSnapshot["calendar"];
    const view = render(
      <CalendarFeedback
        calendar={calendar}
        error={null}
        onRetry={() => undefined}
        pending={false}
      />,
    );
    expect(view.container.textContent).toBe("");
  });
});

describe("SIM-24 el briefing no convierte lo desconocido en un no", () => {
  it("una agenda sin leer no dice nada ni «0 reuniones»", () => {
    const value = {
      ...snapshot(),
      briefing: { sources: { agenda: { state: "unknown", cause: "topic_not_observed" } } },
      calendar: { authorization: "granted", reading: { state: "unknown" }, stale: false },
    } as unknown as AppSnapshot;
    expect(briefingAgendaStatus(value)).toEqual({ known: false, text: null });
  });

  it("sin permiso de Calendarios dice cómo concederlo", () => {
    const value = {
      ...snapshot(),
      briefing: { sources: { agenda: { state: "absent", cause: "calendar_permission_denied" } } },
      calendar: { authorization: "denied", reading: { state: "unavailable" }, stale: false },
    } as unknown as AppSnapshot;
    expect(briefingAgendaStatus(value)).toEqual({
      known: false,
      text: "Concede el permiso de Calendarios para ver tu agenda.",
    });
  });
});
