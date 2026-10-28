import { getCopy } from "../i18n";
import type { FeatureId, FrameRevision } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import { newOp, type Context, type ErrorOrigin } from "../app/interceptor";

import {
  callCeilingMs,
  FIRST_STATE_CEILING_MS,
  FIRST_STATE_FEATURES,
  WORKING_CEILING_MS,
  type DirectCall,
} from "./ceilings";
import { intercept } from "./daemon-watchers";
import { hasNoStateYet, later, reached, stageOf, type Wait } from "./in-flight";
import type { Get, Set } from "./orbit-store";

/**
 * What a direct call rejects with past its ceiling. Its message is the
 * failure the screen says, with its numbers, for a view that shows it.
 */
export class WaitExceeded extends Error {
  constructor(
    readonly subject: string,
    readonly ceilingMs: number,
    op: string,
  ) {
    super(getCopy().extras.waits.exceeded(ceilingMs, op.slice(0, 6).toUpperCase()));
  }
}

interface Exceeding {
  /** Where its failure is said, when not where the wait began. */
  context?: Context;
  then?: () => void;
}

/**
 * The store's side of the registry: it opens and closes the entries of
 * `in-flight.ts`, holds one timer per entry for its ceiling, and keeps the
 * latest frame revision the screen has taken.
 */
export class WaitRegistry {
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly exceeding = new Map<string, Exceeding>();
  private seen: FrameRevision | undefined;

  constructor(
    private readonly port: DaemonPort,
    private readonly get: Get,
    private readonly set: Set,
  ) {}

  isOpen(op: string): boolean {
    return this.get().waits.some((wait) => wait.op === op);
  }

  /**
   * Opens `wait`. If its ceiling passes first, its failure is said with
   * `exceeding.context` (by default where it began) and `exceeding.then` runs.
   */
  open(wait: Omit<Wait, "startedMs">, exceeding?: Exceeding): void {
    this.set({ waits: [...this.get().waits, { ...wait, startedMs: Date.now() }] });
    if (exceeding) this.exceeding.set(wait.op, exceeding);
    this.timers.set(
      wait.op,
      setTimeout(() => this.exceed(wait.op), wait.ceilingMs),
    );
  }

  close(op: string): void {
    clearTimeout(this.timers.get(op));
    this.timers.delete(op);
    this.exceeding.delete(op);
    if (this.isOpen(op)) this.set({ waits: this.get().waits.filter((wait) => wait.op !== op) });
  }

  /** An order's receipt: closed now if the screen already shows it. */
  receipt(op: string, revision: FrameRevision | undefined): void {
    if (revision === undefined || reached(this.seen, revision)) {
      this.close(op);
      return;
    }
    this.set({
      waits: this.get().waits.map((wait) =>
        wait.op === op ? { ...wait, awaiting: revision } : wait,
      ),
    });
  }

  /** The screen took frames up to `revision`: every order it shows is closed. */
  observe(revision: FrameRevision): void {
    const seen = later(this.seen, revision);
    this.seen = seen;
    for (const wait of this.get().waits)
      if (wait.awaiting && reached(seen, wait.awaiting)) this.close(wait.op);
  }

  /** A direct call: open while it runs; past its ceiling it rejects. */
  readonly track = <T>(
    call: DirectCall,
    feature: FeatureId,
    work: () => Promise<T>,
    origin?: ErrorOrigin,
  ): Promise<T> => {
    const op = newOp();
    const ceilingMs = callCeilingMs(call);
    return new Promise<T>((resolve, reject) => {
      this.open(
        { op, kind: "call", feature, command: call, origin, ceilingMs },
        { then: () => reject(new WaitExceeded(call, ceilingMs, op)) },
      );
      const settle = (done: () => void) => {
        if (!this.isOpen(op)) return;
        this.close(op);
        done();
      };
      work().then(
        (value) => settle(() => resolve(value)),
        (error: unknown) => settle(() => reject(error)),
      );
    });
  };

  private exceed(op: string): void {
    const wait = this.get().waits.find((item) => item.op === op);
    const handler = this.exceeding.get(op);
    this.close(op);
    if (!wait) return;
    // Said where the wait began; a first state belongs to no view.
    const context: Context =
      handler?.context ??
      (wait.kind === "first_state" ? {} : { place: wait.origin, feature: wait.feature });
    const incoming = {
      kind: "exceeded" as const,
      op,
      stage: stageOf(wait),
      subject: wait.command ?? wait.feature,
      feature: wait.feature,
      elapsedMs: Date.now() - wait.startedMs,
      ceilingMs: wait.ceilingMs,
    };
    intercept(this.port, this.get, this.set, incoming, context);
    handler?.then?.();
  }
}

/**
 * The first state of each feature that says when it has none: one wait while
 * it has none; past its ceiling, one failure, withdrawn when it arrives. Only
 * the main window follows them, as it alone says the daemon's notices.
 */
export function firstStateFollower(registry: WaitRegistry, get: Get, mainWindow: boolean) {
  const followed = new Map<FeatureId, { op: string; failed: boolean }>();
  return () => {
    if (!mainWindow) return;
    const { features } = get();
    for (const feature of FIRST_STATE_FEATURES) {
      const entry = followed.get(feature);
      if (!hasNoStateYet(features, feature)) {
        if (!entry) continue;
        followed.delete(feature);
        registry.close(entry.op);
        if (entry.failed) get().dismiss(entry.op);
      } else if (!entry) {
        const op = newOp();
        followed.set(feature, { op, failed: false });
        registry.open(
          { op, kind: "first_state", feature, ceilingMs: FIRST_STATE_CEILING_MS },
          { then: () => followed.set(feature, { op, failed: true }) },
        );
      }
    }
  };
}

/**
 * A working message the daemon said, which only a later code of its op ends:
 * past its ceiling (`WORKING_CEILING_MS`) it goes, and the log says it stayed.
 */
export function workingFollower(port: DaemonPort, get: Get, set: Set) {
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const expire = (code: string, op: string, startedMs: number, ceilingMs: number) => {
    timers.delete(op);
    const shown = get().messages.some(
      (message) => message.op === op && message.class === "working",
    );
    if (!shown) return;
    const elapsedMs = Date.now() - startedMs;
    const incoming = { kind: "exceeded" as const, op, stage: "working" as const, subject: code };
    intercept(port, get, set, { ...incoming, feature: "status", elapsedMs, ceilingMs });
  };
  return (code: string, op: string | undefined) => {
    const ceilingMs = WORKING_CEILING_MS[code];
    if (ceilingMs === undefined || op === undefined || timers.has(op)) return;
    const startedMs = Date.now();
    timers.set(
      op,
      setTimeout(() => expire(code, op, startedMs, ceilingMs), ceilingMs),
    );
  };
}
