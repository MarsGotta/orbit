import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AppCommand, CommandReceipt } from "../bridge/generated";
import type { AddedAttachment } from "../bridge/generated/AddedAttachment";
import type { AgentAnswer } from "../bridge/generated/AgentAnswer";
import type { AgentCall } from "../bridge/generated/AgentCall";
import type { AttachmentKind } from "../bridge/generated/AttachmentKind";
import type { AttachmentSource } from "../bridge/attachments";
import type { MessagesPage, SessionPage } from "../bridge/reads";
import { bootstrapOf, framesOf } from "./base";
import { attachmentBytes } from "./images";
import { reactTo } from "./reactions";
import type { HubData, Scenario, Views } from "./scenario";

/** The daemon of the showcase: what it publishes, and the next frame's counter. */
interface Daemon {
  views: Views;
  hub: HubData;
  counter: number;
}

/** Publishes `views` as the daemon does, one frame per function, newer than the last. */
export async function publish(daemon: Daemon, views: Partial<Views>): Promise<void> {
  daemon.counter += 1;
  daemon.views = { ...daemon.views, ...views };
  for (const frame of framesOf(views, daemon.counter))
    await emit(`orbit://feature/${frame.feature}`, frame);
}

function page<T>(items: T[]) {
  return {
    state: "present" as const,
    value: { items, next: null, total: items.length, revision: 1 },
    as_of_ms: Date.now(),
  };
}

function agentAnswer(hub: HubData, call: AgentCall): AgentAnswer {
  if (call.call === "config") {
    const config = hub.agents[call.agent_id];
    return config
      ? { call, outcome: { outcome: "config", config } }
      : {
          call,
          outcome: {
            outcome: "refused",
            refusal: { code: "agent_not_found", status: 404, numbers: {} },
          },
        };
  }
  if (call.call === "skill") {
    const skill = hub.skills[call.name];
    if (skill) return { call, outcome: { outcome: "skill", skill } };
  }
  return { call, outcome: { outcome: "done" } };
}

const KINDS: Record<string, [AttachmentKind, string]> = {
  png: ["image", "image/png"],
  jpg: ["image", "image/jpeg"],
  pdf: ["pdf", "application/pdf"],
  md: ["text", "text/markdown"],
  txt: ["text", "text/plain"],
};

function added(source: AttachmentSource, index: number): AddedAttachment {
  const name =
    source.from === "path"
      ? (source.path.split("/").pop() ?? "archivo")
      : source.from === "bytes"
        ? source.name
        : "Ventana.png";
  const [kind, mime] = KINDS[name.split(".").pop()?.toLowerCase() ?? ""] ?? ["text", "text/plain"];
  const id = `${kind === "pdf" ? "pdf" : "chart"}-draft-${index}`;
  return {
    attachment: {
      id,
      name,
      kind,
      mime,
      bytes: 180_000 + index * 52_000,
      ...(kind === "pdf" ? { pages: 4 } : {}),
    },
  };
}

function send(
  daemon: Daemon,
  scenario: Scenario,
  command: AppCommand,
): CommandReceipt | Promise<never> {
  if (scenario.hold?.(command) === true) return new Promise(() => undefined);
  const refusal = scenario.refuse?.(command) ?? null;
  if (refusal) throw { refusal };
  const next = scenario.react?.(daemon.views, command) ?? reactTo(daemon.views, command);
  if (next) void publish(daemon, next);
  return { revision: { epoch: 1, counter: daemon.counter }, delivery: { state: "reflected" } };
}

type Args = Record<string, unknown>;

function handler(daemon: Daemon, scenario: Scenario, connected: () => void) {
  let uploads = 0;
  const commands: Record<string, (args: Args) => unknown> = {
    orbit_snapshot: () => {
      // The window takes the snapshot in the tasks after this answer.
      window.setTimeout(connected, 0);
      if (scenario.connect === "hang") return new Promise(() => undefined);
      if (scenario.connect === "fail")
        throw { refusal: { code: "daemon_reply_invalid", retry: { kind: "never" } } };
      return bootstrapOf(daemon.views);
    },
    orbit_send: (args) => send(daemon, scenario, args.command as AppCommand),
    orbit_panel_send: (args) => send(daemon, scenario, args.command as AppCommand),
    orbit_read_sessions_page: (): SessionPage | Promise<SessionPage> =>
      scenario.stall === true ? new Promise(() => undefined) : page(daemon.hub.sessions),
    orbit_read_session_messages: (args): MessagesPage | Promise<MessagesPage> =>
      scenario.stall === true
        ? new Promise(() => undefined)
        : page(daemon.hub.messages[String(args.sessionId)] ?? []),
    orbit_agent: (args) => agentAnswer(daemon.hub, args.call as AgentCall),
    orbit_attachment_content: (args) => attachmentBytes(String(args.attachmentId)),
    orbit_attachment_add: (args) => added(args.source as AttachmentSource, (uploads += 1)),
    "plugin:dialog|open": () => [
      "/Users/lucia/Desktop/Propuesta piloto Ribera.pdf",
      "/Users/lucia/Desktop/ocupacion-semana-44.png",
    ],
  };
  return (command: string, payload?: unknown) =>
    (commands[command] ?? (() => null))((payload ?? {}) as Args);
}

/** Installs the daemon of `scenario` behind Tauri's IPC, as the window it names. */
export function installDaemon(
  scenario: Scenario,
  views: Views,
  hub: HubData,
  connected: () => void,
): Daemon {
  const daemon: Daemon = { views, hub, counter: 1 };
  mockWindows(scenario.panel === true ? "panel" : "main");
  mockIPC(handler(daemon, scenario, connected), { shouldMockEvents: true });
  return daemon;
}
