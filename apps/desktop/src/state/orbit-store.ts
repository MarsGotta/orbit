import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  AppBootstrap,
  AppCommand,
  FeatureFrame,
  FeatureId,
  FrameRevision,
} from "../bridge/generated";
import type { DaemonPort, Unsubscribe } from "../bridge/port";
import { commandError, refusalOf } from "../bridge/refusal";
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
import { commandFeature } from "../bridge/generated/commandFeatures";

export type { ErrorOrigin } from "../app/interceptor";

export interface OrbitState {
  snapshot: AppView | null;
  features: Partial<Record<FeatureId, FeatureFrame>>;
  /** What the interceptor shows: at most one message per op. */
  messages: Message[];
  error: string | null;
  connect: () => Promise<Unsubscribe>;
  send: (command: AppCommand, origin?: ErrorOrigin) => Promise<void>;
  /** A view that failed to paint or an uncaught error; returns its words. */
  crash: (code: CrashCode) => string;
  dismiss: (op: string) => void;
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

async function connectPort(
  port: DaemonPort,
  acceptFrame: (frame: FeatureFrame) => void,
  acceptBootstrap: (bootstrap: AppBootstrap) => void,
  onError: (message: string) => void,
): Promise<Unsubscribe> {
  const pending: FeatureFrame[] = [];
  let ready = false;
  let unsubscribe: Unsubscribe | undefined;
  try {
    unsubscribe = await port.subscribeFrames((frame) => {
      if (ready) acceptFrame(frame);
      else pending.push(frame);
    });
    acceptBootstrap(await port.snapshot());
    ready = true;
    for (const frame of pending) acceptFrame(frame);
    return unsubscribe;
  } catch (error: unknown) {
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

function sender(port: DaemonPort, get: Get, set: Set): OrbitState["send"] {
  return async (command, origin) => {
    const feature = commandFeature[command.command];
    const op = newOp();
    try {
      await port.send(command, op);
    } catch (error: unknown) {
      const { refusal } = commandError(error, command);
      const context = { place: origin, feature, command };
      // One refusal per function, as before: the newest replaces the last.
      set({ messages: get().messages.filter((message) => message.feature !== feature) });
      intercept(port, get, set, { kind: "refusal", op, refusal }, context);
      return;
    }
    // An accepted order clears its function; rearm also lifts privacy refusals elsewhere.
    set({
      messages: get().messages.filter(
        (message) =>
          message.feature !== feature &&
          !(command.command === "rearm" && message.refusalCode === "privacy_stopped"),
      ),
    });
  };
}

export function createOrbitStore(port: DaemonPort, options: StoreOptions = { notices: true }) {
  return createStore<OrbitState>((set, get) => {
    const revisions: Revisions = {};
    const { notices, failures } = daemonWatchers(port, get, set, options);
    const acceptFrame = (frame: FeatureFrame) => {
      if (!acceptFeatureFrame(frame, revisions, get, set)) return;
      notices([frame]);
      failures();
    };
    const bootstrap = bootstrapper(revisions, get, set);
    return {
      snapshot: null,
      features: {},
      messages: [],
      error: null,
      connect: () =>
        connectPort(
          port,
          acceptFrame,
          (snapshot) => {
            notices(bootstrap(snapshot));
            failures();
          },
          (message) => set({ error: message }),
        ),
      send: sender(port, get, set),
      crash: (code) => {
        const op = newOp();
        const place = code === "ui_render_failed" ? "boundary" : undefined;
        const messages = intercept(port, get, set, { kind: "crash", op, code }, { place });
        return messages.find((message) => message.op === op)?.text ?? "";
      },
      dismiss: (op) => set({ messages: get().messages.filter((message) => message.op !== op) }),
    };
  });
}
