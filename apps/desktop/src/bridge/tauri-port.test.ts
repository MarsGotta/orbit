import { afterEach, describe, expect, it, vi } from "vitest";

// Orbit.app asks the main window to hand over what it holds before it quits
// (`src-tauri/src/interface_flush.rs`), and waits for its answer.

const listeners = new Map<string, () => void>();
const invoked: string[] = [];

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, listener: () => void) => {
    listeners.set(name, listener);
    return Promise.resolve(() => listeners.delete(name));
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string) => {
    invoked.push(command);
    return Promise.resolve();
  },
}));

afterEach(() => {
  listeners.clear();
  invoked.length = 0;
});

describe("the quit flush on Orbit.app", () => {
  it("answers Orbit.app only once the window has handed everything over", async () => {
    const { TauriPort } = await import("./tauri-port");
    let finish: () => void = () => undefined;
    const flush = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    const stop = await new TauriPort().onFlushBeforeQuit(flush);
    listeners.get("orbit://flush-before-quit")?.();
    await Promise.resolve();
    expect(flush).toHaveBeenCalledTimes(1);
    expect(invoked).toEqual([]);
    finish();
    await vi.waitFor(() => expect(invoked).toEqual(["orbit_interface_flushed"]));
    stop();
    expect(listeners.size).toBe(0);
  });

  it("answers even when a flush fails, so the quit is never held by it", async () => {
    const { TauriPort } = await import("./tauri-port");
    await new TauriPort().onFlushBeforeQuit(() => Promise.reject(new Error("roto")));
    listeners.get("orbit://flush-before-quit")?.();
    await vi.waitFor(() => expect(invoked).toEqual(["orbit_interface_flushed"]));
  });
});
