import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  AppBootstrap,
  AppCommand,
  FeatureFrame,
  FeatureId,
  FrameRevision,
} from "../bridge/generated";
import type { DaemonPort, Unsubscribe } from "../bridge/port";
import { refusalOf } from "../bridge/refusal";
import { daemonWatchers, intercept } from "./daemon-watchers";
import {
  newOp,
  refusalReport,
  uiReport,
  type CrashCode,
  type ErrorOrigin,
  type Message,
} from "../app/interceptor";
import { applyFrame, newer } from "./feature-frame";
import { composeView, type AppView } from "./app-view";
import { CONNECT_CEILING_MS } from "./ceilings";
import type { Wait } from "./in-flight";
import { sender } from "./sender";
import { firstStateFollower, WaitRegistry, workingFollower } from "./wait-registry";

export type { ErrorOrigin } from "../app/interceptor";

export interface OrbitState {
  snapshot: AppView | null;
  features: Partial<Record<FeatureId, FeatureFrame>>;
  /** What the interceptor shows: at most one message per op. */
  messages: Message[];
  /** What is in flight, oldest first (`in-flight.ts` reads it). */
  waits: Wait[];
  error: string | null;
  connect: () => Promise<Unsubscribe>;
  /**
   * Sends an order; `true` once it was accepted, `false` when it was refused
   * or went past its ceiling (its message already says which).
   */
  send: (command: AppCommand, origin?: ErrorOrigin) => Promise<boolean>;
  /** A call to the app outside `send`, followed by the registry (`WaitRegistry.track`). */
  track: WaitRegistry["track"];
  /** A view that failed to paint or an uncaught error; returns its words. */
  crash: (code: CrashCode) => string;
  dismiss: (op: string) => void;
  /** «Reintentar»: sends a failure's order again from where it was sent, in its place. */
  retry: (op: string) => Promise<void>;
}

export interface StoreOptions {
  /** Whether the daemon's notices are shown here: the main window's, not the panel's. */
  notices: boolean;
}

export type OrbitStore = ReturnType<typeof createOrbitStore>;
type Revisions = Partial<Record<FeatureId, FrameRevision>>;
export type Get = StoreApi<OrbitState>["getState"];
export type Set = StoreApi<OrbitState>["setState"];

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Rejects once `ms` pass, unless `promise` settles first. */
function within<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const ceiling = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} did not settle within ${ms} ms`)), ms);
  });
  return Promise.race([promise, ceiling]).finally(() => clearTimeout(timer));
}

async function connectPort(
  port: DaemonPort,
  acceptFrame: (frame: FeatureFrame) => void,
  acceptBootstrap: (bootstrap: AppBootstrap) => void,
  onError: (message: string) => void,
): Promise<Unsubscribe> {
  const pending: FeatureFrame[] = [];
  let ready = false;
  let closed = false;
  let unsubscribe: Unsubscribe | undefined;
  try {
    // Both are answered by the app from what it holds: past one call's
    // bound the window says it could not connect (`CONNECT_CEILING_MS`).
    const subscribing = port.subscribeFrames((frame) => {
      if (closed) return;
      if (ready) acceptFrame(frame);
      else pending.push(frame);
    });
    // A subscription that arrives after the window gave up is let go.
    void subscribing.then(
      (stop) => {
        if (closed) stop();
      },
      () => undefined,
    );
    unsubscribe = await within(subscribing, CONNECT_CEILING_MS, "subscribing to frames");
    acceptBootstrap(await within(port.snapshot(), CONNECT_CEILING_MS, "reading the first view"));
    ready = true;
    for (const frame of pending) acceptFrame(frame);
    return unsubscribe;
  } catch (error: unknown) {
    closed = true;
    unsubscribe?.();
    // The screen says what to do; the log keeps which failure it was.
    const refusal = refusalOf(error);
    const op = newOp();
    const line =
      typeof error === "object" && error !== null && "refusal" in error
        ? refusalReport(op, refusal)
        : uiReport(op, "connect_failed");
    void port.report(line).catch(() => undefined);
    onError(errorMessage(error));
    return () => undefined;
  }
}

function acceptFeatureFrame(frame: FeatureFrame, revisions: Revisions, get: Get, set: Set) {
  if (!newer(frame.revision, revisions[frame.feature])) return false;
  const current = get().snapshot;
  if (!current) return false;
  revisions[frame.feature] = frame.revision;
  set({
    snapshot: applyFrame(current, frame),
    features: { ...get().features, [frame.feature]: frame },
    error: null,
  });
  return true;
}

function bootstrapper(revisions: Revisions, get: Get, set: Set) {
  return (bootstrap: AppBootstrap): FeatureFrame[] => {
    const features: Partial<Record<FeatureId, FeatureFrame>> = { ...get().features };
    for (const frame of bootstrap.frames) {
      if (newer(frame.revision, revisions[frame.feature])) {
        revisions[frame.feature] = frame.revision;
        features[frame.feature] = frame;
      }
    }
    const frames = Object.values(features);
    set({
      snapshot: composeView(frames, bootstrap.revision),
      features,
      error: null,
    });
    return frames;
  };
}

export function createOrbitStore(port: DaemonPort, options: StoreOptions = { notices: true }) {
  return createStore<OrbitState>((set, get) => {
    const revisions: Revisions = {};
    const waits = new WaitRegistry(port, get, set);
    const followFirstStates = firstStateFollower(waits, get, options.notices);
    const working = workingFollower(port, get, set);
    const { notices, failures } = daemonWatchers(port, get, set, options, working);
    const acceptFrame = (frame: FeatureFrame) => {
      const accepted = acceptFeatureFrame(frame, revisions, get, set);
      // A frame not newer for its feature was already shown: still taken.
      waits.observe(frame.revision);
      if (!accepted) return;
      notices([frame]);
      failures();
      followFirstStates();
    };
    const bootstrap = bootstrapper(revisions, get, set);
    const send = sender(port, get, set, waits);
    return {
      snapshot: null,
      features: {},
      messages: [],
      waits: [],
      error: null,
      connect: () =>
        connectPort(
          port,
          acceptFrame,
          (snapshot) => {
            notices(bootstrap(snapshot));
            waits.observe(snapshot.revision);
            failures();
            followFirstStates();
          },
          (message) => set({ error: message }),
        ),
      send,
      track: waits.track,
      crash: (code) => {
        const op = newOp();
        const place = code === "ui_render_failed" ? "boundary" : undefined;
        const messages = intercept(port, get, set, { kind: "crash", op, code }, { place });
        return messages.find((message) => message.op === op)?.text ?? "";
      },
      dismiss: (op) => set({ messages: get().messages.filter((message) => message.op !== op) }),
      retry: async (op) => {
        const failed = get().messages.find((message) => message.op === op);
        const command = failed?.failure?.retry;
        if (!failed || !command) return;
        set({ messages: get().messages.filter((message) => message.op !== op) });
        await send(command, failed.place === "boundary" ? undefined : failed.place);
      },
    };
  });
}
