import { afterEach, describe, expect, it, vi } from "vitest";
import type { PluginCall } from "orbit-ui/bridge/generated/PluginCall";
import type { PluginOutcome } from "orbit-ui/bridge/generated/PluginOutcome";
import type { DaemonPort } from "orbit-ui/bridge/port";

// A plugin call from the interface: Orbit.app's `orbit_plugin` order, with the
// three outcomes the daemon gives, each passed on as it came.

const sent: { command: string; args: unknown }[] = [];
let reply: () => Promise<unknown> = () => Promise.resolve(undefined);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    sent.push({ command, args });
    return reply();
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));

afterEach(() => {
  sent.length = 0;
  reply = () => Promise.resolve(undefined);
});

const call = {
  name: "list_distillations",
  input: { meeting_id: null, page: 1 },
} satisfies PluginCall;

const outcomes = {
  answered: { outcome: "answered", value: { entries: [], next_page: null } },
  refused: {
    outcome: "refused",
    status: 404,
    body: {
      code: "plugin_call_unknown",
      message: "The plugin has no such call.",
      op: "op_invented",
      refusal: { code: "plugin_call_unknown", retry: { kind: "after_fix" } },
    },
  },
  failed: {
    outcome: "failed",
    refusal: {
      code: "plugin_call_too_large",
      bytes: 1_048_577,
      max_bytes: 1_048_576,
      retry: { kind: "after_fix" },
    },
  },
} satisfies Record<PluginOutcome["outcome"], PluginOutcome>;

describe("a plugin call on Orbit.app", () => {
  it("sends the plugin's id and the call, once, as orbit_plugin", async () => {
    const { TauriPort } = await import("./tauri-port");
    const port: DaemonPort = new TauriPort();
    reply = () => Promise.resolve(outcomes.answered);
    await port.plugin?.("vault", call);
    expect(sent).toEqual([{ command: "orbit_plugin", args: { plugin: "vault", call } }]);
  });

  it.each(Object.values(outcomes))("passes the $outcome outcome on whole", async (outcome) => {
    const { TauriPort } = await import("./tauri-port");
    reply = () => Promise.resolve(structuredClone(outcome));
    await expect(new TauriPort().plugin("vault", call)).resolves.toEqual(outcome);
  });

  it("leaves a failure of the local channel a rejection, with its refusal", async () => {
    const { TauriPort } = await import("./tauri-port");
    const error = {
      code: "daemon_error",
      message: "daemon_error: orbit_closing",
      refusal: { code: "internal", retry: { kind: "never" } },
    };
    reply = () => Promise.reject(error);
    await expect(new TauriPort().plugin("vault", call)).rejects.toEqual(error);
  });
});
