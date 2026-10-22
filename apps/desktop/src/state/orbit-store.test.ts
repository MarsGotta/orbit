import type { AppBootstrap, FeatureFrame } from "../bridge/generated";
import { describe, expect, it, vi } from "vitest";
import type { DaemonPort } from "../bridge/port";
import type { Notice } from "../bridge/generated/Notice";
import { freshNotices, receive, type Message } from "../app/interceptor";
import { createOrbitStore } from "./orbit-store";

function port(send: DaemonPort["send"]) {
  const report = vi.fn<DaemonPort["report"]>(() => Promise.resolve());
  const fake: DaemonPort = {
    snapshot: () => Promise.reject(new Error("sin arranque en esta prueba")),
    subscribeFrames: () => Promise.resolve(() => undefined),
    send,
    report,
  };
  return { fake, report };
}

describe("the op of an order, from the interface", () => {
  it("each order carries a new op, and its refusal is shown and reported under it", async () => {
    const ops: string[] = [];
    const { fake, report } = port((_command, op) => {
      ops.push(op);
      return Promise.reject({ refusal: { code: "timeout", retry: "never" } });
    });
    const store = createOrbitStore(fake);
    await store.getState().send({ command: "rearm" }, "settings");
    await store.getState().send({ command: "rearm" }, "settings");
    expect(ops).toHaveLength(2);
    expect(ops[0]).toMatch(/^[A-Za-z0-9-]{8,64}$/);
    expect(ops[0]).not.toBe(ops[1]);
    const messages = store.getState().messages;
    expect(messages.map((message) => message.op)).toEqual([ops[1]]);
    expect(messages[0]).toMatchObject({ class: "failed", place: "settings" });
    expect(report.mock.calls.map(([line]) => [line.op, line.source, line.code])).toEqual([
      [ops[0], "app", "timeout"],
      [ops[1], "app", "timeout"],
    ]);
  });
});

describe("the daemon's notices, each once and in order", () => {
  const fixable: Notice = {
    code: "microphone_permission_missing",
    op: "0123abcd-turn",
    sequence: 2,
  };
  const silent: Notice = { code: "recording_empty", op: "0123abcd-turn", sequence: 3 };

  it("a notice that must be seen is not hidden by a silent one said after it", () => {
    const earlier: Notice = { code: "recording_started", op: "fedcba98-earlier", sequence: 1 };
    const opened = freshNotices([earlier], null);
    // One turn says the permission is missing and then the empty recording:
    // both arrive in one frame, and both are taken, in order.
    const turn = freshNotices([earlier, fixable, silent], opened.last);
    expect(turn.fresh).toEqual([fixable, silent]);
    let messages: Message[] = [];
    for (const notice of turn.fresh)
      messages = receive(messages, { kind: "notice", notice }).messages;
    expect(messages.map((message) => message.class)).toEqual(["fixable"]);
  });

  it("takes each notice once, and only the latest when the window opens", () => {
    const first = freshNotices([{ ...silent, sequence: 1 }, fixable], null);
    expect(first.fresh).toEqual([fixable]);
    expect(freshNotices([{ ...silent, sequence: 1 }, fixable], first.last).fresh).toEqual([]);
    const later: Notice = { code: "transcript_retry_requested", op: "fedcba98-later", sequence: 4 };
    expect(freshNotices([{ ...silent, sequence: 1 }, fixable, later], first.last).fresh).toEqual([
      later,
    ]);
  });
});

