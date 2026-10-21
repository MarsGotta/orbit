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

// When everything is fine the screen says nothing; a problem says
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

describe("the status pill", () => {
  it("with everything fine there is no pill", () => {
    const { view } = renderTopBar(snapshot());
    expect(screen.queryByRole("status")).toBeNull();
    expectQuiet(view.container.textContent);
  });

  it("offline, the pill says what to do", () => {
    const { view } = renderTopBar(
      snapshot({ connection: { hub: "offline" }, hub: { connection: "offline" } }),
    );
    const pill = screen.getByRole("status");
    expect(pill.textContent).toMatch(/Sin conexión/);
    expect(pill.textContent).toMatch(/Comprueba/);
    expectQuiet(view.container.textContent);
  });

  it("a reconnection does not make the pill flicker: connecting still says offline", () => {
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

  it("at startup, while it connects for the first time, it says nothing", () => {
    renderTopBar(
      snapshot({ connection: { hub: "connecting" }, hub: { connection: "connecting" } }),
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("when a permission is missing, the pill says so and leads to Ajustes", async () => {
    const { opened } = renderTopBar(snapshot({ permissions: { microphone: "denied" } }));
    const pill = screen.getByRole("status");
    expect(pill.textContent).toMatch(/Falta el permiso de micrófono/);
    await userEvent.click(screen.getByRole("button", { name: "Abrir Ajustes" }));
    expect(opened).toEqual(["settings"]);
  });

  it("an unknown permission is not a missing permission", () => {
    renderTopBar(snapshot({ permissions: { accessibility: "unknown" } }));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("with privacy stopped, the pill offers to rearm", async () => {
    const { sent } = renderTopBar(snapshot({ privacy_stopped: true }));
    expect(screen.getByRole("status").textContent).toMatch(/Privacidad detenida/);
    await userEvent.click(screen.getByRole("button", { name: "Rearmar" }));
    expect(sent).toEqual([{ command: "rearm" }]);
  });

  it("unpaired, the pill leads to pairing", async () => {
    const { opened } = renderTopBar(
      snapshot({ connection: { hub: "unpaired" }, hub: { paired: false, connection: "offline" } }),
    );
    expect(screen.getByRole("status").textContent).toMatch(/no está emparejado/);
    await userEvent.click(screen.getByRole("button", { name: "Emparejar" }));
    expect(opened).toEqual(["settings"]);
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
  it("an unknown voice note and an unknown call say nothing in the assistant", () => {
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

  it("an unknown assistant is an empty view", () => {
    const value = {
      ...snapshot(),
      chat: { ...chat, availability: { state: "unknown", cause: "pending:not_observed_yet" } },
      call_availability: { state: "unknown", cause: "pending:not_observed_yet" },
    } as unknown as AppSnapshot;
    const view = render(<UnknownAssistant snapshot={value} />);
    expect(view.container.textContent).toBe("");
  });

  it("an unknown recording paints nothing beside the bar", () => {
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

  it("without notices, the notice strip paints nothing", () => {
    const view = render(
      <NoticeBanner messages={[]} onAction={() => undefined} onDismiss={() => undefined} />,
    );
    expect(view.container.textContent).toBe("");
  });
});

describe("the calendar does not explain what is fine", () => {
  it("with the agenda read there is no «Deimos la lee» block", () => {
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

describe("the briefing does not turn the unknown into a no", () => {
  it("an agenda not read yet says nothing, not even «0 reuniones»", () => {
    const value = {
      ...snapshot(),
      briefing: { sources: { agenda: { state: "unknown", cause: "topic_not_observed" } } },
      calendar: { authorization: "granted", reading: { state: "unknown" }, stale: false },
    } as unknown as AppSnapshot;
    expect(briefingAgendaStatus(value)).toEqual({ known: false, text: null });
  });

  it("without the Calendarios permission it says how to grant it", () => {
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
