import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { ChatSnapshot } from "../bridge/generated/ChatSnapshot";
import { AssistantComposer } from "../features/assistant/AssistantComposer";
import { UnknownAssistant } from "../features/assistant/AssistantAvailability";
import { briefingAgendaStatus } from "../features/briefing/BriefingSections";
import { CalendarFeedback } from "../features/calendar/CalendarFeedback";
import type { AppView as AppSnapshot } from "../state/app-view";
import { NoticeBanner } from "./NoticeBanner";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";

// When everything is fine the screen says nothing; a problem says
// what to do; an unknown is not a failure and is not painted as one.

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
    call: null,
    chat: { pending: null },
  } as unknown as AppSnapshot;
}

function renderTopBar(value: AppSnapshot) {
  const sent: AppCommand[] = [];
  let paired = 0;
  const view_ = render(
    <TopBar
      onOpenPalette={() => undefined}
      onPair={() => (paired += 1)}
      send={async (command) => {
        sent.push(command);
      }}
      snapshot={value}
    />,
  );
  return { sent, paired: () => paired, view: view_ };
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

describe("the notice of the top bar", () => {
  it("with everything fine there is no notice", () => {
    const { view } = renderTopBar(snapshot());
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
  });

  it("offline, the notice says so and that Orbit keeps trying", () => {
    const { view } = renderTopBar(
      snapshot({ connection: { hub: "offline" }, hub: { connection: "offline" } }),
    );
    const notice = screen.getByRole("status");
    expect(notice.textContent).toMatch(/Sin conexión con el hub/);
    expect(notice.textContent).toMatch(/Reintentando…/);
    expectQuiet(view.container.textContent);
  });

  it("a reconnection does not make the notice flicker: connecting still says offline", () => {
    const offline = snapshot({ connection: { hub: "offline" }, hub: { connection: "offline" } });
    const { view } = renderTopBar(offline);
    view.rerender(
      <TopBar
        onOpenPalette={() => undefined}
        onPair={() => undefined}
        send={async () => undefined}
        snapshot={snapshot({
          connection: { hub: "connecting" },
          hub: { connection: "connecting" },
        })}
      />,
    );
    expect(screen.getByRole("status").textContent).toMatch(/Sin conexión con el hub/);
  });

  // «Nada en blanco»: the first connection is shown as on its way,
  // never as a failure.
  it("at startup, while it connects for the first time, it says it connects", () => {
    renderTopBar(
      snapshot({ connection: { hub: "connecting" }, hub: { connection: "connecting" } }),
    );
    expect(screen.getByRole("status").textContent).toBe("Conectando con tu hub…");
  });

  it("when a permission is missing, the notice says so and leads to its section", () => {
    renderTopBar(snapshot({ permissions: { microphone: "denied" } }));
    expect(screen.getByRole("status").textContent).toMatch(/Falta el permiso de micrófono/);
    expect(screen.getByRole("link", { name: "Arreglar" }).getAttribute("href")).toBe(
      "#/settings/privacidad",
    );
  });

  it("an unknown permission is not a missing permission", () => {
    renderTopBar(snapshot({ permissions: { accessibility: "unknown" } }));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("with everything stopped, the notice reactivates Orbit and says so until Orbit answers", async () => {
    const sent: AppCommand[] = [];
    let answer = () => undefined as void;
    render(
      <TopBar
        onOpenPalette={() => undefined}
        onPair={() => undefined}
        send={(command) => {
          sent.push(command);
          return new Promise<void>((resolve) => (answer = resolve));
        }}
        snapshot={snapshot({ privacy_stopped: true })}
      />,
    );
    expect(screen.getByRole("status").textContent).toMatch(/Todo está parado/);
    await userEvent.click(screen.getByRole("button", { name: "Reactivar" }));
    expect(sent).toEqual([{ command: "rearm" }]);
    expect(screen.getByRole("status").textContent).toMatch(/Reactivando…/);
    expect(screen.queryByRole("button", { name: "Reactivar" })).toBeNull();
    await act(async () => answer());
    expect(screen.getByRole("button", { name: "Reactivar" })).toBeTruthy();
  });

  it("unpaired, the notice leads to pairing", async () => {
    const { paired } = renderTopBar(
      snapshot({ connection: { hub: "unpaired" }, hub: { paired: false, connection: "offline" } }),
    );
    expect(screen.getByRole("status").textContent).toMatch(/no está emparejado/);
    await userEvent.click(screen.getByRole("button", { name: "Emparejar" }));
    expect(paired()).toBe(1);
  });

  it("an unknown state paints nothing", () => {
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

describe("an unknown availability paints nothing", () => {
  it("with everything fine the composer says nothing", () => {
    const view = render(
      <AssistantComposer chat={chat} daemonReady hub="online" paired send={() => undefined} />,
    );
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
    expect(view.container.textContent).not.toMatch(/Nota de voz:|Contexto:/);
  });

  it("an unknown assistant waits, saying what it reads, never as a failure", () => {
    // «Nada en blanco»: what is not known yet shows it is being read.
    const value = {
      ...snapshot(),
      chat: { ...chat, availability: { state: "unknown", cause: "pending:not_observed_yet" } },
      call_availability: { state: "unknown", cause: "pending:not_observed_yet" },
    } as unknown as AppSnapshot;
    const view = render(<UnknownAssistant snapshot={value} />);
    expect(view.container.querySelector('[aria-busy="true"]')).not.toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Leyendo tus conversaciones del hub…");
    expectQuiet(view.container.textContent);
  });

  // «Grabar reunión» is always offered: the new recording window says what
  // is missing, if anything. An unknown recording shows no card and no state.
  it("an unknown recording paints no state beside «Grabar reunión»", () => {
    render(
      <Sidebar
        onRecord={() => undefined}
        send={() => undefined}
        snapshot={
          {
            ...snapshot({
              recording_availability: { state: "unknown", cause: "pending:helper_starting" },
            }),
            notes: { list: [] },
            settings: { profile: null },
          } as unknown as AppSnapshot
        }
        view="briefing"
      />,
    );
    expect(screen.getByRole("button", { name: "Grabar reunión" })).toBeTruthy();
    expect(screen.queryByRole("region")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("without notices, the notice strip paints nothing", () => {
    const view = render(
      <NoticeBanner
        messages={[]}
        onAction={() => undefined}
        onDismiss={() => undefined}
        onRetry={() => undefined}
      />,
    );
    expect(view.container.textContent).toBe("");
  });
});

describe("the calendar does not explain what is fine", () => {
  it("with the agenda read there is no «Deimos la lee» block", () => {
    const view = render(<CalendarFeedback error={null} onRetry={() => undefined} />);
    expect(view.container.textContent).toBe("");
  });
});

describe("the briefing does not turn the unknown into a no", () => {
  it("an agenda not read yet is being read, never «0 reuniones»", () => {
    const value = {
      ...snapshot(),
      briefing: { sources: { agenda: { state: "unknown", cause: "topic_not_observed" } } },
      calendar: { authorization: "granted", reading: { state: "unknown" }, stale: false },
    } as unknown as AppSnapshot;
    expect(briefingAgendaStatus(value)).toEqual({ state: "reading" });
  });

  it("without the Calendarios permission it offers to grant it", () => {
    const value = {
      ...snapshot(),
      briefing: { sources: { agenda: { state: "absent", cause: "calendar_permission_denied" } } },
      calendar: { authorization: "denied", reading: { state: "unavailable" }, stale: false },
    } as unknown as AppSnapshot;
    expect(briefingAgendaStatus(value)).toEqual({ state: "permission", authorization: "denied" });
  });
});
