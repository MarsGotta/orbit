import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import type { AppBootstrap, FeatureFrame, FeatureId } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import type { SessionPage } from "../bridge/reads";
import { reportingReads } from "../app/report-failure";
import { createOrbitStore, type OrbitStore } from "./orbit-store";
import { OrbitStoreProvider, useInFlight, useNoStateYet, useTrack } from "./use-in-flight";

// The hooks the loading states are drawn from, over the window's store.

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

async function connected(send: DaemonPort["send"], extra: Partial<DaemonPort> = {}) {
  const port: DaemonPort = {
    subscribeFrames: async () => () => undefined,
    snapshot: async (): Promise<AppBootstrap> => ({
      revision: { epoch: 1, counter: 1 },
      frames: FEATURES.map(
        (feature) =>
          ({
            feature,
            revision: { epoch: 1, counter: 1 },
            view:
              feature === "assistant"
                ? { availability: { state: "unknown", cause: "topic_connecting" } }
                : feature === "notes" || feature === "control"
                  ? null
                  : { availability: { state: "current" }, settings: {}, notices: [] },
          }) as unknown as FeatureFrame,
      ),
    }),
    send,
    report: vi.fn(async () => undefined),
    ...extra,
  };
  const store = createOrbitStore(port, { notices: false });
  await store.getState().connect();
  return { store, port };
}

function within(store: OrbitStore) {
  return ({ children }: { children: ReactNode }) => (
    <OrbitStoreProvider store={store}>{children}</OrbitStoreProvider>
  );
}

describe("what is in flight, as hooks", () => {
  it("an order is in flight by its command, its feature and its view, and overall", async () => {
    let answer: () => void = () => undefined;
    const { store } = await connected(
      () =>
        new Promise((resolve) => {
          answer = () => resolve({ revision: { epoch: 1, counter: 1 } });
        }),
    );
    const { result } = renderHook(
      () => ({
        command: useInFlight({ command: "update_check" }),
        feature: useInFlight({ feature: "local" }),
        origin: useInFlight({ origin: "settings" }),
        other: useInFlight({ origin: "calendar" }),
        overall: useInFlight(),
      }),
      { wrapper: within(store) },
    );
    expect(result.current.overall).toBe(false);
    void store.getState().send({ command: "update_check" }, "settings");
    await waitFor(() =>
      expect(result.current).toEqual({
        command: true,
        feature: true,
        origin: true,
        other: false,
        overall: true,
      }),
    );
    answer();
    await waitFor(() => expect(result.current.overall).toBe(false));
  });

  it("a feature without a state yet is one to draw a skeleton for", async () => {
    const { store } = await connected(vi.fn());
    const { result } = renderHook(
      () => ({ assistant: useNoStateYet("assistant"), status: useNoStateYet("status") }),
      { wrapper: within(store) },
    );
    expect(result.current).toEqual({ assistant: true, status: false });
  });

  it("a call through `useTrack` is in flight until it settles; alone, it just runs", async () => {
    const { store } = await connected(vi.fn());
    const { result } = renderHook(() => ({ track: useTrack(), busy: useInFlight() }), {
      wrapper: within(store),
    });
    let finish: () => void = () => undefined;
    const call = result.current.track(
      "attachment_add",
      "assistant",
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    await waitFor(() => expect(result.current.busy).toBe(true));
    finish();
    await call;
    await waitFor(() => expect(result.current.busy).toBe(false));
    const alone = renderHook(() => useTrack());
    await expect(alone.result.current("agent", "assistant", async () => "hecho")).resolves.toBe(
      "hecho",
    );
  });

  it("the assistant's reads go through the registry", async () => {
    let answer: (page: SessionPage) => void = () => undefined;
    const readSessionsPage = vi.fn(
      () =>
        new Promise<SessionPage>((resolve) => {
          answer = resolve;
        }),
    );
    const { store, port } = await connected(vi.fn(), { readSessionsPage });
    const reads = reportingReads(port, store.getState().track);
    const reading = reads.readSessionsPage?.(null);
    expect(store.getState().waits).toMatchObject([
      { kind: "call", command: "sessions_page", feature: "assistant", origin: "assistant" },
    ]);
    answer({ state: "unknown", why: "not_observed_yet" });
    await reading;
    expect(store.getState().waits).toEqual([]);
  });
});
