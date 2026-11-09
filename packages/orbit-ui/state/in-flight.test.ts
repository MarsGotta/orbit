import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap, FeatureFrame, FeatureId } from "../bridge/generated";
import type { Notice } from "../bridge/generated/Notice";
import type { DaemonPort } from "../bridge/port";
import {
  CALL_TIMEOUT_SECONDS,
  CONNECT_CEILING_MS,
  DELIVERY_MS,
  FIRST_STATE_CEILING_MS,
  WORKING_CEILING_MS,
  callCeilingMs,
  orderCeilingMs,
} from "./ceilings";
import { hasNoStateYet, inFlight, reached } from "./in-flight";
import { createOrbitStore } from "./orbit-store";
import { WaitExceeded } from "./wait-registry";

const FEATURES: FeatureId[] = [
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
];

const current = { state: "current" } as const;
const pending = { state: "unknown", cause: "pending:not_observed_yet" } as const;

/** One frame of `feature` at `counter`; the views only carry what these tests read. */
function frame(feature: FeatureId, counter: number, known = true): FeatureFrame {
  const availability = known ? current : pending;
  const views: Partial<Record<FeatureId, unknown>> = {
    status: { availability, notices: [], settings: {} },
    permissions: { availability },
    recording: { availability, call_availability: availability, meeting: {}, call: null },
    assistant: { availability },
    control: null,
    notes: null,
  };
  return {
    feature,
    revision: { epoch: 1, counter },
    view: feature in views ? views[feature] : {},
  } as FeatureFrame;
}

function bootstrap(unknown: FeatureId[] = []): AppBootstrap {
  return {
    revision: { epoch: 1, counter: 1 },
    frames: FEATURES.map((feature) => frame(feature, 1, !unknown.includes(feature))),
  };
}