describe("the next accepted order clears only its function's refusal", () => {
  it("keeps refusals across frames, other functions and rejected orders", async () => {
    const send = vi.fn<DaemonPort["send"]>();
    const { fake } = port(send);
    let acceptFrame: ((frame: FeatureFrame) => void) | undefined;
    fake.subscribeFrames = async (accept) => {
      acceptFrame = accept;
      return () => undefined;
    };
    fake.snapshot = async () =>
      ({
        revision: { epoch: 1, counter: 1 },
        frames: [
          "status",
          "permissions",
          "agenda",
          "notes",
          "note",
          "recording",
          "assistant",
          "control",
          "briefing",
          "local",
        ].map((feature) => ({
          feature,
          revision: { epoch: 1, counter: 1 },
          view: feature === "notes" ? null : {},
        })),
      }) as AppBootstrap;
    const store = createOrbitStore(fake, { notices: false });
    await store.getState().connect();
    // Only permissions are read by these frames; no provider or host participates.
    const frame = (counter: number, microphone: "denied" | "granted") => ({
      feature: "permissions" as const,
      revision: { epoch: 1, counter },
      view: {
        microphone,
        screen: "unknown" as const,
        accessibility: "unknown" as const,
        calendars: "unknown" as const,
      },
    });
    acceptFrame?.(frame(2, "denied"));
    send.mockRejectedValueOnce({
      refusal: { code: "permission_denied", permission: "microphone" },
    });
    await store.getState().send({ command: "permissions", request: ["microphone"] });
    const first = store.getState().messages.at(-1);
    expect(first?.feature).toBe("permissions");
    acceptFrame?.(frame(3, "granted"));
    expect(store.getState().snapshot?.permissions.microphone).toBe("granted");
    expect(store.getState().messages).toContainEqual(first);
    send.mockRejectedValueOnce({ refusal: { code: "timeout" } });
    await store.getState().send({ command: "call_start", duration_minutes: 60 });
    expect(store.getState().messages).toContainEqual(first);
    send.mockResolvedValueOnce({ revision: { epoch: 1, counter: 4 } });
    await store.getState().send({ command: "call_stop" });
    expect(store.getState().messages).toEqual([first]);
    send.mockRejectedValueOnce({ refusal: { code: "timeout" } });
    await store.getState().send({ command: "call_start", duration_minutes: 60 });
    const other = store.getState().messages.at(-1);
    send.mockRejectedValueOnce({ refusal: { code: "timeout" } });
    await store.getState().send({ command: "permissions", request: ["microphone"] });
    const latest = store.getState().messages.find((message) => message.feature === "permissions");
    expect(latest).toBeDefined();
    expect(latest?.op).not.toBe(first?.op);
    send.mockResolvedValueOnce({ revision: { epoch: 1, counter: 4 } });
    await store.getState().send({ command: "permissions", request: ["microphone"] });
    expect(store.getState().messages).toEqual([other]);
  });
});

it.each([
  ["la ventana principal", true],
  ["el panel de la llamada", false],
] as const)(
  "un rearm aceptado quita las negativas de privacidad de cualquier función en %s",
  async (_surface, notices) => {
    const send = vi.fn<DaemonPort["send"]>(async () => ({ revision: { epoch: 1, counter: 1 } }));
    const { fake } = port(send);
    const store = createOrbitStore(fake, { notices });
    send.mockRejectedValueOnce({ refusal: { code: "privacy_stopped", retry: "never" } });
    await store.getState().send({ command: "meeting_finish" }, "recording");
    send.mockRejectedValueOnce({ refusal: { code: "timeout", retry: "never" } });
    await store.getState().send({ command: "call_start", duration_minutes: 60 }, "assistant");
    const unrelated = store.getState().messages.at(-1);
    send.mockRejectedValueOnce({ refusal: { code: "privacy_stopped", retry: "never" } });
    await store.getState().send({ command: "calendar_refresh" }, "calendar");
    expect(store.getState().messages.map((message) => message.feature)).toEqual([
      "recording",
      "assistant",
      "agenda",
    ]);
    expect(store.getState().messages[0]?.text).toBe(
      "Todo está parado. Reactiva Orbit para continuar.",
    );
    await store.getState().send({ command: "rearm" });
    expect(store.getState().messages).toEqual([unrelated]);
  },
);

it("keeps the privacy refusal when rearm fails or another order is accepted", async () => {
  const send = vi.fn<DaemonPort["send"]>(async () => ({ revision: { epoch: 1, counter: 1 } }));
  const { fake } = port(send);
  const store = createOrbitStore(fake);
  send.mockRejectedValueOnce({ refusal: { code: "privacy_stopped", retry: "never" } });
  await store.getState().send({ command: "meeting_finish" }, "recording");
  const stopped = store.getState().messages[0];
  await store.getState().send({ command: "update_check" });
  expect(store.getState().messages).toEqual([stopped]);
  send.mockRejectedValueOnce({ refusal: { code: "timeout", retry: "never" } });
  await store.getState().send({ command: "rearm" });
  expect(store.getState().messages).toContainEqual(stopped);
});