async function connected(
  send: DaemonPort["send"],
  options: { unknown?: FeatureId[]; mainWindow?: boolean } = {},
) {
  let accept: (frame: FeatureFrame) => void = () => {
    throw new Error("sin suscripción");
  };
  const report = vi.fn<DaemonPort["report"]>(async () => undefined);
  const port: DaemonPort = {
    subscribeFrames: async (listener) => {
      accept = listener;
      return () => undefined;
    },
    snapshot: async () => bootstrap(options.unknown),
    send,
    report,
  };
  const store = createOrbitStore(port, { notices: options.mainWindow ?? true });
  await store.getState().connect();
  return { store, report, accept: (next: FeatureFrame) => accept(next) };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("an order in flight", () => {
  it("opens when it leaves, with its feature, command, origin and start", async () => {
    let answer: (value: { revision: { epoch: number; counter: number } }) => void = () => undefined;
    const { store } = await connected(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const sent = store.getState().send({ command: "call_stop" }, "assistant");
    const [wait] = store.getState().waits;
    expect(wait).toMatchObject({
      kind: "order",
      feature: "assistant",
      command: "call_stop",
      origin: "assistant",
      startedMs: Date.now(),
    });
    expect(inFlight(store.getState().waits, { command: "call_stop" })).toHaveLength(1);
    answer({ revision: { epoch: 1, counter: 1 } });
    await expect(sent).resolves.toBe(true);
  });

  it("closes when the screen takes the frame carrying its receipt's revision, of any feature", async () => {
    const { store, accept } = await connected(async () => ({ revision: { epoch: 1, counter: 4 } }));
    await store.getState().send({ command: "call_stop" }, "assistant");
    expect(store.getState().waits).toMatchObject([
      { command: "call_stop", awaiting: { epoch: 1, counter: 4 } },
    ]);
    accept(frame("assistant", 3));
    expect(store.getState().waits).toHaveLength(1);
    // The counter is shared by every feature: a later status frame shows it too.
    accept(frame("status", 4));
    expect(store.getState().waits).toEqual([]);
  });

  it("closes with its receipt when the screen already shows that revision", async () => {
    const { store, accept } = await connected(async () => ({ revision: { epoch: 1, counter: 2 } }));
    accept(frame("recording", 2));
    await store.getState().send({ command: "meeting_pause" }, "recording");
    expect(store.getState().waits).toEqual([]);
  });

  it("closes through the interceptor when it is refused", async () => {
    const { store } = await connected(async () => {
      throw { refusal: { code: "privacy_stopped", retry: "never" } };
    });
    await expect(store.getState().send({ command: "rearm" }, "settings")).resolves.toBe(false);
    expect(store.getState().waits).toEqual([]);
    expect(store.getState().messages).toMatchObject([{ place: "settings", feature: "status" }]);
  });

  it("past its ceiling with no receipt, fails with its numbers where it was sent", async () => {
    const { store, report } = await connected(() => new Promise(() => undefined));
    const sent = store
      .getState()
      .send({ command: "open_url", url: "https://example.test" }, "calendar");
    const ceiling = orderCeilingMs("open_url");
    expect(ceiling).toBe(CALL_TIMEOUT_SECONDS * 1_000 + DELIVERY_MS);
    await vi.advanceTimersByTimeAsync(ceiling - 1);
    expect(store.getState().waits).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    await expect(sent).resolves.toBe(false);
    expect(store.getState().waits).toEqual([]);
    const [message] = store.getState().messages;
    expect(message).toMatchObject({ class: "failed", place: "calendar", feature: "status" });
    expect(message?.text).toContain(`${ceiling / 1_000} s`);
    expect(report.mock.calls.at(-1)?.[0]).toMatchObject({
      code: "ui_wait_exceeded",
      class: "failed",
      source: "ui",
      data: { stage: "receipt", code: "open_url", elapsed_ms: ceiling, timeout_ms: ceiling },
    });
  });

  it("past its ceiling with a receipt but no frame, fails naming the frame", async () => {
    const { store, report } = await connected(async () => ({ revision: { epoch: 1, counter: 9 } }));
    await store.getState().send({ command: "call_stop" }, "assistant");
    await vi.advanceTimersByTimeAsync(orderCeilingMs("call_stop"));
    expect(store.getState().waits).toEqual([]);
    expect(report.mock.calls.at(-1)?.[0]?.data).toMatchObject({
      stage: "frame",
      code: "call_stop",
    });
  });

  it("a refusal that arrives after the ceiling is not shown as the order's", async () => {
    let refuse: (error: unknown) => void = () => undefined;
    const { store } = await connected(
      () =>
        new Promise((_resolve, reject) => {
          refuse = reject;
        }),
    );
    const sent = store.getState().send({ command: "rearm" }, "settings");
    await vi.advanceTimersByTimeAsync(orderCeilingMs("rearm"));
    await sent;
    const exceeded = store.getState().messages;
    refuse({ refusal: { code: "privacy_stopped", retry: "never" } });
    await vi.advanceTimersByTimeAsync(0);
    expect(store.getState().messages).toEqual(exceeded);
  });
});

describe("a direct call in flight", () => {
  it("is open while it runs and closes when it settles", async () => {
    const { store } = await connected(vi.fn());
    let finish: (value: string) => void = () => undefined;
    const call = store.getState().track(
      "sessions_page",
      "assistant",
      () =>
        new Promise<string>((resolve) => {
          finish = resolve;
        }),
      "assistant",
    );
    expect(inFlight(store.getState().waits, { feature: "assistant" })).toMatchObject([
      { kind: "call", command: "sessions_page", origin: "assistant" },
    ]);
    finish("página");
    await expect(call).resolves.toBe("página");
    expect(store.getState().waits).toEqual([]);
  });

  it("past its ceiling rejects, and says so with its numbers", async () => {
    const { store, report } = await connected(vi.fn());
    const call = store
      .getState()
      .track("attachment_open", "assistant", () => new Promise<void>(() => undefined), "assistant");
    const settled = call.catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(callCeilingMs("attachment_open"));
    expect(await settled).toBeInstanceOf(WaitExceeded);
    expect(store.getState().waits).toEqual([]);
    expect(store.getState().messages).toMatchObject([{ class: "failed", place: "assistant" }]);
    expect(report.mock.calls.at(-1)?.[0]?.data).toMatchObject({
      stage: "call",
      code: "attachment_open",
    });
  });
});

describe("a feature's first state", () => {
  it("is a wait while the feature says it has none, closed when it arrives", async () => {
    const { store, accept } = await connected(vi.fn(), { unknown: ["assistant"] });
    expect(hasNoStateYet(store.getState().features, "assistant")).toBe(true);
    expect(inFlight(store.getState().waits, { feature: "assistant" })).toMatchObject([
      { kind: "first_state" },
    ]);
    accept(frame("assistant", 2));
    expect(hasNoStateYet(store.getState().features, "assistant")).toBe(false);
    expect(store.getState().waits).toEqual([]);
  });

  it("past its ceiling is said as a failure, and withdrawn when the state arrives", async () => {
    const { store, accept, report } = await connected(vi.fn(), { unknown: ["permissions"] });
    await vi.advanceTimersByTimeAsync(FIRST_STATE_CEILING_MS);
    expect(store.getState().waits).toEqual([]);
    const [failure] = store.getState().messages;
    expect(failure).toMatchObject({ class: "failed" });
    expect(failure?.place).toBeUndefined();
    expect(failure?.text).toBe(
      "No se pudo leer los permisos de este Mac en 75 s. Cierra Orbit y vuelve a abrirlo.",
    );
    expect(report.mock.calls.at(-1)?.[0]?.data).toMatchObject({
      stage: "first_state",
      code: "permissions",
    });
    accept(frame("permissions", 2));
    expect(store.getState().messages).toEqual([]);
  });

  it("only the main window says it", async () => {
    const { store } = await connected(vi.fn(), { unknown: ["assistant"], mainWindow: false });
    expect(store.getState().waits).toEqual([]);
  });
});

describe("a working message the daemon never closed", () => {
  it("goes past its ceiling, and the log says how long it stayed", async () => {
    const { store, accept, report } = await connected(vi.fn());
    const working = { code: "voice_transcribing", op: "voice-turn-0001", sequence: 1 } as Notice;
    const status = frame("status", 2);
    accept({ ...status, view: { ...status.view, notices: [working] } } as FeatureFrame);
    accept({
      ...status,
      revision: { epoch: 1, counter: 3 },
      view: { ...status.view, notices: [working] },
    } as FeatureFrame);
    expect(store.getState().messages).toMatchObject([{ class: "working" }]);
    const ceiling = WORKING_CEILING_MS.voice_transcribing ?? 0;
    await vi.advanceTimersByTimeAsync(ceiling);
    expect(store.getState().messages).toEqual([]);
    expect(report.mock.calls.at(-1)?.[0]).toMatchObject({
      code: "ui_wait_exceeded",
      data: { stage: "working", code: "voice_transcribing", timeout_ms: ceiling },
    });
  });
});

it("a connection whose frames never subscribe says it could not connect", async () => {
  const port: DaemonPort = {
    subscribeFrames: () => new Promise(() => undefined),
    snapshot: async () => bootstrap(),
    send: vi.fn(),
    report: vi.fn(async () => undefined),
  };
  const store = createOrbitStore(port);
  void store.getState().connect();
  await vi.advanceTimersByTimeAsync(CONNECT_CEILING_MS);
  expect(store.getState().error).not.toBeNull();
});

it("a revision is reached by a later counter or a later epoch, never by an earlier one", () => {
  expect(reached(undefined, { epoch: 1, counter: 1 })).toBe(false);
  expect(reached({ epoch: 1, counter: 3 }, { epoch: 1, counter: 3 })).toBe(true);
  expect(reached({ epoch: 1, counter: 2 }, { epoch: 1, counter: 3 })).toBe(false);
  expect(reached({ epoch: 2, counter: 0 }, { epoch: 1, counter: 9 })).toBe(true);
  expect(reached({ epoch: 1, counter: 9 }, { epoch: 2, counter: 0 })).toBe(false);
});
